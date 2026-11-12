//! 房间历史清理与启动恢复测试。

mod common;

use axum::http::StatusCode;
use common::{create_agent, create_room, get, patch, post, setup, workspace};
use moon_server::domain::message::{MessageKind, MessageSenderType, MessageStatus};
use moon_server::services::message_artifacts::archive_path;
use moon_server::services::recovery::{
    recover_interrupted, RecoveryResult, RECOVERY_DISCUSSION_MESSAGE, RECOVERY_SYSTEM_MESSAGE,
};
use moon_server::store::discussions::CreateDiscussionInput;
use moon_server::store::messages::AppendMessageInput;
use moon_server::store::runs::CreateRunInput;
use moon_server::store::{DiscussionStore, MessageStore, RunStore};
use serde_json::{json, Value};
use std::path::Path;

async fn append_message(
    app: &common::TestApp,
    room_id: i64,
    agent_id: i64,
    content: &str,
    task_id: Option<i64>,
) -> i64 {
    let store = MessageStore::new(app.state.db.clone());
    store
        .append(AppendMessageInput {
            room_id,
            sender_type: if task_id.is_some() {
                MessageSenderType::Agent
            } else {
                MessageSenderType::User
            },
            sender_agent_id: if task_id.is_some() { Some(agent_id) } else { None },
            content: content.to_string(),
            status: MessageStatus::Final,
            kind: MessageKind::Reply,
            run_id: None,
            task_id,
            reply_to_message_id: None,
            discussion: None,
            collaboration: None,
        })
        .await
        .unwrap()
        .id
}

async fn message_ids(app: &common::TestApp, room_id: i64) -> Vec<i64> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM messages WHERE room_id = ? ORDER BY seq")
        .bind(room_id)
        .fetch_all(app.state.db.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn cleanup_keeps_recent_pure_chat_and_removes_artifacts() {
    let app = setup().await;
    let agent = create_agent(&app, "清理助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "清理房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let workspace_dir = workspace(&app);

    let task = post(
        &app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({ "title": "保留任务消息" }),
    )
    .await
    .body;
    let task_id = task["id"].as_i64().unwrap();

    let first = append_message(&app, room_id, agent_id, "纯聊天一", None).await;
    let second = append_message(&app, room_id, agent_id, "纯聊天二", None).await;
    let third = append_message(&app, room_id, agent_id, "纯聊天三", None).await;
    let task_message = append_message(&app, room_id, agent_id, "任务进展", Some(task_id)).await;

    let artifact = archive_path(&workspace_dir, room_id, first);
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, "完整归档").unwrap();
    assert!(Path::new(&artifact).exists());

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": 1 }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["deleted"], 2);
    assert_eq!(response.body["artifactsDeleted"], 1);
    assert!(!artifact.exists(), "归档文件应随消息删除");

    let remaining = message_ids(&app, room_id).await;
    assert_eq!(remaining, vec![third, task_message]);
    assert!(!remaining.contains(&first));
    assert!(!remaining.contains(&second));
}

#[tokio::test]
async fn cleanup_protects_referenced_messages_and_validates_input() {
    let app = setup().await;
    let agent = create_agent(&app, "保护测试").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "保护房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let builtin = common::builtin_moderator_id(&app).await;

    let referenced = append_message(&app, room_id, agent_id, "委派触发消息", None).await;
    let ordinary = append_message(&app, room_id, agent_id, "普通聊天", None).await;

    sqlx::query(
        "INSERT INTO collaboration_batches (
           room_id, moderator_agent_id, user_message_id, status, current_round,
           max_rounds, stop_requested, created_at, updated_at
         ) VALUES (?, ?, ?, 'completed', 1, 1, 0, 1, 1)",
    )
    .bind(room_id)
    .bind(builtin)
    .bind(referenced)
    .execute(app.state.db.pool())
    .await
    .unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({}),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["deleted"], 1);
    let remaining = message_ids(&app, room_id).await;
    assert_eq!(remaining, vec![referenced]);

    let negative = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": -1 }),
    )
    .await;
    assert_eq!(negative.status, StatusCode::BAD_REQUEST);

    let too_large = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": 5001 }),
    )
    .await;
    assert_eq!(too_large.status, StatusCode::BAD_REQUEST);

    let not_integer = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": "3" }),
    )
    .await;
    assert_eq!(not_integer.status, StatusCode::BAD_REQUEST);

    let missing_room = post(
        &app,
        "/api/rooms/9999/messages/cleanup",
        json!({}),
    )
    .await;
    assert_eq!(missing_room.status, StatusCode::NOT_FOUND);

    let _ = ordinary;
}

#[tokio::test]
async fn cleanup_rejects_room_with_running_run() {
    let app = setup().await;
    let agent = create_agent(&app, "运行保护").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "运行房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    RunStore::new(app.state.db.clone())
        .create(CreateRunInput {
            room_id,
            agent_id,
            task_id: None,
            prompt: "长任务".to_string(),
        })
        .await
        .unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({}),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.body["error"]["code"], "CONSTRAINT_VIOLATION");
}

async fn create_in_progress_task(app: &common::TestApp, room_id: i64, title: &str) -> i64 {
    let task = post(
        app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({ "title": title }),
    )
    .await
    .body;
    let task_id = task["id"].as_i64().unwrap();
    let moved = patch(
        app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK);
    task_id
}

