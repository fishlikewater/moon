//! 编排引擎端到端测试：收敛、熔断、暂停与激活边界。

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use common::{builtin_moderator_id, create_agent, get, patch, post, setup_with_adapter};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::error::AppError;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Default)]
struct Ids {
    a: i64,
    moderator: i64,
}

type MemberFn = Arc<dyn Fn(i64) -> String + Send + Sync>;

struct Behavior {
    ids: Mutex<Ids>,
    plans: Mutex<Vec<String>>,
    plan_calls: AtomicUsize,
    /// 每次主持人计划运行的模拟耗时；用于重复激活与暂停竞态。
    moderator_delay_ms: AtomicU64,
    fail_agents: Mutex<HashSet<i64>>,
    member: MemberFn,
    runs: Mutex<Vec<(i64, String)>>,
}

impl Behavior {
    fn new(plans: Vec<String>, member: MemberFn) -> Self {
        Self {
            ids: Mutex::new(Ids::default()),
            plans: Mutex::new(plans),
            plan_calls: AtomicUsize::new(0),
            moderator_delay_ms: AtomicU64::new(0),
            fail_agents: Mutex::new(HashSet::new()),
            member,
            runs: Mutex::new(Vec::new()),
        }
    }

    fn set_ids(&self, a: i64, moderator: i64) {
        *self.ids.lock().unwrap() = Ids { a, moderator };
    }

    fn fail(&self, agent_id: i64) {
        self.fail_agents.lock().unwrap().insert(agent_id);
    }

    fn set_moderator_delay(&self, milliseconds: u64) {
        self.moderator_delay_ms.store(milliseconds, Ordering::SeqCst);
    }

    fn next_plan(&self) -> String {
        let call = self.plan_calls.fetch_add(1, Ordering::SeqCst);
        let plans = self.plans.lock().unwrap();
        plans[call.min(plans.len() - 1)].clone()
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

struct OrchestratorAdapter {
    agent_id: i64,
    behavior: Arc<Behavior>,
}

impl HarnessAdapter for OrchestratorAdapter {
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
                if prompt.contains("assignments.agentId 必须严格使用下列 id") {
                    let delay = behavior.moderator_delay_ms.load(Ordering::SeqCst);
                    if delay > 0 {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                    return Ok(RunEvent::Final(behavior.next_plan()));
                }
                return Ok(RunEvent::Final("主持人普通回复".to_string()));
            }

            if behavior.fail_agents.lock().unwrap().contains(&agent_id) {
                return Err(AppError::harness_run_failed("模拟成员故障"));
            }
            Ok(RunEvent::Final((behavior.member)(agent_id)))
        }))
    }
}

fn factory(behavior: Arc<Behavior>) -> AdapterFactory {
    Arc::new(move |agent: &Agent| {
        Ok(Arc::new(OrchestratorAdapter {
            agent_id: agent.id,
            behavior: Arc::clone(&behavior),
        }) as Arc<dyn HarnessAdapter>)
    })
}

fn assign(agent_id: i64, instruction: &str, kind: &str) -> String {
    json!({
        "type": "assign",
        "assignments": [{
            "agentId": agent_id,
            "instruction": instruction,
            "kind": kind,
            "expectedOutput": "可核对产出",
            "acceptanceCriteria": ["AC-001"],
        }]
    })
    .to_string()
}

fn done(summary: &str) -> String {
    json!({
        "type": "done",
        "summary": summary,
        "verification": ["cargo test --manifest-path server-rs/Cargo.toml"],
    })
    .to_string()
}

async fn prepare(behavior: Arc<Behavior>) -> (common::TestApp, i64, i64, i64) {
    let app = setup_with_adapter(Some(factory(behavior.clone()))).await;
    let first = create_agent(&app, "执行甲").await;
    let second = create_agent(&app, "执行乙").await;
    let a_id = first["id"].as_i64().unwrap();
    let b_id = second["id"].as_i64().unwrap();
    let moderator_id = builtin_moderator_id(&app).await;
    behavior.set_ids(a_id, moderator_id);

    let room = post(
        &app,
        "/api/rooms",
        json!({
            "name": "编排测试房间",
            "workspaceDir": app.dir.path().to_string_lossy(),
            "memberIds": [a_id, b_id],
            "replyMode": "moderator",
        }),
    )
    .await;
    assert_eq!(room.status, StatusCode::CREATED, "{:?}", room.body);
    let room_id = room.body["id"].as_i64().unwrap();
    (app, room_id, a_id, b_id)
}

