//! 统一业务错误与错误码。
//!
//! 对外固定输出 `{ "error": { "code", "message" } }`，与现有前端
//! `web/src/api/client.ts` 的解析契约一致；底层异常只保留在内部 `source`，
//! 不原样泄露给调用方（对齐 `.cowork-flow/spec/backend/error-handling.md`）。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// 可预期业务失败的错误码，沿用前端已依赖的历史契约。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// 请求参数不合法（400）
    ValidationFailed,
    /// 资源不存在（404）
    NotFound,
    /// 打破业务约束，如主持人必须是房间成员（409）
    ConstraintViolation,
    /// 任务状态机非法流转（409）
    InvalidTransition,
    /// harness 类型尚未实现（500）
    HarnessUnsupported,
    /// harness 运行失败（500）
    HarnessRunFailed,
    /// 未预期的系统异常兜底（500）
    InternalError,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::NotFound => "NOT_FOUND",
            Self::ConstraintViolation => "CONSTRAINT_VIOLATION",
            Self::InvalidTransition => "INVALID_TRANSITION",
            Self::HarnessUnsupported => "HARNESS_UNSUPPORTED",
            Self::HarnessRunFailed => "HARNESS_RUN_FAILED",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }

    pub const fn status(self) -> StatusCode {
        match self {
            Self::ValidationFailed => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ConstraintViolation | Self::InvalidTransition => StatusCode::CONFLICT,
            Self::HarnessUnsupported | Self::HarnessRunFailed | Self::InternalError => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

/// 统一业务错误载体：`code` 决定 HTTP 状态码，`source` 仅用于内部诊断。
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        code: ErrorCode,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationFailed, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn constraint(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ConstraintViolation, message)
    }

    pub fn invalid_transition(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidTransition, message)
    }

    pub fn harness_unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::HarnessUnsupported, message)
    }

    pub fn harness_run_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::HarnessRunFailed, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::with_source(ErrorCode::InternalError, "数据库操作失败", error)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::with_source(ErrorCode::InternalError, "JSON 序列化失败", error)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.code.status();
        if status.is_server_error() {
            tracing::error!(
                code = self.code.as_str(),
                message = %self.message,
                source = ?self.source,
                "request failed"
            );
        } else {
            tracing::warn!(code = self.code.as_str(), message = %self.message, "request rejected");
        }

        let body = json!({
            "error": {
                "code": self.code.as_str(),
                "message": self.message,
            }
        });
        (status, Json(body)).into_response()
    }
}

/// 业务代码里常用的结果别名。
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_frontend_contract() {
        assert_eq!(ErrorCode::ValidationFailed.as_str(), "VALIDATION_FAILED");
        assert_eq!(ErrorCode::NotFound.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            ErrorCode::ConstraintViolation.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ErrorCode::InternalError.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
