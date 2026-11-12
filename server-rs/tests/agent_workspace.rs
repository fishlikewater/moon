mod common;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{
    builtin_moderator_id, get, patch, post, setup_with_adapter, wait_for_runs,
};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use serde_json::json;

struct RecordingAdapter {
    inputs: Arc<Mutex<Vec<RunInput>>>,
}

impl HarnessAdapter for RecordingAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.inputs.lock().unwrap().push(input);
        Box::pin(tokio_stream::iter(vec![Ok(RunEvent::Final(
            "完成".to_string(),
        ))]))
    }
}

fn recording_factory() -> (AdapterFactory, Arc<Mutex<Vec<RunInput>>>) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let captured = inputs.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(RecordingAdapter {
            inputs: captured.clone(),
        }))
    });
    (factory, inputs)
}

fn agent_workspace(body: &serde_json::Value) -> Option<&str> {
    body["workspaceDir"].as_str()
}

#[tokio::test]
async fn create_and_update_agent_workspace() {
    let app = setup_with_adapter(None).await;
    let workspace = app.dir.path().join("agent-workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let workspace_text = workspace.to_string_lossy().into_owned();

    let created = post(
        &app,
        "/api/agents",
        json!({
            "name": "工作区智能体",
            "persona": "负责整理文件",
            "harness": "fake",
            "workspaceDir": format!("  {workspace_text}  "),
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    assert_eq!(
        agent_workspace(&created.body),
        Some(workspace_text.as_str()),
        "workspaceDir 应 trim 后保存"
    );
    let agent_id = created.body["id"].as_i64().unwrap();

    let listed = get(&app, "/api/agents").await;
    assert_eq!(listed.status, StatusCode::OK);
    assert!(listed
        .body
        .as_array()
        .unwrap()
        .iter()
        .any(|agent| agent["id"] == agent_id
            && agent["workspaceDir"].as_str() == Some(workspace_text.as_str())));

    // 不传键时保留原值。
    let renamed = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({ "persona": "改名不改目录" }),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
    assert_eq!(agent_workspace(&renamed.body), Some(workspace_text.as_str()));

    // null 清空。
    let cleared = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({ "workspaceDir": null }),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK, "{:?}", cleared.body);
    assert!(cleared.body["workspaceDir"].is_null());

    // 空串同样清空。
    let set_again = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({ "workspaceDir": workspace_text }),
    )
    .await;
    assert_eq!(set_again.status, StatusCode::OK, "{:?}", set_again.body);
    let cleared_by_empty = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({ "workspaceDir": "  " }),
    )
    .await;
    assert_eq!(
        cleared_by_empty.status,
        StatusCode::OK,
        "{:?}",
        cleared_by_empty.body
    );
    assert!(cleared_by_empty.body["workspaceDir"].is_null());
}

#[tokio::test]
async fn invalid_agent_workspace_is_rejected() {
    let app = setup_with_adapter(None).await;
    let missing = app.dir.path().join("missing-workspace");
    let missing_response = post(
        &app,
        "/api/agents",
        json!({
            "name": "缺失目录",
            "persona": "人设",
            "harness": "fake",
            "workspaceDir": missing.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(
        missing_response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        missing_response.body
    );

    let file_path = app.dir.path().join("not-a-directory.txt");
    std::fs::write(&file_path, "x").unwrap();
    let file_response = post(
        &app,
        "/api/agents",
        json!({
            "name": "文件路径",
            "persona": "人设",
            "harness": "fake",
            "workspaceDir": file_path.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(
        file_response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        file_response.body
    );

    let agent = post(
        &app,
        "/api/agents",
        json!({
            "name": "合法智能体",
            "persona": "人设",
            "harness": "fake",
        }),
    )
    .await;
    let agent_id = agent.body["id"].as_i64().unwrap();
    let update_missing = patch(
        &app,
        &format!("/api/agents/{agent_id}"),
        json!({ "workspaceDir": missing.to_string_lossy() }),
    )
    .await;
    assert_eq!(
        update_missing.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        update_missing.body
    );
}

#[tokio::test]
async fn builtin_moderator_rejects_workspace() {
    let app = setup_with_adapter(None).await;
    let moderator_id = builtin_moderator_id(&app).await;
    let workspace = app.dir.path().join("moderator-workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let response = patch(
        &app,
        &format!("/api/agents/{moderator_id}"),
        json!({ "workspaceDir": workspace.to_string_lossy() }),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CONFLICT,
        "{:?}",
        response.body
    );
}

/// 创建带独立工作区的智能体，返回 (id, 工作区路径)。
async fn create_workspace_agent(app: &common::TestApp, name: &str, dir: &std::path::Path) -> i64 {
    std::fs::create_dir_all(dir).unwrap();
    let response = post(
        app,
        "/api/agents",
        json!({
            "name": name,
            "persona": format!("{name} 的人设"),
            "harness": "fake",
            "workspaceDir": dir.to_string_lossy(),
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body["id"].as_i64().unwrap()
}

#[tokio::test]
async fn run_uses_agent_workspace_and_member_dirs() {
    let (factory, inputs) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;

    let agent_one_dir = app.dir.path().join("agent-one");
    let agent_two_dir = app.dir.path().join("agent-two");
    let room_dir_a = app.dir.path().join("room-a");
    let room_dir_b = app.dir.path().join("room-b");
    for dir in [&room_dir_a, &room_dir_b] {
        std::fs::create_dir_all(dir).unwrap();
    }

    let agent_one = create_workspace_agent(&app, "甲", &agent_one_dir).await;
    let agent_two = create_workspace_agent(&app, "乙", &agent_two_dir).await;

    let room = post(
        &app,
        "/api/rooms",
        json!({
            "name": "工作区房间",
            "workspaceDirs": [
                room_dir_a.to_string_lossy(),
                room_dir_b.to_string_lossy(),
            ],
            "memberIds": [agent_one, agent_two],
            "permissionMode": "auto",
        }),
    )
    .await;
    assert_eq!(room.status, StatusCode::CREATED, "{:?}", room.body);
    let room_id = room.body["id"].as_i64().unwrap();

    let posted = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@甲 看一下" }),
    )
    .await;
    assert_eq!(posted.status, StatusCode::ACCEPTED, "{:?}", posted.body);
    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "succeeded", "{:?}", runs[0]);

    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    let input = &captured[0];
    assert_eq!(
        input.workspace_dir,
        agent_one_dir.to_string_lossy(),
        "运行 cwd 应使用该智能体的独立工作区"
    );
    let expected: Vec<String> = vec![
        room_dir_a.to_string_lossy().into_owned(),
        room_dir_b.to_string_lossy().into_owned(),
        agent_two_dir.to_string_lossy().into_owned(),
    ];
    assert_eq!(input.additional_directories, expected);
}

#[tokio::test]
async fn additional_directories_dedup_and_skip_missing() {
    let (factory, inputs) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;

    let agent_one_dir = app.dir.path().join("dedup-agent-one");
    let missing_dir = app.dir.path().join("dedup-missing");
    let room_dir_a = app.dir.path().join("dedup-room-a");
    let room_dir_b = app.dir.path().join("dedup-room-b");
    std::fs::create_dir_all(&room_dir_a).unwrap();
    std::fs::create_dir_all(&room_dir_b).unwrap();

    let agent_one = create_workspace_agent(&app, "去重甲", &agent_one_dir).await;
    // 成员工作区与房间目录 A 相同：附加目录不得重复。
    let agent_two = create_workspace_agent(&app, "去重乙", &room_dir_a).await;
    // 成员工作区在运行前删除：附加目录不得包含失效路径。
    let agent_three = create_workspace_agent(&app, "去重丙", &missing_dir).await;
    std::fs::remove_dir_all(&missing_dir).unwrap();

    let room = post(
        &app,
        "/api/rooms",
        json!({
            "name": "去重房间",
            "workspaceDirs": [
                room_dir_a.to_string_lossy(),
                room_dir_b.to_string_lossy(),
            ],
            "memberIds": [agent_one, agent_two, agent_three],
            "permissionMode": "auto",
        }),
    )
    .await;
    assert_eq!(room.status, StatusCode::CREATED, "{:?}", room.body);
    let room_id = room.body["id"].as_i64().unwrap();

    let posted = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@去重甲 检查附加目录" }),
    )
    .await;
    assert_eq!(posted.status, StatusCode::ACCEPTED, "{:?}", posted.body);
    wait_for_runs(&app, room_id, 1).await;

    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].additional_directories,
        vec![
            room_dir_a.to_string_lossy().into_owned(),
            room_dir_b.to_string_lossy().into_owned(),
        ],
        "重复路径应去重，已删除的成员工作区应跳过"
    );
}

#[tokio::test]
async fn missing_agent_workspace_falls_back_to_room_dir() {
    let (factory, inputs) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;

    let agent_dir = app.dir.path().join("removed-agent-workspace");
    let agent_id = create_workspace_agent(&app, "回退者", &agent_dir).await;
    let room = common::create_room(&app, "回退房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    let room_workspace = room["workspaceDir"].as_str().unwrap().to_string();

    std::fs::remove_dir_all(&agent_dir).unwrap();

    let posted = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@回退者 继续" }),
    )
    .await;
    assert_eq!(posted.status, StatusCode::ACCEPTED, "{:?}", posted.body);
    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "succeeded", "{:?}", runs[0]);

    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].workspace_dir, room_workspace);
}
