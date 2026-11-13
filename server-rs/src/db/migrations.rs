//! 数据库迁移：v1–v10 DDL（历史契约来自已移除的 TypeScript 后端）。
//!
//! 契约保持：
//! - 版本号存放在 `PRAGMA user_version`，按版本单调递增执行；
//! - 每个版本的全部语句与版本号推进在同一事务内，失败整体回滚；
//! - `ADD COLUMN` 做幂等检查，允许回退 `user_version` 后重跑；
//! - 已发布的语句不可修改，结构变更只能追加新版本。

use sqlx::sqlite::Sqlite;
use sqlx::{Row, SqlitePool, Transaction};

use crate::error::AppResult;

pub const LATEST_VERSION: i64 = 21;

enum Step {
    /// 一段可包含多条语句的 DDL/DML。
    Sql(&'static str),
    /// 幂等追加列；`PRAGMA table_info` 已存在该列时跳过。
    AddColumn {
        table: &'static str,
        column: &'static str,
        definition: &'static str,
    },
    /// v3：回填 v2 之前写入消息的 `task_id`。
    BackfillTaskIds,
}

struct Migration {
    version: i64,
    steps: &'static [Step],
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        steps: &[Step::Sql(V1)],
    },
    Migration {
        version: 2,
        steps: &[Step::Sql(V2)],
    },
    Migration {
        version: 3,
        steps: &[Step::BackfillTaskIds],
    },
    Migration {
        version: 4,
        steps: &[
            Step::AddColumn {
                table: "agents",
                column: "builtin_role",
                definition: "TEXT",
            },
            Step::Sql(V4_INDEX),
            Step::AddColumn {
                table: "rooms",
                column: "reply_mode",
                definition: "TEXT NOT NULL DEFAULT 'mentions'",
            },
            Step::AddColumn {
                table: "tasks",
                column: "plan_json",
                definition: "TEXT",
            },
            Step::AddColumn {
                table: "tasks",
                column: "created_by_agent_id",
                definition: "INTEGER REFERENCES agents (id)",
            },
        ],
    },
    Migration {
        version: 5,
        steps: &[
            Step::Sql(V5_TABLES),
            Step::AddColumn {
                table: "messages",
                column: "discussion_id",
                definition: "INTEGER REFERENCES discussions (id)",
            },
            Step::AddColumn {
                table: "messages",
                column: "discussion_round",
                definition: "INTEGER",
            },
            Step::AddColumn {
                table: "messages",
                column: "discussion_stance",
                definition: "TEXT CHECK (discussion_stance IN ('propose', 'support', 'oppose', 'question'))",
            },
            Step::AddColumn {
                table: "messages",
                column: "discussion_reply_to_message_id",
                definition: "INTEGER REFERENCES messages (id)",
            },
            Step::AddColumn {
                table: "messages",
                column: "discussion_stop_vote",
                definition: "TEXT CHECK (discussion_stop_vote IN ('continue', 'stop'))",
            },
            Step::AddColumn {
                table: "messages",
                column: "discussion_stop_reason",
                definition: "TEXT",
            },
            Step::Sql(V5_MESSAGE_INDEX),
        ],
    },
    Migration {
        version: 6,
        steps: &[Step::AddColumn {
            table: "rooms",
            column: "discussion_max_rounds",
            definition: "INTEGER NOT NULL DEFAULT 5",
        }],
    },
    Migration {
        version: 7,
        steps: &[
            Step::Sql(V7_TABLES),
            Step::AddColumn {
                table: "messages",
                column: "collaboration_batch_id",
                definition: "INTEGER REFERENCES collaboration_batches (id)",
            },
            Step::AddColumn {
                table: "messages",
                column: "collaboration_round",
                definition: "INTEGER",
            },
            Step::AddColumn {
                table: "messages",
                column: "collaboration_role",
                definition: "TEXT CHECK (collaboration_role IN ('assignment', 'summary', 'system'))",
            },
            Step::Sql(V7_MESSAGE_INDEX),
        ],
    },
    Migration {
        version: 8,
        steps: &[
            Step::AddColumn {
                table: "messages",
                column: "kind",
                definition: "TEXT NOT NULL DEFAULT 'reply' CHECK (kind IN ('reply', 'progress'))",
            },
            Step::AddColumn {
                table: "messages",
                column: "run_id",
                definition: "INTEGER REFERENCES agent_runs (id)",
            },
            Step::Sql(V8_INDEX),
        ],
    },
    Migration {
        version: 9,
        steps: &[
            Step::Sql(V9_TABLES),
            Step::AddColumn {
                table: "room_members",
                column: "reuse_session",
                definition: "INTEGER NOT NULL DEFAULT 0",
            },
            Step::AddColumn {
                table: "agent_runs",
                column: "harness_session_id",
                definition: "TEXT",
            },
        ],
    },
    Migration {
        version: 10,
        steps: &[Step::AddColumn {
            table: "rooms",
            column: "permission_mode",
            definition: "TEXT NOT NULL DEFAULT 'full' CHECK (permission_mode IN ('plan','auto','full'))",
        }],
    },
    Migration {
        version: 11,
        steps: &[
            Step::AddColumn {
                table: "rooms",
                column: "workspace_dirs",
                definition: "TEXT",
            },
            Step::Sql(BACKFILL_WORKSPACE_DIRS),
        ],
    },
    Migration {
        version: 12,
        steps: &[
            Step::Sql(V12_TABLES),
            Step::AddColumn {
                table: "agents",
                column: "custom_harness_id",
                definition: "INTEGER REFERENCES custom_harnesses (id)",
            },
        ],
    },
    Migration {
        version: 13,
        steps: &[Step::Sql(V13_SETTINGS)],
    },
    Migration {
        version: 14,
        steps: &[
            Step::AddColumn {
                table: "custom_harnesses",
                column: "capabilities_json",
                definition: "TEXT",
            },
            Step::AddColumn {
                table: "custom_harnesses",
                column: "probed_at",
                definition: "INTEGER",
            },
        ],
    },
    Migration {
        version: 15,
        steps: &[Step::Sql(V15_ATTACHMENTS)],
    },
    Migration {
        version: 16,
        steps: &[
            Step::AddColumn {
                table: "agent_runs",
                column: "token_count",
                definition: "INTEGER",
            },
            Step::AddColumn {
                table: "agent_runs",
                column: "context_size",
                definition: "INTEGER",
            },
        ],
    },
    Migration {
        version: 17,
        steps: &[
            Step::AddColumn {
                table: "messages",
                column: "reply_to_message_id",
                // 不建外键：目标被清理后保留 id，前端据此显示「原消息已清理」。
                definition: "INTEGER",
            },
            Step::Sql(V17_REACTIONS),
        ],
    },
    Migration {
        version: 18,
        steps: &[
            Step::AddColumn {
                table: "rooms",
                column: "queue_paused",
                definition: "INTEGER NOT NULL DEFAULT 0",
            },
            Step::Sql(V18_QUEUE_JOBS),
        ],
    },
    Migration {
        version: 19,
        steps: &[Step::AddColumn {
            table: "agents",
            column: "workspace_dir",
            definition: "TEXT",
        }],
    },
    Migration {
        version: 20,
        steps: &[
            Step::AddColumn {
                table: "tasks",
                column: "priority",
                definition: "TEXT NOT NULL DEFAULT 'normal'",
            },
            Step::AddColumn {
                table: "tasks",
                column: "assignee_agent_id",
                definition: "INTEGER",
            },
            Step::AddColumn {
                table: "tasks",
                column: "due_at",
                definition: "INTEGER",
            },
            Step::AddColumn {
                table: "tasks",
                column: "scheduled_at",
                definition: "INTEGER",
            },
            Step::Sql(
                "CREATE INDEX IF NOT EXISTS idx_tasks_scheduled
                 ON tasks (status, scheduled_at);",
            ),
        ],
    },
    Migration {
        version: 21,
        steps: &[
            Step::AddColumn {
                table: "agent_sessions",
                column: "last_context_message_id",
                definition: "INTEGER NOT NULL DEFAULT 0",
            },
            Step::Sql(V21_CONTEXT_WATERMARK),
        ],
    },
];

