//! 队列持久化、暂停与启动恢复的集成测试。

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use common::{
    create_agent, create_room, delete, get, post, put, setup, setup_with_adapter, wait_for_runs,
    workspace, TestApp,
};
use moon_server::adapters::{AdapterFactory, EventStream, HarnessAdapter};
use moon_server::db::Db;
use moon_server::domain::agent::{Agent, HarnessKind};
use moon_server::domain::run_event::{RunEvent, RunInput};
use moon_server::store::queue::{NewQueueJob, QueueStore};
use moon_server::{build_app, AppState, AppStateOptions};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio_stream::wrappers::ReceiverStream;

struct RecordingAdapter {
    inputs: Arc<Mutex<Vec<String>>>,
}

impl HarnessAdapter for RecordingAdapter {
    fn kind(&self) -> HarnessKind {
        HarnessKind::Fake
    }

    fn run(&self, input: RunInput) -> EventStream {
        self.inputs.lock().unwrap().push(input.prompt);
        Box::pin(tokio_stream::iter(vec![Ok(RunEvent::Final(
            "完成".to_string(),
        ))]))
    }
}

fn recording_factory() -> (AdapterFactory, Arc<Mutex<Vec<String>>>) {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let captured = inputs.clone();
    let factory: AdapterFactory = Arc::new(move |_agent: &Agent| {
        Ok(Arc::new(RecordingAdapter {
            inputs: captured.clone(),
        }))
    });
    (factory, inputs)
}

/// 首次运行阻塞到测试释放，用于制造「运行中」与「排队」状态。
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
            let _ = sender
                .send(Ok(RunEvent::Final("完成".to_string())))
                .await;
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

/// 用指定临时目录装配应用；重启测试复用同一目录。
async fn setup_in(dir: tempfile::TempDir, factory: Option<AdapterFactory>) -> TestApp {
    let db = Db::open(dir.path().join("moon.db")).await.unwrap();
    let state = AppState::new_with_options(
        db,
        dir.path().join("sessions"),
        AppStateOptions {
            adapter_factory: factory,
            hub: None,
        },
    )
    .await
    .unwrap();
    let router = build_app(state.clone());
    TestApp { router, state, dir }
}

/// 丢弃进程内状态并用同一数据库重新装配，模拟服务重启。
async fn restart(app: TestApp, factory: Option<AdapterFactory>) -> TestApp {
    let TestApp { dir, .. } = app;
    setup_in(dir, factory).await
}

fn store_of(app: &TestApp) -> QueueStore {
    QueueStore::new(app.state.db.clone())
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

async fn room_runs(app: &TestApp, room_id: i64) -> Vec<Value> {
    let response = get(app, &format!("/api/rooms/{room_id}/runs")).await;
    assert_eq!(response.status, StatusCode::OK);
    response.body.as_array().unwrap().clone()
}

#[tokio::test]
async fn pause_persists_and_holds_queue_until_resume() {
    let (factory, inputs) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "闸门").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "暂停房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let paused = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": true }),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK, "{:?}", paused.body);
    assert_eq!(paused.body["paused"], true);

    let bad = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": "yes" }),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{:?}", bad.body);

    let missing = put(
        &app,
        "/api/rooms/9999/queue/pause",
        json!({ "paused": true }),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    for content in ["@闸门 第一条", "@闸门 第二条"] {
        let sent = post(
            &app,
            &format!("/api/rooms/{room_id}/messages"),
            json!({ "content": content }),
        )
        .await;
        assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);
    }
    let body = wait_for_queue_len(&app, room_id, 2).await;
    assert_eq!(body["paused"], true);
    for job in body["jobs"].as_array().unwrap() {
        assert_eq!(job["state"], "queued", "暂停期间任务不能开跑：{job:?}");
    }
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        room_runs(&app, room_id).await.is_empty(),
        "暂停期间不能创建运行记录"
    );

    let resumed = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": false }),
    )
    .await;
    assert_eq!(resumed.status, StatusCode::OK);
    assert_eq!(resumed.body["paused"], false);

    let runs = wait_for_runs(&app, room_id, 2).await;
    assert_eq!(runs.len(), 2);
    let prompts = inputs.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2, "两次运行都必须真正执行");
    assert!(
        prompts[0].contains("第一条") && prompts[1].contains("第二条"),
        "恢复后必须按 FIFO 执行：{prompts:?}"
    );
    wait_for_queue_len(&app, room_id, 0).await;
}

