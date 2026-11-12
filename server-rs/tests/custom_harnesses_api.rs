//! 自定义 ACP agent 注册表 API 与 acp 智能体校验集成测试。

use axum::http::StatusCode;
use serde_json::{json, Value};

mod common;

use common::*;

#[tokio::test]
async fn custom_harness_crud_and_validation() {
    let app = setup().await;

    // 创建：空白名称拒绝。
    let blank = post(&app, "/api/custom-harnesses", json!({"name": "  ", "command": "node"})).await;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST, "{:?}", blank.body);
    let blank_command = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "x", "command": ""}),
    )
    .await;
    assert_eq!(
        blank_command.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        blank_command.body
    );

    // 创建：args/env 形状校验。
    let bad_args = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "x", "command": "node", "args": [1, 2]}),
    )
    .await;
    assert_eq!(bad_args.status, StatusCode::BAD_REQUEST);
    let bad_env = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "x", "command": "node", "env": {"KEY": 1}}),
    )
    .await;
    assert_eq!(bad_env.status, StatusCode::BAD_REQUEST);

    // 正常创建并回读。
    let created = post(
        &app,
        "/api/custom-harnesses",
        json!({
            "name": "我的 ACP agent",
            "command": "node",
            "args": ["agent.mjs", "--stdio"],
            "env": {"ACP_DEBUG": "1"},
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let harness_id = created.body["id"].as_i64().unwrap();
    assert_eq!(created.body["name"], "我的 ACP agent");
    assert_eq!(created.body["args"][0], "agent.mjs");
    assert_eq!(created.body["env"]["ACP_DEBUG"], "1");
    assert_eq!(created.body["agentCount"].as_i64(), Some(0), "新条目没有引用");

    // 再建一条验证列表按 id 升序。
    let second = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "第二个 agent", "command": "npx"}),
    )
    .await;
    assert_eq!(second.status, StatusCode::CREATED);
    let second_id = second.body["id"].as_i64().unwrap();
    assert!(second_id > harness_id, "自增 id 应递增");

    let list = get(&app, "/api/custom-harnesses").await;
    assert_eq!(list.status, StatusCode::OK);
    let items = list.body.as_array().unwrap();
    assert_eq!(items.len(), 2);
    let ids: Vec<i64> = items
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![harness_id, second_id], "列表应按 id 升序");
    assert_eq!(agent_count(&list.body, harness_id), 0);
    assert_eq!(agent_count(&list.body, second_id), 0);

    // 更新：部分字段。
    let updated = patch(
        &app,
        &format!("/api/custom-harnesses/{harness_id}"),
        json!({"command": "npx", "args": ["-y", "acp-agent"]}),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{:?}", updated.body);
    assert_eq!(updated.body["command"], "npx");
    assert_eq!(updated.body["name"], "我的 ACP agent", "未传字段应保持原值");
    assert_eq!(updated.body["args"][1], "acp-agent");
    assert_eq!(updated.body["agentCount"].as_i64(), Some(0));

    // 删除被引用的定义：409。
    let agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "acp-成员",
            "persona": "自定义 ACP 成员",
            "harness": "acp",
            "customHarnessId": harness_id,
        }),
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{:?}", agent.body);

    // 引用数是实时统计：创建引用后列表与 PATCH 响应都应反映。
    let list = get(&app, "/api/custom-harnesses").await;
    assert_eq!(agent_count(&list.body, harness_id), 1);
    let renamed = patch(
        &app,
        &format!("/api/custom-harnesses/{harness_id}"),
        json!({"name": "改名后的 agent"}),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
    assert_eq!(
        renamed.body["agentCount"].as_i64(),
        Some(1),
        "PATCH 响应应携带当前引用数"
    );

    let in_use = delete(&app, &format!("/api/custom-harnesses/{harness_id}")).await;
    assert_eq!(in_use.status, StatusCode::CONFLICT, "{:?}", in_use.body);

    // 第二个引用：引用数变为 2。
    let second_agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "acp-成员-2",
            "persona": "第二个引用",
            "harness": "acp",
            "customHarnessId": harness_id,
        }),
    )
    .await;
    assert_eq!(
        second_agent.status,
        StatusCode::CREATED,
        "{:?}",
        second_agent.body
    );
    let list = get(&app, "/api/custom-harnesses").await;
    assert_eq!(agent_count(&list.body, harness_id), 2);

    // 解绑后再删成功。
    let detached = patch(
        &app,
        &format!("/api/agents/{}", agent.body["id"].as_i64().unwrap()),
        json!({"harness": "fake"}),
    )
    .await;
    assert_eq!(detached.status, StatusCode::OK, "{:?}", detached.body);
    // 离开 acp 后绑定应被自动清空。
    assert_eq!(detached.body["customHarnessId"], Value::Null);

    let list = get(&app, "/api/custom-harnesses").await;
    assert_eq!(agent_count(&list.body, harness_id), 1);

    let detached_second = patch(
        &app,
        &format!("/api/agents/{}", second_agent.body["id"].as_i64().unwrap()),
        json!({"harness": "fake"}),
    )
    .await;
    assert_eq!(
        detached_second.status,
        StatusCode::OK,
        "{:?}",
        detached_second.body
    );
    let list = get(&app, "/api/custom-harnesses").await;
    assert_eq!(
        agent_count(&list.body, harness_id),
        0,
        "全部解绑后引用数应实时归零"
    );

    let removed = delete(&app, &format!("/api/custom-harnesses/{harness_id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);

    let missing = delete(&app, &format!("/api/custom-harnesses/{harness_id}")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

/// 从注册表响应中取指定 id 的引用数。
fn agent_count(body: &Value, harness_id: i64) -> i64 {
    body.as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"].as_i64() == Some(harness_id))
        .and_then(|item| item["agentCount"].as_i64())
        .unwrap_or_else(|| panic!("注册表响应缺少 id={harness_id} 的条目"))
}

#[tokio::test]
async fn acp_agent_requires_existing_custom_harness() {
    let app = setup().await;

    // 无 customHarnessId：400。
    let missing = post(
        &app,
        "/api/agents",
        json!({"name": "a1", "persona": "p", "harness": "acp"}),
    )
    .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST, "{:?}", missing.body);

    // 指向不存在的定义：400。
    let dangling = post(
        &app,
        "/api/agents",
        json!({"name": "a2", "persona": "p", "harness": "acp", "customHarnessId": 999}),
    )
    .await;
    assert_eq!(dangling.status, StatusCode::BAD_REQUEST);

    // 非 acp harness 显式携带绑定：400（不静默忽略）。
    let misplaced = post(
        &app,
        "/api/agents",
        json!({"name": "a3", "persona": "p", "harness": "fake", "customHarnessId": 1}),
    )
    .await;
    assert_eq!(misplaced.status, StatusCode::BAD_REQUEST);

    // 合法创建并回读一致。
    let harness = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "gemini", "command": "gemini", "args": ["--experimental-acp"]}),
    )
    .await;
    assert_eq!(harness.status, StatusCode::CREATED);
    let harness_id = harness.body["id"].as_i64().unwrap();

    let agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "acp-合法",
            "persona": "自定义 ACP 成员",
            "harness": "acp",
            "customHarnessId": harness_id,
        }),
    )
    .await;
    assert_eq!(agent.status, StatusCode::CREATED, "{:?}", agent.body);
    assert_eq!(agent.body["harness"], "acp");
    assert_eq!(agent.body["customHarnessId"].as_i64().unwrap(), harness_id);

    // 更新换成 acp 定义：生效。
    let other = post(
        &app,
        "/api/custom-harnesses",
        json!({"name": "claude-acp", "command": "npx", "args": ["claude-code-acp"]}),
    )
    .await;
    let other_id = other.body["id"].as_i64().unwrap();
    let rebound = patch(
        &app,
        &format!("/api/agents/{}", agent.body["id"].as_i64().unwrap()),
        json!({"customHarnessId": other_id}),
    )
    .await;
    assert_eq!(rebound.status, StatusCode::OK, "{:?}", rebound.body);
    assert_eq!(rebound.body["customHarnessId"].as_i64().unwrap(), other_id);
}

