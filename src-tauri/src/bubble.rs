//! 对话框使用独立透明窗口；标题可拖动，不抢焦点。
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

static CONTENT: Mutex<Option<Value>> = Mutex::new(None);
static DETACHED: AtomicBool = AtomicBool::new(false);
static IS_CHAT: AtomicBool = AtomicBool::new(false);
static AVOID_POINT: Mutex<Option<[i32; 2]>> = Mutex::new(None);
// 主画布320×360中的当前可见包围盒，前端包含实际锚点、缩放及摆动变换。
static ANCHOR: Mutex<[f64; 4]> = Mutex::new([0.0, 28.0, 320.0, 360.0]);

/// 人物在桌面上的物理边界，供扩展窗口打开时定位，后续不跟随。
pub fn visible_rect(app: &tauri::AppHandle) -> Option<[i32; 4]> {
    let main = app.get_webview_window("main")?;
    let p = main.outer_position().ok()?;
    let size = main.outer_size().ok()?;
    let b = *ANCHOR.lock().ok()?;
    let sx = size.width as f64 / 320.;
    let sy = size.height as f64 / 360.;
    Some([
        p.x + (b[0] * sx).floor() as i32,
        p.y + (b[1] * sy).floor() as i32,
        p.x + (b[2] * sx).ceil() as i32,
        p.y + (b[3] * sy).ceil() as i32,
    ])
}
fn matches_owner(owner: &str) -> bool {
    CONTENT.lock().ok().and_then(|v|v.as_ref().map(|v|v["owner"].as_str()==Some(owner))).unwrap_or(false)
}
#[tauri::command]
pub fn bubble_hover(window: tauri::WebviewWindow, hovered: bool, owner: String) -> Result<(), String> {
    if window.label() != "bubble" {
        return Err("无效的对白窗口".into());
    }
    if !matches_owner(&owner) {return Ok(());}
    let _ = window.app_handle().emit_to("main", "bubble-hover", serde_json::json!({"owner":owner,"hovered":hovered}));
    Ok(())
}
#[tauri::command]
pub fn bubble_dismiss(window: tauri::WebviewWindow, owner: String) -> Result<(), String> {
    if window.label() != "bubble" {
        return Err("无效的对白窗口".into());
    }
    if !matches_owner(&owner) {return Ok(());}
    let _ = window.app_handle().emit_to("main", "bubble-dismiss", serde_json::json!({"owner":owner}));
    Ok(())
}

#[tauri::command]
pub fn bubble_anchor(app: tauri::AppHandle, bounds: [f64; 4]) -> Result<(), String> {
    if bounds
        .iter()
        .any(|v| !v.is_finite() || *v < -640.0 || *v > 1000.0)
        || bounds[2] <= bounds[0]
        || bounds[3] <= bounds[1]
    {
        return Err("无效的角色可见边界".into());
    }
    *ANCHOR.lock().map_err(|_| "气泡锚点锁不可用")? = bounds;
    if app
        .get_webview_window("bubble")
        .and_then(|w| w.is_visible().ok())
        == Some(true)
    {
        position(&app);
    }
    Ok(())
}

#[tauri::command]
pub fn drag_bubble(app: tauri::AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("bubble").ok_or("对话框不存在")?;
    // 先启动拖动再置 DETACHED：启动失败（如气泡恰好被隐藏）不能留下卡死的 DETACHED 状态
    super::panel_drag::start(window.clone())?;
    DETACHED.store(true, Ordering::SeqCst);
    let _ = window.emit("bubble-placement", "side");
    Ok(())
}

#[tauri::command]
pub fn bubble_content() -> Option<Value> {
    CONTENT.lock().ok().and_then(|content| content.clone())
}

