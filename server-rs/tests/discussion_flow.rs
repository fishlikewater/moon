//! 讨论端到端测试：自然语言发起、协议剥离、轮次推进、停止与失败可见。

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use common::{
    builtin_moderator_id, create_agent, enable_reuse, get, post, setup_with_adapter,
    wait_for_system_message,
};
use futures_util::StreamExt;
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::discussion::{DiscussionStatus, DiscussionStopReason};
use moon_server::domain::message::MessageSenderType;
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::error::AppError;
use moon_server::store::discussions::{CreateDiscussionInput, FinishDiscussionInput};
use moon_server::store::messages::AppendMessageInput;
use moon_server::store::{DiscussionStore, MessageStore};
use moon_server::{build_app, AppState};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, Copy, Default)]
struct Ids {
    a: i64,
    b: i64,
    moderator: i64,
}

type TurnFn = Arc<dyn Fn(i64) -> String + Send + Sync>;
type JudgeFn = Arc<dyn Fn(i64, &str) -> String + Send + Sync>;

struct Behavior {
    ids: Mutex<Ids>,
    topic: Mutex<String>,
    include_max_rounds: Mutex<bool>,
    start_max_rounds: Mutex<Option<i64>>,
    fail_a: bool,
    b_failures: Mutex<usize>,
    judge_failures: Mutex<usize>,
    delay_a_ms: u64,
    delay_judge_ms: u64,
    a_turn: TurnFn,
    b_turn: TurnFn,
    judge: JudgeFn,
    runs: Mutex<Vec<(i64, String)>>,
}

impl Behavior {
    fn new(a_turn: TurnFn, b_turn: TurnFn, judge: JudgeFn) -> Self {
        Self {
            ids: Mutex::new(Ids::default()),
            topic: Mutex::new("是否采用方案 A".to_string()),
            include_max_rounds: Mutex::new(true),
            start_max_rounds: Mutex::new(Some(3)),
            fail_a: false,
            b_failures: Mutex::new(0),
            judge_failures: Mutex::new(0),
            delay_a_ms: 0,
            delay_judge_ms: 0,
            a_turn,
            b_turn,
            judge,
            runs: Mutex::new(Vec::new()),
        }
    }

    fn set_ids(&self, a: i64, b: i64, moderator: i64) {
        *self.ids.lock().unwrap() = Ids { a, b, moderator };
    }

    fn with_delay_a(mut self, delay_a_ms: u64) -> Self {
        self.delay_a_ms = delay_a_ms;
        self
    }
}

struct DiscussionAdapter {
    agent_id: i64,
    behavior: Arc<Behavior>,
}

fn prompt_round(prompt: &str) -> i64 {
    prompt
        .split("当前轮次：第 ")
        .nth(1)
        .and_then(|rest| rest.split(" /").next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(1)
}

impl HarnessAdapter for DiscussionAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        let behavior = Arc::clone(&self.behavior);
        let agent_id = self.agent_id;
        let session = futures_util::stream::iter(vec![Ok(RunEvent::Session(format!(
            "sess-{agent_id}"
        )))]);
        let rest = futures_util::stream::once(async move {
            let prompt = input.prompt.clone();
            behavior
                .runs
                .lock()
                .unwrap()
                .push((agent_id, prompt.clone()));
            let ids = *behavior.ids.lock().unwrap();
            let round = prompt_round(&prompt);

            if agent_id == ids.moderator {
                if prompt.contains("moon-discussion-judge") {
                    {
                        let mut left = behavior.judge_failures.lock().unwrap();
                        if *left > 0 {
                            *left -= 1;
                            return Err(AppError::harness_run_failed("模拟裁决故障"));
                        }
                    }
                    if behavior.delay_judge_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(behavior.delay_judge_ms)).await;
                    }
                    return Ok(RunEvent::Final((behavior.judge)(round, &prompt)));
                }
                if prompt.contains("moon-discussion") {
                    let mut payload = json!({
                        "topic": behavior.topic.lock().unwrap().clone(),
                        "participantIds": [ids.a, ids.b],
                    });
                    if *behavior.include_max_rounds.lock().unwrap() {
                        if let Some(rounds) = *behavior.start_max_rounds.lock().unwrap() {
                            payload["maxRounds"] = json!(rounds);
                        }
                    }
                    return Ok(RunEvent::Final(format!(
                        "我来组织一次讨论。\n```moon-discussion\n{}\n```",
                        payload
                    )));
                }
                return Ok(RunEvent::Final("主持人普通回复".to_string()));
            }

            if agent_id == ids.a {
                if behavior.delay_a_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(behavior.delay_a_ms)).await;
                }
                if behavior.fail_a {
                    return Err(AppError::harness_run_failed("模拟讨论成员故障"));
                }
                return Ok(RunEvent::Final((behavior.a_turn)(round)));
            }
            if agent_id == ids.b {
                {
                    let mut left = behavior.b_failures.lock().unwrap();
                    if *left > 0 {
                        *left -= 1;
                        return Err(AppError::harness_run_failed("模拟讨论成员故障"));
                    }
                }
                return Ok(RunEvent::Final((behavior.b_turn)(round)));
            }
            Err(AppError::harness_run_failed("意外的智能体运行"))
        });
        Box::pin(session.chain(rest))
    }
}

