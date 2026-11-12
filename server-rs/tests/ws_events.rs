mod common;

use std::sync::Arc;

use common::{create_agent, create_room, post, setup_with_adapter};
use futures_util::StreamExt;
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::error::AppError;
use moon_server::{build_app, AppState};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

struct ScriptAdapter {
    events: Vec<RunEvent>,
}

impl HarnessAdapter for ScriptAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, _input: RunInput) -> EventStream {
        Box::pin(tokio_stream::iter(
            self.events.clone().into_iter().map(Ok::<_, AppError>),
        ))
    }
}

fn factory(events: Vec<RunEvent>) -> AdapterFactory {
    Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(ScriptAdapter {
            events: events.clone(),
        }) as Arc<dyn HarnessAdapter>)
    })
}

async fn start_server(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, build_app(state)).await;
    });
    (format!("127.0.0.1:{}", address.port()), handle)
}

#[tokio::test]
async fn ws_receives_run_and_message_events_for_subscribed_room() {
    let app = setup_with_adapter(Some(factory(vec![
        RunEvent::TextDelta("流式".to_string()),
        RunEvent::Final("流式回复".to_string()),
    ])))
    .await;
    let agent = create_agent(&app, "WS 成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "WS 房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let (address, server) = start_server(app.state.clone()).await;
    let url = format!("ws://{address}/ws?roomId={room_id}");
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("connect ws");

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@WS 成员 你好" }),
    )
    .await;

    let mut events: Vec<Value> = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let incoming = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            socket.next(),
        )
        .await;
        let Some(Ok(Message::Text(text))) = incoming.ok().flatten() else {
            continue;
        };
        let event: Value = serde_json::from_str(text.as_str()).unwrap();
        events.push(event.clone());
        if event["type"] == "message_final"
            && event["message"]["senderType"] == "agent"
        {
            break;
        }
    }

    assert!(events.iter().any(|event| event["type"] == "run_started"));
    assert!(events.iter().any(|event| event["type"] == "message_delta"));
    let final_event = events
        .iter()
        .find(|event| {
            event["type"] == "message_final" && event["message"]["senderType"] == "agent"
        })
        .expect("agent message_final");
    assert_eq!(final_event["roomId"], room_id);
    assert_eq!(final_event["message"]["content"], "流式回复");

    server.abort();
}

#[tokio::test]
async fn ws_rejects_missing_or_invalid_room_id() {
    let app = setup_with_adapter(None).await;
    let (address, server) = start_server(app.state.clone()).await;

    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .expect("connect ws");
    let frame = socket.next().await.expect("close frame").unwrap();
    match frame {
        Message::Close(Some(close)) => {
            assert_eq!(u16::from(close.code), 1008);
            assert!(close.reason.as_str().contains("roomId"));
        }
        other => panic!("expected close frame, got {other:?}"),
    }

    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/ws?roomId=abc"
    ))
    .await
    .expect("connect ws");
    let frame = socket.next().await.expect("close frame").unwrap();
    match frame {
        Message::Close(Some(close)) => {
            assert_eq!(u16::from(close.code), 1008);
        }
        other => panic!("expected close frame, got {other:?}"),
    }

    server.abort();
}

#[tokio::test]
async fn ws_subscriber_counts_track_connections() {
    let app = setup_with_adapter(None).await;
    let (address, server) = start_server(app.state.clone()).await;
    assert_eq!(app.state.ws_hub.subscriber_count(1), 0);

    let (socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{address}/ws?roomId=1"))
            .await
            .expect("connect ws");
    // 等待升级完成并注册订阅。
    for _ in 0..100 {
        if app.state.ws_hub.subscriber_count(1) == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(app.state.ws_hub.subscriber_count(1), 1);

    drop(socket);
    server.abort();
}

#[tokio::test]
async fn ws_failure_still_broadcasts_final_and_system_message() {
    let app = setup_with_adapter(Some(factory(vec![RunEvent::Error(
        "ws boom".to_string(),
    )])))
    .await;
    let agent = create_agent(&app, "WS 失败成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "WS 失败房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let (address, server) = start_server(app.state.clone()).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/ws?roomId={room_id}"
    ))
    .await
    .expect("connect ws");

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@WS 失败成员 执行" }),
    )
    .await;

    let mut saw_failed = false;
    let mut saw_system = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && !(saw_failed && saw_system) {
        let incoming = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            socket.next(),
        )
        .await;
        let Some(Ok(Message::Text(text))) = incoming.ok().flatten() else {
            continue;
        };
        let event: Value = serde_json::from_str(text.as_str()).unwrap();
        if event["type"] == "message_final" {
            if event["message"]["senderType"] == "agent"
                && event["message"]["status"] == "failed"
            {
                saw_failed = true;
            }
            if event["message"]["senderType"] == "system"
                && event["message"]["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("运行失败"))
            {
                saw_system = true;
            }
        }
    }

    assert!(saw_failed, "应广播 failed 消息");
    assert!(saw_system, "应广播失败系统消息");
    server.abort();
}

#[tokio::test]
async fn ws_broadcasts_task_changed_and_history_cleared() {
    let app = setup_with_adapter(None).await;
    let agent = create_agent(&app, "看板成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "看板房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    // 留一条纯聊天消息，让清理真正发生并触发广播。
    moon_server::store::MessageStore::new(app.state.db.clone())
        .append(
            moon_server::store::messages::AppendMessageInput::final_message(
                room_id,
                moon_server::domain::message::MessageSenderType::User,
                None,
                "待清理",
            ),
        )
        .await
        .unwrap();

    let (address, server) = start_server(app.state.clone()).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/ws?roomId={room_id}"
    ))
    .await
    .expect("connect ws");

    let task = post(
        &app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({ "title": "看板任务" }),
    )
    .await
    .body;
    let task_id = task["id"].as_i64().unwrap();
    common::patch(
        &app,
        &format!("/api/tasks/{task_id}/status"),
        json!({ "status": "in_progress" }),
    )
    .await;
    post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({}),
    )
    .await;

    let mut task_changed = None;
    let mut history_cleared = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && (task_changed.is_none() || !history_cleared)
    {
        let incoming = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            socket.next(),
        )
        .await;
        let Some(Ok(Message::Text(text))) = incoming.ok().flatten() else {
            continue;
        };
        let event: Value = serde_json::from_str(text.as_str()).unwrap();
        if event["type"] == "task_changed" {
            task_changed = Some(event);
        } else if event["type"] == "history_cleared" {
            assert_eq!(event["roomId"], room_id);
            history_cleared = true;
        }
    }

    let task_changed = task_changed.expect("应广播 task_changed");
    assert_eq!(task_changed["roomId"], room_id);
    assert_eq!(task_changed["taskId"], task_id);
    assert_eq!(task_changed["status"], "in_progress");
    assert!(history_cleared, "应广播 history_cleared");
    server.abort();
}

#[tokio::test]
async fn ws_is_room_scoped() {
    let app = setup_with_adapter(Some(factory(vec![
        RunEvent::TextDelta("ok".to_string()),
        RunEvent::Final("ok".to_string()),
    ])))
    .await;
    let agent = create_agent(&app, "隔离成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "隔离房间", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let (address, server) = start_server(app.state.clone()).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{address}/ws?roomId={}",
        room_id + 99
    ))
    .await
    .expect("connect ws");

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@隔离成员 你好" }),
    )
    .await;
    common::wait_for_runs(&app, room_id, 1).await;

    let incoming = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        socket.next(),
    )
    .await;
    assert!(incoming.is_err(), "其他房间不应收到事件");
    server.abort();
}
