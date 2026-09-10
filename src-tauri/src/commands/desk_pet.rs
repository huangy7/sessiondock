//! 桌宠窗口（仅 macOS 开放）：透明、无边框、置顶的小窗口，展示动画 SVG 桌宠。
//! 设计文档：docs/superpowers/specs/2026-08-04-desk-pet-design.md

use crate::app_db::DeskPetPosition;
use crate::error::AppResult;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use tauri::Emitter;
use tauri::Manager;

pub const DESK_PET_LABEL: &str = "desk-pet";
#[cfg(any(target_os = "macos", test))]
const DESK_PET_WIDTH: f64 = 200.0;
#[cfg(any(target_os = "macos", test))]
const DESK_PET_HEIGHT: f64 = 200.0;

pub fn read_desk_pet_enabled() -> bool {
    crate::app_db::read_desk_pet_enabled().unwrap_or(false)
}

/// 将窗口位置钳制在屏幕可见范围内（逻辑像素）
#[cfg(any(target_os = "macos", test))]
fn clamp_position(x: f64, y: f64, screen_w: f64, screen_h: f64) -> DeskPetPosition {
    let max_x = (screen_w - DESK_PET_WIDTH).max(0.0);
    let max_y = (screen_h - DESK_PET_HEIGHT).max(0.0);
    DeskPetPosition {
        x: x.clamp(0.0, max_x),
        y: y.clamp(0.0, max_y),
    }
}

/// 默认位置：屏幕右下角，留出边距
#[cfg(any(target_os = "macos", test))]
fn default_position(screen_w: f64, screen_h: f64) -> DeskPetPosition {
    DeskPetPosition {
        x: (screen_w - DESK_PET_WIDTH - 40.0).max(0.0),
        y: (screen_h - DESK_PET_HEIGHT - 80.0).max(0.0),
    }
}

#[cfg(target_os = "macos")]
fn primary_screen_size(app: &tauri::AppHandle) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let size = monitor.size();
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

#[cfg(target_os = "macos")]
fn resolve_initial_position(app: &tauri::AppHandle) -> DeskPetPosition {
    let Some((w, h)) = primary_screen_size(app) else {
        return DeskPetPosition { x: 100.0, y: 100.0 };
    };
    let saved = crate::app_db::read_desk_pet_position().ok().flatten();
    let pos = saved.unwrap_or_else(|| default_position(w, h));
    clamp_position(pos.x, pos.y, w, h)
}

/// 创建（或复用并显示）桌宠窗口。仅 macOS 实际创建，其他平台为空操作。
pub fn show_desk_pet_window(app: &tauri::AppHandle) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        if let Some(win) = app.get_webview_window(DESK_PET_LABEL) {
            let _ = win.show();
            return Ok(());
        }
        let pos = resolve_initial_position(app);
        tauri::WebviewWindowBuilder::new(
            app,
            DESK_PET_LABEL,
            tauri::WebviewUrl::App("#/desktop-pet".into()),
        )
        .title("桌宠")
        .inner_size(DESK_PET_WIDTH, DESK_PET_HEIGHT)
        .position(pos.x, pos.y)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .transparent(true)
        .shadow(false)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .build()
        .map_err(|e| crate::error::AppError::business(format!("创建桌宠窗口失败: {}", e)))?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_desk_pet_enabled() -> AppResult<bool> {
    Ok(read_desk_pet_enabled())
}

#[tauri::command]
pub async fn set_desk_pet_enabled(app: tauri::AppHandle, enabled: bool) -> AppResult<()> {
    crate::app_db::write_desk_pet_enabled(enabled)?;
    if enabled {
        show_desk_pet_window(&app)?;
    } else if let Some(win) = app.get_webview_window(DESK_PET_LABEL) {
        let _ = win.close();
    }
    Ok(())
}

/// 拖拽结束后保存当前窗口位置（转为逻辑像素存储）
#[tauri::command]
pub async fn save_desk_pet_position(app: tauri::AppHandle) -> AppResult<()> {
    if let Some(win) = app.get_webview_window(DESK_PET_LABEL) {
        match (win.outer_position(), win.scale_factor()) {
            (Ok(pos), Ok(factor)) => {
                crate::app_db::write_desk_pet_position(DeskPetPosition {
                    x: pos.x as f64 / factor,
                    y: pos.y as f64 / factor,
                })?;
            }
            _ => {
                tracing::warn!("保存桌宠位置失败: 无法获取窗口位置或缩放因子");
            }
        }
    }
    Ok(())
}