#[tokio::test]
async fn acp_model_catalog_is_unavailable_with_hint() {
    let app = setup().await;
    let response = get(&app, "/api/harnesses/acp/models").await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["harness"], "acp");
    assert_eq!(response.body["available"], false);
    let message = response.body["message"].as_str().unwrap();
    assert!(message.contains("agent"), "说明应指向 agent 侧模型：{message}");
}

const FAKE_AGENT_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/acp_fake_agent.mjs"
);

fn require_node() {
    let ok = std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    assert!(ok, "探测测试需要 PATH 中可用的 node 来运行 fake ACP agent");
}

#[tokio::test]
async fn probe_capabilities_persist_and_failure_keeps_cache() {
    require_node();
    let app = setup().await;

    let created = post(
        &app,
        "/api/custom-harnesses",
        json!({
            "name": "fake-acp",
            "command": "node",
            "args": [FAKE_AGENT_SCRIPT, "--mode", "record"],
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let id = created.body["id"].as_i64().unwrap();

    let probe = post_empty(&app, &format!("/api/custom-harnesses/{id}/probe")).await;
    assert_eq!(probe.status, StatusCode::OK, "{:?}", probe.body);
    assert_eq!(probe.body["models"].as_array().unwrap().len(), 2);
    assert_eq!(probe.body["modes"].as_array().unwrap().len(), 4);
    assert_eq!(probe.body["configOptions"].as_array().unwrap().len(), 2);
    assert_eq!(probe.body["currentModeId"], "default");
    assert_eq!(probe.body["currentModelId"], "m1");
    let probed_at = probe.body["probedAt"].as_i64().unwrap();
    assert!(probed_at > 0, "probedAt 应为 epoch 毫秒：{probed_at}");

    // GET 单条返回同一份缓存。
    let fetched = get(&app, &format!("/api/custom-harnesses/{id}")).await;
    assert_eq!(fetched.status, StatusCode::OK, "{:?}", fetched.body);
    assert_eq!(fetched.body["capabilities"]["models"][1]["value"], "m2");
    assert_eq!(fetched.body["probedAt"].as_i64(), Some(probed_at));

    // 失败探测不覆盖旧缓存。
    let broken = patch(
        &app,
        &format!("/api/custom-harnesses/{id}"),
        json!({"args": ["missing-script.mjs"]}),
    )
    .await;
    assert_eq!(broken.status, StatusCode::OK, "{:?}", broken.body);
    let failed = post_empty(&app, &format!("/api/custom-harnesses/{id}/probe")).await;
    assert_eq!(
        failed.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{:?}",
        failed.body
    );

    let cached = get(&app, &format!("/api/custom-harnesses/{id}")).await;
    assert_eq!(cached.body["capabilities"]["models"][0]["value"], "m1");
    assert_eq!(cached.body["probedAt"].as_i64(), Some(probed_at), "旧缓存不应被覆盖");
}

#[tokio::test]
async fn get_custom_harness_returns_404_when_missing() {
    let app = setup().await;
    let response = get(&app, "/api/custom-harnesses/999").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND, "{:?}", response.body);
}
