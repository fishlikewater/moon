//! 智能体创建助手：草稿协议、校验与聊天路由。

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{post, setup_with_adapter};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, ModelConfig};
use moon_server::domain::harness::{HarnessModelCatalog, HarnessModelOption};
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::services::agent_builder::{
    build_agent_builder_prompt, extract_agent_draft_block, resolve_agent_draft,
};
use serde_json::{json, Value};

fn catalog() -> HarnessModelCatalog {
    HarnessModelCatalog {
        harness: moon_server::domain::agent::HarnessKind::Zcode,
        available: true,
        models: vec![HarnessModelOption {
            key: "[\"prov-a\",\"model-1\"]".to_string(),
            provider_id: "prov-a".to_string(),
            provider_name: "Alpha".to_string(),
            model_id: "model-1".to_string(),
            reasoning_levels: vec!["low".to_string(), "high".to_string()],
            default_reasoning_level: Some("low".to_string()),
        }],
        message: None,
    }
}

fn names(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn draft_block_extraction_uses_last_block_and_strips_all() {
    let text = "先给一版。\n```moon-agent-draft\n{\"name\":\"旧\",\"persona\":\"p\",\"harness\":\"zcode\"}\n```\n\n\n更新：\n```MOON-AGENT-DRAFT\n{\"name\":\"新\",\"persona\":\"p2\",\"harness\":\"codex\"}\n```\n完成。";
    let extracted = extract_agent_draft_block(text);
    assert!(!extracted.malformed);
    assert_eq!(extracted.raw.as_ref().unwrap()["name"], "新");
    assert!(!extracted.reply.contains("moon-agent-draft"));
    assert!(!extracted.reply.contains("\"name\":\"旧\""));
    assert!(!extracted.reply.contains("\"name\":\"新\""));
    assert!(extracted.reply.contains("先给一版。"));
    assert!(extracted.reply.contains("完成。"));
    assert!(!extracted.reply.contains("\n\n\n"));

    let malformed = extract_agent_draft_block("```moon-agent-draft\n{not json}\n```\n还有文字");
    assert!(malformed.malformed);
    assert!(malformed.raw.is_none());
    assert_eq!(malformed.reply, "还有文字");

    let missing = extract_agent_draft_block("没有协议块");
    assert!(missing.raw.is_none());
    assert!(!missing.malformed);
    assert_eq!(missing.reply, "没有协议块");
}

#[test]
fn draft_resolution_validates_model_provider_and_reasoning() {
    let catalog = catalog();

    let resolved = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "测试助手",
            "persona": "负责测试",
            "harness": "zcode",
            "model": "model-1",
            "reasoningLevel": "high"
        }))
        .unwrap(),
        &catalog,
        &names(&[]),
    );
    assert!(resolved.notes.is_empty());
    let config = resolved.draft.model_config.unwrap();
    assert_eq!(config["model"], "model-1");
    assert_eq!(config["providerId"], "prov-a");
    assert_eq!(config["reasoningLevel"], "high");

    let unknown = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "x",
            "persona": "p",
            "harness": "zcode",
            "model": "not-in-catalog"
        }))
        .unwrap(),
        &catalog,
        &names(&[]),
    );
    assert!(unknown.draft.model_config.is_none());
    assert!(unknown.notes.iter().any(|note| note.contains("不在 zcode 模型目录中")));

    let unsupported_level = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "x",
            "persona": "p",
            "harness": "zcode",
            "model": "model-1",
            "reasoningLevel": "max"
        }))
        .unwrap(),
        &catalog,
        &names(&[]),
    );
    let config = unsupported_level.draft.model_config.unwrap();
    assert!(config.get("reasoningLevel").is_none());
    assert!(unsupported_level
        .notes
        .iter()
        .any(|note| note.contains("不支持思考等级")));

    let provider_mismatch = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "x",
            "persona": "p",
            "harness": "zcode",
            "model": "model-1",
            "providerId": "prov-other"
        }))
        .unwrap(),
        &catalog,
        &names(&[]),
    );
    let config = provider_mismatch.draft.model_config.unwrap();
    assert_eq!(config["providerId"], "prov-a");
    assert!(provider_mismatch
        .notes
        .iter()
        .any(|note| note.contains("已按目录纠正")));

    let duplicate = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "已有",
            "persona": "p",
            "harness": "zcode"
        }))
        .unwrap(),
        &catalog,
        &names(&["已有"]),
    );
    assert!(duplicate.notes.iter().any(|note| note.contains("已存在")));

    let fallback = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "x",
            "persona": "p",
            "harness": "unknown-harness"
        }))
        .unwrap(),
        &catalog,
        &names(&[]),
    );
    assert_eq!(
        serde_json::to_value(&fallback.draft.harness).unwrap(),
        "zcode"
    );
    assert!(fallback.notes.iter().any(|note| note.contains("已回退为 zcode")));

    let kimi = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "x",
            "persona": "p",
            "harness": "kimi-code"
        }))
        .unwrap(),
        &catalog,
        &names(&[]),
    );
    assert!(kimi
        .notes
        .iter()
        .any(|note| note.contains("运行适配器尚未实现")));
}