fn factory(behavior: Arc<Behavior>) -> AdapterFactory {
    Arc::new(move |agent: &Agent| {
        Ok(Arc::new(DiscussionAdapter {
            agent_id: agent.id,
            behavior: Arc::clone(&behavior),
        }) as Arc<dyn HarnessAdapter>)
    })
}

fn turn_text(stance: &str, reply_to: Option<i64>, stop_vote: &str, reason: &str) -> String {
    let mut payload = json!({
        "stance": stance,
        "stopVote": stop_vote,
        "stopReason": reason,
    });
    if let Some(reply_to) = reply_to {
        payload["replyToAgentId"] = json!(reply_to);
    }
    format!(
        "这是我的讨论发言。\n```moon-discussion-turn\n{}\n```",
        payload
    )
}

fn judge_text(decision: &str, summary: &str) -> String {
    format!(
        "主持人裁决。\n```moon-discussion-judge\n{}\n```",
        json!({
            "decision": decision,
            "reason": if decision == "stop" { "已收敛" } else { "仍需讨论" },
            "summary": summary,
        })
    )
}

async fn create_discussion_room(
    app: &common::TestApp,
    member_ids: &[i64],
    discussion_max_rounds: i64,
) -> i64 {
    let response = post(
        app,
        "/api/rooms",
        json!({
            "name": "讨论测试房间",
            "workspaceDir": app.dir.path().to_string_lossy(),
            "memberIds": member_ids,
            "replyMode": "moderator",
            "discussionMaxRounds": discussion_max_rounds,
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body["id"].as_i64().unwrap()
}

async fn wait_discussion(
    app: &common::TestApp,
    room_id: i64,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..400 {
        let response = get(app, &format!("/api/rooms/{room_id}/discussions/latest")).await;
        assert_eq!(response.status, StatusCode::OK);
        if !response.body.is_null() && predicate(&response.body) {
            return response.body;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let latest = get(app, &format!("/api/rooms/{room_id}/discussions/latest")).await;
    let messages = room_messages(app, room_id).await;
    panic!(
        "等待讨论状态超时：latest={:?} messages={:?}",
        latest.body,
        messages
            .iter()
            .map(|message| (
                message["senderType"].clone(),
                message["content"].clone(),
                message["discussion"].clone()
            ))
            .collect::<Vec<_>>()
    );
}

async fn room_messages(app: &common::TestApp, room_id: i64) -> Vec<Value> {
    get(app, &format!("/api/rooms/{room_id}/messages"))
        .await
        .body
        .as_array()
        .unwrap()
        .clone()
}

async fn start_server(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, build_app(state)).await;
    });
    (format!("127.0.0.1:{}", address.port()), handle)
}

#[tokio::test]
async fn starts_from_chat_strips_protocol_and_completes_on_all_stop() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "方案足够")),
        Arc::new(|_| turn_text("oppose", None, "stop", "已无新信息")),
        Arc::new(|_, _| judge_text("stop", "采用方案 A，先做小流量验证。")),
    ));
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "方案作者").await;
    let second = create_agent(&app, "评审员").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, b_id, moderator_id);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 5).await;
    // 讨论成员与裁决使用 skip_room_context，房间闲聊不得双注入。
    MessageStore::new(app.state.db.clone())
        .append(AppendMessageInput::final_message(
            room_id,
            MessageSenderType::User,
            None,
            "房间闲聊不应进入讨论",
        ))
        .await
        .unwrap();

    let (address, server) = start_server(app.state.clone()).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/ws?roomId={room_id}"
    ))
    .await
    .expect("connect ws");

    let accepted = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请你们讨论一下是否采用方案 A" }),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);

    let discussion = wait_discussion(&app, room_id, |value| value["status"] == "completed").await;
    assert_eq!(discussion["topic"], "是否采用方案 A");
    assert_eq!(discussion["status"], "completed");
    assert_eq!(discussion["stopReason"], "all_voted_stop");
    assert_eq!(discussion["currentRound"], 1);
    assert_eq!(discussion["maxRounds"], 3);
    assert_eq!(discussion["participantIds"], json!([a_id, b_id]));
    assert_eq!(discussion["summary"], "采用方案 A，先做小流量验证。");

    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    assert_eq!(runs, vec![moderator_id, a_id, b_id, moderator_id]);
    let prompts = behavior.runs.lock().unwrap().clone();
    assert!(prompts[0].1.contains("moon-discussion"));
    assert!(prompts[1].1.contains("moon-discussion-turn"));
    assert!(prompts[1].1.contains("是否采用方案 A"));
    assert!(prompts[3].1.contains("moon-discussion-judge"));
    for (index, (_, prompt)) in prompts.iter().enumerate().skip(1) {
        assert!(
            !prompt.contains("房间闲聊不应进入讨论"),
            "讨论第 {index} 次运行不应注入房间历史：{prompt}"
        );
    }

    let messages = room_messages(&app, room_id).await;
    let serialized = serde_json::to_string(&messages).unwrap();
    assert!(!serialized.contains("moon-discussion"));
    let turns: Vec<&Value> = messages
        .iter()
        .filter(|message| {
            message["senderType"] == "agent" && !message["discussion"]["stance"].is_null()
        })
        .collect();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["discussion"]["discussionId"], discussion["id"]);
    assert_eq!(turns[0]["discussion"]["stance"], "propose");
    assert_eq!(turns[0]["discussion"]["stopVote"], "stop");
    assert_eq!(turns[1]["discussion"]["stance"], "oppose");
    assert_eq!(turns[1]["discussion"]["stopVote"], "stop");
    assert!(messages.iter().any(|message| {
        message["senderType"] == "system"
            && message["discussion"]["discussionId"] == discussion["id"]
            && message["content"]
                .as_str()
                .is_some_and(|content| content.starts_with("讨论已开始"))
    }));
    assert!(messages.iter().any(|message| {
        message["senderType"] == "agent"
            && message["senderAgentId"] == moderator_id
            && message["discussion"]["discussionId"] == discussion["id"]
            && message["discussion"]["stance"].is_null()
    }));

    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let incoming =
            tokio::time::timeout(Duration::from_millis(200), socket.next()).await;
        let Some(Ok(Message::Text(text))) = incoming.ok().flatten() else {
            continue;
        };
        events.push(serde_json::from_str::<Value>(text.as_str()).unwrap());
    }
    assert!(events.iter().any(|event| {
        event["type"] == "discussion_changed" && event["discussion"]["status"] == "running"
    }));
    assert!(events.iter().any(|event| {
        event["type"] == "discussion_changed"
            && event["discussion"]["status"] == "completed"
            && event["discussion"]["stopReason"] == "all_voted_stop"
    }));
    server.abort();
}

