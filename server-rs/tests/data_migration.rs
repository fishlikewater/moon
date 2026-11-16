//! 迁移产物可用性集成测试：旧库搬移后 `Db::open` 能查到原有数据（AC-001）。

use std::path::Path;

use moon_server::db::Db;
use moon_server::domain::room::{
    CreateRoomInput, RoomReplyMode, DEFAULT_ROOM_PERMISSION_MODE,
};
use moon_server::migration::migrate_legacy_desktop_data;
use moon_server::store::RoomStore;

/// 构造一个含房间的旧版布局；返回前必须让连接全部释放（Windows 文件锁
/// 会阻塞后续 rename），因此 Db 与 store 在闭包内用完即 drop。
fn seed_legacy_room(legacy_dir: &Path, room_name: &str) -> i64 {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let room_id = runtime.block_on(async {
        let db = Db::open(&legacy_dir.join("moon.db")).await.unwrap();
        let rooms = RoomStore::new(db.clone());
        let room = rooms
            .create(CreateRoomInput {
                name: room_name.to_string(),
                workspace_dir: ".".to_string(),
                workspace_dirs: vec![".".to_string()],
                moderator_agent_id: None,
                member_ids: Vec::new(),
                reply_mode: RoomReplyMode::Mentions,
                discussion_max_rounds: 5,
                permission_mode: DEFAULT_ROOM_PERMISSION_MODE,
            })
            .await
            .unwrap();
        room.id
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
    // SQLite 干净关闭会 checkpoint 并删除 wal/shm；连接关闭是异步的，
    // 稍等它清理完成，避免迁移撞上「文件正被删除」的窗口。
    let wal = legacy_dir.join("moon.db-wal");
    for _ in 0..40 {
        if !wal.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    room_id
}

#[test]
fn legacy_data_is_queryable_after_migration() {
    let legacy = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let legacy_dir = legacy.path().join("com.moon.workbench");
    let target_dir = target.path().join(".moon");
    let room_id = seed_legacy_room(&legacy_dir, "迁移前房间");

    let report = migrate_legacy_desktop_data(&legacy_dir, &target_dir).unwrap();
    assert!(report.migrated);

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let db = Db::open(&target_dir.join("moon.db")).await.unwrap();
        let rooms = RoomStore::new(db);
        let room = rooms
            .get_by_id(room_id)
            .await
            .unwrap()
            .expect("迁移后的库必须能查到迁移前写入的房间");
        assert_eq!(room.name, "迁移前房间");
        assert_eq!(room.reply_mode, RoomReplyMode::Mentions);
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
}