#[test]
fn draft_harness_accepts_acp_and_all_supported_kinds() {
    for requested in ["zcode", "codex", "kimi-code", "fake", "acp"] {
        let resolved = resolve_agent_draft(
            &serde_json::from_value(json!({
                "name": "x",
                "persona": "p",
                "harness": requested
            }))
            .unwrap(),
            &catalog(),
            &names(&[]),
        );
        assert_eq!(
            serde_json::to_value(&resolved.draft.harness).unwrap(),
            requested
        );
        assert!(!resolved
            .notes
            .iter()
            .any(|note| note.contains("已回退为 zcode")));
    }
}

#[test]
fn draft_acp_model_is_not_validated_against_zcode_catalog() {
    let resolved = resolve_agent_draft(
        &serde_json::from_value(json!({
            "name": "x",
            "persona": "p",
            "harness": "acp",
            "model": "acp/model-x",
            "providerId": "acp-prov",
            "reasoningLevel": "high"
        }))
        .unwrap(),
        &catalog(),
        &names(&[]),
    );
    assert_eq!(
        serde_json::to_value(&resolved.draft.harness).unwrap(),
        "acp"
    );
    let config = resolved.draft.model_config.as_ref().unwrap();
    assert_eq!(config["model"], "acp/model-x");
    assert_eq!(config["providerId"], "acp-prov");
    assert_eq!(config["reasoningLevel"], "high");
    assert!(resolved
        .notes
        .iter()
        .any(|note| note.contains("未做全局校验")));
    assert!(!resolved
        .notes
        .iter()
        .any(|note| note.contains("不在") && note.contains("模型目录")));
}

#[test]
fn prompt_includes_conversation_names_and_safe_model_fields() {
    let messages = vec![
        moon_server::domain::agent_builder::AgentBuilderMessage {
            role: "user".to_string(),
            content: "做个审查助手".to_string(),
        },
        moon_server::domain::agent_builder::AgentBuilderMessage {
            role: "assistant".to_string(),
            content: "好的。\n```moon-agent-draft\n{\"name\":\"审查者\",\"persona\":\"p\",\"harness\":\"zcode\"}\n```".to_string(),
        },
        moon_server::domain::agent_builder::AgentBuilderMessage {
            role: "user".to_string(),
            content: "再加一点测试能力".to_string(),
        },
    ];
    let existing: Vec<String> = names(&["主持人"]).into_iter().collect();
    let prompt = build_agent_builder_prompt(&messages, &catalog(), &existing);
    assert!(prompt.contains("用户：做个审查助手"));
    assert!(prompt.contains("助手：好的。"));
    assert!(prompt.contains("用户：再加一点测试能力"));
    assert!(!prompt.contains("moon-agent-draft\n{\"name\":\"审查者\""));
    assert!(prompt.contains("现有智能体名称（新名称不能重复）：主持人"));
    assert!(prompt.contains("Alpha / prov-a / model-1 / 思考等级：low, high"));
    assert!(prompt.contains("上一版草稿："));
    assert!(prompt.contains("zcode/codex/kimi-code/fake/acp"));
    assert!(prompt.contains("harness 为 acp 时 model 留空"));
    assert!(!prompt.contains("sk-"));
}

struct CaptureAdapter {
    runs: Arc<Mutex<Vec<(String, Option<ModelConfig>)>>>,
}

impl HarnessAdapter for CaptureAdapter {
    fn kind(&self) -> moon_server::domain::agent::HarnessKind {
        moon_server::domain::agent::HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.runs
            .lock()
            .unwrap()
            .push((input.prompt.clone(), input.config.clone()));
        let prompt = input.prompt;
        let events = if prompt.contains("失败") {
            vec![RunEvent::Error("boom".to_string())]
        } else if prompt.contains("空输出") {
            vec![RunEvent::Final(String::new())]
        } else if prompt.contains("请报告用量") {
            // 工具 + 用量事件只进入运行详情，不改变回复文本。
            vec![
                RunEvent::ToolStart {
                    tool_call_id: "tool-1".to_string(),
                    name: "read".to_string(),
                    input: json!({}),
                },
                RunEvent::Usage {
                    token_count: Some(1234),
                    context_size: Some(4096),
                    tool_call_count: Some(2),
                    duration_ms: Some(880),
                },
                RunEvent::Final("完成。".to_string()),
            ]
        } else {
            vec![RunEvent::Final(
                "已整理好草稿。\n```moon-agent-draft\n{\"name\":\"测试助手\",\"persona\":\"负责测试的智能体\",\"harness\":\"zcode\",\"model\":\"model-9\",\"providerId\":\"prov-x\",\"reasoningLevel\":\"high\"}\n```"
                    .to_string(),
            )]
        };
        Box::pin(tokio_stream::iter(
            events
                .into_iter()
                .map(Ok::<_, moon_server::error::AppError>)
                .collect::<Vec<_>>(),
        ))
    }
}

