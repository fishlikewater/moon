//! 上下文用量、压缩预览与运行用量落库集成测试。

mod common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::db::Db;
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::message::MessageSenderType;
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::store::messages::AppendMessageInput;
use moon_server::store::MessageStore;
use moon_server::{build_app, AppState, AppStateOptions};
use serde_json::{json, Value};
use tower::ServiceExt;

use common::*;

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

fn done_events() -> Vec<RunEvent> {
    vec![
        RunEvent::TextDelta("完成".to_string()),
        RunEvent::Final("完成".to_string()),
    ]
}

async fn append_messages(app: &TestApp, room_id: i64, count: usize, content_len: usize) {
    let store = MessageStore::new(app.state.db.clone());
    for index in 0..count {
        let content = format!("第{index}条{}", "字".repeat(content_len));
        store
            .append(AppendMessageInput::final_message(
                room_id,
                MessageSenderType::User,
                None,
                content,
            ))
            .await
            .unwrap();
    }
}

async fn disable_reuse(app: &TestApp, room_id: i64, agent_id: i64) {
    let response = put(
        app,
        &format!("/api/rooms/{room_id}/members/{agent_id}/session-settings"),
        json!({ "reuseSession": false }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
}

async fn wait_for_watermark(app: &TestApp, room_id: i64, agent_id: i64) {
    for _ in 0..250 {
        let session = app
            .state
            .session_service
            .active_session(room_id, agent_id)
            .await
            .unwrap();
        if session.is_some_and(|session| session.last_context_message_id > 0) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("等待上下文水位提交超时");
}

async fn get_reopened(app: &TestApp, uri: &str) -> (StatusCode, Value) {
    let db = Db::open(app.dir.path().join("moon.db")).await.unwrap();
    let state = AppState::new_with_options(
        db,
        app.dir.path().join("sessions-reopened"),
        AppStateOptions::default(),
    )
    .await
    .unwrap();
    let router = build_app(state);
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap();
    (status, body)
}

#[tokio::test]
async fn run_usage_is_persisted_and_survives_reopen() {
    let (factory, _inputs) = recording_factory(vec![
        RunEvent::Usage {
            token_count: Some(42),
            context_size: Some(1000),
            tool_call_count: Some(3),
            duration_ms: Some(1200),
        },
        RunEvent::Final("完成".to_string()),
    ]);
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "用量助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "用量房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@用量助手 你好" }),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{:?}", response.body);
    let runs = wait_for_runs(&app, room_id, 1).await;
    let run_id = runs[0]["id"].as_i64().unwrap();
    assert_eq!(runs[0]["tokenCount"], 42);
    assert_eq!(runs[0]["contextSize"], 1000);

    let (status, detail) = get_reopened(&app, &format!("/api/rooms/{room_id}/runs/{run_id}")).await;
    assert_eq!(status, StatusCode::OK, "{detail:?}");
    assert_eq!(detail["run"]["tokenCount"], 42);
    assert_eq!(detail["run"]["contextSize"], 1000);
}

#[tokio::test]
async fn context_preview_reports_compaction_and_matches_prompt() {
    let (factory, inputs) = recording_factory(done_events());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "上下文助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "上下文房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    disable_reuse(&app, room_id, agent_id).await;
    append_messages(&app, room_id, 25, 4).await;

    let preview = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={agent_id}&instruction=问题"),
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    assert_eq!(preview.body["enabled"], true);
    assert_eq!(preview.body["budgetChars"], 24_000);
    assert_eq!(preview.body["includedMessages"], 20);
    assert_eq!(preview.body["compressedMessages"], 14);
    assert_eq!(preview.body["omittedMessages"], 0);
    assert_eq!(preview.body["sessionReused"], false);
    let effective = preview.body["effectiveChars"].as_u64().unwrap();
    assert!(effective > 0);
    assert_eq!(
        preview.body["estimatedTokens"].as_u64().unwrap(),
        effective.div_ceil(4)
    );

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@上下文助手 最新问题" }),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{:?}", response.body);
    wait_for_runs(&app, room_id, 1).await;
    let captured = inputs.lock().unwrap();
    let prompt = &captured[0].prompt;
    assert!(
        prompt.contains("（已压缩 14 条早期消息，省略 0 条）"),
        "{prompt}"
    );
    assert!(prompt.contains("第24条"), "最新历史应保留全文：{prompt}");
    assert!(!prompt.contains("第0条"), "最旧历史应被压缩：{prompt}");
}

#[tokio::test]
async fn context_preview_handles_budget_disabled_and_invalid_agent() {
    let app = setup().await;
    let agent = create_agent(&app, "预算助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "预算房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    disable_reuse(&app, room_id, agent_id).await;
    append_messages(&app, room_id, 25, 300).await;

    let missing = get(&app, &format!("/api/rooms/{room_id}/context")).await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST, "{:?}", missing.body);

    let other = create_agent(&app, "非成员").await;
    let other_id = other["id"].as_i64().unwrap();
    let non_member = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={other_id}"),
    )
    .await;
    assert_eq!(
        non_member.status,
        StatusCode::NOT_FOUND,
        "{:?}",
        non_member.body
    );

    let limited = patch(
        &app,
        "/api/settings",
        json!({ "contextBudgetChars": 2_000 }),
    )
    .await;
    assert_eq!(limited.status, StatusCode::OK, "{:?}", limited.body);
    let preview = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={agent_id}"),
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    assert!(
        preview.body["omittedMessages"].as_u64().unwrap() > 0,
        "{:?}",
        preview.body
    );
    assert_eq!(
        preview.body["includedMessages"].as_u64().unwrap()
            + preview.body["omittedMessages"].as_u64().unwrap(),
        20
    );
    assert!(preview.body["effectiveChars"].as_u64().unwrap() <= 2_000);

    let disabled = patch(
        &app,
        "/api/settings",
        json!({ "contextCompressionEnabled": false }),
    )
    .await;
    assert_eq!(disabled.status, StatusCode::OK, "{:?}", disabled.body);
    let preview = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={agent_id}"),
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    assert_eq!(preview.body["enabled"], false);
    assert_eq!(preview.body["compressedMessages"], 0);
    assert_eq!(preview.body["omittedMessages"], 0);
    assert_eq!(preview.body["includedMessages"], 20);
}

