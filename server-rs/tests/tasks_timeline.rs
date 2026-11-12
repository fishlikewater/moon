//! 任务 API、状态机与时间线排序契约测试。

mod common;

use axum::http::StatusCode;
use common::{create_agent, create_room, get, patch, post, setup};
use moon_server::domain::message::{MessageKind, MessageSenderType, MessageStatus};
use moon_server::store::messages::AppendMessageInput;
use moon_server::store::runs::CreateRunInput;
use moon_server::store::{MessageStore, RunStore};
use serde_json::{json, Value};

fn plan() -> Value {
    json!({
        "goal": "完成 Rust 任务 API",
        "acceptanceCriteria": ["创建返回 201", "非法迁移 409"],
        "verification": ["cargo test"],
        "scope": ["server-rs"],
    })
}

async fn create_task(app: &common::TestApp, room_id: i64) -> Value {
    let response = post(
        app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({
            "title": "实现任务 API",
            "description": "按计划移植",
            "plan": plan(),
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body
}

#[tokio::test]
async fn create_list_and_validate_tasks() {
    let app = setup().await;
    let agent = create_agent(&app, "任务开发者").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "任务房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let task = create_task(&app, room_id).await;
    assert_eq!(task["roomId"], room_id);
    assert_eq!(task["status"], "backlog");
    assert_eq!(task["plan"]["goal"], "完成 Rust 任务 API");
    assert!(task["createdByAgentId"].is_null());

    let listed = get(&app, &format!("/api/rooms/{room_id}/tasks")).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.body.as_array().unwrap().len(), 1);

    let events = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM task_events WHERE task_id = ?",
    )
        .bind(task["id"].as_i64().unwrap())
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(events, 1, "created 事件必须与任务同事务写入");

    let bad_plan = post(
        &app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({ "title": "坏计划", "plan": { "goal": "g", "acceptanceCriteria": [], "verification": ["v"] } }),
    )
    .await;
    assert_eq!(bad_plan.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad_plan.body["error"]["code"], "VALIDATION_FAILED");

    let no_title = post(&app, &format!("/api/rooms/{room_id}/tasks"), json!({})).await;
    assert_eq!(no_title.status, StatusCode::BAD_REQUEST);

    let missing_room = get(&app, "/api/rooms/9999/tasks").await;
    assert_eq!(missing_room.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn transition_follows_state_machine_and_rejects_illegal_moves() {
    let app = setup().await;
    let agent = create_agent(&app, "状态机开发者").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "状态房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let task = create_task(&app, room_id).await;
    let task_id = task["id"].as_i64().unwrap();

    for status in ["in_progress", "review", "done"] {
        let response = patch(
            &app,
            &format!("/api/tasks/{task_id}/status"),
            json!({ "status": status }),
        )
        .await;
        assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
        assert_eq!(response.body["status"], status);
    }

    let events_before = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM task_events WHERE task_id = ?",
    )
        .bind(task_id)
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();

    let illegal = patch(
        &app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(illegal.status, StatusCode::CONFLICT);
    assert_eq!(illegal.body["error"]["code"], "INVALID_TRANSITION");

    let events_after = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM task_events WHERE task_id = ?",
    )
        .bind(task_id)
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(events_before, events_after, "非法迁移不得产生事件");

    let listed = get(&app, &format!("/api/rooms/{room_id}/tasks")).await;
    assert_eq!(listed.body[0]["status"], "done");

    let bad_status = patch(
        &app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "archived" }),
    )
    .await;
    assert_eq!(bad_status.status, StatusCode::BAD_REQUEST);

    let missing = patch(
        &app,
        "/api/tasks/9999/status",
        json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn timeline_merges_and_orders_entries() {
    let app = setup().await;
    let agent = create_agent(&app, "时间线开发者").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "时间线房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let task = create_task(&app, room_id).await;
    let task_id = task["id"].as_i64().unwrap();

    let runs = RunStore::new(app.state.db.clone());
    let run = runs
        .create(CreateRunInput {
            room_id,
            agent_id,
            task_id: Some(task_id),
            prompt: "执行任务".to_string(),
        })
        .await
        .unwrap();

    let messages = MessageStore::new(app.state.db.clone());
    let message = messages
        .append(
            AppendMessageInput {
                room_id,
                sender_type: MessageSenderType::Agent,
                sender_agent_id: Some(agent_id),
                content: "任务完成".to_string(),
                status: MessageStatus::Final,
                kind: MessageKind::Reply,
                run_id: Some(run.id),
                task_id: Some(task_id),
                reply_to_message_id: None,
                discussion: None,
                collaboration: None,
            },
        )
        .await
        .unwrap();
    // 进度消息不进入时间线，只保留最终回复。
    messages
        .append(AppendMessageInput {
            room_id,
            sender_type: MessageSenderType::Agent,
            sender_agent_id: Some(agent_id),
            content: "阶段总结".to_string(),
            status: MessageStatus::Final,
            kind: MessageKind::Progress,
            run_id: Some(run.id),
            task_id: Some(task_id),
            reply_to_message_id: None,
            discussion: None,
            collaboration: None,
        })
        .await
        .unwrap();

    let same_ms = 1_700_000_000_000_i64;
    sqlx::query("UPDATE task_events SET created_at = ? WHERE task_id = ?")
        .bind(same_ms)
        .bind(task_id)
        .execute(app.state.db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE agent_runs SET created_at = ? WHERE id = ?")
        .bind(same_ms)
        .bind(run.id)
        .execute(app.state.db.pool())
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET created_at = ? WHERE id IN (?, ?)")
        .bind(same_ms)
        .bind(message.id)
        .bind(message.id + 1)
        .execute(app.state.db.pool())
        .await
        .unwrap();

    let response = get(&app, &format!("/api/tasks/{task_id}/timeline")).await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["taskId"], task_id);
    let entries = response.body["entries"].as_array().unwrap();
    let kinds: Vec<&str> = entries
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["task_event", "run", "message"]);
    for entry in entries {
        assert_eq!(entry["taskId"], task_id);
        assert_eq!(entry["createdAt"], same_ms);
    }
    assert_eq!(entries[2]["message"]["id"], message.id);

    let bad_id = get(&app, "/api/tasks/not-a-number/timeline").await;
    assert_eq!(bad_id.status, StatusCode::BAD_REQUEST);
    let missing = get(&app, "/api/tasks/9999/timeline").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}
