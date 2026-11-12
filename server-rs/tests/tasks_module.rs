//! 任务模块扩展契约测试：新字段、取消终态、定时调度与校验。

mod common;

use axum::http::StatusCode;
use common::{create_agent, create_room, get, patch, post, setup};
use moon_server::domain::now_ms;
use moon_server::domain::task::TaskStatus;
use moon_server::error::ErrorCode;
use moon_server::store::tasks::UpdateTaskFields;
use moon_server::store::TaskStore;
use serde_json::{json, Value};

async fn create_task_with_fields(
    app: &common::TestApp,
    room_id: i64,
    fields: Value,
) -> Value {
    let mut body = json!({ "title": "扩展任务" });
    for (key, value) in fields.as_object().unwrap() {
        body[key] = value.clone();
    }
    let response = post(app, &format!("/api/rooms/{room_id}/tasks"), body).await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body
}

async fn create_task(app: &common::TestApp, room_id: i64) -> Value {
    create_task_with_fields(app, room_id, json!({})).await
}

async fn event_types(app: &common::TestApp, task_id: i64) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT type FROM task_events WHERE task_id = ? ORDER BY id",
    )
    .bind(task_id)
    .fetch_all(app.state.db.pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn task_fields_roundtrip_and_validation() {
    let app = setup().await;
    let member = create_agent(&app, "任务成员").await;
    let member_id = member["id"].as_i64().unwrap();
    let outsider = create_agent(&app, "房间外成员").await;
    let outsider_id = outsider["id"].as_i64().unwrap();
    let room = create_room(&app, "字段房间", &[member_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let due_at = now_ms() + 3_600_000;
    let scheduled_at = now_ms() + 7_200_000;
    let task = create_task_with_fields(
        &app,
        room_id,
        json!({
            "priority": "high",
            "assigneeAgentId": member_id,
            "dueAt": due_at,
            "scheduledAt": scheduled_at,
        }),
    )
    .await;
    let task_id = task["id"].as_i64().unwrap();
    assert_eq!(task["priority"], "high");
    assert_eq!(task["assigneeAgentId"], member_id);
    assert_eq!(task["dueAt"], due_at);
    assert_eq!(task["scheduledAt"], scheduled_at);
    assert_eq!(task["status"], "backlog");

    let listed = get(&app, &format!("/api/rooms/{room_id}/tasks")).await;
    let stored = listed.body.as_array().unwrap()[0].clone();
    assert_eq!(stored["priority"], "high");
    assert_eq!(stored["assigneeAgentId"], member_id);
    assert_eq!(stored["scheduledAt"], scheduled_at);

    // 未传字段保持不变，显式 null 清除可空字段。
    let updated = patch(
        &app,
        &format!("/api/tasks/{task_id}"),
        json!({ "priority": "urgent", "dueAt": null }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{:?}", updated.body);
    assert_eq!(updated.body["priority"], "urgent");
    assert!(updated.body["dueAt"].is_null());
    assert_eq!(updated.body["scheduledAt"], scheduled_at);
    assert_eq!(updated.body["assigneeAgentId"], member_id);
    assert_eq!(updated.body["title"], "扩展任务");

    let outsider_assign = patch(
        &app,
        &format!("/api/tasks/{task_id}"),
        json!({ "assigneeAgentId": outsider_id }),
    )
    .await;
    assert_eq!(outsider_assign.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        outsider_assign.body["error"]["message"],
        "指派对象必须是房间成员"
    );

    let outsider_create = post(
        &app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({ "title": "坏指派", "assigneeAgentId": outsider_id }),
    )
    .await;
    assert_eq!(outsider_create.status, StatusCode::BAD_REQUEST);

    for (body, label) in [
        (json!({ "priority": "p0" }), "非法优先级"),
        (json!({ "dueAt": -1 }), "负时间"),
        (json!({ "dueAt": "明天" }), "非数字时间"),
    ] {
        let response = patch(&app, &format!("/api/tasks/{task_id}"), body).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{label}");
    }

    // 更新写入 updated 事件，且 changed 只含本次字段。
    let types = event_types(&app, task_id).await;
    assert_eq!(types.first().map(String::as_str), Some("created"));
    assert!(types.contains(&"updated".to_string()));
    let payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM task_events WHERE task_id = ? AND type = 'updated' ORDER BY id DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_one(app.state.db.pool())
    .await
    .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["changed"], json!(["priority", "dueAt"]));
    assert_eq!(payload["priority"], "urgent");
    assert!(payload["dueAt"].is_null());
}

#[tokio::test]
async fn scheduled_start_only_allowed_for_backlog() {
    let app = setup().await;
    let room = create_room(&app, "定时房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let task = create_task(&app, room_id).await;
    let task_id = task["id"].as_i64().unwrap();

    let started = patch(
        &app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(started.status, StatusCode::OK);

    let scheduled = patch(
        &app,
        &format!("/api/tasks/{task_id}"),
        json!({ "scheduledAt": now_ms() + 60_000 }),
    )
    .await;
    assert_eq!(scheduled.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        scheduled.body["error"]["message"],
        "只有待办任务可以设置定时启动"
    );
}

#[tokio::test]
async fn cancelled_is_terminal_and_clears_schedule() {
    let app = setup().await;
    let room = create_room(&app, "取消房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let task = create_task_with_fields(
        &app,
        room_id,
        json!({ "scheduledAt": now_ms() + 60_000 }),
    )
    .await;
    let task_id = task["id"].as_i64().unwrap();

    let cancelled = patch(
        &app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "cancelled" }),
    )
    .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:?}", cancelled.body);
    assert_eq!(cancelled.body["status"], "cancelled");
    assert!(
        cancelled.body["scheduledAt"].is_null(),
        "取消后定时必须清除"
    );

    let stored: Option<i64> =
        sqlx::query_scalar("SELECT scheduled_at FROM tasks WHERE id = ?")
            .bind(task_id)
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
    assert_eq!(stored, None);

    let reopen = patch(
        &app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "backlog" }),
    )
    .await;
    assert_eq!(reopen.status, StatusCode::CONFLICT);
    assert_eq!(reopen.body["error"]["code"], "INVALID_TRANSITION");

    let cancel_done = create_task(&app, room_id).await;
    let done_id = cancel_done["id"].as_i64().unwrap();
    for status in ["in_progress", "review", "done"] {
        let response = patch(
            &app,
            &format!("/api/tasks/{done_id}/status"),
            json!({ "status": status }),
        )
        .await;
        assert_eq!(response.status, StatusCode::OK, "{status}: {:?}", response.body);
    }
    let done_cancel = patch(
        &app,
        &format!("/api/tasks/{done_id}/status"),
        json!({ "status": "cancelled" }),
    )
    .await;
    assert_eq!(done_cancel.status, StatusCode::CONFLICT, "已完成不可取消");
}

#[tokio::test]
async fn scheduler_starts_due_tasks_once() {
    let app = setup().await;
    let room = create_room(&app, "调度房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let scheduled_at = now_ms() - 1_000;
    let task = create_task_with_fields(
        &app,
        room_id,
        json!({ "scheduledAt": scheduled_at }),
    )
    .await;
    let task_id = task["id"].as_i64().unwrap();

    let future = create_task_with_fields(
        &app,
        room_id,
        json!({ "scheduledAt": now_ms() + 600_000 }),
    )
    .await;
    let future_id = future["id"].as_i64().unwrap();
    let plain = create_task(&app, room_id).await;
    let plain_id = plain["id"].as_i64().unwrap();

    let report = app.state.task_scheduler.tick(now_ms()).await.unwrap();
    assert_eq!(report.started, vec![task_id]);
    assert!(report.failed.is_empty());

    let started = get(&app, &format!("/api/rooms/{room_id}/tasks")).await;
    let by_id = |id: i64| {
        started
            .body
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == json!(id))
            .cloned()
            .unwrap()
    };
    assert_eq!(by_id(task_id)["status"], "in_progress");
    assert_eq!(by_id(future_id)["status"], "backlog");
    assert_eq!(by_id(plain_id)["status"], "backlog");
    assert!(
        by_id(task_id)["scheduledAt"].is_null(),
        "启动成功后定时应清除，原时间记入 scheduled_started 事件"
    );

    let types = event_types(&app, task_id).await;
    assert!(types.contains(&"status_changed".to_string()));
    assert!(types.contains(&"scheduled_started".to_string()));

    let second = app.state.task_scheduler.tick(now_ms()).await.unwrap();
    assert!(second.started.is_empty(), "已启动任务不得重复启动");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_events WHERE task_id = ? AND type = 'scheduled_started'",
    )
    .bind(task_id)
    .fetch_one(app.state.db.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn scheduler_clears_schedule_when_activation_fails() {
    let app = setup().await;
    let room = create_room(&app, "失败房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let task = create_task_with_fields(
        &app,
        room_id,
        json!({ "scheduledAt": now_ms() - 1_000 }),
    )
    .await;
    let task_id = task["id"].as_i64().unwrap();

    // 清掉主持人，模拟房间前置不满足。
    sqlx::query("UPDATE rooms SET moderator_agent_id = NULL WHERE id = ?")
        .bind(room_id)
        .execute(app.state.db.pool())
        .await
        .unwrap();

    let report = app.state.task_scheduler.tick(now_ms()).await.unwrap();
    assert_eq!(report.failed, vec![task_id]);
    assert!(report.started.is_empty());

    let stored: Option<i64> =
        sqlx::query_scalar("SELECT scheduled_at FROM tasks WHERE id = ?")
            .bind(task_id)
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
    assert_eq!(stored, None, "前置失败后必须清除定时");

    let status: String = sqlx::query_scalar("SELECT status FROM tasks WHERE id = ?")
        .bind(task_id)
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(status, "backlog");

    let types = event_types(&app, task_id).await;
    assert!(types.contains(&"schedule_failed".to_string()));
    let payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM task_events WHERE task_id = ? AND type = 'schedule_failed' ORDER BY id DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_one(app.state.db.pool())
    .await
    .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert!(
        payload["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("主持人")),
        "失败原因应可读：{payload}"
    );

    // 下一轮不再处理已清除定时的任务。
    let second = app.state.task_scheduler.tick(now_ms()).await.unwrap();
    assert!(second.failed.is_empty());
}

#[tokio::test]
async fn unchanged_patch_is_noop_and_changed_lists_only_real_edits() {
    let app = setup().await;
    let room = create_room(&app, "编辑房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let due_at = now_ms() + 3_600_000;
    let scheduled_at = now_ms() + 7_200_000;
    let task = create_task_with_fields(
        &app,
        room_id,
        json!({
            "priority": "high",
            "dueAt": due_at,
            "scheduledAt": scheduled_at,
        }),
    )
    .await;
    let task_id = task["id"].as_i64().unwrap();
    let events_before = event_types(&app, task_id).await;

    // 编辑弹层会提交全部字段；与现有值完全相同时不得写库或产生 updated 事件。
    let unchanged = patch(
        &app,
        &format!("/api/tasks/{task_id}"),
        json!({
            "title": "扩展任务",
            "description": "",
            "priority": "high",
            "assigneeAgentId": null,
            "dueAt": due_at,
            "scheduledAt": scheduled_at,
        }),
    )
    .await;
    assert_eq!(unchanged.status, StatusCode::OK, "{:?}", unchanged.body);
    assert_eq!(
        unchanged.body["updatedAt"], task["updatedAt"],
        "无变化的补丁不应刷新更新时间"
    );
    assert_eq!(
        event_types(&app, task_id).await,
        events_before,
        "无变化的补丁不应写 updated 事件"
    );

    // 只改截止：changed 只应包含 dueAt，同值的标题/优先级/指派不进入事件。
    let edited = patch(
        &app,
        &format!("/api/tasks/{task_id}"),
        json!({
            "title": "扩展任务",
            "priority": "high",
            "assigneeAgentId": null,
            "scheduledAt": scheduled_at,
            "dueAt": due_at + 60_000,
        }),
    )
    .await;
    assert_eq!(edited.status, StatusCode::OK, "{:?}", edited.body);
    let payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM task_events WHERE task_id = ? AND type = 'updated' ORDER BY id DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_one(app.state.db.pool())
    .await
    .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["changed"], json!(["dueAt"]));
    assert_eq!(payload["dueAt"], due_at + 60_000);
}

#[tokio::test]
async fn leaving_backlog_clears_scheduled_at() {
    let app = setup().await;
    let room = create_room(&app, "手动启动房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    // 手动启动与进入阻塞都离开 backlog，定时必须一并清除。
    for target in ["in_progress", "blocked"] {
        let task = create_task_with_fields(
            &app,
            room_id,
            json!({ "scheduledAt": now_ms() + 60_000 }),
        )
        .await;
        let task_id = task["id"].as_i64().unwrap();
        let moved = patch(
            &app,
            &format!("/api/tasks/{task_id}/status"),
            json!({ "status": target }),
        )
        .await;
        assert_eq!(moved.status, StatusCode::OK, "{target}: {:?}", moved.body);
        assert!(
            moved.body["scheduledAt"].is_null(),
            "{target} 后定时必须清除"
        );
        let stored: Option<i64> =
            sqlx::query_scalar("SELECT scheduled_at FROM tasks WHERE id = ?")
                .bind(task_id)
                .fetch_one(app.state.db.pool())
                .await
                .unwrap();
        assert_eq!(stored, None, "{target} 后库内定时必须为空");
    }
}

#[tokio::test]
async fn activate_reports_only_real_backlog_start() {
    let app = setup().await;
    let room = create_room(&app, "激活房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let task = create_task(&app, room_id).await;
    let task_id = task["id"].as_i64().unwrap();

    let (started, started_now) = app
        .state
        .task_service
        .activate(&task_id.to_string())
        .await
        .unwrap();
    assert_eq!(started.status, TaskStatus::InProgress);
    assert!(started_now, "首次激活必须报告真正启动");

    let (again, started_again) = app
        .state
        .task_service
        .activate(&task_id.to_string())
        .await
        .unwrap();
    assert_eq!(again.status, TaskStatus::InProgress);
    assert!(!started_again, "重复激活不得再次报告启动");
}

#[tokio::test]
async fn store_rejects_schedule_outside_backlog_inside_transaction() {
    let app = setup().await;
    let room = create_room(&app, "事务校验房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let task = create_task(&app, room_id).await;
    let task_id = task["id"].as_i64().unwrap();
    let started = patch(
        &app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "in_progress" }),
    )
    .await;
    assert_eq!(started.status, StatusCode::OK);

    // 绕过服务层预检直接调用 store，证明写事务内仍有状态复核。
    let store = TaskStore::new(app.state.db.clone());
    let error = store
        .update_fields(
            task_id,
            UpdateTaskFields {
                scheduled_at: Some(Some(now_ms() + 60_000)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ValidationFailed);
    assert_eq!(error.message, "只有待办任务可以设置定时启动");

    let stored: Option<i64> =
        sqlx::query_scalar("SELECT scheduled_at FROM tasks WHERE id = ?")
            .bind(task_id)
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
    assert_eq!(stored, None, "被拒绝的事务不得留下定时");
}