#[tokio::test]
async fn context_preview_reports_session_reuse() {
    let (factory, _inputs) = recording_factory(vec![
        RunEvent::Session("harness-1".to_string()),
        RunEvent::Final("好".to_string()),
    ]);
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "复用助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "复用房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    append_messages(&app, room_id, 10, 2).await;
    enable_reuse(&app, room_id, agent_id).await;

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@复用助手 首次" }),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{:?}", response.body);
    wait_for_runs(&app, room_id, 1).await;
    wait_for_watermark(&app, room_id, agent_id).await;

    let preview = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={agent_id}&instruction=增量"),
    )
    .await;
    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    assert_eq!(preview.body["sessionReused"], true);
    assert_eq!(preview.body["includedMessages"], 0);
    assert_eq!(preview.body["compressedMessages"], 0);
    assert_eq!(preview.body["effectiveChars"], 0);
    assert_eq!(preview.body["estimatedTokens"], 0);
}

#[tokio::test]
async fn context_preview_reports_reuse_before_first_run() {
    // 首次开启复用、尚未写入 harness 会话 ID 时，运行按 bootstrap
    // 注入最近房间历史；预览必须与真实运行一致。
    let (factory, _inputs) = recording_factory(vec![RunEvent::Final("好".to_string())]);
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "复用助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "复用房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    append_messages(&app, room_id, 10, 2).await;
    enable_reuse(&app, room_id, agent_id).await;

    let preview = get(
        &app,
        &format!("/api/rooms/{room_id}/context?agentId={agent_id}&instruction=增量"),
    )
    .await;

    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    assert_eq!(preview.body["sessionReused"], true);
    assert_eq!(preview.body["includedMessages"], 10);
    assert_eq!(preview.body["compressedMessages"], 4);
    assert!(preview.body["effectiveChars"].as_u64().unwrap() > 0);
    assert_eq!(
        preview.body["estimatedTokens"].as_u64().unwrap(),
        preview.body["effectiveChars"].as_u64().unwrap().div_ceil(4)
    );
}
