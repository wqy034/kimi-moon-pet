use tauri::{Emitter, Manager};
use std::sync::atomic::{AtomicBool, Ordering};
static PREFERENCES: AtomicBool = AtomicBool::new(false);

pub fn preferences_active() -> bool { PREFERENCES.load(Ordering::SeqCst) }

pub fn cancel_preview(app: &tauri::AppHandle) {
    PREFERENCES.store(false, Ordering::SeqCst);
    super::restore_settings(app);
}

fn position(app: &tauri::AppHandle) -> Result<(), String> {
    use device_query::{DeviceQuery, DeviceState};
    let (x, y) = DeviceState::new().get_mouse().coords;
    position_at(app, x, y)
}

fn position_at(app: &tauri::AppHandle, x: i32, y: i32) -> Result<(), String> {
    let window = app.get_webview_window("pet-menu").ok_or("菜单窗口不存在")?;
    let monitor = window.monitor_from_point(x as f64, y as f64)
        .map_err(|e| e.to_string())?.ok_or("找不到菜单所在屏幕")?;
    let area = monitor.work_area();
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let gap = (8.0 * monitor.scale_factor()) as i32;
    let left = area.position.x + gap;
    let top = area.position.y + gap;
    let right = area.position.x + area.size.width as i32 - size.width as i32 - gap;
    let bottom = area.position.y + area.size.height as i32 - size.height as i32 - gap;
    window.set_position(tauri::PhysicalPosition::new(x.clamp(left, right.max(left)), y.clamp(top, bottom.max(top))))
        .map_err(|e| e.to_string())
}

pub fn show(app: &tauri::AppHandle, preferences: bool) -> Result<(), String> {
    super::cursor_carry::cancel();
    cancel_preview(app);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.emit("pet-interaction", ());
    }
    PREFERENCES.store(preferences, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("pet-menu") {
        position(app)?;
        window.emit("menu-open", preferences).map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    } else {
        tauri::WebviewWindowBuilder::new(app, "pet-menu", tauri::WebviewUrl::App("menu.html".into()))
            .title("Moon·月 · 菜单").inner_size(244.0, 374.0)
            .transparent(true).decorations(false).shadow(false).resizable(false)
            .always_on_top(true).skip_taskbar(true).focused(false).visible(false)
            .build().map_err(|e| e.to_string())?;
        position(app)?;
    }
    let _ = app.emit("pet-menu-visible", true);
    Ok(())
}

#[tauri::command]
pub fn menu_preferences() -> bool { preferences_active() }

#[tauri::command]
pub fn menu_view(app: tauri::AppHandle, preferences: bool) -> Result<(), String> {
    if !preferences { cancel_preview(&app); }
    PREFERENCES.store(preferences, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("pet-menu") {
        let pos = window.outer_position().map_err(|e| e.to_string())?;
        window.set_size(tauri::LogicalSize::new(if preferences { 310.0 } else { 244.0 }, if preferences { 624.0 } else { 374.0 }))
            .map_err(|e| e.to_string())?;
        position_at(&app, pos.x, pos.y)?;
    }
    Ok(())
}

#[tauri::command]
pub fn menu_ready(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("pet-menu") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn menu_action(app: tauri::AppHandle, action: String) -> Result<(), String> {
    cancel_preview(&app);
    if let Some(window) = app.get_webview_window("pet-menu") {
        window.hide().map_err(|e| e.to_string())?;
    }
    let _ = app.emit("pet-menu-visible", false);
    match action.as_str() {
        "dismiss" => {},
        "life" => super::extension_windows::open(&app, "life")?,
        "chat" => super::input_bubble::open(&app,true)?,
        "chat_settings" => super::chat::open_view(&app,"settings")?,
        "chat_history" => super::chat::open_view(&app,"history")?,
        "music" => super::extension_windows::open(&app, "music")?,
        "usage" => {
            if let Some(window) = app.get_webview_window("main") {
                window.emit("show-usage", ()).map_err(|e| e.to_string())?;
            }
        },
        "quit" => app.exit(0),
        _ => return Err("未知菜单操作".into()),
    }
    Ok(())
}
