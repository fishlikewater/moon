//! 房间附件 API 集成测试：上传、下载、绑定、清理与边界。

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::message::MessageSenderType;
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::error::ErrorCode;
use moon_server::store::messages::AppendMessageInput;
use moon_server::store::MessageStore;
use serde_json::{json, Value};
use tower::ServiceExt;

use common::*;

const BOUNDARY: &str = "moon-attachment-test-boundary";

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

fn recording_factory() -> (AdapterFactory, Arc<Mutex<Vec<RunInput>>>) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let captured = inputs.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(RecordingAdapter {
            events: vec![
                RunEvent::TextDelta("收到".to_string()),
                RunEvent::Final("收到".to_string()),
            ],
            inputs: captured.clone(),
        }))
    });
    (factory, inputs)
}

fn multipart_payload(file_name: &str, mime_type: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {mime_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn upload_file(
    app: &TestApp,
    room_id: i64,
    file_name: &str,
    mime_type: &str,
    bytes: &[u8],
) -> ApiResponse {
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/rooms/{room_id}/attachments"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_payload(file_name, mime_type, bytes)))
        .unwrap();
    send_raw(app, request).await
}

async fn send_raw(app: &TestApp, request: Request<Body>) -> ApiResponse {
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    ApiResponse { status, body }
}

async fn download_raw(app: &TestApp, id: i64) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/attachments/{id}"))
        .body(Body::empty())
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

async fn storage_path(app: &TestApp, id: i64) -> String {
    sqlx::query_scalar::<_, String>("SELECT storage_path FROM message_attachments WHERE id = ?")
        .bind(id)
        .fetch_one(app.state.db.pool())
        .await
        .unwrap()
}

async fn attachment_count(app: &TestApp, room_id: i64) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM message_attachments WHERE room_id = ?")
        .bind(room_id)
        .fetch_one(app.state.db.pool())
        .await
        .unwrap()
}

async fn message_count(app: &TestApp, room_id: i64) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages WHERE room_id = ?")
        .bind(room_id)
        .fetch_one(app.state.db.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn upload_download_and_delete_unbound() {
    let app = setup().await;
    let room = create_room(&app, "附件房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let upload = upload_file(&app, room_id, "note.txt", "text/plain", b"hello moon").await;
    assert_eq!(upload.status, StatusCode::CREATED, "{:?}", upload.body);
    assert_eq!(upload.body["name"], "note.txt");
    assert_eq!(upload.body["mimeType"], "text/plain");
    assert_eq!(upload.body["size"], 10);
    assert!(
        upload.body.get("storagePath").is_none(),
        "磁盘路径不能返回前端：{:?}",
        upload.body
    );
    let id = upload.body["id"].as_i64().unwrap();

    let path = storage_path(&app, id).await;
    let path = Path::new(&path);
    assert!(path.is_file(), "上传后磁盘文件应存在：{}", path.display());
    assert!(
        path.parent().unwrap().ends_with(
            Path::new(".moon")
                .join("attachments")
                .join(format!("room-{room_id}"))
        ),
        "附件应落在房间工作区附件目录：{}",
        path.display()
    );

    let (status, headers, bytes) = download_raw(&app, id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "text/plain");
    assert!(headers
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("inline;"));
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(bytes, b"hello moon");

    let removed = delete(&app, &format!("/api/attachments/{id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(!path.exists(), "删除未绑定附件应同时删除磁盘文件");

    let (status, _, _) = download_raw(&app, id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let again = delete(&app, &format!("/api/attachments/{id}")).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rejects_oversize_empty_and_sanitizes_path_like_names() {
    let app = setup().await;
    let room = create_room(&app, "边界房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let patched = patch(
        &app,
        "/api/settings",
        json!({ "attachmentMaxFileBytes": 1_048_576 }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);

    let oversize = vec![b'x'; 1_048_577];
    let response = upload_file(
        &app,
        room_id,
        "big.bin",
        "application/octet-stream",
        &oversize,
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        response.body
    );
    assert_eq!(response.body["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(attachment_count(&app, room_id).await, 0);

    let empty = upload_file(&app, room_id, "empty.txt", "text/plain", b"").await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{:?}", empty.body);
    assert_eq!(attachment_count(&app, room_id).await, 0);

    let weird = upload_file(&app, room_id, "../evil.txt", "text/plain", b"evil").await;
    assert_eq!(weird.status, StatusCode::CREATED, "{:?}", weird.body);
    assert_eq!(weird.body["name"], "evil.txt", "展示名只保留最后一段");
    let id = weird.body["id"].as_i64().unwrap();
    let path = storage_path(&app, id).await;
    let path = Path::new(&path);
    assert!(!path.components().any(|part| part.as_os_str() == ".."));
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with("evil.txt"));
    assert!(path.starts_with(app.dir.path().canonicalize().unwrap()));
}

#[tokio::test]
async fn upload_requires_existing_room_and_workspace() {
    let app = setup().await;
    let room = create_room(&app, "工作区房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let missing = upload_file(&app, 9999, "note.txt", "text/plain", b"hello").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{:?}", missing.body);

    sqlx::query("UPDATE rooms SET workspace_dir = ? WHERE id = ?")
        .bind(app.dir.path().join("missing-workspace").to_string_lossy())
        .bind(room_id)
        .execute(app.state.db.pool())
        .await
        .unwrap();
    let unavailable = upload_file(&app, room_id, "note.txt", "text/plain", b"hello").await;
    assert_eq!(
        unavailable.status,
        StatusCode::CONFLICT,
        "{:?}",
        unavailable.body
    );
    assert_eq!(unavailable.body["error"]["code"], "CONSTRAINT_VIOLATION");
    assert_eq!(attachment_count(&app, room_id).await, 0);
}

#[tokio::test]
async fn message_binds_attachments_and_prompt_carries_paths() {
    let (factory, inputs) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "附件助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "绑定房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let image = upload_file(&app, room_id, "pic.png", "image/png", b"png-bytes").await;
    let note = upload_file(&app, room_id, "note.txt", "text/plain", b"note-body").await;
    assert_eq!(image.status, StatusCode::CREATED, "{:?}", image.body);
    assert_eq!(note.status, StatusCode::CREATED, "{:?}", note.body);
    let image_id = image.body["id"].as_i64().unwrap();
    let note_id = note.body["id"].as_i64().unwrap();

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({
            "content": "@附件助手 看附件",
            "attachmentIds": [image_id, note_id],
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{:?}", response.body);
    let attachments = response.body["userMessage"]["attachments"]
        .as_array()
        .unwrap();
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0]["id"], image_id);
    assert_eq!(attachments[0]["name"], "pic.png");
    assert_eq!(attachments[0]["mimeType"], "image/png");
    assert_eq!(attachments[1]["name"], "note.txt");
    assert!(attachments[0].get("storagePath").is_none());

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    let listed = messages
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["senderType"] == "user")
        .unwrap();
    assert_eq!(listed["attachments"].as_array().unwrap().len(), 2);

    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs[0]["status"], "succeeded");
    let captured = inputs.lock().unwrap();
    assert_eq!(captured.len(), 1);
    let prompt = &captured[0].prompt;
    let image_path = storage_path(&app, image_id).await;
    let note_path = storage_path(&app, note_id).await;
    assert!(
        prompt.contains(&image_path),
        "提示词应带图片绝对路径：{prompt}"
    );
    assert!(
        prompt.contains(&note_path),
        "提示词应带文件绝对路径：{prompt}"
    );
    assert!(prompt.contains("pic.png"), "提示词应带展示名：{prompt}");
    assert!(prompt.contains("note.txt"), "提示词应带展示名：{prompt}");
}

#[tokio::test]
async fn rejects_cross_room_bound_and_malformed_attachment_ids() {
    let app = setup().await;
    let room_a = create_room(&app, "房间 A", &[]).await;
    let room_b = create_room(&app, "房间 B", &[]).await;
    let room_a_id = room_a["id"].as_i64().unwrap();
    let room_b_id = room_b["id"].as_i64().unwrap();

    let upload = upload_file(&app, room_a_id, "a.txt", "text/plain", b"a").await;
    let attachment_id = upload.body["id"].as_i64().unwrap();
    let other = upload_file(&app, room_a_id, "b.txt", "text/plain", b"b").await;
    let other_id = other.body["id"].as_i64().unwrap();

    let cross_room = post(
        &app,
        &format!("/api/rooms/{room_b_id}/messages"),
        json!({ "content": "跨房间", "attachmentIds": [attachment_id] }),
    )
    .await;
    assert_eq!(
        cross_room.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        cross_room.body
    );
    assert_eq!(cross_room.body["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(attachment_count(&app, room_b_id).await, 0);

    let duplicate = post(
        &app,
        &format!("/api/rooms/{room_a_id}/messages"),
        json!({ "content": "重复", "attachmentIds": [attachment_id, attachment_id] }),
    )
    .await;
    assert_eq!(
        duplicate.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        duplicate.body
    );

    let unknown = post(
        &app,
        &format!("/api/rooms/{room_a_id}/messages"),
        json!({ "content": "未知", "attachmentIds": [9999] }),
    )
    .await;
    assert_eq!(
        unknown.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        unknown.body
    );

    let malformed = post(
        &app,
        &format!("/api/rooms/{room_a_id}/messages"),
        json!({ "content": "坏参数", "attachmentIds": ["x"] }),
    )
    .await;
    assert_eq!(
        malformed.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        malformed.body
    );

    let empty_message = post(
        &app,
        &format!("/api/rooms/{room_a_id}/messages"),
        json!({ "content": "   " }),
    )
    .await;
    assert_eq!(
        empty_message.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        empty_message.body
    );

    let first = post(
        &app,
        &format!("/api/rooms/{room_a_id}/messages"),
        json!({ "attachmentIds": [attachment_id] }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);

    let rebind = post(
        &app,
        &format!("/api/rooms/{room_a_id}/messages"),
        json!({ "content": "再绑定", "attachmentIds": [attachment_id] }),
    )
    .await;
    assert_eq!(rebind.status, StatusCode::BAD_REQUEST, "{:?}", rebind.body);

    let delete_bound = delete(&app, &format!("/api/attachments/{attachment_id}")).await;
    assert_eq!(
        delete_bound.status,
        StatusCode::CONFLICT,
        "{:?}",
        delete_bound.body
    );

    // 未被绑定过的同房间附件仍可删除。
    let removed = delete(&app, &format!("/api/attachments/{other_id}")).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn cleanup_removes_attachment_rows_and_files_only_for_chat_messages() {
    let app = setup().await;
    let agent = create_agent(&app, "清理附件助手").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "清理附件房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let chat_attachment = upload_file(&app, room_id, "chat.txt", "text/plain", b"chat").await;
    let chat_attachment_id = chat_attachment.body["id"].as_i64().unwrap();
    let chat_path = storage_path(&app, chat_attachment_id).await;
    let chat_message = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({
            "content": "纯聊天",
            "attachmentIds": [chat_attachment_id],
        }),
    )
    .await;
    assert_eq!(
        chat_message.status,
        StatusCode::ACCEPTED,
        "{:?}",
        chat_message.body
    );

    let task = post(
        &app,
        &format!("/api/rooms/{room_id}/tasks"),
        json!({ "title": "保留任务" }),
    )
    .await
    .body;
    let task_id = task["id"].as_i64().unwrap();
    let task_attachment = upload_file(&app, room_id, "task.txt", "text/plain", b"task").await;
    let task_attachment_id = task_attachment.body["id"].as_i64().unwrap();
    let task_path = storage_path(&app, task_attachment_id).await;
    let store = MessageStore::new(app.state.db.clone());
    let task_message = store
        .append_with_attachments(
            AppendMessageInput {
                task_id: Some(task_id),
                ..AppendMessageInput::final_message(
                    room_id,
                    MessageSenderType::Agent,
                    Some(agent_id),
                    "任务消息",
                )
            },
            &[task_attachment_id],
        )
        .await
        .unwrap();
    assert_eq!(task_message.attachments.len(), 1);

    let response = post(
        &app,
        &format!("/api/rooms/{room_id}/messages/cleanup"),
        json!({ "keepRecent": 0 }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["deleted"], 1);
    assert_eq!(response.body["attachmentsDeleted"], 1);
    assert!(
        !Path::new(chat_path.as_str()).exists(),
        "纯聊天附件文件应删除"
    );
    assert!(
        Path::new(task_path.as_str()).exists(),
        "任务消息附件文件必须保留"
    );

    let remaining =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM message_attachments WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 1, "只保留任务消息的附件行");
    let remaining_id =
        sqlx::query_scalar::<_, i64>("SELECT id FROM message_attachments WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(app.state.db.pool())
            .await
            .unwrap();
    assert_eq!(remaining_id, task_attachment_id);

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    assert!(
        messages.body.as_array().unwrap().is_empty(),
        "清理后任务消息也应从聊天窗口隐藏"
    );
    let stored = store.get_by_id(task_message.id).await.unwrap().unwrap();
    assert_eq!(
        stored.attachments[0].id, task_attachment_id,
        "任务消息仍应保留其附件"
    );
}

#[tokio::test]
async fn append_with_attachments_rolls_back_when_binding_fails() {
    let app = setup().await;
    let room = create_room(&app, "回滚房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let upload = upload_file(&app, room_id, "a.txt", "text/plain", b"a").await;
    let attachment_id = upload.body["id"].as_i64().unwrap();
    let store = MessageStore::new(app.state.db.clone());

    let first = store
        .append_with_attachments(
            AppendMessageInput::final_message(room_id, MessageSenderType::User, None, "第一条"),
            &[attachment_id],
        )
        .await
        .unwrap();
    assert_eq!(first.attachments.len(), 1);

    // 同一附件不能二次绑定；失败的第二次插入必须整体回滚。
    let error = store
        .append_with_attachments(
            AppendMessageInput::final_message(room_id, MessageSenderType::User, None, "第二条"),
            &[attachment_id],
        )
        .await
        .expect_err("重复绑定必须失败");
    assert_eq!(error.code, ErrorCode::ValidationFailed);
    assert_eq!(message_count(&app, room_id).await, 1, "失败消息不能落库");
    let second: Option<String> =
        sqlx::query_scalar("SELECT content FROM messages WHERE room_id = ? AND content = ?")
            .bind(room_id)
            .bind("第二条")
            .fetch_optional(app.state.db.pool())
            .await
            .unwrap();
    assert!(second.is_none());
}

#[tokio::test]
async fn rejects_too_many_attachments_per_message() {
    let app = setup().await;
    let room = create_room(&app, "数量房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let patched = patch(
        &app,
        "/api/settings",
        json!({ "attachmentMaxFilesPerMessage": 2 }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);

    let mut ids = Vec::new();
    for name in ["a.txt", "b.txt", "c.txt"] {
        let upload = upload_file(&app, room_id, name, "text/plain", name.as_bytes()).await;
        assert_eq!(upload.status, StatusCode::CREATED, "{:?}", upload.body);
        ids.push(upload.body["id"].as_i64().unwrap());
    }

    let too_many = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "三个附件", "attachmentIds": ids }),
    )
    .await;
    assert_eq!(
        too_many.status,
        StatusCode::BAD_REQUEST,
        "{:?}",
        too_many.body
    );
    assert_eq!(too_many.body["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(message_count(&app, room_id).await, 0);

    let accepted = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "两个附件", "attachmentIds": [ids[0], ids[1]] }),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{:?}", accepted.body);
    assert_eq!(
        accepted.body["userMessage"]["attachments"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn upload_reports_unwritable_attachment_directory() {
    let app = setup().await;
    let room = create_room(&app, "只读房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    // 用同名文件占住附件目录路径，模拟目录不可创建/不可写。
    let blocked = app
        .dir
        .path()
        .join(".moon")
        .join("attachments")
        .join(format!("room-{room_id}"));
    std::fs::create_dir_all(blocked.parent().unwrap()).unwrap();
    std::fs::write(&blocked, b"not a directory").unwrap();

    let response = upload_file(&app, room_id, "a.txt", "text/plain", b"a").await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "CONSTRAINT_VIOLATION");
    assert_eq!(attachment_count(&app, room_id).await, 0);
}

#[tokio::test]
async fn dangerous_mime_is_served_as_attachment_and_nosniff() {
    let app = setup().await;
    let room = create_room(&app, "MIME 房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let html = upload_file(
        &app,
        room_id,
        "page.html",
        "text/html",
        b"<script>alert(1)</script>",
    )
    .await;
    assert_eq!(html.status, StatusCode::CREATED, "{:?}", html.body);
    let (status, headers, bytes) = download_raw(&app, html.body["id"].as_i64().unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/octet-stream"
    );
    assert!(headers
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("attachment;"));
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(bytes, b"<script>alert(1)</script>");

    let image = upload_file(&app, room_id, "pic.png", "image/png", b"png").await;
    let (_, headers, _) = download_raw(&app, image.body["id"].as_i64().unwrap()).await;
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/png");
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
}