async fn create_task(app: &common::TestApp, room_id: i64, title: &str) -> i64 {
    let response = post(
        app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({
            "title": title,
            "description": "端到端编排测试任务",
            "plan": {
                "goal": "完成可核对的功能",
                "acceptanceCriteria": ["AC-001 功能可用"],
                "verification": ["cargo test"],
                "scope": ["server-rs"]
            }
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body["id"].as_i64().unwrap()
}

async fn activate(app: &common::TestApp, task_id: i64) -> common::ApiResponse {
    post(app, &format!("/api/tasks/{task_id}/activate"), Value::Null).await
}

async fn task_status(app: &common::TestApp, room_id: i64, task_id: i64) -> Value {
    let response = get(app, &format!("/api/rooms/{room_id}/tasks")).await;
    assert_eq!(response.status, StatusCode::OK);
    response
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == task_id)
        .cloned()
        .expect("task in list")
}

async fn wait_task_status(
    app: &common::TestApp,
    room_id: i64,
    task_id: i64,
    expected: &str,
) -> Value {
    for _ in 0..400 {
        let task = task_status(app, room_id, task_id).await;
        if task["status"] == expected {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let task = task_status(app, room_id, task_id).await;
    let messages = get(app, &format!("/api/rooms/{room_id}/messages")).await;
    panic!("等待任务状态 {expected} 超时：task={task:?} messages={:?}", messages.body);
}

async fn room_runs(app: &common::TestApp, room_id: i64) -> Vec<Value> {
    let response = get(app, &format!("/api/rooms/{room_id}/runs")).await;
    assert_eq!(response.status, StatusCode::OK);
    response.body.as_array().unwrap().clone()
}

#[tokio::test]
async fn converges_after_work_review_then_done() {
    let behavior = Arc::new(Behavior::new(
        vec![],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成产出")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    let moderator_id = behavior.ids.lock().unwrap().moderator;
    *behavior.plans.lock().unwrap() = vec![
        assign(a_id, "实现功能", "work"),
        assign(a_id, "复核功能", "review"),
        done("功能已完成并复核"),
    ];
    let task_id = create_task(&app, room_id, "实现并复核功能").await;

    let activated = activate(&app, task_id).await;
    assert_eq!(activated.status, StatusCode::ACCEPTED, "{:?}", activated.body);
    assert_eq!(activated.body["status"], "in_progress");

    let task = wait_task_status(&app, room_id, task_id, "review").await;
    assert_eq!(task["status"], "review");

    assert_eq!(
        behavior.run_ids(),
        vec![moderator_id, a_id, moderator_id, a_id, moderator_id]
    );
    assert_eq!(behavior.plan_calls.load(Ordering::SeqCst), 3);
    let runs = room_runs(&app, room_id).await;
    assert_eq!(runs.len(), 5);
    assert!(runs.iter().all(|run| run["taskId"] == task_id));
    assert!(runs.iter().all(|run| run["status"] == "succeeded"));
    let member_prompts: Vec<String> = behavior
        .runs
        .lock()
        .unwrap()
        .iter()
        .filter(|(agent_id, _)| *agent_id == a_id)
        .map(|(_, prompt)| prompt.clone())
        .collect();
    assert!(member_prompts[0].contains("主持人安排你执行：实现功能"));
    assert!(member_prompts[1].contains("主持人安排你复核：复核功能"));

    let timeline = get(&app, &format!("/api/tasks/{task_id}/timeline")).await;
    assert_eq!(timeline.status, StatusCode::OK);
    let entries = timeline.body["entries"].as_array().unwrap();
    let event_types: Vec<&str> = entries
        .iter()
        .filter(|entry| entry["kind"] == "task_event")
        .filter_map(|entry| entry["event"]["type"].as_str())
        .collect();
    assert_eq!(
        event_types,
        vec![
            "created",
            "status_changed",
            "moderator_plan",
            "moderator_plan",
            "moderator_done",
            "status_changed"
        ]
    );
    let plan_events: Vec<&Value> = entries
        .iter()
        .filter(|entry| {
            entry["kind"] == "task_event" && entry["event"]["type"] == "moderator_plan"
        })
        .collect();
    assert_eq!(plan_events[0]["event"]["payload"]["round"], 1);
    assert_eq!(plan_events[1]["event"]["payload"]["round"], 2);

    let messages = common::wait_for_system_message(&app, room_id, "主持人总结：功能已完成并复核").await;
    let summary = messages
        .iter()
        .find(|message| {
            message["content"]
                .as_str()
                .is_some_and(|content| content.starts_with("主持人总结："))
        })
        .unwrap();
    assert_eq!(summary["taskId"], task_id);
    assert!(summary["content"]
        .as_str()
        .unwrap()
        .contains("1. cargo test --manifest-path server-rs/Cargo.toml"));
}

#[tokio::test]
async fn blocks_after_eight_rounds() {
    let behavior = Arc::new(Behavior::new(
        vec![],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    let moderator_id = behavior.ids.lock().unwrap().moderator;
    *behavior.plans.lock().unwrap() = vec![assign(a_id, "继续实现", "work")];
    let task_id = create_task(&app, room_id, "永不收敛的任务").await;

    let activated = activate(&app, task_id).await;
    assert_eq!(activated.status, StatusCode::ACCEPTED);

    wait_task_status(&app, room_id, task_id, "blocked").await;
    assert_eq!(behavior.plan_calls.load(Ordering::SeqCst), 8);
    assert_eq!(behavior.run_ids().iter().filter(|id| **id == a_id).count(), 8);
    assert_eq!(behavior.run_ids().iter().filter(|id| **id == moderator_id).count(), 8);
    assert_eq!(room_runs(&app, room_id).await.len(), 16);

    let messages = common::wait_for_system_message(&app, room_id, "已达最大轮次").await;
    let blocked = messages
        .iter()
        .find(|message| {
            message["content"]
                .as_str()
                .is_some_and(|content| content.contains("已达最大轮次"))
        })
        .unwrap();
    assert_eq!(blocked["senderType"], "system");
    assert_eq!(blocked["taskId"], task_id);
}

#[tokio::test]
async fn retries_invalid_plan_once_then_blocks() {
    let behavior = Arc::new(Behavior::new(
        vec!["这不是 JSON".to_string()],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, _, _) = prepare(behavior.clone()).await;
    let task_id = create_task(&app, room_id, "非法计划任务").await;

    let activated = activate(&app, task_id).await;
    assert_eq!(activated.status, StatusCode::ACCEPTED);
    wait_task_status(&app, room_id, task_id, "blocked").await;

    assert_eq!(behavior.plan_calls.load(Ordering::SeqCst), 2);
    assert_eq!(room_runs(&app, room_id).await.len(), 2);
    let messages =
        common::wait_for_system_message(&app, room_id, "无法解析为约定 JSON").await;
    assert!(messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|content| content.contains("编排已熔断"))
    }));
}

#[tokio::test]
async fn done_without_work_blocks() {
    let behavior = Arc::new(Behavior::new(
        vec![done("没有工作也宣布完成")],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, _, _) = prepare(behavior.clone()).await;
    let task_id = create_task(&app, room_id, "没有工作的任务").await;

    activate(&app, task_id).await;
    wait_task_status(&app, room_id, task_id, "blocked").await;
    assert_eq!(behavior.plan_calls.load(Ordering::SeqCst), 1);

    let messages =
        common::wait_for_system_message(&app, room_id, "尚未安排任何成员执行工作").await;
    assert!(messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|content| content.contains("不能宣布完成"))
    }));
}

#[tokio::test]
async fn done_without_review_is_corrected_once_then_blocks() {
    let behavior = Arc::new(Behavior::new(
        vec![],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    *behavior.plans.lock().unwrap() = vec![
        assign(a_id, "实现功能", "work"),
        done("第一次宣布完成"),
        done("第二次宣布完成"),
    ];
    let task_id = create_task(&app, room_id, "缺少复核的任务").await;

    activate(&app, task_id).await;
    wait_task_status(&app, room_id, task_id, "blocked").await;
    assert_eq!(behavior.plan_calls.load(Ordering::SeqCst), 3);

    let messages = common::wait_for_system_message(&app, room_id, "请先安排至少一次").await;
    assert!(messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|content| content.contains("kind:\"review\""))
    }));
    let messages =
        common::wait_for_system_message(&app, room_id, "连续两轮未安排成员复核").await;
    assert!(messages.iter().any(|message| message["taskId"] == task_id));
}

#[tokio::test]
async fn self_or_non_member_assignment_blocks() {
    let behavior = Arc::new(Behavior::new(
        vec![],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, _, _) = prepare(behavior.clone()).await;
    let moderator_id = behavior.ids.lock().unwrap().moderator;

    *behavior.plans.lock().unwrap() = vec![assign(moderator_id, "自己干", "work")];
    let task_id = create_task(&app, room_id, "自派任务").await;
    activate(&app, task_id).await;
    wait_task_status(&app, room_id, task_id, "blocked").await;
    let messages = common::wait_for_system_message(&app, room_id, "主持人不能指派自己").await;
    assert!(messages.iter().any(|message| message["taskId"] == task_id));

    behavior.plans.lock().unwrap().clear();
    *behavior.plans.lock().unwrap() = vec![assign(999_999, "外部人员干", "work")];
    let second_task = create_task(&app, room_id, "非成员任务").await;
    activate(&app, second_task).await;
    wait_task_status(&app, room_id, second_task, "blocked").await;
    let messages =
        common::wait_for_system_message(&app, room_id, "主持人指派了非房间成员").await;
    assert!(messages.iter().any(|message| message["taskId"] == second_task));
}

#[tokio::test]
async fn member_failure_blocks_task() {
    let behavior = Arc::new(Behavior::new(
        vec![],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    behavior.fail(a_id);
    *behavior.plans.lock().unwrap() = vec![assign(a_id, "注定失败的工作", "work")];
    let task_id = create_task(&app, room_id, "成员失败任务").await;

    activate(&app, task_id).await;
    wait_task_status(&app, room_id, task_id, "blocked").await;
    let messages =
        common::wait_for_system_message(&app, room_id, "运行失败：模拟成员故障").await;
    assert!(messages.iter().any(|message| {
        message["taskId"] == task_id
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("执行甲"))
    }));
}

#[tokio::test]
async fn pause_prevents_dispatch_until_resume() {
    let behavior = Arc::new(Behavior::new(
        vec![],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    *behavior.plans.lock().unwrap() = vec![
        assign(a_id, "实现功能", "work"),
        assign(a_id, "复核功能", "review"),
        done("暂停后继续完成"),
    ];
    let task_id = create_task(&app, room_id, "暂停任务").await;

    let paused = post(
        &app,
        &format!("/api/rooms/{room_id}/orchestration/pause"),
        Value::Null,
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK);
    assert_eq!(paused.body["paused"], true);

    let activated = activate(&app, task_id).await;
    assert_eq!(activated.status, StatusCode::ACCEPTED);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(room_runs(&app, room_id).await.is_empty());
    assert_eq!(behavior.plan_calls.load(Ordering::SeqCst), 0);

    let state = get(&app, &format!("/api/rooms/{room_id}/orchestration")).await;
    assert_eq!(state.body["paused"], true);

    let resumed = post(
        &app,
        &format!("/api/rooms/{room_id}/orchestration/resume"),
        Value::Null,
    )
    .await;
    assert_eq!(resumed.status, StatusCode::OK);
    assert_eq!(resumed.body["paused"], false);

    wait_task_status(&app, room_id, task_id, "review").await;
    assert_eq!(behavior.plan_calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn pause_and_resume_are_idempotent_and_validate_room() {
    let behavior = Arc::new(Behavior::new(
        vec![done("未使用")],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, _, _) = prepare(behavior).await;

    for _ in 0..2 {
        let paused = post(
            &app,
            &format!("/api/rooms/{room_id}/orchestration/pause"),
            Value::Null,
        )
        .await;
        assert_eq!(paused.status, StatusCode::OK);
        assert_eq!(paused.body["paused"], true);
    }
    let state = get(&app, &format!("/api/rooms/{room_id}/orchestration")).await;
    assert_eq!(state.body["paused"], true);
    assert_eq!(state.body["roomId"], room_id);

    for _ in 0..2 {
        let resumed = post(
            &app,
            &format!("/api/rooms/{room_id}/orchestration/resume"),
            Value::Null,
        )
        .await;
        assert_eq!(resumed.status, StatusCode::OK);
        assert_eq!(resumed.body["paused"], false);
    }

    let invalid = get(&app, "/api/rooms/abc/orchestration").await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    let missing = get(&app, "/api/rooms/999999/orchestration").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let missing_pause = post(&app, "/api/rooms/999999/orchestration/pause", Value::Null).await;
    assert_eq!(missing_pause.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn activate_rejects_illegal_state_and_duplicate_start() {
    let behavior = Arc::new(Behavior::new(
        vec![],
        Arc::new(|agent_id| format!("成员 {agent_id} 完成")),
    ));
    let (app, room_id, a_id, _) = prepare(behavior.clone()).await;
    *behavior.plans.lock().unwrap() = vec![
        assign(a_id, "实现功能", "work"),
        assign(a_id, "复核功能", "review"),
        done("完成"),
    ];

    let missing = activate(&app, 999_999).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let review_task = create_task(&app, room_id, "已在评审的任务").await;
    let transitioned = patch(
        &app,
        &format!("/api/tasks/{review_task}/status"),
        json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(transitioned.status, StatusCode::OK);
    let transitioned = patch(
        &app,
        &format!("/api/tasks/{review_task}/status"),
        json!({ "status": "review" }),
    )
    .await;
    assert_eq!(transitioned.status, StatusCode::OK);
    let rejected = activate(&app, review_task).await;
    assert_eq!(rejected.status, StatusCode::CONFLICT);
    assert_eq!(rejected.body["error"]["code"], "INVALID_TRANSITION");

    behavior.set_moderator_delay(250);
    let task_id = create_task(&app, room_id, "重复激活任务").await;
    let first = activate(&app, task_id).await;
    assert_eq!(first.status, StatusCode::ACCEPTED);
    let second = activate(&app, task_id).await;
    assert_eq!(second.status, StatusCode::CONFLICT);
    assert_eq!(second.body["error"]["code"], "CONSTRAINT_VIOLATION");
    assert!(second.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("编排已在运行中"));

    wait_task_status(&app, room_id, task_id, "review").await;
}
