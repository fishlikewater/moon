//! 会话复用下的房间上下文可见性集成测试。
//!
//! 覆盖 bootstrap、增量、历史清理、失败重试、超窗摘要与会话归档；
//! 这些用例必须能在水位键、提交时机或清理边界被破坏时失败。

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{
    create_agent, create_room, get, patch, post, setup_with_adapter, wait_for_runs, TestApp,
};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::message::MessageSenderType;
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::error::AppError;
use moon_server::store::messages::AppendMessageInput;
use moon_server::store::MessageStore;
use serde_json::json;
use tokio::sync::{mpsc, Notify};
use tokio_stream::wrappers::ReceiverStream;

struct RecordingAdapter {
    events: Vec<RunEvent>,
    inputs: Arc<Mutex<Vec<RunInput>>>,
}

impl HarnessAdapter for RecordingAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.inputs.lock().unwrap().push(input);
        Box::pin(tokio_stream::iter(
            self.events
                .clone()
                .into_iter()
                .map(Ok::<_, moon_server::error::AppError>),
        ))
    }
}

fn recording_factory(events: Vec<RunEvent>) -> (AdapterFactory, Arc<Mutex<Vec<RunInput>>>) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let captured = inputs.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(RecordingAdapter {
            events: events.clone(),
            inputs: captured.clone(),
        }))
    });
    (factory, inputs)
}

/// 首次运行在发出 final 前等待放行，用于模拟运行中产生新消息。
struct GateAdapter {
    inputs: Arc<Mutex<Vec<RunInput>>>,
    started: Arc<Notify>,
    release: Arc<Notify>,
    gate_next: AtomicBool,
}

impl HarnessAdapter for GateAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.inputs.lock().unwrap().push(input);
        let gated = self.gate_next.swap(false, Ordering::SeqCst);
        let started = self.started.clone();
        let release = self.release.clone();
        let (sender, receiver) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = sender
                .send(Ok(RunEvent::Session("harness-1".to_string())))
                .await;
            started.notify_one();
            if gated {
                release.notified().await;
            }
            let _ = sender.send(Ok(RunEvent::Final("完成".to_string()))).await;
        });
        Box::pin(ReceiverStream::new(receiver))
    }
}

fn gated_factory() -> (
    AdapterFactory,
    Arc<Mutex<Vec<RunInput>>>,
    Arc<Notify>,
    Arc<Notify>,
) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let adapter: Arc<dyn HarnessAdapter> = Arc::new(GateAdapter {
        inputs: inputs.clone(),
        started: started.clone(),
        release: release.clone(),
        gate_next: AtomicBool::new(true),
    });
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| Ok(adapter.clone()));
    (factory, inputs, started, release)
}

fn success_events() -> Vec<RunEvent> {
    vec![
        RunEvent::Session("harness-1".to_string()),
        RunEvent::Final("完成".to_string()),
    ]
}

async fn append_user(app: &TestApp, room_id: i64, content: impl Into<String>) -> i64 {
    let store = MessageStore::new(app.state.db.clone());
    let message = store
        .append(AppendMessageInput::final_message(
            room_id,
            MessageSenderType::User,
            None,
            content,
        ))
        .await
        .unwrap();
    message.id
}

async fn append_agent(app: &TestApp, room_id: i64, agent_id: i64, content: impl Into<String>) {
    let store = MessageStore::new(app.state.db.clone());
    store
        .append(AppendMessageInput::final_message(
            room_id,
            MessageSenderType::Agent,
            Some(agent_id),
            content,
        ))
        .await
        .unwrap();
}

