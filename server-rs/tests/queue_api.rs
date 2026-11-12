mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use common::{create_agent, create_room, delete, get, post, setup, setup_with_adapter, TestApp};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::queue::{QueueJobInfo, QueueJobKind};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio_stream::wrappers::ReceiverStream;

/// 首次运行阻塞到测试释放，用于制造「运行中 + 排队」状态。
struct GatedAdapter {
    started: Arc<Semaphore>,
    release: Arc<Semaphore>,
}

impl HarnessAdapter for GatedAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, _input: RunInput) -> EventStream {
        let started = self.started.clone();
        let release = self.release.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            started.add_permits(1);
            let _permit = release.acquire().await.unwrap();
            let _ = sender.send(Ok(RunEvent::Final("完成".to_string()))).await;
        });
        Box::pin(ReceiverStream::new(receiver))
    }
}

fn gated_factory() -> (AdapterFactory, Arc<Semaphore>, Arc<Semaphore>) {
    let started = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let factory_started = started.clone();
    let factory_release = release.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(GatedAdapter {
            started: factory_started.clone(),
            release: factory_release.clone(),
        }))
    });
    (factory, started, release)
}

async fn queue_body(app: &TestApp, room_id: i64) -> Value {
    let response = get(app, &format!("/api/rooms/{room_id}/queue")).await;
    assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
    assert_eq!(response.body["roomId"], room_id);
    response.body
}

async fn wait_for_queue_len(app: &TestApp, room_id: i64, len: usize) -> Value {
    for _ in 0..200 {
        let body = queue_body(app, room_id).await;
        if body["jobs"].as_array().map(Vec::len) == Some(len) {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("队列未达到 {len} 个任务");
}

#[tokio::test]
async fn empty_queue_and_unknown_room() {
    let app = setup().await;
    let room = create_room(&app, "空队列房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let body = queue_body(&app, room_id).await;
    assert!(body["jobs"].as_array().unwrap().is_empty());

    let missing = get(&app, "/api/rooms/9999/queue").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.body["error"]["code"], "NOT_FOUND");

    let missing_cancel = delete(&app, "/api/rooms/9999/queue/1").await;
    assert_eq!(missing_cancel.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancel_unknown_job_404() {
    let app = setup().await;
    let room = create_room(&app, "未知任务房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let response = delete(&app, &format!("/api/rooms/{room_id}/queue/999")).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND, "{:?}", response.body);
    assert_eq!(response.body["error"]["code"], "NOT_FOUND");
    assert!(response.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("队列任务不存在"));
}

#[tokio::test]
async fn cancel_running_job_conflict() {
    let (factory, started, release) = gated_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "运行中").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "运行冲突房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let sent = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@运行中 第一条" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);
    let _started = started.acquire().await.unwrap();

    let body = queue_body(&app, room_id).await;
    let job = &body["jobs"][0];
    assert_eq!(job["state"], "running");
    let response = delete(
        &app,
        &format!("/api/rooms/{room_id}/queue/{}", job["id"]),
    )
    .await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert!(response.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("已开始运行"));

    release.add_permits(1);
    wait_for_queue_len(&app, room_id, 0).await;
}

#[tokio::test]
async fn cancel_queued_chat_skips_run_without_error_message() {
    let (factory, started, release) = gated_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "闸门").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "取消房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@闸门 第一条" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    let _started = started.acquire().await.unwrap();

    let running = queue_body(&app, room_id).await;
    let running_job = &running["jobs"][0];
    assert_eq!(running_job["kind"], "chat");
    assert_eq!(running_job["label"], "闸门");
    assert_eq!(running_job["state"], "running");
    assert_eq!(running_job["position"], 0);
    assert!(running_job["startedAt"].as_i64().is_some());
    assert!(running_job["agentId"].as_i64().is_some());

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@闸门 第二条" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);

    let body = wait_for_queue_len(&app, room_id, 2).await;
    let queued = body["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["state"] == "queued")
        .expect("应有排队任务")
        .clone();
    assert_eq!(queued["position"], 1);
    assert!(queued["startedAt"].is_null());

    let cancelled = delete(
        &app,
        &format!("/api/rooms/{room_id}/queue/{}", queued["id"]),
    )
    .await;
    assert_eq!(
        cancelled.status,
        StatusCode::NO_CONTENT,
        "{:?}",
        cancelled.body
    );
    wait_for_queue_len(&app, room_id, 1).await;

    release.add_permits(1);
    wait_for_queue_len(&app, room_id, 0).await;

    let runs = get(&app, &format!("/api/rooms/{room_id}/runs")).await;
    assert_eq!(
        runs.body.as_array().unwrap().len(),
        1,
        "取消的排队运行不能创建 run 记录"
    );

    let messages = get(&app, &format!("/api/rooms/{room_id}/messages")).await;
    let failures: Vec<&Value> = messages
        .body
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| {
            message["kind"] == "system"
                && message["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("失败")
        })
        .collect();
    assert!(failures.is_empty(), "取消不应写失败系统消息：{failures:?}");
}

#[tokio::test]
async fn non_cancelable_queued_job_conflict_and_queue_order() {
    let app = setup().await;
    let room = create_room(&app, "不可取消房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();
    let queue = app.state.queue.clone();
    let first_started = Arc::new(Semaphore::new(0));
    let first_release = Arc::new(Semaphore::new(0));

    let first = {
        let queue = queue.clone();
        let started = first_started.clone();
        let release = first_release.clone();
        tokio::spawn(async move {
            queue
                .enqueue_managed(
                    room_id,
                    QueueJobInfo {
                        agent_id: None,
                        label: "编排任务".to_string(),
                        kind: QueueJobKind::Task,
                        cancelable: false,
                    },
                    || async move {
                        started.add_permits(1);
                        let _permit = release.acquire().await.unwrap();
                    },
                )
                .await
        })
    };
    let _started = first_started.acquire().await.unwrap();

    let second = {
        let queue = queue.clone();
        tokio::spawn(async move {
            queue
                .enqueue_managed(
                    room_id,
                    QueueJobInfo {
                        agent_id: None,
                        label: "讨论".to_string(),
                        kind: QueueJobKind::Discussion,
                        cancelable: false,
                    },
                    || async {},
                )
                .await
        })
    };
    let body = wait_for_queue_len(&app, room_id, 2).await;
    let jobs = body["jobs"].as_array().unwrap();
    assert_eq!(jobs[0]["label"], "编排任务");
    assert_eq!(jobs[0]["state"], "running");
    assert_eq!(jobs[0]["kind"], "task");
    assert_eq!(jobs[0]["position"], 0);
    assert_eq!(jobs[1]["label"], "讨论");
    assert_eq!(jobs[1]["state"], "queued");
    assert_eq!(jobs[1]["kind"], "discussion");
    assert_eq!(jobs[1]["position"], 1);

    let response = delete(&app, &format!("/api/rooms/{room_id}/queue/{}", jobs[1]["id"])).await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{:?}", response.body);
    assert!(response.body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("不支持队列取消"));

    first_release.add_permits(1);
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    wait_for_queue_len(&app, room_id, 0).await;
}
