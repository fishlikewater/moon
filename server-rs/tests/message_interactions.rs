//! 回复引用与表情反应的 API 契约：持久化、房间边界、清理与广播。

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{create_room, get, post, setup, TestApp};
use futures_util::StreamExt;
use moon_server::{build_app, AppState};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

async fn send_message(app: &TestApp, room_id: i64, body: Value) -> Value {
    let response = post(app, &format!("/api/rooms/{room_id}/messages"), body).await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{:?}", response.body);
    response.body["userMessage"].clone()
}

async fn messages(app: &TestApp, room_id: i64) -> Vec<Value> {
    let response = get(app, &format!("/api/rooms/{room_id}/messages")).await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    response.body.as_array().cloned().unwrap_or_default()
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
async fn reply_persists_and_rejects_cross_room_or_unknown_target() {
    let app = setup().await;
    let room = create_room(&app, "回复房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let other_room = create_room(&app, "另一间房", &[]).await;
    let other_room_id = other_room["id"].as_i64().unwrap();

    let original = send_message(&app, room_id, json!({ "content": "原始消息" })).await;
    let original_id = original["id"].as_i64().unwrap();

    let reply = send_message(
        &app,
        room_id,
        json!({ "content": "回复内容", "replyToId": original_id }),
    )
    .await;
    assert_eq!(reply["replyToMessageId"], original_id);

    let stored = messages(&app, room_id).await;
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[1]["replyToMessageId"], original_id);

    let foreign = send_message(&app, other_room_id, json!({ "content": "别处消息" })).await;
    let foreign_id = foreign["id"].as_i64().unwrap();
    let cross_room = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "跨房回复", "replyToId": foreign_id }),
    )
    .await;
    assert_eq!(cross_room.status, StatusCode::BAD_REQUEST, "{:?}", cross_room.body);

    let unknown = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "回复幽灵", "replyToId": 999_999 }),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{:?}", unknown.body);

    for invalid in [json!(0), json!(-3), json!(1.5), json!("abc")] {
        let response = post(
            &app,
            &format!("/api/rooms/{room_id}/messages"),
            json!({ "content": "非法目标", "replyToId": invalid }),
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "replyToId={invalid} 应被拒绝：{:?}",
            response.body
        );
    }
}

#[tokio::test]
async fn cleanup_keeps_reply_reference_when_target_removed() {
    let app = setup().await;
    let room = create_room(&app, "引用清理房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let original = send_message(&app, room_id, json!({ "content": "将被删除" })).await;
    let original_id = original["id"].as_i64().unwrap();
    send_message(
        &app,
        room_id,
        json!({ "content": "引用它", "replyToId": original_id }),
    )
    .await;

    // keepRecent=1 只保留最后一条回复，删除更早的被回复消息。
    let cleanup = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": 1 }),
    )
    .await;
    assert_eq!(cleanup.status, StatusCode::OK, "{:?}", cleanup.body);

    let stored = messages(&app, room_id).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["content"], "引用它");
    // 保留悬空 id，前端才能渲染「原消息已清理」。
    assert_eq!(stored[0]["replyToMessageId"], original_id);
}

#[tokio::test]
async fn reactions_toggle_persist_and_are_removed_with_messages() {
    let app = setup().await;
    let room = create_room(&app, "反应房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let other_room = create_room(&app, "反应别处", &[]).await;
    let other_room_id = other_room["id"].as_i64().unwrap();
    let message = send_message(&app, room_id, json!({ "content": "点赞我" })).await;
    let message_id = message["id"].as_i64().unwrap();
    let foreign = send_message(&app, other_room_id, json!({ "content": "别处" })).await;
    let foreign_id = foreign["id"].as_i64().unwrap();

    let added = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/{message_id}/reactions"),
        json!({ "emoji": "👍" }),
    )
    .await;
    assert_eq!(added.status, StatusCode::OK, "{:?}", added.body);
    assert_eq!(
        added.body["reactions"],
        json!([{ "emoji": "👍", "count": 1, "reactedByMe": true }])
    );

    let stored = messages(&app, room_id).await;
    let stored_message = stored.iter().find(|item| item["id"] == message_id).unwrap();
    assert_eq!(
        stored_message["reactions"],
        json!([{ "emoji": "👍", "count": 1, "reactedByMe": true }])
    );

    let removed = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/{message_id}/reactions"),
        json!({ "emoji": "👍" }),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK);
    assert_eq!(removed.body["reactions"], json!([]));

    let invalid = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/{message_id}/reactions"),
        json!({ "emoji": "🦄" }),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{:?}", invalid.body);

    let cross_room = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/{foreign_id}/reactions"),
        json!({ "emoji": "👍" }),
    )
    .await;
    assert_eq!(cross_room.status, StatusCode::NOT_FOUND, "{:?}", cross_room.body);

    let unknown = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/999999/reactions"),
        json!({ "emoji": "👍" }),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{:?}", unknown.body);

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages/{message_id}/reactions"),
        json!({ "emoji": "🎉" }),
    )
    .await;
    let cleanup = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": 0 }),
    )
    .await;
    assert_eq!(cleanup.status, StatusCode::OK, "{:?}", cleanup.body);
    let reaction_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_reactions")
        .fetch_one(app.state.db.pool())
        .await
        .unwrap();
    assert_eq!(reaction_count, 0, "清理消息后不应残留反应");
}

#[tokio::test]
async fn reaction_change_is_broadcast_to_room() {
    let app = setup().await;
    let room = create_room(&app, "反应广播房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let message = send_message(&app, room_id, json!({ "content": "广播我" })).await;
    let message_id = message["id"].as_i64().unwrap();

    let (address, server) = start_server(app.state.clone()).await;
    let url = format!("ws://{address}/ws?roomId={room_id}");
    let (mut socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("connect ws");

    post(
        &app,
        &format!("/api/rooms/{room_id}/messages/{message_id}/reactions"),
        json!({ "emoji": "👀" }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "未收到 message_reactions_changed 事件"
        );
        let incoming =
            tokio::time::timeout(Duration::from_millis(500), socket.next()).await;
        let Some(Ok(Message::Text(text))) = incoming.ok().flatten() else {
            continue;
        };
        let event: Value = serde_json::from_str(text.as_str()).unwrap();
        if event["type"] != "message_reactions_changed" {
            continue;
        }
        assert_eq!(event["roomId"], room_id);
        assert_eq!(event["messageId"], message_id);
        assert_eq!(
            event["reactions"],
            json!([{ "emoji": "👀", "count": 1, "reactedByMe": true }])
        );
        break;
    }
    server.abort();
}