static MINI_HOVER_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 启动 mini 模式 hover 探出监听：轮询全局光标，进入/离开边缘热区时 emit 事件
#[tauri::command]
pub async fn start_desk_pet_mini_hover(
    app: tauri::AppHandle,
    edge: String,
    y_top: f64,
    y_bottom: f64,
    zone_width: f64,
) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        use device_query::{DeviceQuery, DeviceState};
        let generation = MINI_HOVER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        let Some((screen_w, _)) = primary_screen_size(&app) else {
            return Ok(());
        };
        let device = DeviceState::new();
        let mut last_near = false;
        tauri::async_runtime::spawn(async move {
            loop {
                if MINI_HOVER_GENERATION.load(Ordering::SeqCst) != generation {
                    break;
                }
                // 窗口存活锚定：mini 激活时窗口被关闭（设置开关等任意路径）→ 退出轮询
                if app.get_webview_window(DESK_PET_LABEL).is_none() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                let (x, y) = device.get_mouse().coords;
                let (x, y) = (x as f64, y as f64);
                let in_edge_zone = if edge == "right" {
                    x >= screen_w - zone_width
                } else {
                    x <= zone_width
                };
                let near = in_edge_zone && y >= y_top && y <= y_bottom;
                if near != last_near {
                    last_near = near;
                    let _ = app.emit("desk-pet-mini-hover", near);
                }
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, edge, y_top, y_bottom, zone_width);
    }
    Ok(())
}

