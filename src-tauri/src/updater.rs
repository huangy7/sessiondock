use crate::app_db::{self, UpdaterSettings};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

const UPDATE_AVAILABLE_EVENT: &str = "update-available";
const UPDATE_CLEARED_EVENT: &str = "update-cleared";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterState {
    pub current_version: String,
    pub settings: UpdaterSettings,
    pub has_token: bool,
}

/// Pending Tauri updater update, held in app state waiting for install
pub struct PendingTauriUpdate(pub tauri_plugin_updater::Update);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TauriUpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub release_page_url: Option<String>,
}

pub fn current_app_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

fn build_updater_state(app: &AppHandle) -> Result<UpdaterState, String> {
    Ok(UpdaterState {
        current_version: current_app_version(app),
        settings: app_db::read_updater_settings().map_err(|e| e.to_string())?,
        has_token: false,
    })
}

pub fn get_updater_state(app: &AppHandle) -> Result<UpdaterState, String> {
    build_updater_state(app)
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub async fn check_tauri_update(app: &AppHandle) -> Result<Option<TauriUpdateInfo>, String> {
    let updater = app
        .updater_builder()
        .build()
        .map_err(|e| e.to_string())?;

    let update = updater.check().await.map_err(|e| e.to_string())?;

    match update {
        None => {
            write_update_check_result(None)?;
            Ok(None)
        }
        Some(u) => {
            tracing::info!("[update-check] update found: v{}", u.version);
            let info = TauriUpdateInfo {
                version: u.version.clone(),
                notes: u.body.clone(),
                pub_date: u
                    .date
                    .and_then(|d| {
                        d.format(&time::format_description::well_known::Rfc3339)
                            .ok()
                    }),
                release_page_url: None,
            };
            write_update_check_result(Some(&info))?;
            let state = app.state::<Mutex<Option<PendingTauriUpdate>>>();
            *state.lock().map_err(|e| e.to_string())? = Some(PendingTauriUpdate(u));
            Ok(Some(info))
        }
    }
}

fn update_check_summary(update: Option<&TauriUpdateInfo>) -> String {
    match update {
        Some(info) => format!("发现新版本 v{}", info.version),
        None => "当前已是最新版本".to_string(),
    }
}

fn write_update_check_result(update: Option<&TauriUpdateInfo>) -> Result<(), String> {
    let mut settings = app_db::read_updater_settings().map_err(|e| e.to_string())?;
    settings.last_checked_at = Some(now_rfc3339());
    settings.last_check_version = update.map(|info| info.version.clone());
    settings.last_check_summary = Some(update_check_summary(update));
    app_db::write_updater_settings(&settings).map_err(|e| e.to_string())?;
    Ok(())
}

fn publish_update_available(
    app: &AppHandle,
    update_info: &TauriUpdateInfo,
    notify_system: bool,
) -> Result<(), String> {
    let version = update_info.version.clone();

    crate::tray::rebuild_tray_menu_with_update(app, Some(&version))
        .map_err(|e| e.to_string())?;
    tracing::info!("[update-check] tray menu rebuilt with version {}", version);

    app.emit(UPDATE_AVAILABLE_EVENT, update_info)
        .map_err(|e| e.to_string())?;
    tracing::info!("[update-check] emitted update-available event to frontend");

    if notify_system {
        use tauri_plugin_notification::NotificationExt;
        if let Err(err) = app
            .notification()
            .builder()
            .title("SessionDock 有新版本")
            .body(&format!("v{} 已发布，点击打开 SessionDock 查看详情。", version))
            .show()
        {
            tracing::warn!("[update-check] failed to show system notification: {}", err);
        }
    }

    write_update_check_result(Some(update_info))?;
    Ok(())
}

pub fn publish_manual_update_available(
    app: &AppHandle,
    update_info: &TauriUpdateInfo,
) -> Result<(), String> {
    publish_update_available(app, update_info, true)
}

pub fn emit_update_cleared(app: &AppHandle) {
    let _ = app.emit(UPDATE_CLEARED_EVENT, ());
}

pub async fn install_tauri_update(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<Mutex<Option<PendingTauriUpdate>>>();
    let update = state
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or_else(|| "No pending update. Please check for updates first.".to_string())?;

    let app_clone = app.clone();
    match update
        .0
        .download_and_install(
            move |chunk_len, total| {
                let _ = app_clone.emit(
                    "tauri-updater-progress",
                    serde_json::json!({
                        "chunkLen": chunk_len,
                        "total": total
                    }),
                );
            },
            || {},
        )
        .await
    {
        Ok(()) => app.restart(),
        Err(e) => {
            *state.lock().map_err(|e| e.to_string())? = Some(update);
            Err(e.to_string())
        }
    }
}

pub async fn check_and_notify_update(app: &AppHandle) -> Result<(), String> {
    let update_info = match check_tauri_update(app).await? {
        None => return Ok(()),
        Some(info) => info,
    };

    let version = update_info.version.clone();
    tracing::info!("[update-check] check_and_notify_update processing v{}", version);

    let settings = app_db::read_updater_settings().map_err(|e| e.to_string())?;
    
    let is_ignored = settings
        .ignored_update_version
        .as_deref()
        .and_then(|v| semver::Version::parse(v).ok())
        .zip(semver::Version::parse(&version).ok())
        .map(|(ignored, detected)| detected <= ignored)
        .unwrap_or(false);

    if is_ignored {
        tracing::info!("[update-check] version {} is ignored, skipping", version);
        return Ok(());
    }

    publish_update_available(app, &update_info, true)?;

    Ok(())
}