/// v21：会话复用改为默认开启，并按会话状态初始化上下文水位。
///
/// - 新建/无 harness 会话从 0 开始，首次运行可 bootstrap 最近房间历史；
/// - 已有 harness 会话从房间当前最大消息 id 开始，避免重复投递。
const V21_CONTEXT_WATERMARK: &str = r#"
UPDATE room_members SET reuse_session = 1;

UPDATE agent_sessions
   SET last_context_message_id = CASE
         WHEN harness_session_id IS NULL THEN 0
         ELSE (
           SELECT COALESCE(MAX(id), 0)
             FROM messages
            WHERE messages.room_id = agent_sessions.room_id
         )
       END;
"#;

/// 按 `PRAGMA user_version` 推进迁移，返回迁移后的版本号。
pub async fn run_migrations(pool: &SqlitePool) -> AppResult<i64> {
    let current: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?;

    for migration in MIGRATIONS.iter().filter(|item| item.version > current) {
        let mut tx = pool.begin().await?;
        for step in migration.steps {
            apply_step(&mut tx, step).await?;
        }
        sqlx::raw_sql(&format!("PRAGMA user_version = {};", migration.version))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }

    Ok(sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?)
}

async fn apply_step(tx: &mut Transaction<'_, Sqlite>, step: &Step) -> AppResult<()> {
    match step {
        Step::Sql(sql) => {
            sqlx::raw_sql(sql).execute(&mut **tx).await?;
        }
        Step::AddColumn {
            table,
            column,
            definition,
        } => {
            let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
                .fetch_all(&mut **tx)
                .await?;
            let exists = rows
                .iter()
                .any(|row| row.get::<String, _>("name") == *column);
            if !exists {
                sqlx::raw_sql(&format!(
                    "ALTER TABLE {table} ADD COLUMN {column} {definition};"
                ))
                .execute(&mut **tx)
                .await?;
            }
        }
        Step::BackfillTaskIds => {
            sqlx::raw_sql(BACKFILL_TASK_IDS).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

const V1: &str = r#"
CREATE TABLE IF NOT EXISTS agents (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  persona TEXT NOT NULL,
  harness TEXT NOT NULL,
  model_config_json TEXT,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS rooms (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  workspace_dir TEXT NOT NULL,
  moderator_agent_id INTEGER,
  created_at INTEGER NOT NULL,
  FOREIGN KEY (moderator_agent_id) REFERENCES agents (id)
);

CREATE TABLE IF NOT EXISTS room_members (
  room_id INTEGER NOT NULL,
  agent_id INTEGER NOT NULL,
  PRIMARY KEY (room_id, agent_id),
  FOREIGN KEY (room_id) REFERENCES rooms (id),
  FOREIGN KEY (agent_id) REFERENCES agents (id)
);

CREATE TABLE IF NOT EXISTS messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL,
  seq INTEGER NOT NULL,
  sender_type TEXT NOT NULL CHECK (sender_type IN ('user', 'agent', 'system')),
  sender_agent_id INTEGER,
  status TEXT NOT NULL CHECK (status IN ('final', 'streaming', 'failed')),
  content TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  FOREIGN KEY (room_id) REFERENCES rooms (id),
  FOREIGN KEY (sender_agent_id) REFERENCES agents (id)
);

CREATE TABLE IF NOT EXISTS tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (room_id) REFERENCES rooms (id)
);

CREATE TABLE IF NOT EXISTS task_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL,
  type TEXT NOT NULL,
  payload_json TEXT,
  created_at INTEGER NOT NULL,
  FOREIGN KEY (task_id) REFERENCES tasks (id)
);

CREATE TABLE IF NOT EXISTS agent_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL,
  agent_id INTEGER NOT NULL,
  task_id INTEGER,
  status TEXT NOT NULL,
  prompt TEXT NOT NULL,
  result TEXT,
  error TEXT,
  created_at INTEGER NOT NULL,
  finished_at INTEGER,
  FOREIGN KEY (room_id) REFERENCES rooms (id),
  FOREIGN KEY (agent_id) REFERENCES agents (id),
  FOREIGN KEY (task_id) REFERENCES tasks (id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_room_seq
  ON messages (room_id, seq);
CREATE INDEX IF NOT EXISTS idx_task_events_task_id
  ON task_events (task_id);
CREATE INDEX IF NOT EXISTS idx_agent_runs_room_id
  ON agent_runs (room_id);
CREATE INDEX IF NOT EXISTS idx_agent_runs_task_id
  ON agent_runs (task_id);
"#;

const V2: &str = r#"
ALTER TABLE messages ADD COLUMN task_id INTEGER REFERENCES tasks (id);

CREATE INDEX IF NOT EXISTS idx_messages_task_id
  ON messages (task_id);
"#;

const V4_INDEX: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_builtin_role
  ON agents (builtin_role)
  WHERE builtin_role IS NOT NULL;
"#;

const V5_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS discussions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL,
  topic TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'stopped', 'failed')),
  current_round INTEGER NOT NULL,
  max_rounds INTEGER NOT NULL,
  stop_reason TEXT,
  summary TEXT,
  created_by_agent_id INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (room_id) REFERENCES rooms (id),
  FOREIGN KEY (created_by_agent_id) REFERENCES agents (id)
);

