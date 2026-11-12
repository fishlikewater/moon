//! 委派批次端到端测试：自动复核、续轮、失败记录、停止与查询语义。

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use common::{builtin_moderator_id, create_agent, delete, get, post, setup_with_adapter};
use futures_util::StreamExt;
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::error::AppError;
use moon_server::{build_app, AppState};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Clone, Copy, Default)]
struct Ids {
    a: i64,
    b: i64,
    moderator: i64,
}

type ModeratorChatFn = Arc<dyn Fn(Ids) -> String + Send + Sync>;
type ReviewFn = Arc<dyn Fn(usize, Ids) -> String + Send + Sync>;
type MemberFn = Arc<dyn Fn(i64) -> String + Send + Sync>;

struct Behavior {
    ids: Mutex<Ids>,
    fail_agents: Mutex<HashSet<i64>>,
    delays: Mutex<Vec<(i64, u64)>>,
    moderator_chat: ModeratorChatFn,
    review: ReviewFn,
    member: MemberFn,
    review_calls: AtomicUsize,
    runs: Mutex<Vec<(i64, String)>>,
}

impl Behavior {
    fn new(moderator_chat: ModeratorChatFn, review: ReviewFn, member: MemberFn) -> Self {
        Self {
            ids: Mutex::new(Ids::default()),
            fail_agents: Mutex::new(HashSet::new()),
            delays: Mutex::new(Vec::new()),
            moderator_chat,
            review,
            member,
            review_calls: AtomicUsize::new(0),
            runs: Mutex::new(Vec::new()),
        }
    }

    fn set_ids(&self, a: i64, b: i64, moderator: i64) {
        *self.ids.lock().unwrap() = Ids { a, b, moderator };
    }

    fn fail(&self, agent_id: i64) {
        self.fail_agents.lock().unwrap().insert(agent_id);
    }

    fn delay(&self, agent_id: i64, milliseconds: u64) {
        self.delays.lock().unwrap().push((agent_id, milliseconds));
    }

    fn delay_for(&self, agent_id: i64) -> u64 {
        self.delays
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| *id == agent_id)
            .map(|(_, delay)| *delay)
            .unwrap_or(0)
    }

    fn run_ids(&self) -> Vec<i64> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .map(|(agent_id, _)| *agent_id)
            .collect()
    }
}

struct CollaborationAdapter {
    agent_id: i64,
    behavior: Arc<Behavior>,
}

impl HarnessAdapter for CollaborationAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        let behavior = Arc::clone(&self.behavior);
        let agent_id = self.agent_id;
        Box::pin(futures_util::stream::once(async move {
            let prompt = input.prompt.clone();
            behavior
                .runs
                .lock()
                .unwrap()
                .push((agent_id, prompt.clone()));
            let ids = *behavior.ids.lock().unwrap();

            if agent_id == ids.moderator {
                if prompt.contains("moon-round-review") {
                    let call = behavior.review_calls.fetch_add(1, Ordering::SeqCst);
                    return Ok(RunEvent::Final((behavior.review)(call, ids)));
                }
                if prompt.contains("请四选一") {
                    return Ok(RunEvent::Final((behavior.moderator_chat)(ids)));
                }
                return Ok(RunEvent::Final("主持人普通回复".to_string()));
            }

            if behavior.fail_agents.lock().unwrap().contains(&agent_id) {
                return Err(AppError::harness_run_failed("模拟成员故障"));
            }
            let delay = behavior.delay_for(agent_id);
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Ok(RunEvent::Final((behavior.member)(agent_id)))
        }))
    }
}

fn factory(behavior: Arc<Behavior>) -> AdapterFactory {
    Arc::new(move |agent: &Agent| {
        Ok(Arc::new(CollaborationAdapter {
            agent_id: agent.id,
            behavior: Arc::clone(&behavior),
        }) as Arc<dyn HarnessAdapter>)
    })
}

fn delegation_text(ids: Ids, include_b: bool) -> String {
    let mut assignments = vec![json!({"agentId": ids.a, "instruction": "完成 A 的工作"})];
    if include_b {
        assignments.push(json!({"agentId": ids.b, "instruction": "完成 B 的工作"}));
    }
    format!(
        "我来安排成员执行。\n```moon-delegation\n{}\n```",
        json!({"summary": "安排本轮工作", "assignments": assignments})
    )
}

