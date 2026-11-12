use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use moon_server::db::Db;
use moon_server::{build_app, AppState};
use serde_json::Value;
use tower::ServiceExt;

async fn test_app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().await.expect("open db");
    let state = AppState::new(db, dir.path().join("sessions"))
        .await
        .expect("build state");
    (build_app(state), dir)
}

#[tokio::test]
async fn health_returns_ok() {
    let (app, _dir) = test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, serde_json::json!({ "ok": true }));
}

#[tokio::test]
async fn unknown_route_uses_error_contract() {
    let (app, _dir) = test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "NOT_FOUND");
    assert_eq!(body["error"]["message"], "接口不存在：GET /api/nope");
}
