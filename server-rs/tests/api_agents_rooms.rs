mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{
    builtin_moderator_id, create_agent, create_room, delete, get, patch, post, setup, workspace,
};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn agent_crud_validates_contract() {
    let app = setup().await;

    let created = create_agent(&app, "研究员").await;
    assert_eq!(created["name"], "研究员");
    assert_eq!(created["harness"], "fake");
    assert_eq!(created["modelConfig"], serde_json::Value::Null);
    assert_eq!(created["builtinRole"], serde_json::Value::Null);
    assert!(created["createdAt"].as_i64().unwrap() > 0);

    let list = get(&app, "/api/agents").await;
    assert_eq!(list.status, StatusCode::OK);
    let agents = list.body.as_array().unwrap();
    assert_eq!(agents[0]["builtinRole"], "moderator", "内置主持人置顶");
    assert_eq!(agents[0]["name"], "Moon 主持人");
    assert!(agents.iter().any(|agent| agent["name"] == "研究员"));

    let invalid_harness = post(
        &app,
        "/api/agents",
        json!({ "name": "无效", "persona": "p", "harness": "nope" }),
    )
    .await;
    assert_eq!(invalid_harness.status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_harness.body["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(
        invalid_harness.body["error"]["message"],
        "harness 必须是 codex | zcode | kimi-code | fake | acp 之一"
    );

    let duplicate = post(
        &app,
        "/api/agents",
        json!({ "name": "研究员", "persona": "p", "harness": "fake" }),
    )
    .await;
    assert_eq!(duplicate.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicate.body["error"]["message"],
        "智能体名称已存在：研究员"
    );

    let not_object = post(&app, "/api/agents", json!(["nope"])).await;
    assert_eq!(not_object.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        not_object.body["error"]["message"],
        "请求体必须是 JSON 对象"
    );

    let malformed = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    let bytes = malformed.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["message"], "请求体不是合法的 JSON");
}

