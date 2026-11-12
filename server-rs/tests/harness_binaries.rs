//! Harness 可执行文件解析 API 与设置覆盖的集成测试。

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use serde_json::json;

use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};

mod common;

use common::*;

const BINARIES_SOURCES: [&str; 7] = [
    "configured",
    "protocol",
    "uninstall",
    "app_paths",
    "path",
    "vendor_default",
    "fallback",
];

struct RecordingAdapter {
    inputs: Arc<Mutex<Vec<RunInput>>>,
}

impl HarnessAdapter for RecordingAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.inputs.lock().unwrap().push(input);
        Box::pin(tokio_stream::iter(
            vec![Ok(RunEvent::Final("ok".to_string()))].into_iter(),
        ))
    }
}

fn recording_factory() -> (AdapterFactory, Arc<Mutex<Vec<RunInput>>>) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let captured = inputs.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(RecordingAdapter {
            inputs: captured.clone(),
        }))
    });
    (factory, inputs)
}

fn assert_status_shape(body: &serde_json::Value, name: &str) {
    let status = &body[name];
    assert!(
        status["path"].is_string(),
        "{name} 缺少 path：{body}"
    );
    let source = status["source"]
        .as_str()
        .unwrap_or_else(|| panic!("{name} 缺少 source：{body}"));
    assert!(
        BINARIES_SOURCES.contains(&source),
        "{name} 来源未知：{source}"
    );
    assert!(status["exists"].is_boolean(), "{name} 缺少 exists：{body}");
    assert!(status["searched"].is_array(), "{name} 缺少 searched：{body}");
}

#[tokio::test]
async fn settings_default_bin_paths_are_empty() {
    let app = setup().await;
    let response = get(&app, "/api/settings").await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["settings"]["zcodeBinPath"], "");
    assert_eq!(response.body["settings"]["codexBinPath"], "");
}

#[tokio::test]
async fn binaries_endpoint_reports_both_harnesses() {
    let app = setup().await;
    let response = get(&app, "/api/harnesses/binaries").await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_status_shape(&response.body, "zcode");
    assert_status_shape(&response.body, "codex");
}

#[tokio::test]
async fn configured_override_wins_and_clearing_restores_detection() {
    let app = setup().await;
    // 真实文件才能验证 exists 探测；放临时目录避免依赖本机安装。
    let fake_zcode = app.dir.path().join("zcode-override.cjs");
    std::fs::write(&fake_zcode, b"// fake").unwrap();
    let fake_path = fake_zcode.to_string_lossy().to_string();

    let patched = patch(&app, "/api/settings", json!({ "zcodeBinPath": fake_path })).await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
    assert_eq!(patched.body["zcodeBinPath"], fake_path);

    let fetched = get(&app, "/api/settings").await;
    assert_eq!(fetched.body["settings"]["zcodeBinPath"], fake_path);

    let binaries = get(&app, "/api/harnesses/binaries").await;
    assert_eq!(binaries.status, StatusCode::OK, "{:?}", binaries.body);
    assert_eq!(binaries.body["zcode"]["source"], "configured");
    assert_eq!(binaries.body["zcode"]["path"], fake_path);
    assert_eq!(binaries.body["zcode"]["exists"], true);

    // 清空覆盖后回到自动检测：来源不再是 configured。
    let cleared = patch(&app, "/api/settings", json!({ "zcodeBinPath": "" })).await;
    assert_eq!(cleared.status, StatusCode::OK, "{:?}", cleared.body);
    let auto = get(&app, "/api/harnesses/binaries").await;
    assert_ne!(auto.body["zcode"]["source"], "configured", "{:?}", auto.body);
    assert!(!auto.body["zcode"]["path"].as_str().unwrap_or("").is_empty());
}

#[tokio::test]
async fn overlong_bin_path_is_rejected_without_writing() {
    let app = setup().await;
    let before = get(&app, "/api/settings").await;

    let too_long = "x".repeat(1_025);
    let rejected = patch(&app, "/api/settings", json!({ "codexBinPath": too_long })).await;
    assert_eq!(
        rejected.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        rejected.body
    );

    let after = get(&app, "/api/settings").await;
    assert_eq!(after.body["settings"], before.body["settings"]);
}

#[tokio::test]
async fn bin_path_accepts_surrounding_whitespace_and_trims_it() {
    let app = setup().await;
    let patched = patch(
        &app,
        "/api/settings",
        json!({ "codexBinPath": "  D:\\tools\\codex.exe  " }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
    assert_eq!(patched.body["codexBinPath"], "D:\\tools\\codex.exe");

    let binaries = get(&app, "/api/harnesses/binaries").await;
    assert_eq!(binaries.body["codex"]["source"], "configured");
    assert_eq!(binaries.body["codex"]["path"], "D:\\tools\\codex.exe");
    assert_eq!(binaries.body["codex"]["exists"], false);
}

#[tokio::test]
async fn configured_override_reaches_harness_run_input() {
    let (factory, inputs) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = post(
        &app,
        "/api/agents",
        json!({ "name": "zcode观察员", "persona": "测试", "harness": "zcode" }),
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{:?}", agent.body);
    let agent_id = agent.body["id"].as_i64().unwrap();
    let room = create_room(&app, "覆盖房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let patched = patch(
        &app,
        "/api/settings",
        json!({ "zcodeBinPath": r"D:\custom\zcode.cjs" }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);

    let sent = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@zcode观察员 你好" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);

    wait_for_runs(&app, room_id, 1).await;
    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    let config = captured[0]
        .config
        .as_ref()
        .expect("覆盖路径应写入运行配置");
    assert_eq!(config["zcodeBin"], r"D:\custom\zcode.cjs");
}