CREATE TABLE IF NOT EXISTS discussion_participants (
  discussion_id INTEGER NOT NULL,
  agent_id INTEGER NOT NULL,
  position INTEGER NOT NULL,
  PRIMARY KEY (discussion_id, agent_id),
  UNIQUE (discussion_id, position),
  FOREIGN KEY (discussion_id) REFERENCES discussions (id),
  FOREIGN KEY (agent_id) REFERENCES agents (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_discussions_room_id
  ON discussions (room_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_discussions_running_room
  ON discussions (room_id)
  WHERE status = 'running';
CREATE INDEX IF NOT EXISTS idx_discussion_participants_agent
  ON discussion_participants (agent_id);
"#;

const V5_MESSAGE_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_messages_discussion_id
  ON messages (discussion_id, seq);
"#;

const V7_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS collaboration_batches (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL,
  moderator_agent_id INTEGER NOT NULL,
  user_message_id INTEGER NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'stopped', 'failed')),
  current_round INTEGER NOT NULL,
  max_rounds INTEGER NOT NULL,
  stop_reason TEXT,
  stop_requested INTEGER NOT NULL DEFAULT 0,
  summary TEXT,
  review_run_id INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (room_id) REFERENCES rooms (id),
  FOREIGN KEY (moderator_agent_id) REFERENCES agents (id),
  FOREIGN KEY (user_message_id) REFERENCES messages (id)
);

CREATE TABLE IF NOT EXISTS collaboration_assignments (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_id INTEGER NOT NULL,
  round INTEGER NOT NULL,
  agent_id INTEGER NOT NULL,
  instruction TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'skipped')),
  run_id INTEGER,
  message_id INTEGER,
  error TEXT,
  created_at INTEGER NOT NULL,
  finished_at INTEGER,
  FOREIGN KEY (batch_id) REFERENCES collaboration_batches (id),
  FOREIGN KEY (agent_id) REFERENCES agents (id),
  FOREIGN KEY (run_id) REFERENCES agent_runs (id),
  FOREIGN KEY (message_id) REFERENCES messages (id)
);