#[tokio::test]
async fn paused_state_survives_restart() {
    let app = setup().await;
    let room = create_room(&app, "暂停持久房", &[]).await;
    let room_id = room["id"].as_i64().unwrap();

    let paused = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": true }),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK, "{:?}", paused.body);

    let app = restart(app, None).await;
    let body = queue_body(&app, room_id).await;
    assert_eq!(body["paused"], true, "重启后暂停状态必须保留");
    assert!(body["jobs"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn queued_rows_restore_in_order_after_restart() {
    let app = setup().await;
    let first = create_agent(&app, "甲").await;
    let second = create_agent(&app, "乙").await;
    let first_id = first["id"].as_i64().unwrap();
    let second_id = second["id"].as_i64().unwrap();
    let room = create_room(&app, "恢复房", &[first_id, second_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let store = store_of(&app);
    store
        .insert(NewQueueJob {
            room_id,
            agent_id: first_id,
            label: "甲".to_string(),
            kind: "chat".to_string(),
            instruction: "@甲 第一条".to_string(),
            moderator_trigger_message_id: None,
            enqueued_at: 1,
        })
        .await
        .unwrap();
    store
        .insert(NewQueueJob {
            room_id,
            agent_id: second_id,
            label: "乙".to_string(),
            kind: "chat".to_string(),
            instruction: "@乙 第二条".to_string(),
            moderator_trigger_message_id: None,
            enqueued_at: 2,
        })
        .await
        .unwrap();

    let (factory, inputs) = recording_factory();
    let app = restart(app, Some(factory)).await;
    let runs = wait_for_runs(&app, room_id, 2).await;
    assert_eq!(runs.len(), 2, "两条待执行行都应恢复为运行");
    assert!(
        runs.iter().all(|run| run["status"] == "succeeded"),
        "恢复后的运行必须成功而不是只创建记录：{runs:?}"
    );
    let prompts = inputs.lock().unwrap().clone();
    assert!(
        prompts[0].contains("第一条") && prompts[1].contains("第二条"),
        "恢复必须按行 id 顺序执行：{prompts:?}"
    );
    assert!(
        store_of(&app).list_pending().await.unwrap().is_empty(),
        "恢复后的行必须在开跑时删除"
    );
    wait_for_queue_len(&app, room_id, 0).await;
}

#[tokio::test]
async fn started_job_row_is_removed_before_run() {
    let (factory, started, release) = gated_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "运行中").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "运行房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let sent = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@运行中 开工" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);
    let _started = started.acquire().await.unwrap();

    assert!(
        store_of(&app).list_pending().await.unwrap().is_empty(),
        "已开跑任务的持久化行必须删除，重启才不会重复执行"
    );

    release.add_permits(1);
    wait_for_runs(&app, room_id, 1).await;
}

#[tokio::test]
async fn canceled_queued_job_row_is_removed() {
    let app = setup().await;
    let agent = create_agent(&app, "取消").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "取消持久房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let paused = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": true }),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK);

    let sent = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@取消 待取消" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);
    let body = wait_for_queue_len(&app, room_id, 1).await;
    let job_id = body["jobs"][0]["id"].as_i64().unwrap();

    let canceled = delete(&app, &format!("/api/rooms/{room_id}/queue/{job_id}")).await;
    assert_eq!(
        canceled.status,
        StatusCode::NO_CONTENT,
        "{:?}",
        canceled.body
    );
    assert!(
        store_of(&app).list_pending().await.unwrap().is_empty(),
        "取消排队任务必须删除持久化行"
    );

    let resumed = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": false }),
    )
    .await;
    assert_eq!(resumed.status, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        room_runs(&app, room_id).await.is_empty(),
        "已取消任务不能因恢复而开跑"
    );
}

