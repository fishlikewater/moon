mod common;

use std::sync::Arc;

use common::{
    builtin_moderator_id, create_agent, create_room, delete, enable_reuse, get, post_empty, put,
    setup,
};
use moon_server::domain::agent::Agent;
use serde_json::json;
use axum::http::StatusCode;

async fn agent_by_id(app: &common::TestApp, agent_id: i64) -> Agent {
    app.state
        .agent_service
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .expect("agent")
}

#[tokio::test]
async fn session_list_hides_storage_and_tracks_reuse_settings() {
    let app = setup().await;
    let agent = create_agent(&app, "会话成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "会话房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let builtin_id = builtin_moderator_id(&app).await;

    let initial = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    assert_eq!(initial.status, StatusCode::OK);
    let members = initial.body["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.iter().any(|item| item["agentId"] == builtin_id));
    assert!(members.iter().all(|item| item["reuseSession"] == true));
    assert_eq!(initial.body["sessions"], json!([]));

    let disabled = put(
        &app,
        &format!("/api/rooms/{room_id}/members/{agent_id}/session-settings"),
        json!({ "reuseSession": false }),
    )
    .await;
    assert_eq!(disabled.status, StatusCode::OK);
    let listed = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    let member = listed.body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["agentId"] == agent_id)
        .unwrap();
    assert_eq!(member["reuseSession"], false);

    enable_reuse(&app, room_id, agent_id).await;
    let listed = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    let member = listed.body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["agentId"] == agent_id)
        .unwrap();
    assert_eq!(member["reuseSession"], true);

    let invalid = put(
        &app,
        &format!("/api/rooms/{room_id}/members/{agent_id}/session-settings"),
        json!({ "reuseSession": "yes" }),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.body["error"]["message"],
        "reuseSession 必须是布尔值"
    );

    let missing_member = put(
        &app,
        &format!("/api/rooms/{room_id}/members/99999/session-settings"),
        json!({ "reuseSession": true }),
    )
    .await;
    assert_eq!(missing_member.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn failed_and_deleted_sessions_follow_room_busy_state() {
    let app = setup().await;
    let agent = create_agent(&app, "归档成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "归档房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    enable_reuse(&app, room_id, agent_id).await;

    let domain_agent = agent_by_id(&app, agent_id).await;
    let resolved = app
        .state
        .session_service
        .resolve_run_session(room_id, &domain_agent)
        .await
        .unwrap()
        .expect("reuse session");
    assert!(std::path::Path::new(&resolved.context.storage_dir).exists());

    let listed = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    let sessions = listed.body["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["status"], "active");
    assert_eq!(sessions[0]["harness"], "fake");
    assert!(sessions[0].get("storageDir").is_none(), "不得泄露内部存储路径");
    let session_id = sessions[0]["id"].as_i64().unwrap();

    let archived = post_empty(
        &app,
        &format!("/api/rooms/{room_id}/sessions/{session_id}/archive"),
    )
    .await;
    assert_eq!(archived.status, StatusCode::OK, "{:?}", archived.body);
    assert_eq!(archived.body["status"], "archived");

    let repeat = post_empty(
        &app,
        &format!("/api/rooms/{room_id}/sessions/{session_id}/archive"),
    )
    .await;
    assert_eq!(repeat.status, StatusCode::CONFLICT);
    assert_eq!(
        repeat.body["error"]["message"],
        "会话已归档，无需重复归档"
    );

    let next = app
        .state
        .session_service
        .resolve_run_session(room_id, &domain_agent)
        .await
        .unwrap()
        .expect("new session after archive");
    assert_ne!(next.id, session_id);

    app.state
        .session_service
        .set_busy_checker(Arc::new(|_| true));
    let busy_archive = post_empty(
        &app,
        &format!("/api/rooms/{room_id}/sessions/{}/archive", next.id),
    )
    .await;
    assert_eq!(busy_archive.status, StatusCode::CONFLICT);
    assert_eq!(
        busy_archive.body["error"]["message"],
        "房间有排队或运行中的任务，请等待完成后再操作会话"
    );
    let busy_delete = delete(
        &app,
        &format!("/api/rooms/{room_id}/sessions/{}", next.id),
    )
    .await;
    assert_eq!(busy_delete.status, StatusCode::CONFLICT);

    app.state
        .session_service
        .set_busy_checker(Arc::new(|_| false));
    let removed = delete(
        &app,
        &format!("/api/rooms/{room_id}/sessions/{}", next.id),
    )
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(
        !std::path::Path::new(&next.context.storage_dir).exists(),
        "删除会话应清理存储目录"
    );

    let after = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    assert_eq!(after.body["sessions"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn removing_member_archives_active_session() {
    let app = setup().await;
    let agent = create_agent(&app, "被移出成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "移出房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    enable_reuse(&app, room_id, agent_id).await;

    let domain_agent = agent_by_id(&app, agent_id).await;
    let resolved = app
        .state
        .session_service
        .resolve_run_session(room_id, &domain_agent)
        .await
        .unwrap()
        .unwrap();

    let removed = delete(&app, &format!("/api/rooms/{room_id}/members/{agent_id}")).await;
    assert_eq!(removed.status, StatusCode::OK);

    let sessions = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    let session = &sessions.body["sessions"].as_array().unwrap()[0];
    assert_eq!(session["status"], "archived");
    assert!(
        std::path::Path::new(&resolved.context.storage_dir).exists(),
        "归档保留存储"
    );
}

#[tokio::test]
async fn switching_harness_archives_old_session_and_creates_new_one() {
    let app = setup().await;
    let agent = create_agent(&app, "换 harness 成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "换 harness 房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    enable_reuse(&app, room_id, agent_id).await;

    let fake_agent = agent_by_id(&app, agent_id).await;
    let first = app
        .state
        .session_service
        .resolve_run_session(room_id, &fake_agent)
        .await
        .unwrap()
        .unwrap();

    let patched = common::patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({ "harness": "zcode" }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK);
    let zcode_agent = agent_by_id(&app, agent_id).await;
    let second = app
        .state
        .session_service
        .resolve_run_session(room_id, &zcode_agent)
        .await
        .unwrap()
        .unwrap();

    assert_ne!(first.id, second.id);
    let sessions = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    let items = sessions.body["sessions"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let first_item = items.iter().find(|item| item["id"] == first.id).unwrap();
    let second_item = items.iter().find(|item| item["id"] == second.id).unwrap();
    assert_eq!(first_item["status"], "archived");
    assert_eq!(second_item["status"], "active");
    assert_eq!(second_item["harness"], "zcode");
}

#[tokio::test]
async fn sessions_are_isolated_per_room() {
    let app = setup().await;
    let agent = create_agent(&app, "跨房间成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room_a = create_room(&app, "房间 A", &[agent_id]).await;
    let room_b = create_room(&app, "房间 B", &[agent_id]).await;
    let room_a_id = room_a["id"].as_i64().unwrap();
    let room_b_id = room_b["id"].as_i64().unwrap();
    enable_reuse(&app, room_a_id, agent_id).await;
    enable_reuse(&app, room_b_id, agent_id).await;

    let domain_agent = agent_by_id(&app, agent_id).await;
    let session_a = app
        .state
        .session_service
        .resolve_run_session(room_a_id, &domain_agent)
        .await
        .unwrap()
        .unwrap();
    let session_b = app
        .state
        .session_service
        .resolve_run_session(room_b_id, &domain_agent)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(session_a.id, session_b.id);

    let cross_archive = post_empty(
        &app,
        &format!("/api/rooms/{room_b_id}/sessions/{}/archive", session_a.id),
    )
    .await;
    assert_eq!(cross_archive.status, StatusCode::NOT_FOUND);

    let cross_delete = delete(
        &app,
        &format!("/api/rooms/{room_b_id}/sessions/{}", session_a.id),
    )
    .await;
    assert_eq!(cross_delete.status, StatusCode::NOT_FOUND);

    let room_storage = app
        .dir
        .path()
        .join("sessions")
        .join(format!("room-{room_a_id}"));
    assert!(room_storage.exists());
    let removed = delete(&app, &format!("/api/rooms/{room_a_id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(!room_storage.exists(), "删除房间应清理会话存储");

    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions WHERE room_id = ?")
            .bind(room_a_id)
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn unknown_room_or_invalid_id_is_rejected() {
    let app = setup().await;

    let missing = get(&app, "/api/rooms/99999/sessions").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.body["error"]["message"], "房间不存在：99999");

    let invalid = get(&app, "/api/rooms/abc/sessions").await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid.body["error"]["message"], "非法的房间 id：abc");
}

#[tokio::test]
async fn sessions_overview_lists_all_rooms_with_names_and_order() {
    let app = setup().await;
    let agent = create_agent(&app, "总览成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room_a = create_room(&app, "总览房间 A", &[agent_id]).await;
    let room_a_id = room_a["id"].as_i64().unwrap();
    let room_b = create_room(&app, "总览房间 B", &[]).await;
    let room_b_id = room_b["id"].as_i64().unwrap();
    let builtin_id = builtin_moderator_id(&app).await;

    enable_reuse(&app, room_a_id, agent_id).await;
    enable_reuse(&app, room_b_id, builtin_id).await;

    let domain_agent = agent_by_id(&app, agent_id).await;
    let first = app
        .state
        .session_service
        .resolve_run_session(room_a_id, &domain_agent)
        .await
        .unwrap()
        .unwrap();
    let builtin = agent_by_id(&app, builtin_id).await;
    let second = app
        .state
        .session_service
        .resolve_run_session(room_b_id, &builtin)
        .await
        .unwrap()
        .unwrap();

    let listed = get(&app, "/api/sessions").await;
    assert_eq!(listed.status, StatusCode::OK, "{:?}", listed.body);
    let sessions = listed.body.as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    // 同毫秒创建时以 id 倒序兜底。
    assert_eq!(sessions[0]["id"], second.id);
    assert_eq!(sessions[1]["id"], first.id);
    assert_eq!(sessions[0]["roomName"], "总览房间 B");
    assert_eq!(sessions[0]["agentName"], "Moon 主持人");
    assert_eq!(sessions[1]["roomName"], "总览房间 A");
    assert_eq!(sessions[1]["agentName"], "总览成员");
    assert!(sessions.iter().all(|item| item.get("storageDir").is_none()));
    assert!(sessions.iter().all(|item| item["status"] == "active"));

    // 归档后仍出现在总览中。
    let archived = post_empty(
        &app,
        &format!("/api/rooms/{room_a_id}/sessions/{}/archive", first.id),
    )
    .await;
    assert_eq!(archived.status, StatusCode::OK, "{:?}", archived.body);
    let after_archive = get(&app, "/api/sessions").await;
    assert!(after_archive
        .body
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == first.id && item["status"] == "archived"));

    // 相同 created_at 且从未使用时，按 id 倒序兜底。
    sqlx::query("UPDATE agent_sessions SET created_at = 1000, last_used_at = NULL")
        .execute(app.state.db.pool())
        .await
        .unwrap();
    let tied = get(&app, "/api/sessions").await;
    assert_eq!(tied.body[0]["id"], second.id);
    assert_eq!(tied.body[1]["id"], first.id);

    // 删除房间后其会话级联消失。
    let removed = delete(&app, &format!("/api/rooms/{room_a_id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{:?}", removed.body);
    let after_delete = get(&app, "/api/sessions").await;
    let remaining = after_delete.body.as_array().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["id"], second.id);
}
