use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};

pub mod desk_pet;
pub mod model_list;
pub mod profile;
pub mod proxy;
pub mod pty;
pub mod session;
pub mod session_index;
pub mod settings;
pub mod system_ops;
pub mod updater;
pub mod workspace_fs;

pub use model_list::*;
pub use profile::*;
pub use proxy::*;
pub use session::*;
pub use session_index::*;
pub use settings::*;
pub use system_ops::*;
pub use updater::*;

/// App data directory: ~/Library/Application Support/com.sessiondock.app/
pub(crate) fn app_data_dir() -> AppResult<PathBuf> {
    let data = dirs::data_dir().ok_or("无法获取应用数据目录")?;
    Ok(data.join("com.sessiondock.app"))
}

/// data/ — legacy application runtime data directory, still used during migration.
pub(crate) fn data_dir() -> AppResult<PathBuf> {
    Ok(app_data_dir()?.join("data"))
}

/// Directory for saved reports: reports/

pub(crate) fn open_path(path: &Path) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::business(e.to_string()))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::business(e.to_string()))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| AppError::business(e.to_string()))?;
    }
    Ok(())
}

/// Format an ISO 8601 timestamp to local time "YYYY-MM-DD HH:MM".
pub(crate) fn format_timestamp(ts: &str) -> String {
    use chrono::{DateTime, Local, Utc};
    match ts.parse::<DateTime<Utc>>() {
        Ok(utc) => {
            let local: DateTime<Local> = utc.with_timezone(&Local);
            local.format("%Y-%m-%d %H:%M").to_string()
        }
        Err(_) => ts.to_string(),
    }
}