#[tokio::test]
async fn omitted_rounds_use_room_ceiling_and_default() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "完成")),
        Arc::new(|_| turn_text("support", None, "stop", "同意")),
        Arc::new(|_, _| judge_text("stop", "达成共识。")),
    ));
    *behavior.include_max_rounds.lock().unwrap() = false;
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "低上限甲").await;
    let second = create_agent(&app, "低上限乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    behavior.set_ids(a_id, b_id, builtin_moderator_id(&app).await);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 2).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let discussion = wait_discussion(&app, room_id, |value| value["status"] == "completed").await;
    assert_eq!(discussion["maxRounds"], 2);
    assert_eq!(discussion["currentRound"], 1);

    // 上限更高时省略轮数取默认 3。
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "完成")),
        Arc::new(|_| turn_text("support", None, "stop", "同意")),
        Arc::new(|_, _| judge_text("stop", "达成共识。")),
    ));
    *behavior.include_max_rounds.lock().unwrap() = false;
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "默认甲").await;
    let second = create_agent(&app, "默认乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    behavior.set_ids(a_id, b_id, builtin_moderator_id(&app).await);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 5).await;
    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let discussion = wait_discussion(&app, room_id, |value| value["status"] == "completed").await;
    assert_eq!(discussion["maxRounds"], 3);
}