#[tokio::test]
async fn agent_update_supports_fields_and_builtin_guards() {
    let app = setup().await;
    let agent = create_agent(&app, "写手").await;
    let id = agent["id"].as_i64().unwrap();

    let updated = patch(
        &app,
        &format!("/api/agents/{id}"),
        json!({
            "name": "技术写手",
            "persona": "写文档",
            "harness": "zcode",
            "modelConfig": { "model": "glm-4.6", "reasoningLevel": "high" },
        }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["name"], "技术写手");
    assert_eq!(updated.body["harness"], "zcode");
    assert_eq!(updated.body["modelConfig"]["model"], "glm-4.6");

    let cleared = patch(
        &app,
        &format!("/api/agents/{id}"),
        json!({ "modelConfig": null }),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK);
    assert_eq!(cleared.body["modelConfig"], serde_json::Value::Null);

    let other = create_agent(&app, "另一个").await;
    let duplicate_name = patch(
        &app,
        &format!("/api/agents/{}", other["id"].as_i64().unwrap()),
        json!({ "name": "技术写手" }),
    )
    .await;
    assert_eq!(duplicate_name.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        duplicate_name.body["error"]["message"],
        "智能体名称已存在：技术写手"
    );

    let builtin_id = builtin_moderator_id(&app).await;
    let builtin_rename = patch(
        &app,
        &format!("/api/agents/{builtin_id}"),
        json!({ "name": "换名" }),
    )
    .await;
    assert_eq!(builtin_rename.status, StatusCode::CONFLICT);
    assert_eq!(builtin_rename.body["error"]["code"], "CONSTRAINT_VIOLATION");

    let builtin_empty = patch(&app, &format!("/api/agents/{builtin_id}"), json!({})).await;
    assert_eq!(builtin_empty.status, StatusCode::BAD_REQUEST);

    let builtin_model = patch(
        &app,
        &format!("/api/agents/{builtin_id}"),
        json!({ "modelConfig": { "model": "glm-4.6" } }),
    )
    .await;
    assert_eq!(builtin_model.status, StatusCode::OK);
    assert_eq!(builtin_model.body["builtinRole"], "moderator");
    assert_eq!(builtin_model.body["modelConfig"]["model"], "glm-4.6");

    // 主持人允许换 harness；ACP 绑定与普通智能体同规则，切离 acp 清空绑定。
    let harness = post(
        &app,
        "/api/custom-harnesses",
        json!({ "name": "本地 ACP", "command": "node" }),
    )
    .await;
    assert_eq!(harness.status, StatusCode::CREATED, "{:?}", harness.body);
    let harness_id = harness.body["id"].as_i64().unwrap();

    let builtin_codex = patch(
        &app,
        &format!("/api/agents/{builtin_id}"),
        json!({ "harness": "codex" }),
    )
    .await;
    assert_eq!(builtin_codex.status, StatusCode::OK, "{:?}", builtin_codex.body);
    assert_eq!(builtin_codex.body["harness"], "codex");
    assert_eq!(builtin_codex.body["customHarnessId"], serde_json::Value::Null);

    let builtin_acp = patch(
        &app,
        &format!("/api/agents/{builtin_id}"),
        json!({ "harness": "acp", "customHarnessId": harness_id }),
    )
    .await;
    assert_eq!(builtin_acp.status, StatusCode::OK, "{:?}", builtin_acp.body);
    assert_eq!(builtin_acp.body["harness"], "acp");
    assert_eq!(builtin_acp.body["customHarnessId"], harness_id);

    let builtin_back = patch(
        &app,
        &format!("/api/agents/{builtin_id}"),
        json!({ "harness": "zcode" }),
    )
    .await;
    assert_eq!(builtin_back.status, StatusCode::OK, "{:?}", builtin_back.body);
    assert_eq!(builtin_back.body["harness"], "zcode");
    assert_eq!(builtin_back.body["customHarnessId"], serde_json::Value::Null);

    let builtin_acp_unbound = patch(
        &app,
        &format!("/api/agents/{builtin_id}"),
        json!({ "harness": "acp" }),
    )
    .await;
    assert_eq!(
        builtin_acp_unbound.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        builtin_acp_unbound.body
    );
    assert_eq!(
        builtin_acp_unbound.body["error"]["message"],
        "harness 为 acp 时必须选择自定义 ACP agent"
    );

    let missing = patch(&app, "/api/agents/99999", json!({ "name": "x" })).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_agent_cleans_references_and_storage() {
    let app = setup().await;
    let agent = create_agent(&app, "待删除").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "删除测试房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    sqlx::query(
        "INSERT INTO agent_runs (room_id, agent_id, status, prompt, created_at)
         VALUES (?, ?, 'succeeded', 'p', 1)",
    )
    .bind(room_id)
    .bind(agent_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();
    let run_id: i64 = sqlx::query_scalar("SELECT id FROM agent_runs WHERE agent_id = ?")
        .bind(agent_id)
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO messages (room_id, seq, sender_type, sender_agent_id, status, content, created_at, run_id)
         VALUES (?, 1, 'agent', ?, 'final', 'hi', 2, ?)",
    )
    .bind(room_id)
    .bind(agent_id)
    .bind(run_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_sessions (room_id, agent_id, harness, status, storage_dir, created_at)
         VALUES (?, ?, 'fake', 'active', ?, 3)",
    )
    .bind(room_id)
    .bind(agent_id)
    .bind(format!("room-{room_id}/agent-{agent_id}/s-test"))
    .execute(app.state.db.pool())
    .await
    .unwrap();
    let storage = app
        .dir
        .path()
        .join("sessions")
        .join(format!("room-{room_id}/agent-{agent_id}/s-test"));
    std::fs::create_dir_all(&storage).unwrap();

    let response = delete(&app, &format!("/api/agents/{agent_id}")).await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{:?}", response.body);

    let missing = get(&app, "/api/agents").await;
    assert!(missing
        .body
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["id"] != agent_id));

    let sender: Option<i64> = sqlx::query_scalar("SELECT sender_agent_id FROM messages")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(sender, None);
    let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(runs, 0);
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(sessions, 0);
    let members: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM room_members WHERE agent_id = ?")
            .bind(agent_id)
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
    assert_eq!(members, 0);
    assert!(!storage.exists(), "智能体存储目录应被清理");
}