#[tokio::test]
async fn recovery_marks_residual_state_and_is_idempotent() {
    let app = setup().await;
    let agent = create_agent(&app, "恢复助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "恢复房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let task_id = create_in_progress_task(&app, room_id, "被打断的任务").await;
    let untouched_task_id = create_in_progress_task(&app, room_id, "没有运行的任务").await;

    let runs = RunStore::new(app.state.db.clone());
    let run = runs
        .create(CreateRunInput {
            room_id,
            agent_id,
            task_id: Some(task_id),
            prompt: "执行中".to_string(),
        })
        .await
        .unwrap();

    let messages = MessageStore::new(app.state.db.clone());
    let streaming = messages
        .append(AppendMessageInput {
            room_id,
            sender_type: MessageSenderType::Agent,
            sender_agent_id: Some(agent_id),
            content: "已流出的内容".to_string(),
            status: MessageStatus::Streaming,
            kind: MessageKind::Reply,
            run_id: Some(run.id),
            task_id: Some(task_id),
            reply_to_message_id: None,
            discussion: None,
            collaboration: None,
        })
        .await
        .unwrap();

    let first: RecoveryResult = recover_interrupted(&app.state.db, None, None, None).await.unwrap();
    assert_eq!(first.streaming_messages, 1);
    assert_eq!(first.running_runs, 1);
    assert_eq!(first.blocked_tasks, 1);
    assert_eq!(first.interrupted_discussions, 0);
    assert_eq!(first.interrupted_collaborations, 0);

    let runs_after = get(&app, &format!("/api/rooms/{room_id}/runs")).await;
    let recovered_run = runs_after
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == run.id)
        .unwrap();
    assert_eq!(recovered_run["status"], "interrupted");
    assert!(!recovered_run["finishedAt"].is_null());

    let stored_message = messages.get_by_id(streaming.id).await.unwrap().unwrap();
    assert_eq!(stored_message.status, MessageStatus::Failed);
    assert_eq!(stored_message.content, "已流出的内容");

    let tasks = get(&app, &format!("/api/rooms/{room_id}/tasks")).await;
    let tasks = tasks.body.as_array().unwrap();
    let blocked = tasks
        .iter()
        .find(|task| task["id"] == task_id)
        .unwrap();
    assert_eq!(blocked["status"], "blocked");
    let untouched = tasks
        .iter()
        .find(|task| task["id"] == untouched_task_id)
        .unwrap();
    assert_eq!(untouched["status"], "in_progress");

    let messages_after = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    let messages_after = messages_after.body.as_array().unwrap();
    let system: Vec<&Value> = messages_after
        .iter()
        .filter(|message| message["senderType"] == "system")
        .collect();
    assert_eq!(system.len(), 1);
    assert_eq!(system[0]["content"], RECOVERY_SYSTEM_MESSAGE);
    assert_eq!(system[0]["taskId"], task_id);

    let timeline = get(&app, &format!("/api/tasks/{task_id}/timeline")).await;
    let blocked_event = timeline.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["kind"] == "task_event"
                && entry["event"]["type"] == "status_changed"
                && entry["event"]["payload"]["to"] == "blocked"
        })
        .expect("恢复必须追加 blocked 状态事件");
    assert_eq!(blocked_event["event"]["payload"]["from"], "in_progress");

    let message_count = messages_after.len();
    let event_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM task_events")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();

    let second = recover_interrupted(&app.state.db, None, None, None).await.unwrap();
    assert_eq!(second.streaming_messages, 0);
    assert_eq!(second.running_runs, 0);
    assert_eq!(second.blocked_tasks, 0);

    let messages_again = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    assert_eq!(messages_again.body.as_array().unwrap().len(), message_count);
    let events_again = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM task_events")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(events_again, event_count);
}


#[tokio::test]
async fn recovery_marks_running_discussions_failed_once() {
    let app = setup().await;
    let agent = create_agent(&app, "讨论恢复成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "讨论恢复房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let builtin = common::builtin_moderator_id(&app).await;
    let discussions = DiscussionStore::new(app.state.db.clone());

    let discussion = discussions
        .create(CreateDiscussionInput {
            room_id,
            topic: "会被中断的讨论".to_string(),
            max_rounds: 2,
            participant_ids: vec![agent_id, builtin],
            created_by_agent_id: Some(builtin),
        })
        .await
        .unwrap();

    let first = recover_interrupted(&app.state.db, Some(&discussions), None, None)
        .await
        .unwrap();
    assert_eq!(first.interrupted_discussions, 1);

    let recovered = discussions.get_by_id(discussion.id).await.unwrap().unwrap();
    assert_eq!(
        recovered.status,
        moon_server::domain::discussion::DiscussionStatus::Failed
    );
    assert_eq!(
        recovered.stop_reason,
        Some(moon_server::domain::discussion::DiscussionStopReason::Interrupted)
    );

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    let system = messages
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["content"] == RECOVERY_DISCUSSION_MESSAGE)
        .expect("应写入讨论中断系统消息");
    assert_eq!(system["discussion"]["discussionId"], discussion.id);
    assert_eq!(system["discussion"]["round"], 1);

    let second = recover_interrupted(&app.state.db, Some(&discussions), None, None)
        .await
        .unwrap();
    assert_eq!(second.interrupted_discussions, 0);
    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    assert_eq!(
        messages
            .body
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["content"] == RECOVERY_DISCUSSION_MESSAGE)
            .count(),
        1,
        "恢复必须幂等"
    );
}
