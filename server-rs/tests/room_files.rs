mod common;

use std::path::Path;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use common::{get, post, setup_with_adapter, TestApp};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

async fn raw_get(app: &TestApp, uri: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, bytes)
}

fn write_file(root: &Path, relative: &str, content: &[u8]) -> std::path::PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

async fn create_workspace_agent(app: &TestApp, name: &str, dir: &Path) -> i64 {
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

async fn create_workspace_room(app: &TestApp, name: &str, dirs: &[&Path], member_ids: &[i64]) -> i64 {
    let response = post(
        app,
        "/api/rooms",
        json!({
            "name": name,
            "workspaceDirs": dirs.iter().map(|dir| dir.to_string_lossy()).collect::<Vec<_>>(),
            "memberIds": member_ids,
            "permissionMode": "auto",
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
    response.body["id"].as_i64().unwrap()
}

fn find_root<'a>(body: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    body["roots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|root| root["id"] == id)
        .unwrap_or_else(|| panic!("missing root {id}: {body}"))
}

#[tokio::test]
async fn lists_room_and_member_workspace_files() {
    let app = setup_with_adapter(None).await;
    let room_a = app.dir.path().join("room-a");
    let room_b = app.dir.path().join("room-b");
    let agent_dir = app.dir.path().join("agent-files");
    let outsider_dir = app.dir.path().join("outsider");
    std::fs::create_dir_all(&room_a).unwrap();
    std::fs::create_dir_all(&room_b).unwrap();

    write_file(&room_a, "shared.txt", b"hello");
    write_file(&room_a, "sub/nested.txt", b"nested");
    write_file(&room_a, ".secret", b"hidden");
    write_file(&room_a, "node_modules/skip.txt", b"skip");
    write_file(&room_a, "target/skip.txt", b"skip");
    write_file(&room_b, "other.txt", b"other");
    let agent_id = create_workspace_agent(&app, "文件成员", &agent_dir).await;
    write_file(&agent_dir, "out.txt", b"agent output");
    let outsider_id = create_workspace_agent(&app, "外部智能体", &outsider_dir).await;
    write_file(&outsider_dir, "private.txt", b"private");

    let room_id = create_workspace_room(&app, "文件房间", &[&room_a, &room_b], &[agent_id]).await;

    let response = get(&app, &format!("/api/rooms/{room_id}/files")).await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["roomId"], room_id);
    let roots = response.body["roots"].as_array().unwrap();
    assert_eq!(roots.len(), 3, "房间两个目录 + 成员工作区：{roots:?}");
    assert_eq!(roots[0]["kind"], "room");
    assert_eq!(roots[0]["label"], "房间工作区");
    assert_eq!(roots[1]["kind"], "room");
    assert_eq!(roots[2]["kind"], "agent");
    assert_eq!(roots[2]["agentId"], agent_id);
    assert_eq!(roots[2]["label"], "智能体：文件成员");

    let room_a_root = find_root(&response.body, "room-0");
    let paths: Vec<&str> = room_a_root["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["shared.txt", "sub/nested.txt"]);
    assert_eq!(room_a_root["files"][0]["size"], 5);
    assert_eq!(room_a_root["truncated"], false);

    let room_b_root = find_root(&response.body, "room-1");
    assert_eq!(room_b_root["files"][0]["path"], "other.txt");

    let agent_root = find_root(&response.body, &format!("agent-{agent_id}"));
    assert_eq!(agent_root["files"][0]["path"], "out.txt");
    assert_eq!(agent_root["files"][0]["size"], 12);

    assert!(
        !response
            .body
            .to_string()
            .contains(&format!("agent-{outsider_id}")),
        "非成员智能体工作区不得出现在清单中"
    );
}

#[tokio::test]
async fn downloads_member_file_with_attachment_headers() {
    let app = setup_with_adapter(None).await;
    let room_dir = app.dir.path().join("download-room");
    let agent_dir = app.dir.path().join("download-agent");
    std::fs::create_dir_all(&room_dir).unwrap();
    let agent_id = create_workspace_agent(&app, "下载成员", &agent_dir).await;
    let content = "第一行\n第二行\n".as_bytes();
    write_file(&agent_dir, "报告.txt", content);
    let room_id = create_workspace_room(&app, "下载房间", &[&room_dir], &[agent_id]).await;

    let (status, headers, bytes) = raw_get(
        &app,
        &format!(
            "/api/rooms/{room_id}/files/download?root=agent-{agent_id}&path=%E6%8A%A5%E5%91%8A.txt"
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, content);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/octet-stream"
    );
    assert_eq!(
        headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
        "nosniff"
    );
    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(disposition.starts_with("attachment;"), "{disposition}");
    assert!(disposition.contains("filename="), "{disposition}");
}

#[tokio::test]
async fn rejects_escapes_unknown_roots_and_directories() {
    let app = setup_with_adapter(None).await;
    let room_dir = app.dir.path().join("safe-room");
    std::fs::create_dir_all(&room_dir).unwrap();
    write_file(&room_dir, "sub/inside.txt", b"inside");
    write_file(app.dir.path(), "secret.txt", b"secret");
    let room_id = create_workspace_room(&app, "边界房间", &[&room_dir], &[]).await;

    let cases = [
        // 相对路径穿越
        (
            format!("/api/rooms/{room_id}/files/download?root=room-0&path=..%2Fsecret.txt"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/api/rooms/{room_id}/files/download?root=room-0&path=..%5Csecret.txt"),
            StatusCode::BAD_REQUEST,
        ),
        // 绝对路径
        (
            format!("/api/rooms/{room_id}/files/download?root=room-0&path=%2Fetc%2Fpasswd"),
            StatusCode::BAD_REQUEST,
        ),
        // 空路径
        (
            format!("/api/rooms/{room_id}/files/download?root=room-0&path="),
            StatusCode::BAD_REQUEST,
        ),
        // 未知根
        (
            format!("/api/rooms/{room_id}/files/download?root=agent-999&path=secret.txt"),
            StatusCode::NOT_FOUND,
        ),
        // 目录不是文件
        (
            format!("/api/rooms/{room_id}/files/download?root=room-0&path=sub"),
            StatusCode::NOT_FOUND,
        ),
        // 不存在的文件
        (
            format!("/api/rooms/{room_id}/files/download?root=room-0&path=missing.txt"),
            StatusCode::NOT_FOUND,
        ),
        // 未知房间
        (
            "/api/rooms/9999/files/download?root=room-0&path=secret.txt".to_string(),
            StatusCode::NOT_FOUND,
        ),
    ];

    for (uri, expected) in cases {
        let (status, _, bytes) = raw_get(&app, &uri).await;
        assert_eq!(status, expected, "uri={uri} body={:?}", String::from_utf8_lossy(&bytes));
        assert!(!bytes.windows(6).any(|window| window == b"secret"), "uri={uri}");
    }

    // 反斜杠在 Windows 是分隔符，在 Unix 是普通文件名；两边都不允许读到根外文件。
    let (backslash_status, _, _) = raw_get(
        &app,
        &format!("/api/rooms/{room_id}/files/download?root=room-0&path=..%5Csecret.txt"),
    )
    .await;
    assert!(
        backslash_status == StatusCode::BAD_REQUEST || backslash_status == StatusCode::NOT_FOUND
    );

    let unknown_room = get(&app, "/api/rooms/9999/files").await;
    assert_eq!(unknown_room.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rejects_oversized_download() {
    let app = setup_with_adapter(None).await;
    let room_dir = app.dir.path().join("large-room");
    std::fs::create_dir_all(&room_dir).unwrap();
    let large = room_dir.join("large.bin");
    let file = std::fs::File::create(&large).unwrap();
    file.set_len(25 * 1024 * 1024 + 1).unwrap();
    drop(file);
    let room_id = create_workspace_room(&app, "大文件房间", &[&room_dir], &[]).await;

    let (status, _, _) = raw_get(
        &app,
        &format!("/api/rooms/{room_id}/files/download?root=room-0&path=large.bin"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_depth_limit_is_four() {
    let app = setup_with_adapter(None).await;
    let room_dir = app.dir.path().join("depth-room");
    std::fs::create_dir_all(&room_dir).unwrap();
    write_file(&room_dir, "a/b/c/d/visible.txt", b"depth4");
    write_file(&room_dir, "a/b/c/d/e/hidden.txt", b"depth5");
    let room_id = create_workspace_room(&app, "深度房间", &[&room_dir], &[]).await;

    let response = get(&app, &format!("/api/rooms/{room_id}/files")).await;
    assert_eq!(response.status, StatusCode::OK);
    let root = find_root(&response.body, "room-0");
    let paths: Vec<&str> = root["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"a/b/c/d/visible.txt"), "{paths:?}");
    assert!(!paths.contains(&"a/b/c/d/e/hidden.txt"), "{paths:?}");
}

#[tokio::test]
async fn truncates_list_at_file_limit() {
    let app = setup_with_adapter(None).await;
    let room_dir = app.dir.path().join("many-room");
    std::fs::create_dir_all(&room_dir).unwrap();
    for index in 0..201 {
        write_file(&room_dir, &format!("file-{index:03}.txt"), b"x");
    }
    let room_id = create_workspace_room(&app, "多文件房间", &[&room_dir], &[]).await;

    let response = get(&app, &format!("/api/rooms/{room_id}/files")).await;
    assert_eq!(response.status, StatusCode::OK);
    let root = find_root(&response.body, "room-0");
    assert_eq!(root["files"].as_array().unwrap().len(), 200);
    assert_eq!(root["truncated"], true);
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[tokio::test]
async fn symlink_escape_is_skipped_and_rejected() {
    let app = setup_with_adapter(None).await;
    let room_dir = app.dir.path().join("link-room");
    std::fs::create_dir_all(&room_dir).unwrap();
    write_file(app.dir.path(), "outside-target.txt", b"outside");
    let link = room_dir.join("escape.txt");
    if let Err(error) = create_symlink(&app.dir.path().join("outside-target.txt"), &link) {
        // Windows 未开启开发者模式时无法创建符号链接；记录原因，边界仍由路径校验覆盖。
        eprintln!("跳过符号链接测试：无法创建链接（{error}）");
        return;
    }
    let room_id = create_workspace_room(&app, "链接房间", &[&room_dir], &[]).await;

    let response = get(&app, &format!("/api/rooms/{room_id}/files")).await;
    let root = find_root(&response.body, "room-0");
    let paths: Vec<&str> = root["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert!(!paths.contains(&"escape.txt"), "符号链接不应进入清单");

    let (status, _, bytes) = raw_get(
        &app,
        &format!("/api/rooms/{room_id}/files/download?root=room-0&path=escape.txt"),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
    assert!(!bytes.windows(7).any(|window| window == b"outside"));
}
