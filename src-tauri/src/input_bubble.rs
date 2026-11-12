//! 最小输入组件：运行时开关、固定尺寸和焦点；不保存未发送文字。
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

// 串行执行原生变更，但不占用UI线程等锁；窗口调用前必须释放Service锁。
static OPERATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
struct Service {
    revision: u64,
    session: u64,
    mode: String,
    focus_serial: u64,
    focus_pending: bool,
    editing: bool,
}
pub fn own(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "input-bubble" {
        Ok(())
    } else {
        Err("只接受输入气泡操作".into())
    }
}
fn snapshot(app: &tauri::AppHandle) -> Result<Value, String> {
    let state = app.state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "输入组件不可用")?;
    Ok(
        json!({"revision":s.revision,"session":s.session,"mode":s.mode,"focus_serial":s.focus_serial}),
    )
}
fn publish(app: &tauri::AppHandle) {
    if let Ok(value) = snapshot(app) {
        let _ = app.emit_to("input-bubble", "input-state", value);
    }
    let state = app.state::<Mutex<Service>>();
    let presence = state
        .lock()
        .ok()
        .map(|s| json!({"revision":s.revision,"editing":s.editing&&s.mode=="normal"}));
    if let Some(value) = presence {
        let _ = app.emit_to("main", "input-presence", value);
    }
}
pub fn install(app: &tauri::AppHandle) {
    app.manage(Mutex::new(Service {
        revision: 1,
        session: 1,
        mode: "hidden".into(),
        focus_serial: 0,
        focus_pending: false,
        editing: false,
    }));
}
fn trace(app: &tauri::AppHandle, action: &str, stage: &str) {
    let mode = snapshot(app)
        .ok()
        .and_then(|v| v["mode"].as_str().map(str::to_owned))
        .unwrap_or_default();
    crate::append_log(
        "debug.log",
        &format!("input-native action={action} stage={stage} mode={mode}"),
    );
    if !["done", "failed"].contains(&stage) {
        return;
    }
    let actual = app
        .get_webview_window("input-bubble")
        .map(|w| {
            let visible = w.is_visible().unwrap_or(false);
            let size = w.inner_size().ok();
            let outer = w.outer_size().ok();
            let decorated = w.is_decorated().ok();
            format!(
                "visible={visible} size={}x{} outer={}x{} decorated={decorated:?}",
                size.map_or(0, |z| z.width),
                size.map_or(0, |z| z.height),
                outer.map_or(0, |z| z.width),
                outer.map_or(0, |z| z.height)
            )
        })
        .unwrap_or_else(|| "absent".into());
    crate::append_log(
        "debug.log",
        &format!("input-native action={action} stage=observed {actual}"),
    );
}
// 公共入口可能来自同步菜单回调；始终放到异步运行时，避免原生窗口调用堵住UI事件循环。
pub fn open(app: &tauri::AppHandle, focus: bool) -> Result<(), String> {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = OPERATIONS.lock().await;
        trace(&app, "open", "begin");
        if open_now(&app, focus).is_err() {
            trace(&app, "open", "failed");
        } else {
            trace(&app, "open", "done");
        }
    });
    Ok(())
}
fn open_now(app: &tauri::AppHandle, focus: bool) -> Result<(), String> {
    let was_hidden = {
        let state = app.state::<Mutex<Service>>();
        let mut s = state.lock().map_err(|_| "输入组件不可用")?;
        let hidden = s.mode == "hidden";
        if hidden {
            s.session += 1;
        }
        s.mode = "normal".into();
        s.revision += 1;
        s.focus_pending = focus;
        hidden
    };
    crate::extension_windows::open_component(app, "input")?;
    layout(app, was_hidden)?;
    publish(app);
    Ok(())
}
pub fn hide(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = OPERATIONS.lock().await;
        trace(&app, "hide", "begin");
        if hide_now(&app).is_err() {
            trace(&app, "hide", "failed");
        } else {
            trace(&app, "hide", "done");
        }
    });
}
fn hide_now(app: &tauri::AppHandle) -> Result<(), String> {
    {
        let state = app.state::<Mutex<Service>>();
        let mut s = state.lock().map_err(|_| "输入组件不可用")?;
        s.mode = "hidden".into();
        s.session += 1;
        s.revision += 1;
        s.editing = false;
        s.focus_pending = false;
    }
    crate::panel_drag::cancel("input-bubble");
    let result = app.get_webview_window("input-bubble").map_or(Ok(()), |w| {
        w.hide().map_err(|e| e.to_string())?;
        if w.is_visible().map_err(|e| e.to_string())? {
            return Err("输入窗口未成功隐藏，请保留日志".into());
        }
        Ok(())
    });
    publish(app);
    result
}
pub fn reveal(app: &tauri::AppHandle) -> Result<(), String> {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = OPERATIONS.lock().await;
        if layout(&app, false).is_err() {
            trace(&app, "ready", "failed");
        } else {
            trace(&app, "ready", "done");
        }
        publish(&app);
    });
    Ok(())
}
fn layout(app: &tauri::AppHandle, reset: bool) -> Result<(), String> {
    if !crate::extension_windows::component_ready("input") {
        return Ok(());
    }
    let Some(w) = app.get_webview_window("input-bubble") else {
        return Ok(());
    };
    let mode = snapshot(app)?["mode"]
        .as_str()
        .unwrap_or("hidden")
        .to_string();
    if mode == "hidden" {
        return Ok(());
    }
    let current = rect(app);
    let b = crate::bubble::visible_rect(app).ok_or("桌宠位置不可用")?;
    let place = reset || current.is_none();
    let target = if place {
        [(b[0] + b[2]) / 2, b[1]]
    } else {
        let r = current.unwrap();
        [r[0], r[1]]
    };
    let monitor = w
        .monitor_from_point(target[0] as f64, target[1] as f64)
        .map_err(|e| e.to_string())?
        .or(w.current_monitor().map_err(|e| e.to_string())?)
        .ok_or("显示器不可用")?;
    let sf = monitor.scale_factor();
    let area = monitor.work_area();
    let gap = (12. * sf).round() as i32;
    let width = (if mode == "minimized" { 42_f64 } else { 320. })
        .min((area.size.width as f64 / sf - 24.).max(42.));
    let bw = (width * sf).round() as i32;
    let bh = (42. * sf).round() as i32;
    let left = area.position.x + gap;
    let top = area.position.y + gap;
    let right = area.position.x + area.size.width as i32 - gap;
    let bottom = area.position.y + area.size.height as i32 - gap;
    let fits =
        |p: [i32; 2]| p[0] >= left && p[1] >= top && p[0] + bw <= right && p[1] + bh <= bottom;
    let initial = if place {
        // 原生鼠标命中包含人物整块透明画布，默认位置须避开整窗，而非只避开立绘alpha。
        let pet = app.get_webview_window("main").ok_or("桌宠窗口不可用")?;
        let p = pet.outer_position().map_err(|e| e.to_string())?;
        let z = pet.outer_size().map_err(|e| e.to_string())?;
        let centered = (target[0] - bw / 2).clamp(left, (right - bw).max(left));
        let candidates = [
            [centered, p.y - bh - gap],
            [p.x - bw - gap, p.y.clamp(top, (bottom - bh).max(top))],
            [
                p.x + z.width as i32 + gap,
                p.y.clamp(top, (bottom - bh).max(top)),
            ],
            [centered, p.y + z.height as i32 + gap],
        ];
        candidates
            .into_iter()
            .find(|p| fits(*p))
            .unwrap_or([centered, p.y - bh - gap])
    } else {
        target
    };
    let x = initial[0].clamp(left, (right - bw).max(left));
    let y = initial[1].clamp(top, (bottom - bh).max(top));
    // 以下所有原生调用均在没有Service锁的情况下执行。
    // 收成圆球时先隐藏，让系统释放编辑焦点，再以不激活方式显示。
    if mode == "minimized" {
        w.hide().map_err(|e| e.to_string())?;
    }
    w.set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    w.set_size(tauri::PhysicalSize::new(bw as u32, bh as u32))
        .map_err(|e| e.to_string())?;
    w.set_focusable(mode != "minimized")
        .map_err(|e| e.to_string())?;
    // 必须通过框架显示。直接ShowWindow会让Tao仍认为窗口隐藏，
    // 跳过首次无框布局，且后续hide成为空操作（本次304×3内容区/关不掉的根因）。
    w.show().map_err(|e| e.to_string())?;
    let expected = tauri::PhysicalSize::new(bw as u32, bh as u32);
    let mut actual = w.inner_size().map_err(|e| e.to_string())?;
    if actual != expected {
        // 首次显示完成非客户区布局后，最多补一次内容区尺寸，不做周期性修正。
        w.set_size(expected).map_err(|e| e.to_string())?;
        actual = w.inner_size().map_err(|e| e.to_string())?;
    }
    if actual != expected || w.is_decorated().map_err(|e| e.to_string())? {
        return Err("输入窗口内容区或无框状态异常，请保留日志".into());
    }
    let focus = {
        let state = app.state::<Mutex<Service>>();
        let mut s = state.lock().map_err(|_| "输入组件不可用")?;
        let focus = s.focus_pending && s.mode == "normal";
        s.focus_pending = false;
        if focus {
            s.focus_serial += 1;
            s.revision += 1;
        }
        focus
    };
    if focus {
        w.set_focus().map_err(|e| e.to_string())?;
    }
    crate::bubble::position(app);
    Ok(())
}
pub fn stable_pet_rect(app: &tauri::AppHandle) -> Option<[i32; 4]> {
    let pet = app.get_webview_window("main")?;
    let p = pet.outer_position().ok()?;
    let z = pet.outer_size().ok()?;
    let sx = z.width as f64 / 320.;
    let sy = z.height as f64 / 360.;
    Some([
        p.x + (46. * sx).round() as i32,
        p.y + (60. * sy).round() as i32,
        p.x + (274. * sx).round() as i32,
        p.y + (336. * sy).round() as i32,
    ])
}
pub fn rect(app: &tauri::AppHandle) -> Option<[i32; 4]> {
    let w = app.get_webview_window("input-bubble")?;
    if !w.is_visible().ok()? {
        return None;
    }
    let p = w.outer_position().ok()?;
    let z = w.outer_size().ok()?;
    Some([p.x, p.y, p.x + z.width as i32, p.y + z.height as i32])
}
pub fn overlaps(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}
pub fn end_drag(app: &tauri::AppHandle) {
    trace(app, "drag", "done");
    crate::bubble::position(app);
}
pub fn focus_lost(app: &tauri::AppHandle) {
    set_editing(app, false);
}
fn set_editing(app: &tauri::AppHandle, editing: bool) {
    let changed = {
        let state = app.state::<Mutex<Service>>();
        let Ok(mut s) = state.lock() else { return };
        let editing = editing && s.mode == "normal";
        if s.editing == editing {
            false
        } else {
            s.editing = editing;
            s.revision += 1;
            true
        }
    };
    if changed {
        publish(app);
    }
}
#[tauri::command]
pub fn input_state(window: tauri::WebviewWindow) -> Result<Value, String> {
    own(&window)?;
    snapshot(window.app_handle())
}
#[tauri::command]
pub async fn input_mode(window: tauri::WebviewWindow, mode: String) -> Result<Value, String> {
    own(&window)?;
    if !["normal", "minimized", "hidden", "toggle"].contains(&mode.as_str()) {
        return Err("未知输入形态".into());
    }
    let app = window.app_handle();
    crate::append_log(
        "debug.log",
        &format!("input-native action={mode} stage=received"),
    );
    let _guard = OPERATIONS.lock().await;
    trace(app, &mode, "begin");
    let mode = if mode == "toggle" {
        if snapshot(app)?["mode"] == "minimized" {
            "normal"
        } else {
            "minimized"
        }
        .to_string()
    } else {
        mode
    };
    if mode == "hidden" {
        hide_now(app)?;
    } else {
        {
            let state = app.state::<Mutex<Service>>();
            let mut s = state.lock().map_err(|_| "输入组件不可用")?;
            if s.mode == "hidden" {
                return snapshot_without_lock(&s);
            }
            s.mode = mode.clone();
            s.revision += 1;
            s.editing = false;
            s.focus_pending = mode == "normal";
        }
        crate::panel_drag::cancel("input-bubble");
        layout(app, false)?;
        publish(app);
    }
    trace(app, &mode, "done");
    snapshot(app)
}
fn snapshot_without_lock(s: &Service) -> Result<Value, String> {
    Ok(
        json!({"revision":s.revision,"session":s.session,"mode":s.mode,"focus_serial":s.focus_serial}),
    )
}
#[tauri::command]
pub fn input_editing(window: tauri::WebviewWindow, editing: bool) -> Result<(), String> {
    own(&window)?;
    set_editing(window.app_handle(), editing);
    Ok(())
}
#[tauri::command]
pub async fn input_toggle(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只接受桌宠双击入口".into());
    }
    let app = window.app_handle();
    crate::append_log("debug.log", "input-native action=toggle stage=received");
    let _guard = OPERATIONS.lock().await;
    trace(app, "toggle", "begin");
    if snapshot(app)?["mode"] == "hidden" {
        open_now(app, true)?;
    } else {
        hide_now(app)?;
    }
    trace(app, "toggle", "done");
    Ok(())
}
#[tauri::command]
pub async fn input_gesture(window: tauri::WebviewWindow) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("只接受桌宠手势查询".into());
    }
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK,
    };
    let sf = window.scale_factor().unwrap_or(1.);
    unsafe {
        Ok(
            json!({"ms":GetDoubleClickTime().clamp(200,1000),"x":(GetSystemMetrics(SM_CXDOUBLECLK) as f64/sf).max(4.),"y":(GetSystemMetrics(SM_CYDOUBLECLK) as f64/sf).max(4.)}),
        )
    }
}