fn review_continue(ids: Ids, round: usize) -> String {
    format!(
        "第 {round} 轮完成。\n```moon-round-review\n{}\n```",
        json!({
            "decision": "continue",
            "reason": "还需要下一轮",
            "summary": format!("第 {round} 轮汇总"),
            "nextAssignments": [
                {"agentId": ids.a, "instruction": "继续 A"},
                {"agentId": ids.b, "instruction": "继续 B"}
            ],
        })
    )
}

fn review_stop(summary: &str) -> String {
    format!(
        "```moon-round-review\n{}\n```",
        json!({"decision": "stop", "reason": "已完成", "summary": summary})
    )
}

async fn create_room(app: &common::TestApp, member_ids: &[i64]) -> i64 {
    let response = post(
        app,
        "/api/rooms",
        json!({
            "name": "委派测试房间",
            "workspaceDir": app.dir.path().to_string_lossy(),
            "memberIds": member_ids,
            "replyMode": "moderator",
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body["id"].as_i64().unwrap()
}

async fn wait_batch(
    app: &common::TestApp,
    room_id: i64,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..400 {
        let response = get(app, &format!("/api/rooms/{room_id}/collaborations/latest")).await;
        assert_eq!(response.status, StatusCode::OK);
        if !response.body.is_null() && predicate(&response.body) {
            return response.body;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let latest = get(app, &format!("/api/rooms/{room_id}/collaborations/latest")).await;
    panic!("等待委派批次超时：{:?}", latest.body);
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

/// 创建两位成员、设置 id 映射并建立主持人回复模式房间。
async fn prepare(
    behavior: Arc<Behavior>,
) -> (common::TestApp, i64, i64, i64) {
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "执行甲").await;
    let second = create_agent(&app, "执行乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, b_id, moderator_id);
    let room_id = create_room(&app, &[a_id, b_id]).await;
    (app, room_id, a_id, b_id)
}

#[tokio::test]
async fn reviews_completed_round_and_continues_to_next() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|ids| delegation_text(ids, true)),
        Arc::new(|call, ids| {
            if call == 0 {
                review_continue(ids, 1)
            } else {
                review_stop("两轮工作均已完成。")
            }
        }),
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, b_id) = prepare(behavior.clone()).await;
    let moderator_id = behavior.ids.lock().unwrap().moderator;

    let (address, server) = start_server(app.state.clone()).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/ws?roomId={room_id}"
    ))
    .await
    .expect("connect ws");

    let sent = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请安排两位成员各自完成工作" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);

    let batch = wait_batch(&app, room_id, |value| value["status"] == "completed").await;
    assert_eq!(batch["status"], "completed");
    assert_eq!(batch["stopReason"], "moderator_stop");
    assert_eq!(batch["currentRound"], 2);
    assert_eq!(batch["maxRounds"], 3);
    assert_eq!(batch["summary"], "两轮工作均已完成。");
    assert_eq!(batch["userMessageId"], sent.body["userMessage"]["id"]);
    assert_eq!(batch["stopRequested"], false);

    let assignments = batch["assignments"].as_array().unwrap();
    assert_eq!(assignments.len(), 4);
    assert!(assignments
        .iter()
        .all(|assignment| assignment["status"] == "succeeded"));
    assert!(assignments
        .iter()
        .all(|assignment| assignment["messageId"].as_i64().is_some()));
    assert_eq!(
        assignments
            .iter()
            .map(|assignment| assignment["round"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 1, 2, 2]
    );

    assert_eq!(
        behavior.run_ids(),
        vec![
            moderator_id,
            a_id,
            b_id,
            moderator_id,
            a_id,
            b_id,
            moderator_id
        ]
    );
    let prompts = behavior.runs.lock().unwrap().clone();
    assert!(prompts[0].1.contains("moon-delegation"));
    assert!(prompts[3].1.contains("moon-round-review"));
    assert!(prompts[3].1.contains("完成 A 的工作"));
    assert!(prompts[3].1.contains("第 1/3 轮"));

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    let messages = messages.body.as_array().unwrap();
    assert!(messages.iter().any(|message| {
        message["senderType"] == "agent"
            && message["senderAgentId"] == moderator_id
            && message["collaboration"]["role"] == "summary"
            && message["content"] == "两轮工作均已完成。"
    }));
    assert!(messages.iter().any(|message| {
        message["senderType"] == "agent"
            && message["senderAgentId"] == a_id
            && message["collaboration"]["role"] == "assignment"
    }));

    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let incoming = tokio::time::timeout(Duration::from_millis(200), socket.next()).await;
        let Some(Ok(Message::Text(text))) = incoming.ok().flatten() else {
            continue;
        };
        events.push(serde_json::from_str::<Value>(text.as_str()).unwrap());
    }
    assert!(events.iter().any(|event| {
        event["type"] == "collaboration_changed" && event["batch"]["status"] == "running"
    }));
    assert!(events.iter().any(|event| {
        event["type"] == "collaboration_changed"
            && event["batch"]["status"] == "completed"
            && event["batch"]["currentRound"] == 2
    }));
    server.abort();
}

#[tokio::test]
async fn member_failure_is_recorded_and_still_reviewed() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|ids| delegation_text(ids, true)),
        Arc::new(|_, _| review_stop("已处理成员失败。")),
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, b_id) = prepare(behavior.clone()).await;
    let moderator_id = behavior.ids.lock().unwrap().moderator;
    behavior.fail(a_id);

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请安排成员工作，允许失败" }),
    )
    .await;

    let batch = wait_batch(&app, room_id, |value| value["status"] == "completed").await;
    assert_eq!(batch["stopReason"], "moderator_stop");
    let assignments = batch["assignments"].as_array().unwrap();
    assert_eq!(assignments.len(), 2);
    assert_eq!(assignments[0]["agentId"], a_id);
    assert_eq!(assignments[0]["status"], "failed");
    assert!(assignments[0]["error"]
        .as_str()
        .unwrap()
        .contains("模拟成员故障"));
    assert_eq!(assignments[1]["agentId"], b_id);
    assert_eq!(assignments[1]["status"], "succeeded");

    assert_eq!(behavior.review_calls.load(Ordering::SeqCst), 1);
    let review_prompt = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .find(|(agent_id, prompt)| {
            *agent_id == moderator_id && prompt.contains("moon-round-review")
        })
        .map(|(_, prompt)| prompt.clone())
        .expect("review prompt");
    assert!(review_prompt.contains("模拟成员故障"));
    assert!(review_prompt.contains("状态：失败"));
    assert!(review_prompt.contains("状态：成功"));

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    assert!(messages.body.as_array().unwrap().iter().any(|message| {
        message["senderType"] == "agent"
            && message["senderAgentId"] == a_id
            && message["status"] == "failed"
    }));
}