#[tokio::test]
async fn stop_endpoint_halts_remaining_participants_and_is_idempotent() {
    let behavior = Arc::new(
        Behavior::new(
            Arc::new(|_| turn_text("propose", None, "continue", "")),
            Arc::new(|_| turn_text("support", None, "continue", "")),
            Arc::new(|_, _| judge_text("continue", "")),
        )
        .with_delay_a(300),
    );
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "被停止甲").await;
    let second = create_agent(&app, "被停止乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    behavior.set_ids(a_id, b_id, builtin_moderator_id(&app).await);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 5).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let running = wait_discussion(&app, room_id, |value| value["status"] == "running").await;
    let discussion_id = running["id"].as_i64().unwrap();
    // 确保第一个参与者已经在运行，再停止，验证后续参与者不再启动。
    for _ in 0..200 {
        if behavior
            .runs
            .lock()
            .unwrap()
            .iter()
            .any(|(agent_id, _)| *agent_id == a_id)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let stopped = post(
        &app,
        &format!("/api/rooms/{room_id}/discussions/{discussion_id}/stop"),
        json!({}),
    )
    .await;
    assert_eq!(stopped.status, StatusCode::OK, "{:?}", stopped.body);
    assert_eq!(stopped.body["status"], "stopped");
    assert_eq!(stopped.body["stopReason"], "user_stop");

    let again = post(
        &app,
        &format!("/api/rooms/{room_id}/discussions/{discussion_id}/stop"),
        json!({}),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body["status"], "stopped");

    tokio::time::sleep(Duration::from_millis(500)).await;
    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    let moderator_id = builtin_moderator_id(&app).await;
    assert_eq!(runs, vec![moderator_id, a_id], "停止后不应再运行其他参与者");
    let messages = room_messages(&app, room_id).await;
    assert!(messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|content| content.starts_with("讨论已停止"))
    }));
}

