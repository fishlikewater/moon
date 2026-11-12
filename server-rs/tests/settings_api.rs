//! 应用设置 API 集成测试。

use axum::http::StatusCode;
use moon_server::db::Db;
use moon_server::{build_app, AppState, AppStateOptions};
use serde_json::{json, Value};

mod common;

use common::*;

#[tokio::test]
async fn settings_defaults_and_runtime_shape() {
    let app = setup().await;
    let response = get(&app, "/api/settings").await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    let settings = &response.body["settings"];
    assert_eq!(settings["defaultRoomPermissionMode"], "auto");
    assert_eq!(settings["defaultRoomDiscussionRounds"], 5);
    assert_eq!(settings["contextCompressionEnabled"], true);
    assert_eq!(settings["contextBudgetChars"], 24_000);
    assert_eq!(settings["attachmentMaxFileBytes"], 26_214_400);
    assert_eq!(settings["attachmentMaxFilesPerMessage"], 10);
    assert_eq!(
        settings["runTimeoutSecs"], 1800,
        "运行时长默认 1800 秒（环境变量未设置时）：{:?}",
        response.body
    );
}

#[tokio::test]
async fn run_timeout_secs_accepts_zero_and_valid_range() {
    let app = setup().await;

    for value in [0_i64, 60, 86_400] {
        let patched = patch(&app, "/api/settings", json!({ "runTimeoutSecs": value })).await;
        assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
        assert_eq!(patched.body["runTimeoutSecs"], value);
        let fetched = get(&app, "/api/settings").await;
        assert_eq!(fetched.body["settings"]["runTimeoutSecs"], value);
    }

    for value in [59_i64, 86_401] {
        let rejected = patch(&app, "/api/settings", json!({ "runTimeoutSecs": value })).await;
        assert_eq!(rejected.status, StatusCode::BAD_REQUEST, "{:?}", rejected.body);
    }
    let not_number = patch(&app, "/api/settings", json!({ "runTimeoutSecs": "abc" })).await;
    assert_eq!(not_number.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn context_budget_upper_limit_is_one_million() {
    let app = setup().await;

    let patched = patch(
        &app,
        "/api/settings",
        json!({ "contextBudgetChars": 1_000_000 }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
    assert_eq!(patched.body["contextBudgetChars"], 1_000_000);
    let fetched = get(&app, "/api/settings").await;
    assert_eq!(fetched.body["settings"]["contextBudgetChars"], 1_000_000);

    let rejected = patch(
        &app,
        "/api/settings",
        json!({ "contextBudgetChars": 1_000_001 }),
    )
    .await;
    assert_eq!(
        rejected.status, StatusCode::BAD_REQUEST,
        "{:?}",
        rejected.body
    );
}

#[tokio::test]
async fn partial_patch_updates_only_given_keys_and_persists_after_reopen() {
    let app = setup().await;
    let patched = patch(
        &app,
        "/api/settings",
        json!({
            "defaultRoomPermissionMode": "plan",
            "contextBudgetChars": 8000,
        }),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{:?}", patched.body);
    assert_eq!(patched.body["defaultRoomPermissionMode"], "plan");
    assert_eq!(patched.body["contextBudgetChars"], 8000);
    assert_eq!(patched.body["defaultRoomDiscussionRounds"], 5);

    let again = get(&app, "/api/settings").await;
    assert_eq!(again.body["settings"]["defaultRoomPermissionMode"], "plan");
    assert_eq!(again.body["settings"]["contextBudgetChars"], 8000);

    // 用同一数据库文件重新装配 AppState，验证设置不是进程内状态。
    let db_path = app.dir.path().join("moon.db");
    let db = Db::open(&db_path).await.unwrap();
    let reopened = AppState::new_with_options(
        db,
        app.dir.path().join("sessions-2"),
        AppStateOptions::default(),
    )
    .await
    .unwrap();
    let router = build_app(reopened);
    let request = axum::http::Request::builder()
        .method(axum::http::Method::GET)
        .uri("/api/settings")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = tower::ServiceExt::oneshot(router, request).await.unwrap();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["settings"]["defaultRoomPermissionMode"], "plan");
    assert_eq!(body["settings"]["contextBudgetChars"], 8000);
}

#[tokio::test]
async fn invalid_patch_is_rejected_atomically() {
    let app = setup().await;
    let before = get(&app, "/api/settings").await;

    for payload in [
        json!({"defaultRoomPermissionMode": "nope"}),
        json!({"defaultRoomDiscussionRounds": 0}),
        json!({"defaultRoomDiscussionRounds": 51}),
        json!({"contextBudgetChars": 1999}),
        json!({"attachmentMaxFileBytes": 1024}),
        json!({"attachmentMaxFilesPerMessage": 21}),
        json!({"contextCompressionEnabled": "yes"}),
        json!({"unknownKey": 1}),
        json!({"defaultRoomDiscussionRounds": 8, "contextBudgetChars": 1}),
    ] {
        let response = patch(&app, "/api/settings", payload.clone()).await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "应拒绝 {payload}: {:?}",
            response.body
        );
    }

    let after = get(&app, "/api/settings").await;
    assert_eq!(
        before.body["settings"], after.body["settings"],
        "非法 PATCH 不能产生部分写入"
    );
}

#[tokio::test]
async fn settings_reject_non_object_body() {
    let app = setup().await;
    let response = patch(&app, "/api/settings", json!([1, 2, 3])).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST, "{:?}", response.body);
}