#[tokio::test]
async fn user_stop_finishes_batch_without_review() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|ids| delegation_text(ids, false)),
        Arc::new(|_, _| review_stop("不应执行复核")),
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    behavior.delay(a_id, 400);

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请安排甲执行长任务" }),
    )
    .await;

    let running = wait_batch(&app, room_id, |value| {
        value["assignments"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["status"] == "running"))
    })
    .await;
    let batch_id = running["id"].as_i64().unwrap();

    // 等成员运行真正开始：仅看到 assignment 处于 running 时，运行可能还在
    // 队列里，停止请求会被出队守卫拦下，测不到「运行中停止」。
    let mut started = false;
    for _ in 0..200 {
        if behavior.run_ids().contains(&a_id) {
            started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(started, "成员运行未在预期时间内开始");

    let stopped = post(
        &app,
        &format!("/api/rooms/{room_id}/collaborations/{batch_id}/stop"),
        Value::Null,
    )
    .await;
    assert_eq!(stopped.status, StatusCode::OK, "{:?}", stopped.body);
    assert_eq!(stopped.body["status"], "running");
    assert_eq!(stopped.body["stopRequested"], true);

    let batch = wait_batch(&app, room_id, |value| value["status"] == "stopped").await;
    assert_eq!(batch["stopReason"], "user_stop");
    assert_eq!(batch["summary"], "用户停止了委派批次。");
    assert_eq!(batch["stopRequested"], true);
    assert_eq!(behavior.review_calls.load(Ordering::SeqCst), 0);
    let moderator_id = behavior.ids.lock().unwrap().moderator;
    assert_eq!(behavior.run_ids(), vec![moderator_id, a_id]);

    let messages = common::wait_for_system_message(&app, room_id, "主持人委派已停止。").await;
    assert!(messages
        .iter()
        .any(|message| message["senderType"] == "system"
            && message["content"] == "主持人委派已停止。"));
}

#[tokio::test]
async fn continue_at_last_round_degrades_to_max_rounds() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|ids| delegation_text(ids, true)),
        Arc::new(|call, ids| review_continue(ids, call + 1)),
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, _, _) = prepare(behavior.clone()).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请安排两轮以上工作" }),
    )
    .await;

    let batch = wait_batch(&app, room_id, |value| value["status"] == "completed").await;
    assert_eq!(batch["currentRound"], 3);
    assert_eq!(batch["maxRounds"], 3);
    assert_eq!(batch["stopReason"], "max_rounds");
    assert_eq!(batch["summary"], "第 3 轮汇总");
    assert_eq!(batch["assignments"].as_array().unwrap().len(), 6);
    assert_eq!(behavior.review_calls.load(Ordering::SeqCst), 3);
    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    assert!(messages
        .body
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["senderType"] == "system"
            && message["content"].as_str().is_some_and(|content| content.contains("已达到最大轮次"))));
}