/// 停止 mini 模式 hover 监听（代际递增，旧循环自行退出）
#[tauri::command]
pub async fn stop_desk_pet_mini_hover() -> AppResult<()> {
    let _ = MINI_HOVER_GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

static EYE_TRACK_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 转圈检测：累计光标相对眼睛锚点的有符号角位移，满 2 圈触发头晕（对齐原项目 tick.js）
#[cfg(any(target_os = "macos", test))]
struct SpinDetector {
    accumulated: f64,
    last_angle: Option<f64>,
    last_tick_ms: u64,
    cooldown_until_ms: u64,
}

#[cfg(any(target_os = "macos", test))]
impl SpinDetector {
    const MIN_RADIUS: f64 = 24.0;
    const THRESHOLD: f64 = std::f64::consts::PI * 4.0;
    const IDLE_RESET_MS: u64 = 500;
    const COOLDOWN_MS: u64 = 12_000;

    fn new() -> Self {
        Self {
            accumulated: 0.0,
            last_angle: None,
            last_tick_ms: 0,
            cooldown_until_ms: 0,
        }
    }

    /// 每个轮询 tick 调用；返回 true 表示应触发 dizzy
    fn update(&mut self, rel_x: f64, rel_y: f64, now_ms: u64) -> bool {
        let dist = (rel_x * rel_x + rel_y * rel_y).sqrt();
        if dist < Self::MIN_RADIUS {
            // 距离太近角度不可靠，断链重置
            self.last_angle = None;
            self.accumulated = 0.0;
            self.last_tick_ms = now_ms;
            return false;
        }
        let angle = rel_y.atan2(rel_x);
        let stalled =
            self.last_tick_ms > 0 && now_ms.saturating_sub(self.last_tick_ms) > Self::IDLE_RESET_MS;
        if self.last_angle.is_none() || stalled {
            self.accumulated = 0.0;
        } else {
            let mut delta = angle - self.last_angle.unwrap_or(0.0);
            if delta > std::f64::consts::PI {
                delta -= 2.0 * std::f64::consts::PI;
            }
            if delta < -std::f64::consts::PI {
                delta += 2.0 * std::f64::consts::PI;
            }
            self.accumulated += delta;
            if self.accumulated.abs() >= Self::THRESHOLD && now_ms > self.cooldown_until_ms {
                self.accumulated = 0.0;
                self.last_angle = None;
                self.last_tick_ms = 0;
                self.cooldown_until_ms = now_ms + Self::COOLDOWN_MS;
                return true;
            }
        }
        self.last_angle = Some(angle);
        self.last_tick_ms = now_ms;
        false
    }
}

/// 启动眼睛跟随光标轮询：按 theme 的眼睛锚点比例计算偏移并 emit
#[tauri::command]
pub async fn start_desk_pet_eye_track(
    app: tauri::AppHandle,
    eye_ratio_x: f64,
    eye_ratio_y: f64,
    max_offset: f64,
) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        use device_query::{DeviceQuery, DeviceState};
        let generation = EYE_TRACK_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        let device = DeviceState::new();
        let mut last: (f64, f64) = (f64::NAN, f64::NAN);
        let mut last_cursor: (i32, i32) = (i32::MIN, i32::MIN);
        let x_clamp = max_offset * 0.85;
        let y_clamp = max_offset * 0.5;
        let mut spin = SpinDetector::new();
        let start = std::time::Instant::now();
        tauri::async_runtime::spawn(async move {
            loop {
                if EYE_TRACK_GENERATION.load(Ordering::SeqCst) != generation {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let Some(win) = app.get_webview_window(DESK_PET_LABEL) else {
                    break;
                };
                let (Ok(pos), Ok(factor)) = (win.outer_position(), win.scale_factor()) else {
                    continue;
                };
                let win_x = pos.x as f64 / factor;
                let win_y = pos.y as f64 / factor;
                let (cx, cy) = device.get_mouse().coords;
                // 转圈检测仅在光标实际移动时推进（对齐原项目 moved 门控），
                // 否则静止光标会刷新 last_tick_ms 使 500ms 停顿重置失效
                let cursor_moved = (cx, cy) != last_cursor;
                last_cursor = (cx, cy);
                let (cx, cy) = (cx as f64, cy as f64);
                let eye_x = win_x + DESK_PET_WIDTH * eye_ratio_x;
                let eye_y = win_y + DESK_PET_HEIGHT * eye_ratio_y;
                let (rel_x, rel_y) = (cx - eye_x, cy - eye_y);
                let dist = (rel_x * rel_x + rel_y * rel_y).sqrt();
                let (mut dx, mut dy) = (0.0_f64, 0.0_f64);
                if dist > 1.0 {
                    let scale = (dist / 300.0).min(1.0);
                    dx = rel_x / dist * max_offset * scale;
                    dy = rel_y / dist * max_offset * scale;
                }
                dx = (dx * 2.0).round() / 2.0;
                dy = (dy * 2.0).round() / 2.0;
                dx = dx.clamp(-x_clamp, x_clamp);
                dy = dy.clamp(-y_clamp, y_clamp);
                if (dx, dy) != last {
                    last = (dx, dy);
                    let _ = app.emit("desk-pet-eye-move", [dx, dy]);
                }
                let now_ms = start.elapsed().as_millis() as u64;
                if cursor_moved && spin.update(rel_x, rel_y, now_ms) {
                    let _ = app.emit("desk-pet-dizzy", ());
                }
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, eye_ratio_x, eye_ratio_y, max_offset);
    }
    Ok(())
}

/// 停止眼睛跟随轮询（代际递增，旧循环自行退出）
#[tauri::command]
pub async fn stop_desk_pet_eye_track() -> AppResult<()> {
    let _ = EYE_TRACK_GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_visible_position() {
        let pos = clamp_position(500.0, 400.0, 1512.0, 982.0);
        assert_eq!((pos.x, pos.y), (500.0, 400.0));
    }

    #[test]
    fn clamp_pulls_offscreen_back() {
        let pos = clamp_position(9999.0, -50.0, 1512.0, 982.0);
        assert_eq!((pos.x, pos.y), (1312.0, 0.0));
    }

    #[test]
    fn clamp_handles_tiny_screen() {
        let pos = clamp_position(10.0, 10.0, 100.0, 100.0);
        assert_eq!((pos.x, pos.y), (0.0, 0.0));
    }

    #[test]
    fn default_position_is_bottom_right() {
        let pos = default_position(1512.0, 982.0);
        assert_eq!((pos.x, pos.y), (1272.0, 702.0));
    }

    /// 以半径 100、每 tick 固定角步进模拟绕圈
    fn feed_circle(
        det: &mut SpinDetector,
        step_rad: f64,
        ticks: u32,
        start_ms: u64,
    ) -> Option<u64> {
        let mut angle = 0.0_f64;
        for i in 0..ticks {
            angle += step_rad;
            let now = start_ms + i as u64 * 100;
            if det.update(100.0 * angle.cos(), 100.0 * angle.sin(), now) {
                return Some(now);
            }
        }
        None
    }

    #[test]
    fn spin_two_circles_triggers_dizzy() {
        let mut det = SpinDetector::new();
        // 每 tick 0.3 rad，4π ≈ 12.57 rad → 约 42 tick 触发
        assert!(feed_circle(&mut det, 0.3, 60, 0).is_some());
    }

    #[test]
    fn spin_reversed_direction_cancels() {
        let mut det = SpinDetector::new();
        // 正转 1.5 圈后反转 1.5 圈：有符号累计归零附近，不应触发
        assert!(feed_circle(&mut det, 0.3, 32, 0).is_none()); // ~1.53 圈
        assert!(feed_circle(&mut det, -0.3, 32, 5000).is_none());
        // 再正转 1 圈也不够 2 圈
        assert!(feed_circle(&mut det, 0.3, 21, 10000).is_none());
    }

    #[test]
    fn spin_pause_resets_meter() {
        let mut det = SpinDetector::new();
        assert!(feed_circle(&mut det, 0.3, 32, 0).is_none()); // ~1.53 圈
        // 停顿 600ms > 500ms 重置，重新起算 1.53 圈不够触发
        assert!(feed_circle(&mut det, 0.3, 32, 60000).is_none());
    }

    #[test]
    fn spin_close_cursor_resets_meter() {
        let mut det = SpinDetector::new();
        assert!(feed_circle(&mut det, 0.3, 32, 0).is_none());
        // 光标贴近锚点（半径 10 < 24）断链
        det.update(10.0, 0.0, 5000);
        // 重新累计 1.53 圈不够
        assert!(feed_circle(&mut det, 0.3, 32, 6000).is_none());
    }

    #[test]
    fn dizzy_cooldown_blocks_retrigger() {
        let mut det = SpinDetector::new();
        let first = feed_circle(&mut det, 0.3, 60, 0);
        assert!(first.is_some());
        // 冷却 12s 内再转 2 圈不触发
        let t0 = first.unwrap() + 100;
        assert!(feed_circle(&mut det, 0.3, 60, t0).is_none());
    }
}