#[tokio::test]
async fn rows_for_removed_members_are_cleaned_without_running() {
    let app = setup().await;
    let agent = create_agent(&app, "离场").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "离场房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let store = store_of(&app);
    store
        .insert(NewQueueJob {
            room_id,
            agent_id,
            label: "离场".to_string(),
            kind: "chat".to_string(),
            instruction: "@离场 不该执行".to_string(),
            moderator_trigger_message_id: None,
            enqueued_at: 1,
        })
        .await
        .unwrap();

    let removed = delete(
        &app,
        &format!("/api/rooms/{room_id}/members/{agent_id}"),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK, "{:?}", removed.body);

    let (factory, inputs) = recording_factory();
    let app = restart(app, Some(factory)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        room_runs(&app, room_id).await.is_empty(),
        "成员已移除的任务不能恢复执行"
    );
    assert!(inputs.lock().unwrap().is_empty());
    assert!(
        store_of(&app).list_pending().await.unwrap().is_empty(),
        "无法恢复的行必须清理"
    );
}

#[tokio::test]
async fn paused_room_restores_multiple_rows_in_order() {
    let app = setup().await;
    let first = create_agent(&app, "甲").await;
    let second = create_agent(&app, "乙").await;
    let first_id = first["id"].as_i64().unwrap();
    let second_id = second["id"].as_i64().unwrap();
    let room = create_room(&app, "暂停恢复房", &[first_id, second_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let paused = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": true }),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK, "{:?}", paused.body);

    let store = store_of(&app);
    for (agent_id, label, instruction) in [
        (first_id, "甲", "@甲 第一条"),
        (second_id, "乙", "@乙 第二条"),
    ] {
        store
            .insert(NewQueueJob {
                room_id,
                agent_id,
                label: label.to_string(),
                kind: "chat".to_string(),
                instruction: instruction.to_string(),
                moderator_trigger_message_id: None,
                enqueued_at: 1,
            })
            .await
            .unwrap();
    }

    let (factory, inputs) = recording_factory();
    let app = restart(app, Some(factory)).await;
    let body = wait_for_queue_len(&app, room_id, 2).await;
    assert_eq!(body["paused"], true);
    assert!(
        body["jobs"].as_array().unwrap().iter().all(|job| job["state"] == "queued"),
        "暂停房间的两条恢复任务都应保持排队：{body:?}"
    );
    assert!(
        room_runs(&app, room_id).await.is_empty(),
        "暂停期间恢复任务不能开跑"
    );

    let resumed = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": false }),
    )
    .await;
    assert_eq!(resumed.status, StatusCode::OK);

    let runs = wait_for_runs(&app, room_id, 2).await;
    assert!(
        runs.iter().all(|run| run["status"] == "succeeded"),
        "暂停恢复后的运行必须成功：{runs:?}"
    );
    let prompts = inputs.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[0].contains("第一条") && prompts[1].contains("第二条"),
        "暂停房间的多行恢复必须保持行顺序：{prompts:?}"
    );
    wait_for_queue_len(&app, room_id, 0).await;
}