CREATE INDEX IF NOT EXISTS idx_collaboration_batches_room
  ON collaboration_batches (room_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_collaboration_batches_running_room
  ON collaboration_batches (room_id)
  WHERE status = 'running';
CREATE INDEX IF NOT EXISTS idx_collaboration_assignments_batch
  ON collaboration_assignments (batch_id, round, id);
"#;

const V7_MESSAGE_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_messages_collaboration_batch
  ON messages (collaboration_batch_id, seq);
"#;

const V8_INDEX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_messages_run_id
  ON messages (run_id, seq);
"#;

const V9_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS agent_sessions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL,
  agent_id INTEGER NOT NULL,
  harness TEXT NOT NULL,
  harness_session_id TEXT,
  status TEXT NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'archived')),
  storage_dir TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_used_at INTEGER,
  last_run_id INTEGER,
  FOREIGN KEY (room_id) REFERENCES rooms (id),
  FOREIGN KEY (agent_id) REFERENCES agents (id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_sessions_active
  ON agent_sessions (room_id, agent_id)
  WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_agent_sessions_room
  ON agent_sessions (room_id, id);
"#;

/// v12：自定义 ACP harness 注册表。
const V12_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS custom_harnesses (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  command TEXT NOT NULL,
  args_json TEXT NOT NULL DEFAULT '[]',
  env_json TEXT NOT NULL DEFAULT '[]',
  created_at INTEGER NOT NULL
);
"#;

/// v15：消息附件元数据。
const V15_ATTACHMENTS: &str = r#"
CREATE TABLE IF NOT EXISTS message_attachments (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL REFERENCES rooms (id) ON DELETE CASCADE,
  message_id INTEGER REFERENCES messages (id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  size INTEGER NOT NULL,
  storage_path TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_message_attachments_room
  ON message_attachments (room_id, message_id);
"#;

/// v17：消息回复引用与表情反应。
const V17_REACTIONS: &str = r#"
CREATE TABLE IF NOT EXISTS message_reactions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
  emoji TEXT NOT NULL,
  actor TEXT NOT NULL DEFAULT 'user',
  created_at INTEGER NOT NULL,
  UNIQUE (message_id, actor, emoji)
);
CREATE INDEX IF NOT EXISTS idx_message_reactions_message_id
  ON message_reactions (message_id);
"#;

/// v18：房间队列持久化。只存尚未开跑的任务；开跑或取消时删除行。
const V18_QUEUE_JOBS: &str = r#"
CREATE TABLE IF NOT EXISTS room_queue_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  room_id INTEGER NOT NULL REFERENCES rooms (id) ON DELETE CASCADE,
  agent_id INTEGER NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
  label TEXT NOT NULL,
  kind TEXT NOT NULL,
  instruction TEXT NOT NULL,
  moderator_trigger_message_id INTEGER,
  enqueued_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_room_queue_jobs_room
  ON room_queue_jobs (room_id, id);
"#;

/// v13：应用级设置键值表。
const V13_SETTINGS: &str = r#"
CREATE TABLE IF NOT EXISTS app_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
"#;

/// v11：把单工作目录回填进 `workspace_dirs` JSON 数组。
const BACKFILL_WORKSPACE_DIRS: &str = r#"
UPDATE rooms
   SET workspace_dirs = json_array(workspace_dir)
 WHERE workspace_dirs IS NULL OR workspace_dirs = '';
"#;

/// 回填 v2 之前消息的 `task_id`。
///
/// 只处理可确定性归属的消息：agent 消息在运行时间窗口内恰好只有 1 条
/// 关联任务的运行；system 消息同房间内恰好只有 1 个任务在前后 1 秒内
/// 发生 `status_changed`。无法唯一确定时保持 NULL，不做启发式猜测。
const BACKFILL_TASK_IDS: &str = r#"
UPDATE messages
   SET task_id = (
     SELECT r.task_id
       FROM agent_runs AS r
      WHERE r.room_id = messages.room_id
        AND r.agent_id = messages.sender_agent_id
        AND r.task_id IS NOT NULL
        AND messages.created_at BETWEEN r.created_at
            AND COALESCE(r.finished_at, r.created_at + 60000)
   )
 WHERE messages.task_id IS NULL
   AND messages.sender_type = 'agent'
   AND messages.sender_agent_id IS NOT NULL
   AND (
     SELECT COUNT(*)
       FROM agent_runs AS r
      WHERE r.room_id = messages.room_id
        AND r.agent_id = messages.sender_agent_id
        AND r.task_id IS NOT NULL
        AND messages.created_at BETWEEN r.created_at
            AND COALESCE(r.finished_at, r.created_at + 60000)
   ) = 1;

UPDATE messages
   SET task_id = (
     SELECT MIN(e.task_id)
       FROM task_events AS e
       JOIN tasks AS t ON t.id = e.task_id
      WHERE t.room_id = messages.room_id
        AND e.type = 'status_changed'
        AND ABS(e.created_at - messages.created_at) <= 1000
   )
 WHERE messages.task_id IS NULL
   AND messages.sender_type = 'system'
   AND (
     SELECT COUNT(DISTINCT e.task_id)
       FROM task_events AS e
       JOIN tasks AS t ON t.id = e.task_id
      WHERE t.room_id = messages.room_id
        AND e.type = 'status_changed'
        AND ABS(e.created_at - messages.created_at) <= 1000
   ) = 1;
"#;