#[tokio::test]
async fn rejects_rounds_above_room_ceiling_with_visible_message() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "")),
        Arc::new(|_| turn_text("support", None, "stop", "")),
        Arc::new(|_, _| judge_text("stop", "完成")),
    ));
    *behavior.start_max_rounds.lock().unwrap() = Some(3);
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "超限甲").await;
    let second = create_agent(&app, "超限乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    behavior.set_ids(a_id, b_id, builtin_moderator_id(&app).await);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 2).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;

    let mut found = false;
    for _ in 0..200 {
        let messages = room_messages(&app, room_id).await;
        found = messages.iter().any(|message| {
            message["content"].as_str().is_some_and(|content| {
                content.contains("主持人发起讨论失败")
                    && content.contains("讨论轮次必须是 1..2 的整数")
            })
        });
        if found {
            break;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    assert!(found, "应留下可见的讨论创建失败提示");

    let latest = get(&app, &format!("/api/rooms/{room_id}/discussions/latest")).await;
    assert!(latest.body.is_null());
    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    assert_eq!(runs, vec![builtin_moderator_id(&app).await]);
}

#[tokio::test]
async fn participant_failure_fails_discussion_without_judge() {
    let behavior = Arc::new(Behavior {
        fail_a: true,
        ..Behavior::new(
            Arc::new(|_| turn_text("propose", None, "continue", "")),
            Arc::new(|_| turn_text("support", None, "continue", "")),
            Arc::new(|_, _| judge_text("continue", "")),
        )
    });
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "故障甲").await;
    let second = create_agent(&app, "故障乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    behavior.set_ids(a_id, b_id, builtin_moderator_id(&app).await);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 5).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let discussion = wait_discussion(&app, room_id, |value| value["status"] == "failed").await;
    assert_eq!(discussion["stopReason"], "participant_failed");
    wait_for_system_message(&app, room_id, "讨论失败：参与者「故障甲」运行失败").await;
    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    let moderator_id = builtin_moderator_id(&app).await;
    assert_eq!(runs, vec![moderator_id, a_id], "失败后不应运行裁决");
}

#[tokio::test]
async fn resume_failed_discussion_skips_completed_turns_and_finishes() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "")),
        Arc::new(|_| turn_text("support", None, "stop", "")),
        Arc::new(|_, _| judge_text("stop", "已收敛")),
    ));
    *behavior.b_failures.lock().unwrap() = 1;
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "续跑甲").await;
    let second = create_agent(&app, "续跑乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, b_id, moderator_id);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 3).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let failed = wait_discussion(&app, room_id, |value| value["status"] == "failed").await;
    assert_eq!(failed["stopReason"], "participant_failed");
    let discussion_id = failed["id"].as_i64().unwrap();
    let runs_before: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    assert_eq!(runs_before, vec![moderator_id, a_id, b_id]);

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/discussions/{discussion_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["status"], "running");
    assert_eq!(response.body["id"], discussion_id);

    let completed = wait_discussion(&app, room_id, |value| {
        value["status"] == "completed"
    })
    .await;
    assert_eq!(completed["id"], discussion_id, "续跑不得新建讨论");
    assert_eq!(completed["stopReason"], "all_voted_stop");
    wait_for_system_message(&app, room_id, "讨论继续：第 1 轮，从「续跑乙」继续").await;

    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == a_id).count(),
        1,
        "已完成的甲不得重跑：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == b_id).count(),
        2,
        "失败的乙应重跑一次：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == moderator_id).count(),
        2,
        "主持人只应有发起 + 裁决两次运行：{runs:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_resume_only_one_wins() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "")),
        Arc::new(|_| turn_text("support", None, "stop", "")),
        Arc::new(|_, _| judge_text("stop", "已收敛")),
    ));
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "并发甲").await;
    let second = create_agent(&app, "并发乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, b_id, moderator_id);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 3).await;

    // 单次竞态在巧合下可能被预检查挡住；重复三轮，确保真正压到 reopen 的并发窗口。
    for round in 1..=3 {
        *behavior.b_failures.lock().unwrap() = 1;
        post(
            &app,
            &format!("/api/rooms/{room_id}/messages"),
            json!({ "content": "讨论一下" }),
        )
        .await;
        let failed = wait_discussion(&app, room_id, |value| value["status"] == "failed").await;
        let discussion_id = failed["id"].as_i64().unwrap();
        let url = format!("/api/rooms/{room_id}/discussions/{discussion_id}/resume");

        let (first, second) = tokio::join!(
            post(&app, &url, json!({})),
            post(&app, &url, json!({}))
        );
        let ok = [&first, &second]
            .iter()
            .filter(|response| response.status == StatusCode::OK)
            .count();
        let conflict = [&first, &second]
            .iter()
            .filter(|response| response.status == StatusCode::CONFLICT)
            .count();
        assert_eq!(
            (ok, conflict),
            (1, 1),
            "第 {round} 轮并发续跑只能有一个成功：first={:?} second={:?}",
            first.body,
            second.body
        );

        wait_discussion(&app, room_id, |value| value["status"] == "completed").await;
    }

    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == a_id).count(),
        3,
        "三场讨论里甲各发言一次：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == b_id).count(),
        6,
        "三场讨论里乙各失败一次 + 续跑一次：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == moderator_id).count(),
        6,
        "三场讨论各发起 + 裁决一次：{runs:?}"
    );
}