#[tokio::test]
async fn deleting_agent_is_blocked_while_running_discussion() {
    let app = setup().await;
    let agent = create_agent(&app, "讨论参与者").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "讨论房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    sqlx::query(
        "INSERT INTO discussions (room_id, topic, status, current_round, max_rounds, created_at, updated_at)
         VALUES (?, 't', 'running', 1, 5, 1, 1)",
    )
    .bind(room_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();
    let discussion_id: i64 = sqlx::query_scalar("SELECT id FROM discussions")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO discussion_participants (discussion_id, agent_id, position) VALUES (?, ?, 0)",
    )
    .bind(discussion_id)
    .bind(agent_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();

    let response = delete(&app, &format!("/api/agents/{agent_id}")).await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(
        response.body["error"]["message"],
        "智能体正在参与进行中的讨论，请先停止讨论"
    );

    sqlx::query("UPDATE discussions SET status = 'completed'")
        .execute(app.state.db.pool())
        .await
        .unwrap();
    let after = delete(&app, &format!("/api/agents/{agent_id}")).await;
    assert_eq!(after.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn room_creation_uses_builtin_moderator_and_defaults() {
    let app = setup().await;
    let builtin_id = builtin_moderator_id(&app).await;

    let created = post(
        &app,
        "/api/rooms",
        json!({ "name": "默认房间", "workspaceDir": workspace(&app) }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    assert_eq!(created.body["permissionMode"], "auto");
    assert_eq!(created.body["replyMode"], "mentions");
    assert_eq!(created.body["discussionMaxRounds"], 5);
    assert_eq!(created.body["moderatorAgentId"], builtin_id);
    assert_eq!(created.body["memberIds"], json!([builtin_id]));

    let room_id = created.body["id"].as_i64().unwrap();
    let plan = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "permissionMode": "plan" }),
    )
    .await;
    assert_eq!(plan.status, StatusCode::OK);
    assert_eq!(plan.body["permissionMode"], "plan");

    let full = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "permissionMode": "full", "discussionMaxRounds": 12 }),
    )
    .await;
    assert_eq!(full.status, StatusCode::OK);
    assert_eq!(full.body["permissionMode"], "full");
    assert_eq!(full.body["discussionMaxRounds"], 12);

    let invalid_mode = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "permissionMode": "sudo" }),
    )
    .await;
    assert_eq!(invalid_mode.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid_mode.body["error"]["message"],
        "permissionMode 必须是 plan | auto | full 之一"
    );

    let invalid_create = post(
        &app,
        "/api/rooms",
        json!({
            "name": "坏权限",
            "workspaceDir": workspace(&app),
            "permissionMode": "sudo",
        }),
    )
    .await;
    assert_eq!(invalid_create.status, StatusCode::BAD_REQUEST);

    let manual_moderator = post(
        &app,
        "/api/rooms",
        json!({
            "name": "手动主持",
            "workspaceDir": workspace(&app),
            "moderatorAgentId": builtin_id,
        }),
    )
    .await;
    assert_eq!(manual_moderator.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        manual_moderator.body["error"]["message"],
        "主持人由系统内置，不支持手动指定"
    );

    let missing_dir = post(
        &app,
        "/api/rooms",
        json!({
            "name": "坏目录",
            "workspaceDir": app.dir.path().join("missing").to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(missing_dir.status, StatusCode::BAD_REQUEST);
    assert!(missing_dir.body["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("工作空间目录不存在："));
}

#[tokio::test]
async fn room_workspace_dirs_are_persisted_and_primary_is_first() {
    let app = setup().await;
    let primary = app.dir.path().join("primary");
    let extra = app.dir.path().join("extra");
    std::fs::create_dir_all(&primary).unwrap();
    std::fs::create_dir_all(&extra).unwrap();

    let created = post(
        &app,
        "/api/rooms",
        json!({
            "name": "多目录",
            "workspaceDirs": [primary.to_string_lossy().into_owned(), extra.to_string_lossy().into_owned()],
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    assert_eq!(created.body["workspaceDir"], primary.to_string_lossy().into_owned());
    assert_eq!(created.body["workspaceDirs"][0], primary.to_string_lossy().into_owned());
    assert_eq!(created.body["workspaceDirs"][1], extra.to_string_lossy().into_owned());

    let room_id = created.body["id"].as_i64().unwrap();
    let updated = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "workspaceDirs": [extra.to_string_lossy().into_owned(), primary.to_string_lossy().into_owned()] }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["workspaceDir"], extra.to_string_lossy().into_owned());
    assert_eq!(updated.body["workspaceDirs"][1], primary.to_string_lossy().into_owned());

    let empty = post(
        &app,
        "/api/rooms",
        json!({ "name": "空目录", "workspaceDirs": [] }),
    )
    .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);

    let too_many: Vec<String> = (0..33)
        .map(|index| {
            app.dir
                .path()
                .join(format!("dir-{index}"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let rejected = post(
        &app,
        "/api/rooms",
        json!({ "name": "太多目录", "workspaceDirs": too_many }),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        rejected.body["error"]["message"],
        "workspaceDirs 最多 32 个目录"
    );
}

#[tokio::test]
async fn legacy_room_keeps_full_permission_mode() {
    let app = setup().await;
    let builtin_id = builtin_moderator_id(&app).await;

    sqlx::query(
        "INSERT INTO rooms
           (name, workspace_dir, moderator_agent_id, reply_mode,
            discussion_max_rounds, permission_mode, created_at)
         VALUES (?, ?, ?, 'mentions', 5, 'full', 1)",
    )
    .bind("旧房间")
    .bind(workspace(&app))
    .bind(builtin_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();

    let rooms = get(&app, "/api/rooms").await;
    assert_eq!(rooms.status, StatusCode::OK);
    let legacy = rooms
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|room| room["name"] == "旧房间")
        .expect("legacy room");
    assert_eq!(legacy["permissionMode"], "full");
}

#[tokio::test]
async fn room_members_can_be_added_and_removed() {
    let app = setup().await;
    let first = create_agent(&app, "成员甲").await;
    let second = create_agent(&app, "成员乙").await;
    let first_id = first["id"].as_i64().unwrap();
    let second_id = second["id"].as_i64().unwrap();
    let room = create_room(&app, "成员房间", &[first_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let builtin_id = builtin_moderator_id(&app).await;

    let added = post(
        &app,
        &format!("/api/rooms/{room_id}/members"),
        json!({ "agentId": second_id.to_string() }),
    )
    .await;
    assert_eq!(added.status, StatusCode::CREATED);
    assert!(added.body["memberIds"]
        .as_array()
        .unwrap()
        .contains(&json!(second_id)));

    let duplicated = post(
        &app,
        &format!("/api/rooms/{room_id}/members"),
        json!({ "agentId": second_id }),
    )
    .await;
    assert_eq!(duplicated.status, StatusCode::CREATED);
    assert_eq!(
        duplicated.body["memberIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|id| **id == json!(second_id))
            .count(),
        1
    );

    let removed = delete(&app, &format!("/api/rooms/{room_id}/members/{first_id}")).await;
    assert_eq!(removed.status, StatusCode::OK);
    assert!(!removed.body["memberIds"]
        .as_array()
        .unwrap()
        .contains(&json!(first_id)));

    let remove_builtin = delete(&app, &format!("/api/rooms/{room_id}/members/{builtin_id}")).await;
    assert_eq!(remove_builtin.status, StatusCode::CONFLICT);
    assert_eq!(
        remove_builtin.body["error"]["message"],
        "内置主持人不能移出房间"
    );

    let remove_missing = delete(&app, &format!("/api/rooms/{room_id}/members/99999")).await;
    assert_eq!(remove_missing.status, StatusCode::NOT_FOUND);
    assert_eq!(
        remove_missing.body["error"]["message"],
        "房间成员不存在：99999"
    );

    let add_missing = post(
        &app,
        &format!("/api/rooms/{room_id}/members"),
        json!({ "agentId": 99999 }),
    )
    .await;
    assert_eq!(add_missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_room_cleans_room_scoped_data() {
    let app = setup().await;
    let agent = create_agent(&app, "房内成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "待删房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    sqlx::query(
        "INSERT INTO tasks (room_id, title, status, created_at, updated_at)
         VALUES (?, 't', 'backlog', 1, 1)",
    )
    .bind(room_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();
    let task_id: i64 = sqlx::query_scalar("SELECT id FROM tasks")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO task_events (task_id, type, created_at) VALUES (?, 'created', 1)",
    )
    .bind(task_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (room_id, seq, sender_type, status, content, created_at)
         VALUES (?, 1, 'user', 'final', 'hello', 2)",
    )
    .bind(room_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_sessions (room_id, agent_id, harness, status, storage_dir, created_at)
         VALUES (?, ?, 'fake', 'archived', 'room-1/agent-1/s-old', 3)",
    )
    .bind(room_id)
    .bind(agent_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();
    let storage = app.dir.path().join("sessions").join(format!("room-{room_id}"));
    std::fs::create_dir_all(&storage).unwrap();

    let response = delete(&app, &format!("/api/rooms/{room_id}")).await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{:?}", response.body);

    for (table, expected) in [
        ("messages", 0_i64),
        ("tasks", 0),
        ("task_events", 0),
        ("agent_sessions", 0),
        ("room_members", 0),
        ("rooms", 0),
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
        assert_eq!(count, expected, "{table} 应被清空");
    }
    assert!(!storage.exists(), "房间存储目录应被清理");
}

#[tokio::test]
async fn deleting_room_is_blocked_while_discussion_runs() {
    let app = setup().await;
    let room = create_room(&app, "运行中房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    sqlx::query(
        "INSERT INTO discussions (room_id, topic, status, current_round, max_rounds, created_at, updated_at)
         VALUES (?, 't', 'running', 1, 5, 1, 1)",
    )
    .bind(room_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();

    let response = delete(&app, &format!("/api/rooms/{room_id}")).await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(
        response.body["error"]["message"],
        "房间有进行中的讨论，请先停止讨论"
    );

    sqlx::query("UPDATE discussions SET status = 'completed'")
        .execute(app.state.db.pool())
        .await
        .unwrap();
    let after = delete(&app, &format!("/api/rooms/{room_id}")).await;
    assert_eq!(after.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn room_moderator_cannot_be_changed() {
    let app = setup().await;
    let room = create_room(&app, "主持人保护", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let response = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "moderatorAgentId": null }),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        response.body["error"]["message"],
        "主持人由系统内置，不支持修改"
    );
}

#[tokio::test]
async fn room_update_refreshes_builtin_moderator_membership() {
    let app = setup().await;
    let builtin_id = builtin_moderator_id(&app).await;
    let room = create_room(&app, "历史房间修复", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    sqlx::query("DELETE FROM room_members WHERE room_id = ?")
        .bind(room_id)
        .execute(app.state.db.pool())
        .await
        .unwrap();

    let updated = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "name": "历史房间修复-改名" }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["moderatorAgentId"], builtin_id);
    assert!(updated.body["memberIds"]
        .as_array()
        .unwrap()
        .contains(&json!(builtin_id)));
}

#[tokio::test]
async fn room_list_reports_last_activity_at_from_latest_message() {
    let app = setup().await;
    let active = create_room(&app, "活跃房间", &[]).await;
    let active_id = active["id"].as_i64().unwrap();
    let second = create_room(&app, "次活跃房间", &[]).await;
    let second_id = second["id"].as_i64().unwrap();
    let idle = create_room(&app, "空闲房间", &[]).await;
    let idle_id = idle["id"].as_i64().unwrap();

    let find = |body: &serde_json::Value, id: i64| {
        body.as_array()
            .unwrap()
            .iter()
            .find(|room| room["id"] == json!(id))
            .cloned()
            .expect("room")
    };

    // 两个房间各插消息：丢失 GROUP BY 时 SQLite 只按末行返回，第二个房间断言会失败。
    // 活跃房间的 created_at 与 seq 反向，证明取的是 created_at 而非 seq/id。
    for (room_id, seq, created_at) in [
        (active_id, 1, 9_000),
        (active_id, 2, 5_000),
        (second_id, 1, 7_000),
    ] {
        sqlx::query(
            "INSERT INTO messages (room_id, seq, sender_type, status, content, created_at)
             VALUES (?, ?, 'user', 'final', 'hi', ?)",
        )
        .bind(room_id)
        .bind(seq)
        .bind(created_at)
        .execute(app.state.db.pool())
        .await
        .unwrap();
    }

    let rooms = get(&app, "/api/rooms").await;
    assert_eq!(rooms.status, StatusCode::OK);
    assert_eq!(find(&rooms.body, active_id)["lastActivityAt"], 9_000);
    assert_eq!(find(&rooms.body, second_id)["lastActivityAt"], 7_000);
    let idle_room = find(&rooms.body, idle_id);
    assert!(
        idle_room.as_object().unwrap().contains_key("lastActivityAt"),
        "无消息房间也必须携带 lastActivityAt 键"
    );
    assert_eq!(idle_room["lastActivityAt"], serde_json::Value::Null);
}