#[tokio::test]
async fn moderator_job_persists_and_restores_with_trigger_options() {
    let (factory, _) = recording_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "成员").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = post(
        &app,
        "/api/rooms",
        json!({
            "name": "主持人恢复房",
            "workspaceDir": workspace(&app),
            "memberIds": [agent_id],
            "replyMode": "moderator",
            "permissionMode": "auto",
        }),
    )
    .await;
    assert_eq!(room.status, StatusCode::CREATED, "{:?}", room.body);
    let room_id = room.body["id"].as_i64().unwrap();

    let paused = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": true }),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK, "{:?}", paused.body);
    let sent = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "请安排下一步" }),
    )
    .await;
    assert_eq!(sent.status, StatusCode::ACCEPTED, "{:?}", sent.body);

    let body = wait_for_queue_len(&app, room_id, 1).await;
    assert_eq!(body["jobs"][0]["kind"], "moderator");
    assert_eq!(body["jobs"][0]["state"], "queued");

    let (factory, _) = recording_factory();
    let app = restart(app, Some(factory)).await;
    let restored = wait_for_queue_len(&app, room_id, 1).await;
    assert_eq!(restored["paused"], true);
    assert_eq!(restored["jobs"][0]["kind"], "moderator");
    assert_eq!(restored["jobs"][0]["state"], "queued");

    let resumed = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": false }),
    )
    .await;
    assert_eq!(resumed.status, StatusCode::OK);
    let runs = wait_for_runs(&app, room_id, 1).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0]["status"], "succeeded",
        "主持人恢复运行应成功：{:?}",
        runs[0]
    );
    assert!(
        runs[0]["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("请安排下一步")),
        "主持人恢复运行必须携带原指令：{:?}",
        runs[0]["prompt"]
    );
    wait_for_queue_len(&app, room_id, 0).await;
}

#[tokio::test]
async fn pause_does_not_interrupt_running_job() {
    let (factory, started, release) = gated_factory();
    let app = setup_with_adapter(Some(factory)).await;
    let agent = create_agent(&app, "先跑").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "运行中暂停房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    let first = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@先跑 正在运行" }),
    )
    .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{:?}", first.body);
    let _started = started.acquire().await.unwrap();

    let paused = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": true }),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK, "{:?}", paused.body);
    assert!(
        paused.body["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|job| job["state"] == "running"),
        "暂停不能把运行中的任务改回排队：{:?}",
        paused.body
    );

    let second = post(
        &app,
        &format!("/api/rooms/{room_id}/messages"),
        json!({ "content": "@先跑 暂停后入队" }),
    )
    .await;
    assert_eq!(second.status, StatusCode::ACCEPTED, "{:?}", second.body);

    // 两个 permit：运行中的任务继续完成；暂停解除后第二条才拿到 permit。
    release.add_permits(2);
    let body = loop {
        let body = queue_body(&app, room_id).await;
        if body["jobs"].as_array().map(Vec::len) == Some(1)
            && body["jobs"][0]["state"] == "queued"
        {
            break body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(body["paused"], true);
    assert_eq!(
        room_runs(&app, room_id).await.len(),
        1,
        "暂停期间第二条任务不能开跑"
    );

    let resumed = put(
        &app,
        &format!("/api/rooms/{room_id}/queue/pause"),
        json!({ "paused": false }),
    )
    .await;
    assert_eq!(resumed.status, StatusCode::OK);
    let runs = wait_for_runs(&app, room_id, 2).await;
    assert_eq!(runs.len(), 2);
    wait_for_queue_len(&app, room_id, 0).await;
}

#[tokio::test]
async fn running_run_is_interrupted_and_not_replayed_on_restart() {
    let app = setup().await;
    let agent = create_agent(&app, "中断").await;
    let agent_id = agent["id"].as_i64().unwrap();
    let room = create_room(&app, "中断房", &[agent_id]).await;
    let room_id = room["id"].as_i64().unwrap();

    sqlx::query(
        "INSERT INTO agent_runs (room_id, agent_id, status, prompt, created_at)
         VALUES (?, ?, 'running', '中断前的指令', 1)",
    )
    .bind(room_id)
    .bind(agent_id)
    .execute(app.state.db.pool())
    .await
    .unwrap();

    let app = restart(app, None).await;
    let runs = room_runs(&app, room_id).await;
    assert_eq!(runs.len(), 1, "重启不能为运行中的任务新建运行");
    assert_eq!(runs[0]["status"], "interrupted");
    assert!(runs[0]["finishedAt"].as_i64().is_some());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        room_runs(&app, room_id).await.len(),
        1,
        "运行中任务不能在重启后重放"
    );
    assert!(
        store_of(&app).list_pending().await.unwrap().is_empty(),
        "待执行表不能残留运行中任务的行"
    );
}
