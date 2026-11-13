//! 真实 CLI 冒烟：需要本机已安装并登录 zcode / codex，默认不参与常规测试。
//!
//! 运行方式：
//! `cargo test --manifest-path server-rs/Cargo.toml --test real_cli_smoke -- --ignored --nocapture`
//!
//! zcode 用例通过 `MOON_SMOKE_PROVIDER_ID` / `MOON_SMOKE_MODEL`（可选
//! `MOON_SMOKE_REASONING_LEVEL`）注入模型选择，未设置时测试报错退出，
//! 避免 CLI 回落到额度不可用的默认 provider。codex 用例不注入模型选择。

use futures_util::StreamExt;
use moon_server::adapters::{
    CodexAdapter, HarnessAdapter, ZcodeAdapter,
};
use moon_server::domain::agent::ModelConfig;
use moon_server::domain::room::RoomPermissionMode;
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::domain::session::RunSessionContext;

/// 从环境变量注入模型选择，避免回落到默认 provider（其额度可能不可用）。
fn zcode_model_config() -> ModelConfig {
    let mut config = ModelConfig::new();
    for (key, env_key, example) in [
        (
            "providerId",
            "MOON_SMOKE_PROVIDER_ID",
            "account:bigmodel-start-plan",
        ),
        ("model", "MOON_SMOKE_MODEL", "GLM-5.3-Flash"),
    ] {
        let value = std::env::var(env_key).unwrap_or_else(|_| {
            panic!(
                "真实 zcode 冒烟需要设置 {env_key}（示例：{example}），否则 CLI 回落默认 provider"
            )
        });
        config.insert(key.to_string(), serde_json::Value::String(value));
    }
    if let Ok(level) = std::env::var("MOON_SMOKE_REASONING_LEVEL") {
        config.insert(
            "reasoningLevel".to_string(),
            serde_json::Value::String(level),
        );
    }
    config
}

async fn run_round(
    adapter: &dyn HarnessAdapter,
    prompt: &str,
    workspace: &str,
    config: Option<ModelConfig>,
    session: Option<RunSessionContext>,
) -> (String, Option<String>) {
    let mut stream = adapter.run(RunInput {
        prompt: prompt.to_string(),
        workspace_dir: workspace.to_string(),
        additional_directories: Vec::new(),
        config,
        session,
        permission_mode: Some(RoomPermissionMode::Plan),
        timeout: None,
    });
    let mut text = String::new();
    let mut session_id = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(RunEvent::TextDelta(delta)) => text.push_str(&delta),
            Ok(RunEvent::Final(final_text)) => text = final_text,
            Ok(RunEvent::Session(id)) => session_id = Some(id),
            Ok(RunEvent::Reasoning(_))
            | Ok(RunEvent::ToolStart { .. })
            | Ok(RunEvent::ToolEnd { .. })
            | Ok(RunEvent::Usage { .. })
            | Ok(RunEvent::Status(_)) => {}
            Ok(RunEvent::Error(message)) => panic!("harness error event: {message}"),
            Err(error) => panic!("harness run failed: {message}", message = error.message),
        }
    }
    println!("session={session_id:?} reply={text:?}");
    (text, session_id)
}

#[tokio::test]
#[ignore = "需要本机真实 zcode 与模型额度"]
async fn zcode_real_cli_round_trip_and_resume() {
    let workspace = tempfile::tempdir().unwrap();
    let storage = workspace.path().join("zcode-session");
    let adapter = ZcodeAdapter;

    let (first, session_id) = run_round(
        &adapter,
        "只回复两个字：收到",
        &workspace.path().to_string_lossy(),
        Some(zcode_model_config()),
        Some(RunSessionContext {
            harness_session_id: None,
            storage_dir: storage.to_string_lossy().into_owned(),
            last_context_message_id: 0,
        }),
    )
    .await;
    assert!(!first.trim().is_empty(), "zcode 必须有可见回复");
    let session_id = session_id.expect("zcode 必须报告 sessionId");

    let (second, resumed) = run_round(
        &adapter,
        "继续，只回复两个字：继续",
        &workspace.path().to_string_lossy(),
        Some(zcode_model_config()),
        Some(RunSessionContext {
            harness_session_id: Some(session_id.clone()),
            storage_dir: storage.to_string_lossy().into_owned(),
            last_context_message_id: 0,
        }),
    )
    .await;
    assert!(!second.trim().is_empty(), "zcode resume 必须有可见回复");
    assert_eq!(
        resumed.as_deref(),
        Some(session_id.as_str()),
        "resume 应复用同一会话"
    );
}

#[tokio::test]
#[ignore = "需要本机真实 codex 与模型额度"]
async fn codex_real_cli_round_trip_and_resume() {
    let workspace = tempfile::tempdir().unwrap();
    let storage = workspace.path().join("codex-session");
    let adapter = CodexAdapter;

    let (first, session_id) = run_round(
        &adapter,
        "只回复两个字：收到",
        &workspace.path().to_string_lossy(),
        None,
        Some(RunSessionContext {
            harness_session_id: None,
            storage_dir: storage.to_string_lossy().into_owned(),
            last_context_message_id: 0,
        }),
    )
    .await;
    assert!(!first.trim().is_empty(), "codex 必须有可见回复");
    let session_id = session_id.expect("codex 必须报告 thread id");

    let (second, resumed) = run_round(
        &adapter,
        "继续，只回复两个字：继续",
        &workspace.path().to_string_lossy(),
        None,
        Some(RunSessionContext {
            harness_session_id: Some(session_id.clone()),
            storage_dir: storage.to_string_lossy().into_owned(),
            last_context_message_id: 0,
        }),
    )
    .await;
    assert!(!second.trim().is_empty(), "codex resume 必须有可见回复");
    assert_eq!(
        resumed.as_deref(),
        Some(session_id.as_str()),
        "resume 应复用同一 thread"
    );
}