/// `reopen` 的条件 UPDATE 是并发安全的根：直接对同一 failed 讨论并发调用两次，
/// 断言恰好一次拿到 `Some`。该断言与调度顺序无关 —— 后到的 UPDATE 必然匹配 0 行。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reopen_is_atomic_under_concurrency() {
    let app = setup_with_adapter(None).await;
    let store = DiscussionStore::new(app.state.db.clone());
    let agent = create_agent(&app, "原子性参与者").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room_id = create_discussion_room(&app, &[agent_id], 2).await;

    let discussion = store
        .create(CreateDiscussionInput {
            room_id,
            topic: "并发原子性".to_string(),
            max_rounds: 2,
            participant_ids: vec![agent_id],
            created_by_agent_id: None,
        })
        .await
        .unwrap();
    let finished = store
        .finish(
            discussion.id,
            FinishDiscussionInput {
                status: DiscussionStatus::Failed,
                stop_reason: DiscussionStopReason::ParticipantFailed,
                summary: None,
            },
        )
        .await
        .unwrap()
        .expect("running 讨论应能进入 failed");
    assert_eq!(finished.status, DiscussionStatus::Failed);

    let (first, second) = tokio::join!(
        store.reopen(discussion.id),
        store.reopen(discussion.id)
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(
        [first.is_some(), second.is_some()]
            .iter()
            .filter(|won| **won)
            .count(),
        1,
        "同一 failed 讨论只能被 reopen 一次：first={first:?} second={second:?}"
    );
    assert_eq!(
        store.get_by_id(discussion.id).await.unwrap().unwrap().status,
        DiscussionStatus::Running
    );
}

#[tokio::test]
async fn resume_after_judge_failure_reruns_only_judge() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "continue", "")),
        Arc::new(|_| turn_text("support", None, "continue", "")),
        Arc::new(|_, _| judge_text("stop", "已收敛")),
    ));
    *behavior.judge_failures.lock().unwrap() = 2;
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "裁决甲").await;
    let second = create_agent(&app, "裁决乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, b_id, moderator_id);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 3).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let failed = wait_discussion(&app, room_id, |value| value["status"] == "failed").await;
    assert_eq!(failed["stopReason"], "moderator_failed");
    let discussion_id = failed["id"].as_i64().unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/discussions/{discussion_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    wait_for_system_message(&app, room_id, &format!("讨论继续：第 1 轮裁决")).await;
    wait_discussion(&app, room_id, |value| value["status"] == "completed").await;

    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == a_id).count(),
        1,
        "本轮发言已完成，不得重跑：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == b_id).count(),
        1,
        "本轮发言已完成，不得重跑：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == moderator_id).count(),
        4,
        "发起 1 次 + 裁决失败 2 次 + 续跑裁决 1 次：{runs:?}"
    );
}

