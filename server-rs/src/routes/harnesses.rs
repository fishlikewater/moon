//! harness 模型目录只读路由：参数校验与响应组装在服务层。

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::agent::HarnessKind;
use crate::domain::harness::HarnessModelCatalog;
use crate::error::AppResult;
use crate::services::harness_binaries::{snapshot, HarnessBinariesResponse};
use crate::services::harness_models::get_harness_model_catalog;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/harnesses/{harness}/models", get(get_models))
        .route("/api/harnesses/binaries", get(get_binaries))
}

/// 当前 harness 可执行文件的解析结果与来源，供设置页展示与诊断。
async fn get_binaries(
    State(state): State<AppState>,
) -> AppResult<Json<HarnessBinariesResponse>> {
    let settings = state.settings_service.get().await?.settings;
    let zcode = non_empty(&settings.zcode_bin_path);
    let codex = non_empty(&settings.codex_bin_path);
    Ok(Json(snapshot(zcode.as_deref(), codex.as_deref())))
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

async fn get_models(Path(harness): Path<String>) -> AppResult<Json<HarnessModelCatalog>> {
    let harness = HarnessKind::parse(&harness)?;
    Ok(Json(get_harness_model_catalog(harness)))
}
