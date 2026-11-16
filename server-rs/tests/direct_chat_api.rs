//! 智能体直聊 API 集成测试（AC-001~004）。

mod common;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{
    create_agent, create_room, delete, get, post, post_empty, setup, setup_with_adapter,
    wait_for_runs,
};
use http_body_util::BodyExt;
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::room::{
    CreateRoomInput, RoomPermissionMode, RoomReplyMode, RoomType,
};
use moon_server::domain::run_event::{RunEvent, RunInput};
use serde_json::json;
use tower::ServiceExt;

struct RecordingAdapter {
    events: Vec<RunEvent>,
    inputs: Arc<Mutex<Vec<RunInput>>>,
}

impl HarnessAdapter for RecordingAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.inputs.lock().unwrap().push(input);
        Box::pin(tokio_stream::iter(
            self.events
                .clone()
                .into_iter()
                .map(Ok::<_, moon_server::error::AppError>),
        ))
    }
}

fn flow_factory(events: Vec<RunEvent>) -> (AdapterFactory, Arc<Mutex<Vec<RunInput>>>) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let captured = inputs.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(RecordingAdapter {
            events: events.clone(),
            inputs: captured.clone(),
        }))
    });
    (factory, inputs)
}

fn text_events(text: &str) -> Vec<RunEvent> {
    vec![
        RunEvent::TextDelta(text.to_string()),
        RunEvent::Final(text.to_string()),
    ]
}