#[tokio::test]
async fn resume_rejects_non_failed_and_unknown_discussions() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "")),
        Arc::new(|_| turn_text("support", None, "stop", "")),
        Arc::new(|_, _| judge_text("stop", "已收敛")),
    ));
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "状态甲").await;
    let second = create_agent(&app, "状态乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    behavior.set_ids(a_id, b_id, builtin_moderator_id(&app).await);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 3).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let completed = wait_discussion(&app, room_id, |value| {
        value["status"] == "completed"
    })
    .await;
    let discussion_id = completed["id"].as_i64().unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/discussions/{discussion_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);

    let missing = post(
        &app,
        &format!("/api/rooms/{room_id}/discussions/999999/resume"),
        json!({}),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{:?}", missing.body);

    let other_room = create_discussion_room(&app, &[a_id, b_id], 3).await;
    let cross_room = post(
        &app,
        &format!("/api/rooms/{other_room}/discussions/{discussion_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(cross_room.status, StatusCode::NOT_FOUND, "{:?}", cross_room.body);
}

#[tokio::test]
async fn continue_message_resumes_failed_discussion_without_moderator_replan() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "")),
        Arc::new(|_| turn_text("support", None, "stop", "")),
        Arc::new(|_, _| judge_text("stop", "已收敛")),
    ));
    *behavior.b_failures.lock().unwrap() = 1;
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "快路径甲").await;
    let second = create_agent(&app, "快路径乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, b_id, moderator_id);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 3).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let failed = wait_discussion(&app, room_id, |value| value["status"] == "failed").await;
    let discussion_id = failed["id"].as_i64().unwrap();
    let moderator_runs_before = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .filter(|(agent_id, _)| *agent_id == moderator_id)
        .count();
    assert_eq!(moderator_runs_before, 1, "失败前只有主持人发起运行");

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "继续" }),
    )
    .await;

    let completed = wait_discussion(&app, room_id, |value| {
        value["status"] == "completed"
    })
    .await;
    assert_eq!(completed["id"], discussion_id, "「继续」不得新建讨论");
    wait_for_system_message(&app, room_id, "讨论继续：第 1 轮，从「快路径乙」继续").await;

    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == moderator_id).count(),
        2,
        "「继续」不应触发主持人重新规划，只应有发起 + 裁决：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == a_id).count(),
        1,
        "已完成的甲方不得重跑：{runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|agent_id| **agent_id == b_id).count(),
        2,
        "失败的乙方应重跑一次：{runs:?}"
    );
}

#[tokio::test]
async fn judge_continue_advances_round_and_later_all_stop_finishes() {
    let judge_calls = Arc::new(AtomicUsize::new(0));
    let judge_counter = judge_calls.clone();
    let behavior = Arc::new(Behavior::new(
        Arc::new(|round| {
            turn_text(
                "propose",
                None,
                if round == 1 { "continue" } else { "stop" },
                "",
            )
        }),
        Arc::new(|round| {
            turn_text(
                "support",
                None,
                if round == 1 { "continue" } else { "stop" },
                "",
            )
        }),
        Arc::new(move |_, _| {
            let call = judge_counter.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                judge_text("continue", "")
            } else {
                judge_text("stop", "经过两轮讨论达成结论。")
            }
        }),
    ));
    *behavior.start_max_rounds.lock().unwrap() = Some(2);
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "多轮甲").await;
    let second = create_agent(&app, "多轮乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    behavior.set_ids(a_id, b_id, builtin_moderator_id(&app).await);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 2).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    let discussion = wait_discussion(&app, room_id, |value| value["status"] == "completed").await;
    assert_eq!(discussion["currentRound"], 2);
    assert_eq!(discussion["maxRounds"], 2);
    assert_eq!(discussion["stopReason"], "all_voted_stop");
    assert_eq!(discussion["summary"], "经过两轮讨论达成结论。");
    assert_eq!(judge_calls.load(Ordering::SeqCst), 2);
    let runs: Vec<i64> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .map(|(agent_id, _)| *agent_id)
        .collect();
    let moderator_id = builtin_moderator_id(&app).await;
    assert_eq!(
        runs,
        vec![
            moderator_id, a_id, b_id, moderator_id, a_id, b_id, moderator_id
        ]
    );
}

