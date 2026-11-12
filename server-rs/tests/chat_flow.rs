mod common;

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use common::{
    builtin_moderator_id, create_agent, create_room, enable_reuse, get, patch, post,
    setup_with_adapter, wait_for_runs, wait_for_system_message,
};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::room::RoomPermissionMode;
use moon_server::domain::run_event::{RunEvent, RunInput};
use serde_json::json;

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

#[tokio::test]
async fn mention_runs_member_and_persists_messages() {
    let (factory, inputs) = flow_factory(text_events("你好"));
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "研究员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "聊天房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@研究员 你好" }),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{:?}", response.body);
    assert_eq!(response.body["userMessage"]["content"], "@研究员 你好");

    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "succeeded");
    assert_eq!(runs[0]["result"], "你好");

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages"))
        .await
        .body
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["senderType"], "user");
    assert_eq!(messages[1]["senderType"], "agent");
    assert_eq!(messages[1]["content"], "你好");
    assert_eq!(messages[1]["status"], "final");
    assert_eq!(messages[1]["senderAgentId"], agent_id);
    assert_eq!(messages[1]["runId"], runs[0]["id"]);

    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].permission_mode, Some(RoomPermissionMode::Auto));
    assert!(captured[0].session.is_none(), "未开启复用时没有会话上下文");

    let detail = get(
        &app,
        &format!("/api/rooms/{room_id}/runs/{}", runs[0]["id"]),
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.body["run"]["status"], "succeeded");
    assert_eq!(detail.body["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn messages_without_mention_or_moderator_do_not_run() {
    let (factory, inputs) = flow_factory(text_events("不应出现"));
    let app = setup_with_adapter(Some(factory)).await;
    let room = create_room(&app, "安静房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "只是普通消息" }),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED);

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    assert_eq!(messages.body.as_array().unwrap().len(), 1);
    let runs = get(&app, &format!("/api/rooms/{room_id}/runs")).await;
    assert_eq!(runs.body.as_array().unwrap().len(), 0);
    assert!(inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn moderator_reply_mode_takes_over_unmentioned_messages() {
    let (factory, inputs) = flow_factory(text_events("主持人回复"));
    let app = setup_with_adapter(Some(factory)).await;
    let room = create_room(&app, "主持人房间", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let builtin_id = builtin_moderator_id(&app).await;

    let patched = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "replyMode": "moderator" }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK);

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请接管这条消息" }),
    )
    .await;

    wait_for_runs(&app, room_id, 1).await;
    let messages = get(&app, &format!("/api/rooms/{room_id}/messages"))
        .await
        .body
        .as_array()
        .unwrap()
        .clone();
    let reply = messages
        .iter()
        .find(|message| message["senderType"] == "agent")
        .expect("moderator reply");
    assert_eq!(reply["senderAgentId"], builtin_id);
    assert_eq!(reply["content"], "主持人回复");
    assert_eq!(inputs.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_run_keeps_session_and_posts_visible_error() {
    let (factory, inputs) = flow_factory(vec![
        RunEvent::Session("sess-1".to_string()),
        RunEvent::Error("boom".to_string()),
    ]);
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "失败成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "失败房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    enable_reuse(&app, room_id, agent_id).await;

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@失败成员 执行" }),
    )
    .await;

    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "failed");
    assert_eq!(runs[0]["error"], "boom");

    let messages = wait_for_system_message(&app, room_id, "运行失败").await;
    let failed = messages
        .iter()
        .find(|message| message["senderType"] == "agent")
        .expect("failed reply");
    assert_eq!(failed["status"], "failed");

    let sessions = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    let session = &sessions.body["sessions"].as_array().unwrap()[0];
    assert_eq!(session["harnessSessionId"], "sess-1");
    assert_eq!(session["lastRunId"], runs[0]["id"]);

    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].session.is_some());
}

#[tokio::test]
async fn tool_boundaries_split_progress_from_final_reply() {
    let stage = "这是第一阶段的总结，包含足够多的字符用于触发阶段拆分。";
    let (factory, _inputs) = flow_factory(vec![
        RunEvent::TextDelta(stage.to_string()),
        RunEvent::ToolStart {
            tool_call_id: "t1".to_string(),
            name: "shell".to_string(),
            input: json!({ "command": "cargo test" }),
        },
        RunEvent::ToolEnd {
            tool_call_id: Some("t1".to_string()),
            name: Some("shell".to_string()),
            ok: true,
            detail: "ok".to_string(),
        },
        RunEvent::TextDelta("最终结论是完成。".to_string()),
        RunEvent::Final("最终结论是完成。".to_string()),
    ]);
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "阶段成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "阶段房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@阶段成员 开始" }),
    )
    .await;

    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["result"], "最终结论是完成。");

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages"))
        .await
        .body
        .as_array()
        .unwrap()
        .clone();
    let agent_messages: Vec<_> = messages
        .iter()
        .filter(|message| message["senderType"] == "agent")
        .collect();
    assert_eq!(agent_messages.len(), 2);
    assert_eq!(agent_messages[0]["kind"], "progress");
    assert_eq!(agent_messages[0]["content"], stage);
    assert_eq!(agent_messages[1]["kind"], "reply");
    assert_eq!(agent_messages[1]["content"], "最终结论是完成。");

    let detail = get(
        &app,
        &format!("/api/rooms/{room_id}/runs/{}", runs[0]["id"]),
    )
    .await;
    let activities = detail.body["activities"].as_array().unwrap();
    assert!(activities.iter().any(|activity| activity["kind"] == "tool"));
    assert_eq!(detail.body["messages"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn reuse_session_and_permission_mode_are_passed_to_adapter() {
    let (factory, inputs) = flow_factory(vec![
        RunEvent::Session("sess-1".to_string()),
        RunEvent::TextDelta("ok".to_string()),
        RunEvent::Final("ok".to_string()),
    ]);
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "复用成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "复用房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();
    enable_reuse(&app, room_id, agent_id).await;
    let patched = patch(
        &app,
        &format!("/api/rooms/{room_id}"),
        json!({ "permissionMode": "plan" }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK);

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@复用成员 第一轮" }),
    )
    .await;
    wait_for_runs(&app, room_id, 1).await;
    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@复用成员 第二轮" }),
    )
    .await;
    wait_for_runs(&app, room_id, 2).await;

    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 2);
    let first_session = captured[0].session.as_ref().expect("first session");
    assert_eq!(first_session.harness_session_id, None);
    let second_session = captured[1].session.as_ref().expect("second session");
    assert_eq!(
        second_session.harness_session_id.as_deref(),
        Some("sess-1")
    );
    assert_eq!(first_session.storage_dir, second_session.storage_dir);
    assert_eq!(captured[0].permission_mode, Some(RoomPermissionMode::Plan));
    assert_eq!(captured[1].permission_mode, Some(RoomPermissionMode::Plan));

    let sessions = get(&app, &format!("/api/rooms/{room_id}/sessions")).await;
    let items = sessions.body["sessions"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["harnessSessionId"], "sess-1");
    assert_eq!(items[0]["status"], "active");
}

#[tokio::test]
async fn empty_stream_is_reported_as_failure() {
    let (factory, _inputs) = flow_factory(Vec::new());
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "空回复成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "空回复房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@空回复成员 你好" }),
    )
    .await;

    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "failed");
    assert_eq!(runs[0]["error"], "harness 未返回任何回复内容");
}