async fn wait_for_watermark(app: &TestApp, room_id: i64, agent_id: i64) -> i64 {
    for _ in 0..250 {
        let session = app
            .state
            .session_service
            .active_session(room_id, agent_id)
            .await
            .unwrap();
        if let Some(session) = session {
            if session.last_context_message_id > 0 {
                return session.last_context_message_id;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("等待上下文水位提交超时");
}

async fn prompt_at(inputs: &Arc<Mutex<Vec<RunInput>>>, index: usize) -> String {
    for _ in 0..250 {
        if let Some(input) = inputs.lock().unwrap().get(index) {
            return input.prompt.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("等待第 {index} 次运行 prompt 超时");
}

#[tokio::test]
async fn first_reuse_run_bootstraps_history_and_commits_watermark() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "上下文助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "上下文房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    append_user(&app, room_id, "历史消息零").await;
    append_user(&app, room_id, "历史消息一").await;

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@上下文助手 当前问题" }),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{:?}", response.body);
    wait_for_runs(&app, room_id, 1).await;

    let prompt = prompt_at(&inputs, 0).await;
    assert!(prompt.contains("历史消息零"), "{prompt}");
    assert!(prompt.contains("历史消息一"), "{prompt}");
    assert!(prompt.contains("当前问题"), "{prompt}");
    assert!(
        wait_for_watermark(&app, room_id, agent_id).await > 0,
        "成功运行后必须提交水位"
    );
}

#[tokio::test]
async fn delta_skips_old_history_and_own_messages() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "增量助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "增量房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    append_user(&app, room_id, "旧历史").await;

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@增量助手 第一次" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    append_user(&app, room_id, "新的成员发言").await;
    append_agent(&app, room_id, agent_id, "自己的旧回复").await;

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@增量助手 第二次" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("新的成员发言"), "{prompt}");
    assert!(prompt.contains("第二次"), "{prompt}");
    assert!(!prompt.contains("旧历史"), "不应重复旧历史：{prompt}");
    assert!(
        !prompt.contains("自己的旧回复"),
        "自己的回复已在 harness 会话中，不应重复：{prompt}"
    );
}

#[tokio::test]
async fn cleanup_does_not_hide_messages_with_smaller_seq() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "清理助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "清理房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    append_user(&app, room_id, "清理前历史").await;

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@清理助手 第一次" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    let watermark = wait_for_watermark(&app, room_id, agent_id).await;

    let cleanup = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": 0 }),
    )
    .await;
    assert_eq!(cleanup.status, StatusCode::OK, "{:?}", cleanup.body);

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@清理助手 清理后新消息" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages"))
        .await
        .body
        .as_array()
        .unwrap()
        .clone();
    let new_user = messages
        .iter()
        .find(|message| message["content"] == "@清理助手 清理后新消息")
        .expect("清理后用户消息");
    assert!(
        new_user["seq"].as_i64().unwrap() <= watermark,
        "测试必须触及 seq 回退场景：seq={} watermark={watermark}",
        new_user["seq"]
    );

    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("清理后新消息"), "{prompt}");
    assert!(
        !prompt.contains("清理前历史"),
        "清理掉的旧历史不应重新出现：{prompt}"
    );
}

#[tokio::test]
async fn failed_run_does_not_advance_watermark() {
    let (factory, inputs) = recording_factory(vec![RunEvent::Error("模拟失败".to_string())]);
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "失败助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "失败房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    append_user(&app, room_id, "失败前历史").await;

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@失败助手 第一次" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "failed");

    let session = app
        .state
        .session_service
        .active_session(room_id, agent_id)
        .await
        .unwrap()
        .expect("复用会话必须存在");
    assert_eq!(session.last_context_message_id, 0, "失败不得推进水位");

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@失败助手 第二次" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("失败前历史"), "重试必须重新携带历史：{prompt}");
}

#[tokio::test]
async fn over_window_unread_messages_are_summarized_with_artifact_paths() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "摘要助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "摘要房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@摘要助手 启动" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    let store = MessageStore::new(app.state.db.clone());
    let mut older_ids = Vec::new();
    for index in 0..30 {
        let message = store
            .append(AppendMessageInput::final_message(
                room_id,
                MessageSenderType::User,
                None,
                format!("第{index}条{}", "长".repeat(1_000)),
            ))
            .await
            .unwrap();
        if index < 10 {
            // 最近 20 条留在窗口内，更早 10 条进入摘要。
            older_ids.push(message.id);
        }
    }

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@摘要助手 汇总" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("更早未读消息摘要"), "missing digest");
    assert!(prompt.contains("完整内容"), "missing artifact reference");
    let normalized = prompt.replace('\\', "/");
    assert!(
        normalized.contains(".moon/artifacts/room-"),
        "missing artifact path"
    );
    for id in older_ids {
        assert!(
            normalized.contains(&format!("message-{id}.md")),
            "每条被摘要截断的消息都必须有归档路径：message-{id}.md 缺失"
        );
    }
}

#[tokio::test]
async fn archived_session_starts_new_context_from_zero() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "归档助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "归档房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    append_user(&app, room_id, "归档前历史").await;

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@归档助手 第一次" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    let session = app
        .state
        .session_service
        .active_session(room_id, agent_id)
        .await
        .unwrap()
        .expect("active session");
    let archived = post(
        &app,
        &format!("/api/rooms/{room_id}/sessions/{}/archive", session.id),
        json!({}),
    )
    .await;
    assert_eq!(archived.status, StatusCode::OK, "{:?}", archived.body);

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@归档助手 归档后新消息" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("归档前历史"), "新会话应 bootstrap 最近历史：{prompt}");
    assert!(prompt.contains("归档后新消息"), "{prompt}");
}