#[tokio::test]
async fn latest_and_stop_validate_room_and_ids() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|_| turn_text("propose", None, "stop", "")),
        Arc::new(|_| turn_text("support", None, "stop", "")),
        Arc::new(|_, _| judge_text("stop", "完成")),
    ));
    let app = setup_with_adapter(Some(factory(behavior))).await;
    let agent = create_agent(&app, "校验成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room_id = create_discussion_room(&app, &[agent_id], 5).await;

    let missing = get(&app, "/api/rooms/9999/discussions/latest").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let invalid = get(&app, "/api/rooms/abc/discussions/latest").await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    let missing_stop = post(
        &app,
        &format!("/api/rooms/{room_id}/discussions/9999/stop"),
        json!({}),
    )
    .await;
    assert_eq!(missing_stop.status, StatusCode::NOT_FOUND);
    let latest = get(&app, &format!("/api/rooms/{room_id}/discussions/latest")).await;
    assert!(latest.body.is_null());
}

#[tokio::test]
async fn reused_sessions_send_incremental_prompts_after_first_turn() {
    let round_turn = |round: i64, name: &str, stance: &str| {
        format!(
            "{name}第{round}轮发言。\n```moon-discussion-turn\n{}\n```",
            json!({
                "stance": stance,
                "stopVote": if round == 1 { "continue" } else { "stop" },
            })
        )
    };
    let behavior = Arc::new(Behavior::new(
        Arc::new(move |round| round_turn(round, "复用甲", "propose")),
        Arc::new(move |round| round_turn(round, "复用乙", "support")),
        Arc::new(|round, _| {
            if round == 1 {
                judge_text("continue", "")
            } else {
                judge_text("stop", "达成结论")
            }
        }),
    ));
    *behavior.start_max_rounds.lock().unwrap() = Some(2);
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "复用甲").await;
    let second = create_agent(&app, "复用乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, b_id, moderator_id);
    let room_id = create_discussion_room(&app, &[a_id, b_id], 2).await;
    enable_reuse(&app, room_id, a_id).await;
    enable_reuse(&app, room_id, b_id).await;
    enable_reuse(&app, room_id, moderator_id).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "讨论一下" }),
    )
    .await;
    wait_discussion(&app, room_id, |value| value["status"] == "completed").await;

    let runs = behavior.runs.lock().unwrap().clone();
    let turn_prompt = |agent_id: i64, round: i64| {
        runs.iter()
            .find(|(id, prompt)| *id == agent_id && prompt_round(prompt) == round)
            .map(|(_, prompt)| prompt.clone())
            .unwrap_or_else(|| panic!("缺少 agent={agent_id} round={round} 的运行"))
    };
    let judge_prompt = |round: i64| {
        runs.iter()
            .find(|(id, prompt)| {
                *id == moderator_id
                    && prompt.contains("moon-discussion-judge")
                    && prompt_round(prompt) == round
            })
            .map(|(_, prompt)| prompt.clone())
            .unwrap_or_else(|| panic!("缺少 round={round} 的裁决运行"))
    };

    let a_first = turn_prompt(a_id, 1);
    let a_second = turn_prompt(a_id, 2);
    assert!(a_first.contains("复用甲 的人设"));
    assert!(a_first.contains("已有讨论发言"));
    assert!(a_second.contains("本场讨论继续"));
    assert!(a_second.contains("你上次发言之后的新发言"));
    assert!(a_second.contains("复用乙第1轮发言"));
    assert!(!a_second.contains("复用甲第1轮发言"));
    assert!(!a_second.contains("复用甲 的人设"), "复用会话不应重复人设");
    assert!(!a_second.contains("已有讨论发言"));
    assert!(a_second.len() < a_first.len());

    let b_first = turn_prompt(b_id, 1);
    let b_second = turn_prompt(b_id, 2);
    assert!(b_first.contains("复用乙 的人设"));
    assert!(b_second.contains("复用甲第2轮发言"));
    assert!(!b_second.contains("复用乙 的人设"));
    assert!(b_second.len() < b_first.len());

    let judge_first = judge_prompt(1);
    let judge_second = judge_prompt(2);
    assert!(judge_first.contains("复用甲 的人设"));
    assert!(judge_second.contains("继续裁决本场讨论"));
    assert!(judge_second.contains("复用甲第2轮发言"));
    assert!(!judge_second.contains("复用甲第1轮发言"));
    assert!(!judge_second.contains("复用甲 的人设"));
}
