//! harness 模型目录只读路由：参数校验与响应组装在服务层。

use axum::extract::Path;
use axum::routing::get;
use axum::{Json, Router};

use crate::domain::agent::HarnessKind;
use crate::domain::harness::HarnessModelCatalog;
use crate::error::AppResult;
use crate::services::harness_models::get_harness_model_catalog;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/harnesses/{harness}/models", get(get_models))
}

async fn get_models(Path(harness): Path<String>) -> AppResult<Json<HarnessModelCatalog>> {
    let harness = HarnessKind::parse(&harness)?;
    Ok(Json(get_harness_model_catalog(harness)))
}
