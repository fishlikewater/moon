//! harness 本地配置读取错误。
//!
//! 错误消息只包含固定文案，不透传配置文件内容或凭据。

use std::fmt;

#[derive(Debug, Clone)]
pub struct HarnessConfigError(pub String);

impl fmt::Display for HarnessConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HarnessConfigError {}

pub type HarnessConfigResult<T> = Result<T, HarnessConfigError>;
