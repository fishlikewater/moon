//! 模型供应商注册表 API 与 acp 智能体绑定校验集成测试。

use axum::http::StatusCode;
use serde_json::{json, Value};

mod common;

use common::*;

fn valid_provider() -> Value {
    json!({
        "name": "DeepSeek",
        "protocol": "anthropic",
        "baseUrl": "https://api.deepseek.com/anthropic",
        "apiKey": "sk-test",
        "enabled": true,
        "models": [
            {
                "id": "deepseek-chat",
                "name": "DeepSeek Chat",
                "contextWindow": 65536,
                "reasoningLevels": ["standard"],
                "inputTypes": ["text"]
            },
            {
                "id": "deepseek-reasoner",
                "name": "DeepSeek Reasoner",
                "reasoningLevels": [],
                "inputTypes": ["text", "image"]
            }
        ]
    })
}

#[tokio::test]
async fn provider_crud_and_advanced_model_attributes() {
    let app = setup().await;

    // 校验矩阵：空名、坏协议、空 baseUrl、坏 scheme、坏 enabled。
    for (label, body) in [
        (
            "空名称",
            json!({"name": "  ", "protocol": "openai", "baseUrl": "https://x"}),
        ),
        (
            "坏协议",
            json!({"name": "p", "protocol": "gemini", "baseUrl": "https://x"}),
        ),
        ("缺协议", json!({"name": "p", "baseUrl": "https://x"})),
        (
            "空 baseUrl",
            json!({"name": "p", "protocol": "openai", "baseUrl": "  "}),
        ),
        (
            "坏 scheme",
            json!({"name": "p", "protocol": "openai", "baseUrl": "api.example.com"}),
        ),
        (
            "坏 enabled",
            json!({"name": "p", "protocol": "openai", "baseUrl": "https://x", "enabled": "yes"}),
        ),
    ] {
        let response = post(&app, "/api/api-providers", body).await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{label}: {:?}",
            response.body
        );
    }

    // 模型高级属性校验矩阵。
    for (label, models) in [
        (
            "contextWindow 非正整数",
            json!([{"id": "m", "name": "M", "contextWindow": 0}]),
        ),
        (
            "contextWindow 非整数",
            json!([{"id": "m", "name": "M", "contextWindow": 1.5}]),
        ),
        (
            "inputTypes 越界",
            json!([{"id": "m", "name": "M", "inputTypes": ["audio"]}]),
        ),
        ("空模型 id", json!([{"id": "  ", "name": "M"}])),
        ("缺 id", json!([{"name": "M"}])),
        (
            "重复 id",
            json!([{"id": "m", "name": "A"}, {"id": "m", "name": "B"}]),
        ),
        ("models 非数组", json!({"id": "m"})),
    ] {
        let response = post(
            &app,
            "/api/api-providers",
            json!({"name": "p", "protocol": "openai", "baseUrl": "https://x", "models": models}),
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{label}: {:?}",
            response.body
        );
    }

    // 正常创建并回读（含高级属性回显）。
    let created = post(&app, "/api/api-providers", valid_provider()).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let provider_id = created.body["id"].as_i64().unwrap();
    assert_eq!(created.body["name"], "DeepSeek");
    assert_eq!(created.body["protocol"], "anthropic");
    assert_eq!(
        created.body["baseUrl"],
        "https://api.deepseek.com/anthropic"
    );
    assert_eq!(created.body["agentCount"].as_i64(), Some(0));
    let models = created.body["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["contextWindow"].as_i64(), Some(65536));
    assert_eq!(models[0]["reasoningLevels"][0], "standard");
    assert_eq!(models[1]["inputTypes"][1], "image");
    // apiKey 随对象返回（本地明文先例），但不为空。
    assert_eq!(created.body["apiKey"], "sk-test");

    // GET 单体与列表一致。
    let fetched = get(&app, &format!("/api/api-providers/{provider_id}")).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.body["id"].as_i64(), Some(provider_id));

    // 部分更新：只改 enabled，其余保留。
    let patched = patch(
        &app,
        &format!("/api/api-providers/{provider_id}"),
        json!({"enabled": false}),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
    assert_eq!(patched.body["enabled"].as_bool(), Some(false));
    assert_eq!(patched.body["name"], "DeepSeek");
    assert_eq!(patched.body["models"].as_array().unwrap().len(), 2);
    let patched_at = patched.body["updatedAt"].as_i64().unwrap();
    let reread = get(&app, &format!("/api/api-providers/{provider_id}")).await;
    assert_eq!(
        reread.body["updatedAt"].as_i64(),
        Some(patched_at),
        "PATCH 响应的 updatedAt 应与回读一致"
    );

    // 整体替换 models 为空数组。
    let cleared = patch(
        &app,
        &format!("/api/api-providers/{provider_id}"),
        json!({"models": []}),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK);
    assert_eq!(cleared.body["models"].as_array().unwrap().len(), 0);

    // 更新与删除不存在的 id → 404。
    let missing = patch(&app, "/api/api-providers/999999", json!({"enabled": true})).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let missing_delete = delete(&app, "/api/api-providers/999999").await;
    assert_eq!(missing_delete.status, StatusCode::NOT_FOUND);

    // 未引用删除 → 204。
    let removed = delete(&app, &format!("/api/api-providers/{provider_id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let after = get(&app, &format!("/api/api-providers/{provider_id}")).await;
    assert_eq!(after.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn provider_delete_blocked_while_agent_bound() {
    let app = setup().await;

    let created = post(&app, "/api/api-providers", valid_provider()).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let provider_id = created.body["id"].as_i64().unwrap();

    // agent（acp harness）绑定供应商。
    let harness = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "绑定用 agent", "command": "node"}),
    )
    .await;
    let harness_id = harness.body["id"].as_i64().unwrap();
    let agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "绑定供应商的智能体",
            "persona": "测试",
            "harness": "acp",
            "customHarnessId": harness_id,
            "modelConfig": {"apiProviderId": provider_id, "model": "deepseek-chat"}
        }),
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{:?}", agent.body);

    // 列表 agentCount 实时统计。
    let list = get(&app, "/api/api-providers").await;
    let item = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"].as_i64() == Some(provider_id))
        .unwrap();
    assert_eq!(item["agentCount"].as_i64(), Some(1), "{:?}", item);

    // 被引用删除 → 409。
    let blocked = delete(&app, &format!("/api/api-providers/{provider_id}")).await;
    assert_eq!(blocked.status, StatusCode::CONFLICT, "{:?}", blocked.body);
    assert!(blocked.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("绑定"));

    // 解绑（agent 换 modelConfig 去掉 apiProviderId）后删除成功。
    let agent_id = agent.body["id"].as_i64().unwrap();
    let unbind = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({"modelConfig": {"model": "deepseek-chat"}}),
    )
    .await;
    assert_eq!(unbind.status, StatusCode::OK, "{:?}", unbind.body);
    let freed = delete(&app, &format!("/api/api-providers/{provider_id}")).await;
    assert_eq!(freed.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn agent_provider_binding_validation_matrix() {
    let app = setup().await;

    let created = post(&app, "/api/api-providers", valid_provider()).await;
    let provider_id = created.body["id"].as_i64().unwrap();
    let harness = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "校验用 agent", "command": "node"}),
    )
    .await;
    let harness_id = harness.body["id"].as_i64().unwrap();

    // acp 绑定不存在的供应商 → 400。
    let bad_ref = post(
        &app,
        "/api/agents",
        json!({
            "name": "坏绑定", "persona": "p", "harness": "acp",
            "customHarnessId": harness_id,
            "modelConfig": {"apiProviderId": 999999}
        }),
    )
    .await;
    assert_eq!(
        bad_ref.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        bad_ref.body
    );

    // apiProviderId 非整数 → 400。
    let bad_type = post(
        &app,
        "/api/agents",
        json!({
            "name": "坏类型", "persona": "p", "harness": "acp",
            "customHarnessId": harness_id,
            "modelConfig": {"apiProviderId": "1"}
        }),
    )
    .await;
    assert_eq!(
        bad_type.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        bad_type.body
    );

    // 非 acp harness 携带 apiProviderId → 400。
    let wrong_harness = post(
        &app,
        "/api/agents",
        json!({
            "name": "非 acp 绑定", "persona": "p", "harness": "codex",
            "modelConfig": {"apiProviderId": provider_id}
        }),
    )
    .await;
    assert_eq!(
        wrong_harness.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        wrong_harness.body
    );

    // 合法绑定成功；harness 离开 acp 时 modelConfig 里残留的
    // apiProviderId 自动清空（镜像 customHarnessId 清空语义）。
    let agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "合法绑定", "persona": "p", "harness": "acp",
            "customHarnessId": harness_id,
            "modelConfig": {"apiProviderId": provider_id, "model": "deepseek-chat"}
        }),
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{:?}", agent.body);
    let agent_id = agent.body["id"].as_i64().unwrap();

    let switch = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({"harness": "codex"}),
    )
    .await;
    assert_eq!(switch.status, StatusCode::OK, "{:?}", switch.body);
    let config = switch.body["modelConfig"].as_object().unwrap();
    assert!(
        !config.contains_key("apiProviderId"),
        "离开 acp 后残留绑定应被清空：{config:?}"
    );
    assert_eq!(
        config["model"], "deepseek-chat",
        "其余 modelConfig 字段保留"
    );

    // 供应商引用计数归零。
    let fetched = get(&app, &format!("/api/api-providers/{provider_id}")).await;
    assert_eq!(fetched.body["agentCount"].as_i64(), Some(0));
}

