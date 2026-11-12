#![allow(dead_code)]

//! API 集成测试共享装配与请求辅助。

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use moon_server::adapters::AdapterFactory;
use moon_server::db::Db;
use moon_server::{build_app, AppState, AppStateOptions};
use serde_json::{json, Value};
use tower::ServiceExt;

pub struct TestApp {
    pub router: axum::Router,
    pub state: AppState,
    pub dir: tempfile::TempDir,
}

pub struct ApiResponse {
    pub status: StatusCode,
    pub body: Value,
}

pub async fn setup() -> TestApp {
    setup_with_adapter(None).await
}

pub async fn setup_with_adapter(adapter_factory: Option<AdapterFactory>) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("moon.db")).await.unwrap();
    let state = AppState::new_with_options(
        db,
        dir.path().join("sessions"),
        AppStateOptions {
            adapter_factory,
            hub: None,
        },
    )
    .await
    .unwrap();
    let router = build_app(state.clone());
    TestApp { router, state, dir }
}

async fn request(app: &TestApp, method: Method, uri: &str, body: Option<Value>) -> ApiResponse {
    let payload = match body {
        Some(value) => Body::from(serde_json::to_vec(&value).unwrap()),
        None => Body::empty(),
    };
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(payload)
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    ApiResponse { status, body }
}

pub async fn get(app: &TestApp, uri: &str) -> ApiResponse {
    request(app, Method::GET, uri, None).await
}

pub async fn post(app: &TestApp, uri: &str, body: Value) -> ApiResponse {
    request(app, Method::POST, uri, Some(body)).await
}

pub async fn post_empty(app: &TestApp, uri: &str) -> ApiResponse {
    request(app, Method::POST, uri, None).await
}

pub async fn patch(app: &TestApp, uri: &str, body: Value) -> ApiResponse {
    request(app, Method::PATCH, uri, Some(body)).await
}

pub async fn put(app: &TestApp, uri: &str, body: Value) -> ApiResponse {
    request(app, Method::PUT, uri, Some(body)).await
}

pub async fn delete(app: &TestApp, uri: &str) -> ApiResponse {
    request(app, Method::DELETE, uri, None).await
}

pub async fn builtin_moderator_id(app: &TestApp) -> i64 {
    let response = get(app, "/api/agents").await;
    assert_eq!(response.status, StatusCode::OK);
    response
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["builtinRole"] == "moderator")
        .and_then(|agent| agent["id"].as_i64())
        .expect("builtin moderator")
}

pub async fn create_agent(app: &TestApp, name: &str) -> Value {
    let response = post(
        app,
        "/api/agents",
        json!({
            "name": name,
            "persona": format!("{name} 的人设"),
            "harness": "fake",
        }),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "create agent failed: {:?}",
        response.body
    );
    response.body
}

pub async fn create_room(app: &TestApp, name: &str, member_ids: &[i64]) -> Value {
    let response = post(
        app,
        "/api/rooms",
        json!({
            "name": name,
            "workspaceDir": app.dir.path().to_string_lossy(),
            "memberIds": member_ids,
            "permissionMode": "auto",
        }),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "create room failed: {:?}",
        response.body
    );
    response.body
}

pub async fn enable_reuse(app: &TestApp, room_id: i64, agent_id: i64) {
    let response = put(
        app,
        &format!("/api/rooms/{room_id}/members/{agent_id}/session-settings"),
        json!({ "reuseSession": true }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
}

pub fn workspace(app: &TestApp) -> String {
    app.dir.path().to_string_lossy().into_owned()
}

/// 轮询消息直到出现终态智能体消息（成功或失败）。
pub async fn wait_for_agent_reply(app: &TestApp, room_id: i64) -> Vec<Value> {
    for _ in 0..200 {
        let response = get(app, &format!("/api/rooms/{room_id}/messages")).await;
        assert_eq!(response.status, StatusCode::OK);
        let messages = response.body.as_array().unwrap().clone();
        let settled = messages.iter().any(|message| {
            message["senderType"] == "agent"
                && (message["status"] == "final" || message["status"] == "failed")
        });
        if settled {
            return messages;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("等待智能体回复超时");
}

/// 轮询运行列表直到至少 `expected` 条且全部进入终态。
pub async fn wait_for_runs(app: &TestApp, room_id: i64, expected: usize) -> Vec<Value> {
    for _ in 0..250 {
        let response = get(app, &format!("/api/rooms/{room_id}/runs")).await;
        assert_eq!(response.status, StatusCode::OK);
        let runs = response.body.as_array().unwrap().clone();
        if runs.len() >= expected && runs.iter().all(|run| run["status"] != "running") {
            return runs;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("等待运行完成超时");
}

/// 轮询直到出现包含指定文本的系统消息。
pub async fn wait_for_system_message(app: &TestApp, room_id: i64, needle: &str) -> Vec<Value> {
    for _ in 0..200 {
        let response = get(app, &format!("/api/rooms/{room_id}/messages")).await;
        assert_eq!(response.status, StatusCode::OK);
        let messages = response.body.as_array().unwrap().clone();
        let found = messages.iter().any(|message| {
            message["senderType"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains(needle))
        });
        if found {
            return messages;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("等待系统消息超时：{needle}");
}
