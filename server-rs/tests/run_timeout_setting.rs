mod common;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{get, patch, post, setup_with_adapter, wait_for_runs};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use serde_json::json;
use std::time::Duration;

struct RecordingAdapter {
    timeouts: Arc<Mutex<Vec<Option<Duration>>>>,
}

impl HarnessAdapter for RecordingAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.timeouts.lock().unwrap().push(input.timeout);
        Box::pin(tokio_stream::iter(vec![Ok(RunEvent::Final(
            "完成".to_string(),
        ))]))
    }
}

fn recording_factory() -> (AdapterFactory, Arc<Mutex<Vec<Option<Duration>>>>) {
    let timeouts = Arc::new(Mutex::new(Vec::new()));
    let captured = timeouts.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(RecordingAdapter {
            timeouts: captured.clone(),
        }))
    });
    (factory, timeouts)
}

/// 运行总时长上限应来自设置库，而不是环境变量。
#[tokio::test]
async fn chat_run_timeout_follows_run_timeout_secs_setting() {
    let (factory, timeouts) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;

    let agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "超时甲",
            "persona": "超时设置冒烟",
            "harness": "fake",
        }),
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{:?}", agent.body);
    let agent_id = agent.body["id"].as_i64().unwrap();
    let room = common::create_room(&app, "超时房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    // 设置 120 秒后，运行的 timeout 应为 Some(120s)。
    let patched = patch(&app, "/api/settings", json!({ "runTimeoutSecs": 120 })).await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
    let posted = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@超时甲 第一次" }),
    )
    .await;
    assert_eq!(posted.status, StatusCode::ACCEPTED, "{:?}", posted.body);
    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "succeeded", "{:?}", runs[0]);

    // 设为 0（不限制）后，timeout 应为 None。
    let cleared = patch(&app, "/api/settings", json!({ "runTimeoutSecs": 0 })).await;
    assert_eq!(cleared.status, StatusCode::OK, "{:?}", cleared.body);
    let posted = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@超时甲 第二次" }),
    )
    .await;
    assert_eq!(posted.status, StatusCode::ACCEPTED, "{:?}", posted.body);
    wait_for_runs(&app, room_id, 2).await;

    let captured = timeouts.lock().unwrap();
    assert_eq!(captured.len(), 2, "两次运行都应被捕获：{captured:?}");
    assert_eq!(captured[0], Some(Duration::from_secs(120)));
    assert_eq!(captured[1], None, "0 应映射为不限制");
    drop(captured);
    let settings = get(&app, "/api/settings").await;
    assert_eq!(settings.body["settings"]["runTimeoutSecs"], 0);
}
