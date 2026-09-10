use crate::app_db;
use crate::cli::CliKind;
use crate::commands;
use tauri::{
    image::Image,
    menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Runtime,
};

fn tray_cli_label(kind: CliKind) -> &'static str {
    match kind {
        CliKind::Claude => "Claude",
        CliKind::Codex => "Codex",
        CliKind::Gemini => "Gemini",
        CliKind::WorkBuddy => "WorkBuddy",
        CliKind::Dsh => "DSH",
        CliKind::Antigravity => "Antigravity",
    }
}

fn build_cli_profiles_submenu<R: Runtime, M: Manager<R>>(
    manager: &M,
    kind: CliKind,
) -> Result<Submenu<R>, Box<dyn std::error::Error>> {
    let cli_id = kind.id().to_string();
    let profiles = commands::list_profiles(Some(cli_id.clone()), None).unwrap_or_default();
    let active = commands::get_active_profile(Some(cli_id.clone()), None).unwrap_or_default();

    let mut submenu_builder = SubmenuBuilder::with_id(
        manager,
        format!("profiles:{}", cli_id),
        tray_cli_label(kind),
    );

    if profiles.is_empty() {
        let empty_item = MenuItemBuilder::with_id(format!("no_profiles:{}", cli_id), "暂无配置")
            .enabled(false)
            .build(manager)?;
        submenu_builder = submenu_builder.item(&empty_item);
    } else {
        for name in &profiles {
            let prefix = if *name == active { "✓ " } else { "    " };
            let label = format!("{}{}", prefix, name);
            let item = MenuItemBuilder::with_id(format!("profile:{}:{}", cli_id, name), label)
                .build(manager)?;
            submenu_builder = submenu_builder.item(&item);
        }
    }

    Ok(submenu_builder.build()?)
}

fn build_profile_switch_submenu<R: Runtime, M: Manager<R>>(
    manager: &M,
) -> Result<Submenu<R>, Box<dyn std::error::Error>> {
    let claude_submenu = build_cli_profiles_submenu(manager, CliKind::Claude)?;
    let codex_submenu = build_cli_profiles_submenu(manager, CliKind::Codex)?;

    Ok(SubmenuBuilder::with_id(manager, "tray-profiles-root", "修改配置")
        .item(&claude_submenu)
        .item(&codex_submenu)
        .build()?)
}

fn build_tray_menu<R: Runtime, M: Manager<R>>(
    manager: &M,
    update_version: Option<&str>,
) -> Result<Menu<R>, Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", "打开主界面").build(manager)?;
    let profile_switch_submenu = build_profile_switch_submenu(manager)?;
    let quit_item = PredefinedMenuItem::quit(manager, Some("退出"))?;

    let mut builder = MenuBuilder::new(manager).item(&show_item);

    if let Some(version) = update_version {
        let update_item = MenuItemBuilder::with_id(
            "open_update_info",
            format!("有新版本 v{}，点击查看", version),
        )
        .build(manager)?;
        let ignore_item =
            MenuItemBuilder::with_id(format!("ignore_update:{}", version), "忽略此版本")
                .build(manager)?;
        builder = builder
            .separator()
            .item(&update_item)
            .item(&ignore_item);
    }

    Ok(builder
        .separator()
        .item(&profile_switch_submenu)
        .separator()
        .item(&quit_item)
        .build()?)
}

/// Rebuild the tray menu without an update entry.
pub fn rebuild_tray_menu(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    rebuild_tray_menu_with_update(app, None)
}

/// Rebuild the tray menu, optionally showing an update entry for `version`.
///
/// macOS 26 将 NSStatusItem 迁移到 scene 架构后，托盘项必须在主线程上
/// 创建 / 替换 / 销毁，否则会触发 `assertBarrierOnQueue` 硬断言崩溃。
/// 这里把整段逻辑（含 TrayIcon 克隆的 drop）统一派发到主线程执行，
/// 避免 Rc 引用计数跨线程竞争导致的偶发崩溃。
pub fn rebuild_tray_menu_with_update(
    app: &AppHandle,
    update_version: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let version = update_version.map(str::to_owned);
    let app = app.clone();
    let app_for_task = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app_for_task.run_on_main_thread(move || {
        let result: Result<(), String> = (|| {
            let menu = build_tray_menu(&app, version.as_deref())?;
            if let Some(tray) = app.tray_by_id("main") {
                tray.set_menu(Some(menu))?;
            }
            Ok(())
        })()
        .map_err(|e: Box<dyn std::error::Error>| e.to_string());
        // 通过 channel 把结果送回调用线程；错误转成 String 以满足 Send
        let _ = tx.send(result);
    })?;
    rx.recv()
        .map_err(|_| Box::<dyn std::error::Error>::from("tray menu rebuild channel closed"))?
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e).into())
}

/// Create the system tray icon with menu.
pub fn create_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
    let handle = app.handle().clone();
    let menu = build_tray_menu(app, None)?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("SessionDock")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            if id == "show" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            } else if id == "open_update_info" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
                let _ = app.emit("open-settings-about", ());
            } else if let Some(version) = id.strip_prefix("ignore_update:") {
                let version = version.to_string();
                if let Ok(mut settings) = app_db::read_updater_settings() {
                    settings.ignored_update_version = Some(version);
                    let _ = app_db::write_updater_settings(&settings);
                }
                let _ = rebuild_tray_menu_with_update(app, None);
            } else if let Some(profile_id) = id.strip_prefix("profile:") {
                if let Some((cli_id, profile_name)) = profile_id.split_once(':') {
                    let cli_id = cli_id.to_string();
                    let name = profile_name.to_string();
                    let _ = commands::apply_profile(Some(cli_id.clone()), name, None);
                    let _ = rebuild_tray_menu(&handle);
                    let _ = app.emit("profile-changed", serde_json::json!({ "cliId": cli_id }));
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                ..
            } = event
            {
                // 250ms 防抖去重，避免 Down / Up 重复触发
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let last = LAST_TRAY_CLICK_TIME.swap(now, Ordering::Relaxed);
                if now.saturating_sub(last) < 250 {
                    return;
                }

                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

use std::sync::atomic::{AtomicU64, Ordering};

static LAST_TRAY_CLICK_TIME: AtomicU64 = AtomicU64::new(0);
