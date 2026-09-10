use serde::Serialize;

/// Unified application error type.
///
/// Replaces the scattered `Result<T, String>` + `.map_err(|e| format!(...))` pattern.
/// All variants automatically capture the underlying error via `#[from]`.
/// The `Serialize` impl ensures Tauri IPC can return these to the frontend as strings.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("数据库连接池错误: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 序列化失败: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Tantivy 索引错误: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("{0}")]
    Business(String),
}

impl AppError {
    /// Create a business-logic error from a string message.
    /// Use this for validation errors like "模型名称不能为空".
    pub fn business(msg: impl Into<String>) -> Self {
        Self::Business(msg.into())
    }
}

/// Serialize as a plain string for Tauri IPC.
/// The frontend receives `{ error: "..." }` which matches the current `Result<T, String>` behavior.
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Convenience alias used across the crate.
pub type AppResult<T> = Result<T, AppError>;

impl From<String> for AppError {
    fn from(msg: String) -> Self {
        AppError::Business(msg)
    }
}

impl From<&str> for AppError {
    fn from(msg: &str) -> Self {
        AppError::Business(msg.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::Business(err.to_string())
    }
}