/// F1 回归：existing 仍绑定供应商时，单次 PATCH 同时离开 acp 并显式
/// `modelConfig: null`，配置必须被清空——残留 apiProviderId 不得复活旧配置
/// （否则 codex 会拿到 `-m deepseek-chat` 参数）。
#[tokio::test]
async fn explicit_null_model_config_clears_even_when_leaving_acp() {
    let app = setup().await;

    let created = post(&app, "/api/api-providers", valid_provider()).await;
    let provider_id = created.body["id"].as_i64().unwrap();
    let harness = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "显式清空用 agent", "command": "node"}),
    )
    .await;
    let harness_id = harness.body["id"].as_i64().unwrap();
    let agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "显式清空智能体", "persona": "p", "harness": "acp",
            "customHarnessId": harness_id,
            "modelConfig": {"apiProviderId": provider_id, "model": "deepseek-chat"}
        }),
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{:?}", agent.body);
    let agent_id = agent.body["id"].as_i64().unwrap();

    // 单次请求：harness 离开 acp + 显式清空 modelConfig。
    let reset = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({"harness": "codex", "modelConfig": null}),
    )
    .await;
    assert_eq!(reset.status, StatusCode::OK, "{:?}", reset.body);
    assert!(
        reset.body["modelConfig"].is_null(),
        "显式 null 应清空配置：{:?}",
        reset.body
    );
    let reread = get(&app, &format!("/api/agents/{agent_id}")).await;
    assert!(
        reread.body["modelConfig"].is_null(),
        "回读也应为 null：{:?}",
        reread.body
    );
}