#[tokio::test]
async fn preview_and_run_share_incremental_history() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "预览助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "预览房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@预览助手 启动" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    append_user(&app, room_id, "预览增量").await;
    let preview = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={agent_id}&instruction=下一问"),
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    assert_eq!(preview.body["sessionReused"], true);
    assert_eq!(preview.body["includedMessages"], 1);
    assert!(preview.body["effectiveChars"].as_u64().unwrap() > 0);

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@预览助手 下一问" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;
    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("预览增量"), "{prompt}");
}

#[tokio::test]
async fn messages_arriving_during_run_are_delivered_next_time() {
    let (factory, inputs, started, release) = gated_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "并发助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "并发房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@并发助手 第一次" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    // 运行已进入适配器，snapshot_max 已固定；此时追加的消息 id 更大。
    started.notified().await;
    append_user(&app, room_id, "运行中并发消息").await;
    release.notify_one();
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@并发助手 第二次" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("运行中并发消息"), "并发消息不得漏投：{prompt}");
    assert!(prompt.contains("第二次"), "{prompt}");
}

#[tokio::test]
async fn moderator_chat_also_receives_room_increment() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "主持人房成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let created = post(
        &app,
        "/api/rooms",
        json!({
            "name": "主持人增量房",
            "workspaceDir": app.dir.path().to_string_lossy(),
            "memberIds": [agent_id],
            "replyMode": "moderator",
            "permissionMode": "auto",
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let room_id = created.body["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "第一问" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, common::builtin_moderator_id(&app).await).await;

    append_user(&app, room_id, "成员新对话").await;
    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "第二问" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    assert!(prompt.contains("成员新对话"), "{prompt}");
    assert!(prompt.contains("第二问"), "{prompt}");
    assert!(!prompt.contains("第一问"), "已完成水位不应重复：{prompt}");
}

#[tokio::test]
async fn low_budget_caps_unread_digest_and_matches_preview() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "低预算助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "低预算房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@低预算助手 启动" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    let store = MessageStore::new(app.state.db.clone());
    for index in 0..30 {
        store
            .append(AppendMessageInput::final_message(
                room_id,
                MessageSenderType::User,
                None,
                format!("第{index}条{}", "字".repeat(1_000)),
            ))
            .await
            .unwrap();
    }

    let limited = patch(&app, "/api/settings", json!({ "contextBudgetChars": 2_000 })).await;
    assert_eq!(limited.status, StatusCode::OK, "{:?}", limited.body);

    let preview = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={agent_id}&instruction=短问"),
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    let effective = preview.body["effectiveChars"].as_u64().unwrap();
    assert!(
        effective + 2 <= 2_000,
        "预览历史加摘要不得超过预算扣除指令后的额度：{effective}"
    );

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@低预算助手 短问" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    let start = prompt.find("更早未读消息摘要").expect("missing digest");
    let end = prompt.find("房间最近对话").expect("missing recent header");
    let digest_chars = prompt[start..end].chars().count();
    assert!(
        digest_chars <= 1_000,
        "低预算下摘要不得超过可用预算的一半：{digest_chars}"
    );
}

#[tokio::test]
async fn short_unread_messages_are_not_archived() {
    let (factory, _inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "短消息助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "短消息房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@短消息助手 启动" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    let store = MessageStore::new(app.state.db.clone());
    let mut ids = Vec::new();
    for index in 0..21 {
        let message = store
            .append(AppendMessageInput::final_message(
                room_id,
                MessageSenderType::User,
                None,
                format!("短消息{index}{}", "字".repeat(90)),
            ))
            .await
            .unwrap();
        ids.push(message.id);
    }

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@短消息助手 汇总" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let artifact_dir = app
        .dir
        .path()
        .join(".moon")
        .join("artifacts")
        .join(format!("room-{room_id}"));
    for id in ids {
        assert!(
            !artifact_dir.join(format!("message-{id}.md")).exists(),
            "未被摘要截断的短消息不应生成归档：{id}"
        );
    }
}

#[tokio::test]
async fn over_budget_digest_still_lists_every_archived_message() {
    let (factory, inputs) = recording_factory(success_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "溢出助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "溢出房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@溢出助手 启动" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    let store = MessageStore::new(app.state.db.clone());
    let mut older_ids = Vec::new();
    for index in 0..60 {
        let message = store
            .append(AppendMessageInput::final_message(
                room_id,
                MessageSenderType::User,
                None,
                format!("第{index}条{}", "长".repeat(1_000)),
            ))
            .await
            .unwrap();
        if index < 40 {
            older_ids.push(message.id);
        }
    }

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@溢出助手 汇总" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);
    wait_for_runs(&app, room_id, 2).await;

    let prompt = prompt_at(&inputs, 1).await;
    let normalized = prompt.replace('\\', "/");
    for id in older_ids {
        assert!(
            normalized.contains(&format!("message-{id}.md")),
            "固定开销溢出时仍须逐条引用归档文件：message-{id}.md 缺失"
        );
    }
}