#[tauri::command]
pub async fn set_bubble(app: tauri::AppHandle, content: Option<Value>) -> Result<(), String> {
    if content.is_none() && super::panel_drag::active("bubble") {
        // 显式停止/清空必须能撤下旧内容，不能留下无法认领的拖动卡片。
        super::panel_drag::cancel("bubble");
    }
    *CONTENT.lock().map_err(|_| "对话框锁不可用")? = content.clone();
    IS_CHAT.store(
        content.as_ref().is_some_and(|v| v["kind"] == "chat"),
        Ordering::SeqCst,
    );
    let avoid = content
        .as_ref()
        .and_then(|value| value["avoid_point"].as_array())
        .filter(|values| values.len() == 2)
        .and_then(|values| Some([values[0].as_f64()?, values[1].as_f64()?]))
        .filter(|point| point.iter().all(|v| v.is_finite() && v.abs() < 100000.0))
        .map(|point| [point[0].round() as i32, point[1].round() as i32]);
    *AVOID_POINT.lock().map_err(|_| "气泡避让锁不可用")? = avoid;
    if content.is_none() {
        DETACHED.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window("bubble") {
            window.hide().map_err(|e| e.to_string())?;
            window
                .emit("bubble-content", content)
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("bubble") {
        window
            .emit("bubble-content", content)
            .map_err(|e| e.to_string())?;
    } else {
        let window = tauri::WebviewWindowBuilder::new(
            &app,
            "bubble",
            tauri::WebviewUrl::App("bubble.html".into()),
        )
        .title("Moon·月 · 对话")
        .inner_size(292.0, 160.0)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
        window
            .set_ignore_cursor_events(false)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn bubble_ready(app: tauri::AppHandle, revision: u64, height: f64) -> Result<(), String> {
    let content = CONTENT.lock().map_err(|_| "对话框锁不可用")?;
    let Some(value) = content.as_ref() else {
        return Ok(());
    };
    if value["revision"].as_u64() != Some(revision) {
        return Ok(());
    }
    // 共享内容锁不得跨原生窗口调用，避免窗口事件重入时互相等待。
    drop(content);
    let Some(window) = app.get_webview_window("bubble") else {
        return Ok(());
    };
    window
        .set_size(tauri::LogicalSize::new(292.0, height.clamp(70.0, 260.0)))
        .map_err(|e| e.to_string())?;
    position(&app);
    if !CONTENT.lock().map_err(|_| "对话框锁不可用")?.as_ref().is_some_and(|v|v["revision"].as_u64()==Some(revision)) {return Ok(());}
    window.show().map_err(|e| e.to_string())
}

pub fn position(app: &tauri::AppHandle) {
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let Some(bubble) = app.get_webview_window("bubble") else {
        return;
    };
    if DETACHED.load(Ordering::SeqCst) {
        let (Ok(p), Ok(s), Ok(Some(m))) = (
            bubble.outer_position(),
            bubble.outer_size(),
            bubble.current_monitor(),
        ) else {
            return;
        };
        let area = m.work_area();
        let x = p.x.clamp(
            area.position.x,
            (area.position.x + area.size.width as i32 - s.width as i32).max(area.position.x),
        );
        let y = p.y.clamp(
            area.position.y,
            (area.position.y + area.size.height as i32 - s.height as i32).max(area.position.y),
        );
        if x != p.x || y != p.y {
            let _ = bubble.set_position(tauri::PhysicalPosition::new(x, y));
        }
        return;
    }
    let (Ok(p), Ok(s), Ok(bs), Ok(Some(monitor))) = (
        main.outer_position(),
        main.outer_size(),
        bubble.outer_size(),
        main.current_monitor(),
    ) else {
        return;
    };
    let area = monitor.work_area();
    let gap = (10.0 * monitor.scale_factor()).round() as i32;
    let left = area.position.x + gap;
    let top = area.position.y + gap;
    let right = area.position.x + area.size.width as i32 - gap;
    let bottom = area.position.y + area.size.height as i32 - gap;
    let bw = bs.width as i32;
    let bh = bs.height as i32;
    let bounds = ANCHOR
        .lock()
        .map(|v| *v)
        .unwrap_or([0.0, 28.0, 320.0, 360.0]);
    let scale_x = s.width as f64 / 320.0;
    let scale_y = s.height as f64 / 360.0;
    let stable = if IS_CHAT.load(Ordering::SeqCst) {
        super::input_bubble::stable_pet_rect(app)
    } else {
        None
    };
    let pet_left = stable.map_or(p.x + (bounds[0] * scale_x).floor() as i32, |r| r[0]);
    let pet_top = stable.map_or(p.y + (bounds[1] * scale_y).floor() as i32, |r| r[1]);
    let pet_right = stable.map_or(p.x + (bounds[2] * scale_x).ceil() as i32, |r| r[2]);
    let pet_bottom = stable.map_or(p.y + (bounds[3] * scale_y).ceil() as i32, |r| r[3]);
    let mut x = pet_left + (pet_right - pet_left - bw) / 2;
    // 气泡窗口底部有10px透明内边距，三角尾尖约占5px；补偿后保持原站姿距离。
    let inset = (15.0 * monitor.scale_factor()).round() as i32;
    let mut y = pet_top - bh - gap - inset;
    let mut placement;
    if y >= top {
        placement = "above";
    } else if pet_right + gap + bw <= right {
        x = pet_right + gap;
        y = pet_top;
        placement = "side";
    } else if pet_left - gap - bw >= left {
        x = pet_left - gap - bw;
        y = pet_top;
        placement = "side";
    } else {
        y = pet_bottom + gap;
        placement = "below";
    }
    x = x.clamp(left, (right - bw).max(left));
    y = y.clamp(top, (bottom - bh).max(top));
    if let Some(input) = super::input_bubble::rect(app) {
        if super::input_bubble::overlaps([x, y, x + bw, y + bh], input) {
            let candidates = [
                (
                    pet_left + (pet_right - pet_left - bw) / 2,
                    pet_top - bh - gap - inset,
                    "above",
                ),
                (pet_left - gap - bw, pet_top, "side"),
                (pet_right + gap, pet_top, "side"),
                (x, pet_bottom + gap, "below"),
            ];
            if let Some((cx, cy, side)) = candidates.into_iter().find(|(cx, cy, _)| {
                *cx >= left
                    && *cy >= top
                    && *cx + bw <= right
                    && *cy + bh <= bottom
                    && !super::input_bubble::overlaps([*cx, *cy, *cx + bw, *cy + bh], input)
            }) {
                x = cx;
                y = cy;
                placement = side;
            }
        }
    }
    // 光标互动的对白不能盖住它正在呼唤的那个点；手动拖开的卡片不重排。
    if let Some(point) = AVOID_POINT.lock().ok().and_then(|v| *v) {
        let covers = |cx: i32, cy: i32| {
            point[0] >= cx - gap
                && point[0] <= cx + bw + gap
                && point[1] >= cy - gap
                && point[1] <= cy + bh + gap
        };
        if covers(x, y) {
            let candidates = [
                (pet_right + gap, pet_top, "side"),
                (pet_left - gap - bw, pet_top, "side"),
                (x, pet_bottom + gap, "below"),
            ];
            if let Some((cx, cy, side)) = candidates.into_iter().find(|(cx, cy, _)| {
                *cx >= left
                    && *cx + bw <= right
                    && *cy >= top
                    && *cy + bh <= bottom
                    && !covers(*cx, *cy)
            }) {
                x = cx;
                y = cy;
                placement = side;
            }
        }
    }
    if bubble.outer_position().ok() != Some(tauri::PhysicalPosition::new(x, y)) {
        let _ = bubble.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = bubble.emit("bubble-placement", placement);
}
