//! 模型目录 API 与 zcode 运行期模型覆盖。

mod common;

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use common::{get, setup};
use moon_server::adapters::zcode_config::{
    create_zcode_model_override, ZcodeModelSelection,
};
use serde_json::{json, Value};

struct EnvRestore {
    values: Vec<(String, Option<String>)>,
}

impl EnvRestore {
    fn save(keys: &[&str]) -> Self {
        let values = keys
            .iter()
            .map(|key| ((*key).to_string(), std::env::var(key).ok()))
            .collect();
        Self { values }
    }

    fn set(&self, key: &str, value: &Path) {
        std::env::set_var(key, value);
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in &self.values {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn write_json(path: &Path, value: &Value) {
    // serde_json 只产生 UTF-8 字节；显式按字节写入，避免依赖平台默认编码。
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn write_text(path: &Path, text: &str) {
    std::fs::write(path, text.as_bytes()).unwrap();
}

fn sample_zcode_config() -> Value {
    json!({
        "schemaVersion": 1,
        "config": {
            "providerOrder": ["prov-a"],
            "providerConfigRules": {
                "providerRules": [
                    {
                        "providerId": "prov-a",
                        "providerName": "Alpha",
                        "config": {
                            "personalModelIds": ["model-1", "model-2"],
                            "modelOrder": ["model-2", "model-1"],
                            "api": {
                                "type": "openai",
                                "baseUrl": "https://secret.example",
                                "apiKey": "sk-test-secret"
                            }
                        }
                    },
                    {
                        "providerId": "disabled-provider",
                        "providerName": "Disabled",
                        "enabled": false,
                        "config": { "personalModelIds": ["hidden-model"] }
                    }
                ]
            },
            "modelConfigRules": {
                "providerModelRules": [
                    { "providerId": "prov-a", "modelId": "model-1", "config": {} },
                    { "providerId": "prov-a", "modelId": "model-2", "config": {} },
                    { "providerId": "disabled-provider", "modelId": "hidden-model", "config": {} }
                ],
                "manualProviderModelRules": []
            }
        }
    })
}

fn builtin_zcode_config() -> Value {
    json!({
        "schemaVersion": 1,
        "config": {
            "modelConfigRules": {
                "modelRules": [
                    {
                        "modelMatch": ".*",
                        "config": {
                            "optionSpecs": {
                                "reasoningLevel": { "values": ["disabled", "enabled"] }
                            }
                        }
                    },
                    {
                        "modelMatch": ".*model-1.*",
                        "config": {
                            "optionSpecs": {
                                "reasoningLevel": { "values": ["low", "high"] }
                            }
                        }
                    }
                ],
                "modelApiRules": [],
                "providerSiteRules": [],
                "templateModelRules": [],
                "builtinProviderModelRules": []
            }
        }
    })
}

#[tokio::test]
async fn harness_catalogs_and_zcode_model_override_align_with_contract() {
    let env = EnvRestore::save(&[
        "ZCODE_PERSONAL_PROVIDER_CONFIG_FILE",
        "MOON_ZCODE_BUILTIN_CONFIG",
        "CODEX_HOME",
        "KIMI_CODE_HOME",
        "KIMI_HOME",
    ]);
    let root = tempfile::tempdir().unwrap();
    let app = setup().await;

    // ===== zcode：provider 顺序、个人模型列表与内置思考等级 =====
    let zcode_config = root.path().join("provider_config.json");
    write_json(&zcode_config, &sample_zcode_config());
    let builtin = root.path().join("zcode-builtin.json");
    write_json(&builtin, &builtin_zcode_config());
    env.set("ZCODE_PERSONAL_PROVIDER_CONFIG_FILE", &zcode_config);
    env.set("MOON_ZCODE_BUILTIN_CONFIG", &builtin);

    let response = get(&app, "/api/harnesses/zcode/models").await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["harness"], "zcode");
    assert_eq!(response.body["available"], true);
    assert!(response.body["message"].is_null());
    let models = response.body["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["providerId"], "prov-a");
    assert_eq!(models[0]["providerName"], "Alpha");
    assert_eq!(models[0]["modelId"], "model-2");
    assert_eq!(models[0]["reasoningLevels"], json!(["disabled", "enabled"]));
    assert_eq!(models[1]["modelId"], "model-1");
    assert_eq!(models[1]["reasoningLevels"], json!(["low", "high"]));
    assert_eq!(
        models[0]["key"],
        Value::String("[\"prov-a\",\"model-2\"]".to_string())
    );
    let serialized = serde_json::to_string(&response.body).unwrap();
    assert!(!serialized.contains("sk-test-secret"));
    assert!(!serialized.contains("secret.example"));
    assert!(!serialized.contains("hidden-model"));
    assert!(!serialized.contains("disabled-provider"));

    // ===== zcode 运行期模型覆盖：副本写入 defaultModelSelection 并清理 =====
    let original_text = std::fs::read_to_string(&zcode_config).unwrap();
    let selection = ZcodeModelSelection {
        provider_id: "prov-a".to_string(),
        model_id: "model-1".to_string(),
        reasoning_level: Some("high".to_string()),
    };
    let model_override = create_zcode_model_override(&selection).unwrap();
    let override_path = PathBuf::from(&model_override.env[0].1);
    assert!(override_path.is_file());
    let copied: Value =
        serde_json::from_str(&std::fs::read_to_string(&override_path).unwrap()).unwrap();
    assert_eq!(
        copied["config"]["defaultModelSelection"],
        json!({
            "providerId": "prov-a",
            "modelId": "model-1",
            "options": { "reasoningLevel": "high" }
        })
    );
    assert_eq!(
        copied["config"]["providerConfigRules"]["providerRules"][0]["config"]["api"]["apiKey"],
        "sk-test-secret"
    );
    assert_eq!(
        std::fs::read_to_string(&zcode_config).unwrap(),
        original_text,
        "原配置必须保持不变"
    );
    let override_dir = override_path.parent().unwrap().to_path_buf();
    drop(model_override);
    assert!(!override_dir.exists(), "运行结束后必须清理临时配置目录");

    // ===== zcode 配置缺失：available=false 且消息安全 =====
    std::env::set_var(
        "ZCODE_PERSONAL_PROVIDER_CONFIG_FILE",
        root.path().join("missing.json"),
    );
    let missing = get(&app, "/api/harnesses/zcode/models").await;
    assert_eq!(missing.status, StatusCode::OK);
    assert_eq!(
        missing.body,
        json!({
            "harness": "zcode",
            "available": false,
            "models": [],
            "message": "未找到 ZCode 模型配置"
        })
    );

    // ===== codex：本地目录、思考等级与隐藏模型过滤 =====
    let codex_dir = root.path().join("codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    write_text(
        &codex_dir.join("config.toml"),
        "model_provider = \"openai\"\nmodel_catalog_json = \"catalog.json\"\n",
    );
    write_json(
        &codex_dir.join("catalog.json"),
        &json!({
            "models": [
                {
                    "slug": "gpt-6",
                    "default_reasoning_level": "high",
                    "supported_reasoning_levels": [
                        { "effort": "low" },
                        { "effort": "medium" },
                        { "effort": "high" }
                    ],
                    "visibility": "list",
                    "supported_in_api": true,
                    "api_key": "sk-codex-secret"
                },
                { "slug": "gpt-hidden", "visibility": "hidden" },
                { "slug": "gpt-no-api", "supported_in_api": false },
                { "slug": "gpt-6", "supported_reasoning_levels": [{ "effort": "low" }] }
            ]
        }),
    );
    env.set("CODEX_HOME", &codex_dir);
    let codex = get(&app, "/api/harnesses/codex/models").await;
    assert_eq!(codex.status, StatusCode::OK);
    assert_eq!(codex.body["available"], true);
    assert_eq!(codex.body["message"], "模型列表来自 Codex 本地配置。");
    let codex_models = codex.body["models"].as_array().unwrap();
    assert_eq!(codex_models.len(), 1);
    assert_eq!(codex_models[0]["providerId"], "openai");
    assert_eq!(codex_models[0]["providerName"], "Codex");
    assert_eq!(codex_models[0]["modelId"], "gpt-6");
    assert_eq!(
        codex_models[0]["reasoningLevels"],
        json!(["low", "medium", "high"])
    );
    assert_eq!(codex_models[0]["defaultReasoningLevel"], "high");
    assert!(!serde_json::to_string(&codex.body)
        .unwrap()
        .contains("sk-codex-secret"));

    // ===== kimi-code：模型节、多行思考等级与运行缺口提示 =====
    let kimi_dir = root.path().join("kimi");
    std::fs::create_dir_all(&kimi_dir).unwrap();
    write_text(
        &kimi_dir.join("config.toml"),
        &[
            r#"default_model = "moonshot/kimi-k3""#,
            "",
            r#"[models."moonshot/kimi-k3"]"#,
            r#"provider = "moonshot""#,
            r#"model = "kimi-k3""#,
            "support_efforts = [",
            r#"  "low","#,
            r#"  "medium", # 注释不影响解析"#,
            r#"  "high","#,
            "]",
            r#"default_effort = "medium""#,
            r#"api_key = "sk-kimi-secret" # 不得出现在结果中"#,
            "",
            r#"[models."custom/provider-model"]"#,
            "support_efforts = []",
            "",
            "[models.plain]",
            r#"model = "plain-model""#,
        ]
        .join("\n"),
    );
    std::env::remove_var("KIMI_HOME");
    env.set("KIMI_CODE_HOME", &kimi_dir);
    let kimi = get(&app, "/api/harnesses/kimi-code/models").await;
    assert_eq!(kimi.status, StatusCode::OK);
    assert_eq!(kimi.body["available"], true);
    assert!(kimi.body["message"]
        .as_str()
        .unwrap()
        .contains("运行适配器尚未实现"));
    let kimi_models = kimi.body["models"].as_array().unwrap();
    assert_eq!(kimi_models.len(), 3);
    assert_eq!(kimi_models[0]["providerId"], "moonshot");
    assert_eq!(kimi_models[0]["modelId"], "kimi-k3");
    assert_eq!(
        kimi_models[0]["reasoningLevels"],
        json!(["low", "medium", "high"])
    );
    assert_eq!(kimi_models[0]["defaultReasoningLevel"], "medium");
    assert_eq!(kimi_models[1]["providerId"], "custom");
    assert_eq!(kimi_models[1]["modelId"], "provider-model");
    assert_eq!(kimi_models[2]["providerId"], "plain");
    assert_eq!(kimi_models[2]["modelId"], "plain-model");
    assert!(!serde_json::to_string(&kimi.body)
        .unwrap()
        .contains("sk-kimi-secret"));

    // ===== fake 与非法 harness =====
    let fake = get(&app, "/api/harnesses/fake/models").await;
    assert_eq!(fake.status, StatusCode::OK);
    assert_eq!(fake.body["available"], false);
    assert_eq!(fake.body["models"], json!([]));
    assert!(fake.body["message"].as_str().unwrap().contains("fake"));

    let invalid = get(&app, "/api/harnesses/gpt-4/models").await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid.body["error"]["code"], "VALIDATION_FAILED");
    assert!(invalid.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("harness"));
}