#[tokio::test]
async fn chat_route_returns_validated_draft_and_maps_failures() {
    // 固定到不存在的 zcode 配置，避免读取开发机真实目录影响断言。
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(
        "ZCODE_PERSONAL_PROVIDER_CONFIG_FILE",
        dir.path().join("missing.json"),
    );
    let runs = Arc::new(Mutex::new(Vec::new()));
    let factory: AdapterFactory = {
        let runs = runs.clone();
        Arc::new(move |_agent: &Agent| {
            Ok(Arc::new(CaptureAdapter { runs: runs.clone() }) as Arc<dyn HarnessAdapter>)
        })
    };
    let app = setup_with_adapter(Some(factory)).await;

    let response = post(
        &app,
        "/api/agent-builder/chat",
        json!({
            "messages": [{"role": "user", "content": "需要一个测试助手"}],
            "assistantModelConfig": {"model": "assistant-model", "providerId": "p"}
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["reply"], "已整理好草稿。");
    assert_eq!(response.body["draft"]["name"], "测试助手");
    assert_eq!(response.body["draft"]["persona"], "负责测试的智能体");
    assert_eq!(response.body["draft"]["harness"], "zcode");
    assert_eq!(response.body["draft"]["modelConfig"]["model"], "model-9");
    assert_eq!(response.body["draft"]["modelConfig"]["providerId"], "prov-x");
    assert_eq!(
        response.body["draft"]["modelConfig"]["reasoningLevel"],
        "high"
    );
    assert!(response.body["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("ZCode 模型配置")));
    // 没有 Usage 事件时仍返回耗时，其余用量字段为空而不是伪造值。
    assert_eq!(response.body["run"]["status"], "completed");
    assert_eq!(response.body["run"]["model"], "assistant-model");
    assert!(
        response.body["run"]["durationMs"]
            .as_i64()
            .is_some_and(|value| value >= 0)
    );
    assert!(response.body["run"]["tokenCount"].is_null());
    assert!(response.body["run"]["toolCallCount"].is_null());

    let captured = runs.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].0.contains("用户：需要一个测试助手"));
    assert_eq!(
        captured[0].1.as_ref().unwrap()["model"],
        Value::String("assistant-model".to_string())
    );

    let usage = post(
        &app,
        "/api/agent-builder/chat",
        json!({
            "messages": [{"role": "user", "content": "请报告用量"}],
            "assistantModelConfig": {"model": "assistant-model", "providerId": "p"}
        }),
    )
    .await;
    assert_eq!(usage.status, StatusCode::OK, "{:?}", usage.body);
    assert_eq!(usage.body["run"]["status"], "completed");
    assert_eq!(usage.body["run"]["model"], "assistant-model");
    assert_eq!(usage.body["run"]["durationMs"], 880);
    assert_eq!(usage.body["run"]["tokenCount"], 1234);
    assert_eq!(usage.body["run"]["toolCallCount"], 2);
    assert_eq!(usage.body["run"]["contextSize"], 4096);

    for body in [
        json!({"messages": []}),
        json!({"messages": [{"role": "assistant", "content": "hi"}]}),
        json!({"messages": [{"role": "user", "content": ""}]}),
        json!({"messages": "bad"}),
        json!({"messages": [{"role": "user", "content": "x"}], "assistantModelConfig": "bad"}),
    ] {
        let invalid = post(&app, "/api/agent-builder/chat", body).await;
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{:?}", invalid.body);
        assert_eq!(invalid.body["error"]["code"], "VALIDATION_FAILED");
    }

    let failed = post(
        &app,
        "/api/agent-builder/chat",
        json!({"messages": [{"role": "user", "content": "请失败"}]}),
    )
    .await;
    assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failed.body["error"]["code"], "HARNESS_RUN_FAILED");
    assert_eq!(
        failed.body["error"]["message"],
        "创建助手运行失败：boom"
    );

    let empty = post(
        &app,
        "/api/agent-builder/chat",
        json!({"messages": [{"role": "user", "content": "请空输出"}]}),
    )
    .await;
    assert_eq!(empty.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(empty.body["error"]["code"], "HARNESS_RUN_FAILED");
    assert_eq!(
        empty.body["error"]["message"],
        "创建助手未返回任何内容"
    );

    std::env::remove_var("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE");
}
