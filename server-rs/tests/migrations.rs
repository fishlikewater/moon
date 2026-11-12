use moon_server::db::migrations::LATEST_VERSION;
use moon_server::db::Db;
use sqlx::Row;

async fn room_columns(db: &Db) -> Vec<String> {
    sqlx::query("PRAGMA table_info(rooms)")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect()
}

#[tokio::test]
async fn fresh_database_reaches_latest_version() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("fresh.db")).await.unwrap();

    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);

    let tables: Vec<String> = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    for table in [
        "agents",
        "rooms",
        "room_members",
        "messages",
        "tasks",
        "task_events",
        "agent_runs",
        "discussions",
        "discussion_participants",
        "collaboration_batches",
        "collaboration_assignments",
        "agent_sessions",
        "app_settings",
        "message_attachments",
    ] {
        assert!(tables.contains(&table.to_string()), "missing table {table}");
    }

    let columns = room_columns(&db).await;
    assert!(columns.contains(&"permission_mode".to_string()));
    assert!(columns.contains(&"discussion_max_rounds".to_string()));
    assert!(columns.contains(&"reply_mode".to_string()));
    assert!(columns.contains(&"workspace_dirs".to_string()));

    let run_columns: Vec<String> = sqlx::query("PRAGMA table_info(agent_runs)")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(run_columns.contains(&"token_count".to_string()));
    assert!(run_columns.contains(&"context_size".to_string()));

    let task_columns: Vec<String> = sqlx::query("PRAGMA table_info(tasks)")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    for column in [
        "priority",
        "assignee_agent_id",
        "due_at",
        "scheduled_at",
    ] {
        assert!(
            task_columns.contains(&column.to_string()),
            "missing tasks.{column}"
        );
    }

    db.close().await;
}

#[tokio::test]
async fn reopening_does_not_reapply_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reopen.db");

    let db = Db::open(&path).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);
    db.close().await;

    let db = Db::open(&path).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);
    db.close().await;
}

#[tokio::test]
async fn legacy_v9_database_is_upgraded_with_full_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");

    let db = Db::open(&path).await.unwrap();
    sqlx::raw_sql("ALTER TABLE rooms DROP COLUMN permission_mode; PRAGMA user_version = 9;")
        .execute(db.pool())
        .await
        .unwrap();
    db.close().await;

    let db = Db::open(&path).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);
    assert!(room_columns(&db)
        .await
        .contains(&"permission_mode".to_string()));

    sqlx::query("INSERT INTO rooms (name, workspace_dir, created_at) VALUES (?, ?, ?)")
        .bind("legacy")
        .bind("E:/tmp/legacy")
        .bind(1_i64)
        .execute(db.pool())
        .await
        .unwrap();
    let mode: String = sqlx::query_scalar("SELECT permission_mode FROM rooms WHERE name = ?")
        .bind("legacy")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(mode, "full", "旧房间迁移后应保持完全访问");

    db.close().await;
}

#[tokio::test]
async fn existing_v10_room_gets_workspace_dirs_backfill() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v10-workspace.db");

    let db = Db::open(&path).await.unwrap();
    sqlx::query(
        "INSERT INTO rooms (name, workspace_dir, permission_mode, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind("legacy-workspace")
    .bind("E:/tmp/repo")
    .bind("auto")
    .bind(3_i64)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::raw_sql("UPDATE rooms SET workspace_dirs = NULL; PRAGMA user_version = 10;")
        .execute(db.pool())
        .await
        .unwrap();
    db.close().await;

    let db = Db::open(&path).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);
    let dirs: String = sqlx::query_scalar("SELECT workspace_dirs FROM rooms WHERE name = ?")
        .bind("legacy-workspace")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(dirs, "[\"E:/tmp/repo\"]");
    db.close().await;
}

#[tokio::test]
async fn existing_v10_permission_mode_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v10.db");

    let db = Db::open(&path).await.unwrap();
    sqlx::query(
        "INSERT INTO rooms (name, workspace_dir, permission_mode, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind("plan-room")
    .bind("E:/tmp/plan")
    .bind("plan")
    .bind(2_i64)
    .execute(db.pool())
    .await
    .unwrap();
    db.close().await;

    let db = Db::open(&path).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);
    let mode: String = sqlx::query_scalar("SELECT permission_mode FROM rooms WHERE name = ?")
        .bind("plan-room")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(mode, "plan");
    db.close().await;
}

#[tokio::test]
async fn v12_adds_custom_harness_registry_and_agent_column() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v12.db");

    // 先造一条 v11 的既有智能体，验证升级不改旧数据。
    let db = Db::open(&path).await.unwrap();
    sqlx::query(
        "INSERT INTO agents (name, persona, harness, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind("旧成员")
    .bind("人设")
    .bind("fake")
    .bind(1_i64)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::raw_sql("PRAGMA user_version = 11;")
        .execute(db.pool())
        .await
        .unwrap();
    db.close().await;

    let db = Db::open(&path).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);

    let tables: Vec<String> =
        sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(db.pool())
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
    assert!(tables.contains(&"custom_harnesses".to_string()));

    let columns: Vec<String> = sqlx::query("PRAGMA table_info(agents)")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(columns.contains(&"custom_harness_id".to_string()));

    // 注册表可插入，旧智能体的 harness 与绑定保持不变。
    sqlx::query(
        "INSERT INTO custom_harnesses (name, command, args_json, env_json, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind("gemini")
    .bind("gemini")
    .bind("[]")
    .bind("{}")
    .bind(2_i64)
    .execute(db.pool())
    .await
    .unwrap();

    let harness_text: String =
        sqlx::query_scalar("SELECT harness FROM agents WHERE name = ?")
            .bind("旧成员")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(harness_text, "fake");
    let binding: Option<i64> =
        sqlx::query_scalar("SELECT custom_harness_id FROM agents WHERE name = ?")
            .bind("旧成员")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(binding, None);

    // 幂等：再次打开不重复执行。
    db.close().await;
    let db = Db::open(&path).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);
    db.close().await;
}

#[tokio::test]
async fn v14_adds_acp_capability_columns() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("v14.db")).await.unwrap();
    assert_eq!(db.user_version().await.unwrap(), LATEST_VERSION);

    let columns: Vec<String> = sqlx::query("PRAGMA table_info(custom_harnesses)")
        .fetch_all(db.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(columns.contains(&"capabilities_json".to_string()));
    assert!(columns.contains(&"probed_at".to_string()));
    db.close().await;
}