#[tokio::test]
async fn latest_and_stop_report_room_scoped_not_found() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|ids| delegation_text(ids, false)),
        Arc::new(|_, _| review_stop("完成。")),
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;

    let invalid = get(&app, "/api/rooms/abc/collaborations/latest").await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid.body["error"]["code"], "VALIDATION_FAILED");

    let missing = get(&app, "/api/rooms/999999/collaborations/latest").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let empty = get(&app, &format!("/api/rooms/{room_id}/collaborations/latest")).await;
    assert_eq!(empty.status, StatusCode::OK);
    assert!(empty.body.is_null());

    let other_response = post(
        &app,
        "/api/rooms",
        json!({
            "name": "另一房间",
            "workspaceDir": app.dir.path().to_string_lossy(),
            "memberIds": [a_id],
            "replyMode": "moderator",
        }),
    )
    .await;
    assert_eq!(other_response.status, StatusCode::CREATED);
    let other_room = other_response.body["id"].as_i64().unwrap();

    let missing_batch = post(
        &app,
        &format!("/api/rooms/{room_id}/collaborations/999999/stop"),
        Value::Null,
    )
    .await;
    assert_eq!(missing_batch.status, StatusCode::NOT_FOUND);

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请安排甲执行" }),
    )
    .await;
    let batch = wait_batch(&app, room_id, |value| value["status"] == "completed").await;
    let batch_id = batch["id"].as_i64().unwrap();

    let cross_room = post(
        &app,
        &format!("/api/rooms/{other_room}/collaborations/{batch_id}/stop"),
        Value::Null,
    )
    .await;
    assert_eq!(cross_room.status, StatusCode::NOT_FOUND);

    let idempotent = post(
        &app,
        &format!("/api/rooms/{room_id}/collaborations/{batch_id}/stop"),
        Value::Null,
    )
    .await;
    assert_eq!(idempotent.status, StatusCode::OK);
    assert_eq!(idempotent.body["status"], "completed");
    assert_eq!(idempotent.body["stopReason"], "moderator_stop");
}

#[tokio::test]
async fn deleting_running_batch_room_or_member_is_rejected() {
    let behavior = Arc::new(Behavior::new(
        Arc::new(|ids| delegation_text(ids, false)),
        Arc::new(|_, _| review_stop("完成。")),
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    behavior.delay(a_id, 300);

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请安排甲执行长任务" }),
    )
    .await;
    let running = wait_batch(&app, room_id, |value| {
        value["assignments"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["status"] == "running"))
    })
    .await;

    let agent_delete = delete(&app, &format!("/api/agents/{a_id}")).await;
    assert_eq!(agent_delete.status, StatusCode::CONFLICT);
    assert_eq!(agent_delete.body["error"]["code"], "CONSTRAINT_VIOLATION");
    assert!(agent_delete.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("进行中的委派"));

    let room_delete = delete(&app, &format!("/api/rooms/{room_id}")).await;
    assert_eq!(room_delete.status, StatusCode::CONFLICT);
    assert!(room_delete.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("进行中的委派"));

    let batch_id = running["id"].as_i64().unwrap();
    post(
        &app,
        &format!("/api/rooms/{room_id}/collaborations/{batch_id}/stop"),
        Value::Null,
    )
    .await;
    wait_batch(&app, room_id, |value| value["status"] == "stopped").await;
}