fn json_ids(list: &serde_json::Value) -> Vec<i64> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn direct_chat_creation_is_idempotent_and_sandboxed() {
    let app = setup().await;
    let agent = create_agent(&app, "直聊伙伴").await;
    let agent_id = agent["id"].as_i64().unwrap();

    let first = post_empty(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    assert_eq!(first.status, StatusCode::CREATED, "{:?}", first.body);
    let room = &first.body;
    assert_eq!(room["roomType"], "direct");
    assert_eq!(room["name"], "与 直聊伙伴 对话");
    assert_eq!(room["memberIds"], json!([agent_id]));
    assert_eq!(room["moderatorAgentId"], serde_json::Value::Null);
    assert_eq!(room["replyMode"], "mentions");
    assert_eq!(room["permissionMode"], "auto");
    let workspace_dir = room["workspaceDir"].as_str().unwrap();
    let expected_tail = format!("direct-workspaces/direct-{agent_id}");
    assert!(
        workspace_dir.replace('\\', "/").ends_with(&expected_tail),
        "沙盒目录应在 direct-workspaces 下：{workspace_dir}"
    );
    assert!(std::path::Path::new(workspace_dir).is_dir());

    // 幂等：第二次 POST 返回同一房间且不新建。
    let second = post_empty(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.body["id"], room["id"]);

    // 只读查询同样命中。
    let fetched = get(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.body["id"], room["id"]);

    // 权限模式可经既有 PATCH 切换；房间仍是单成员、无主持人。
    let patched = common::patch(
        &app,
        &format!("/api/rooms/{}", room["id"].as_i64().unwrap()),
        json!({ "permissionMode": "plan" }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(patched.body["permissionMode"], "plan");
    assert_eq!(
        patched.body["memberIds"],
        json!([agent_id]),
        "PATCH 不得给直聊回填主持人或其他成员"
    );
    assert_eq!(patched.body["moderatorAgentId"], serde_json::Value::Null);
    assert_eq!(patched.body["name"], "与 直聊伙伴 对话");
}

#[tokio::test]
async fn direct_room_rejects_member_changes_and_generic_patch() {
    let app = setup().await;
    let agent = create_agent(&app, "直聊唯一成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let other = create_agent(&app, "无关成员").await;
    let other_id = other["id"].as_i64().unwrap();
    let direct = post_empty(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    let room_id = direct.body["id"].as_i64().unwrap();

    // 增删成员都会破坏「单成员」不变量，必须显式拒绝。
    let added = post(
        &app,
        &format!("/api/rooms/{room_id}/members"),
        json!({ "agentId": other_id }),
    )
    .await;
    assert_eq!(added.status, StatusCode::CONFLICT, "{:?}", added.body);
    let removed = delete(
        &app,
        &format!("/api/rooms/{room_id}/members/{agent_id}"),
    )
    .await;
    assert_eq!(removed.status, StatusCode::CONFLICT, "{:?}", removed.body);

    // 通用 PATCH 只开放权限与工作目录；名称等房间语义字段拒绝。
    let renamed = common::patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "name": "改个名字" }),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::CONFLICT, "{:?}", renamed.body);

    // 工作目录仍可调整（接入真实项目），且不会带回主持人。
    let workspace = app.dir.path().join("direct-custom-workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let moved = common::patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "workspaceDir": workspace.to_string_lossy() }),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{:?}", moved.body);
    assert_eq!(moved.body["memberIds"], json!([agent_id]));
    assert_eq!(moved.body["moderatorAgentId"], serde_json::Value::Null);

    // 房间行保持唯一，成员未变。
    let fetched = get(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.body["memberIds"], json!([agent_id]));
    assert_eq!(fetched.body["moderatorAgentId"], serde_json::Value::Null);
}

#[tokio::test]
async fn get_direct_chat_is_404_before_creation() {
    let app = setup().await;
    let agent = create_agent(&app, "未开聊").await;
    let agent_id = agent["id"].as_i64().unwrap();

    let fetched = get(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    assert_eq!(fetched.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn room_list_hides_direct_rooms() {
    let app = setup().await;
    let agent = create_agent(&app, "隐藏者").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let group_room = create_room(&app, "普通房", &[agent_id]).await;
    let group_id = group_room["id"].as_i64().unwrap();
    let direct = post_empty(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    let direct_id = direct.body["id"].as_i64().unwrap();

    let groups = get(&app, "/api/rooms").await;
    assert_eq!(groups.status, StatusCode::OK);
    let group_ids = json_ids(&groups.body);
    assert!(group_ids.contains(&group_id));
    assert!(!group_ids.contains(&direct_id), "普通房间列表不应出现直聊");

    let directs = get(&app, "/api/rooms?kind=direct").await;
    assert_eq!(directs.status, StatusCode::OK);
    let direct_ids = json_ids(&directs.body);
    assert_eq!(direct_ids, vec![direct_id]);

    let bogus = get(&app, "/api/rooms?kind=bogus").await;
    assert_eq!(bogus.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn direct_room_routes_message_without_mention() {
    let (factory, inputs) = flow_factory(text_events("直聊回复"));
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "被直聊").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let direct = post_empty(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    let room_id = direct.body["id"].as_i64().unwrap();

    let sent = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "你好，不用 @" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);

    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "succeeded");
    assert_eq!(runs[0]["result"], "直聊回复");
    assert_eq!(
        inputs.lock().unwrap().len(),
        1,
        "无 @ 消息也应触发唯一成员的运行"
    );

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages"))
        .await
        .body
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["senderType"], "user");
    assert_eq!(messages[1]["senderType"], "agent");
    assert_eq!(messages[1]["content"], "直聊回复");
    assert_eq!(messages[1]["status"], "final");
}

#[tokio::test]
async fn deleting_agent_removes_direct_room() {
    let app = setup().await;
    let agent = create_agent(&app, "将被删除").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let direct = post_empty(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    let room_id = direct.body["id"].as_i64().unwrap();
    let workspace_dir = direct.body["workspaceDir"].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&workspace_dir).is_dir());

    let removed = delete(&app, &format!("/api/agents/{agent_id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{:?}", removed.body);

    // 房间行、直聊目录、列表条目全部消失。
    let fetched = get(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    assert_eq!(fetched.status, StatusCode::NOT_FOUND);
    let directs = get(&app, "/api/rooms?kind=direct").await;
    assert!(json_ids(&directs.body).is_empty());
    assert!(app.state.room_store.get_by_id(room_id).await.unwrap().is_none());
    assert!(!std::path::Path::new(&workspace_dir).exists());
}

#[tokio::test]
async fn moderator_backfill_skips_direct_rooms() {
    let app = setup().await;
    let agent = create_agent(&app, "直聊幸存者").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let direct = post_empty(&app, &format!("/api/agents/{agent_id}/direct-chat")).await;
    let room_id = direct.body["id"].as_i64().unwrap();

    // 用同一数据库重启一套 AppState，触发 ensure_and_backfill。
    let restarted = moon_server::AppState::new_with_options(
        app.state.db.clone(),
        app.dir.path().join("sessions"),
        Default::default(),
    )
    .await
    .unwrap();

    let room = restarted
        .room_store
        .get_by_id(room_id)
        .await
        .unwrap()
        .expect("direct room");
    assert_eq!(room.room_type, moon_server::domain::room::RoomType::Direct);
    assert_eq!(room.moderator_agent_id, None, "直聊房间不应被回填主持人");
    assert_eq!(room.member_ids, vec![agent_id], "直聊房间成员不应被扩容");
}

fn direct_input(name: &str, workspace_dir: &str, member_ids: Vec<i64>) -> CreateRoomInput {
    CreateRoomInput {
        name: name.to_string(),
        workspace_dir: workspace_dir.to_string(),
        workspace_dirs: vec![workspace_dir.to_string()],
        moderator_agent_id: None,
        member_ids,
        reply_mode: RoomReplyMode::Mentions,
        discussion_max_rounds: 1,
        permission_mode: RoomPermissionMode::Auto,
        room_type: RoomType::Direct,
    }
}

/// 并发创建必须收敛：存在性检查与插入在同一 IMMEDIATE 事务内完成。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_direct_chat_creation_reuses_one_room() {
    let app = setup().await;
    let agent = create_agent(&app, "并发直聊").await;
    let agent_id = agent["id"].as_i64().unwrap();

    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let router = app.router.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .method(axum::http::Method::POST)
                        .uri(format!("/api/agents/{agent_id}/direct-chat"))
                        .header(axum::http::header::CONTENT_TYPE, "application/json")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            (status, body)
        }));
    }

    let mut ids = Vec::new();
    let mut created = 0;
    for handle in handles {
        let (status, body) = handle.await.unwrap();
        assert!(
            status == StatusCode::OK || status == StatusCode::CREATED,
            "并发创建应成功：{status} {body}"
        );
        if status == StatusCode::CREATED {
            created += 1;
        }
        ids.push(body["id"].as_i64().unwrap());
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 1, "并发创建必须返回同一房间：{ids:?}");
    assert_eq!(created, 1, "只有一个请求真正创建房间");

    let directs = get(&app, "/api/rooms?kind=direct").await;
    assert_eq!(json_ids(&directs.body), ids, "列表中也只应有一个直聊");
}

/// 历史并发缺陷可能留下无成员 direct 房间；列表兜底隐藏，避免界面出现无法清理的条目。
#[tokio::test]
async fn direct_room_list_skips_orphan_rooms_without_members() {
    let app = setup().await;
    let workspace = app.dir.path().join("direct-workspaces").join("direct-orphan");
    let orphan = app
        .state
        .room_store
        .create(direct_input(
            "孤儿直聊",
            &workspace.to_string_lossy(),
            Vec::new(),
        ))
        .await
        .unwrap();

    let directs = get(&app, "/api/rooms?kind=direct").await;
    assert_eq!(directs.status, StatusCode::OK);
    assert!(
        json_ids(&directs.body).is_empty(),
        "无成员直聊不可路由，也不应出现在列表中"
    );
    assert!(
        app.state
            .room_store
            .get_by_id(orphan.id)
            .await
            .unwrap()
            .is_some(),
        "过滤只影响列表，不改写已有行"
    );
}

/// 删除智能体要清理该智能体的全部直聊（含历史并发残留），共享沙盒只删一次。
#[tokio::test]
async fn deleting_agent_removes_legacy_duplicate_direct_rooms() {
    let app = setup().await;
    let agent = create_agent(&app, "历史重复").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let workspace = app
        .dir
        .path()
        .join("direct-workspaces")
        .join(format!("direct-{agent_id}"));
    std::fs::create_dir_all(&workspace).unwrap();

    let mut room_ids = Vec::new();
    for name in ["旧直聊一", "旧直聊二"] {
        let room = app
            .state
            .room_store
            .create(direct_input(name, &workspace.to_string_lossy(), vec![agent_id]))
            .await
            .unwrap();
        room_ids.push(room.id);
    }

    let removed = delete(&app, &format!("/api/agents/{agent_id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{:?}", removed.body);

    for room_id in room_ids {
        assert!(
            app.state
                .room_store
                .get_by_id(room_id)
                .await
                .unwrap()
                .is_none(),
            "重复直聊房间 {room_id} 应被删除"
        );
    }
    assert!(json_ids(&get(&app, "/api/rooms?kind=direct").await.body).is_empty());
    assert!(!workspace.exists(), "共享沙盒目录应在删除智能体时清理");
}
