//! 面板运行时拖拽：只读取用户真实光标/按键，不进入原生窗口拖动循环。
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tauri::{Emitter, Manager};

static DRAGS: OnceLock<Mutex<BTreeMap<String, Arc<AtomicU64>>>> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn flags() -> &'static Mutex<BTreeMap<String, Arc<AtomicU64>>> {
    DRAGS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub fn active(label: &str) -> bool {
    flags()
        .lock()
        .ok()
        .and_then(|f| f.get(label).cloned())
        .is_some_and(|f| f.load(Ordering::SeqCst) != 0)
}
pub fn cancel(label: &str) {
    if let Some(flag) = flags().lock().ok().and_then(|f| f.get(label).cloned()) {
        flag.store(0, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn begin_panel_drag(win: tauri::WebviewWindow) -> Result<(), String> {
    start(win)
}

pub fn start(win: tauri::WebviewWindow) -> Result<(), String> {
    if !matches!(win.label(), "pet-menu" | "bubble")
        && !super::extension_windows::registered(win.label())
    {
        return Err("该窗口不支持面板拖动".into());
    }
    if win.is_visible().ok() != Some(true) {
        return Err("窗口未显示".into());
    }
    let flag = flags()
        .lock()
        .map_err(|_| "面板拖动锁不可用")?
        .entry(win.label().into())
        .or_default()
        .clone();
    let seq = SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
    if flag
        .compare_exchange(0, seq, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(());
    }
    let base = match win.outer_position() {
        Ok(pos) => pos,
        Err(e) => {
            let _ = flag.compare_exchange(seq, 0, Ordering::SeqCst, Ordering::SeqCst);
            return Err(e.to_string());
        }
    };
    use device_query::{DeviceQuery, DeviceState};
    let mouse = DeviceState::new().get_mouse().coords;
    let bubble = win.label() == "bubble";
    let bubble_owner=super::bubble::bubble_content().and_then(|v|v["owner"].as_str().map(str::to_owned)).unwrap_or_default();
    super::extension_windows::set_dragging(win.app_handle(), win.label(), true);
    if bubble {
        if let Some(main) = win.app_handle().get_webview_window("main") {
            let _ = main.emit("bubble-dragging", serde_json::json!({"dragging":true,"owner":&bubble_owner}));
        }
    }
    super::append_log(
        "debug.log",
        &format!("panel-drag-start window={}", win.label()),
    );
    std::thread::spawn(move || {
        let ds = DeviceState::new();
        let mut last = base;
        loop {
            let m = ds.get_mouse();
            if flag.load(Ordering::SeqCst) != seq
                || !m.button_pressed.get(1).copied().unwrap_or(false)
                || win.is_visible().ok() != Some(true)
            {
                break;
            }
            let pos = tauri::PhysicalPosition::new(
                base.x + m.coords.0 - mouse.0,
                base.y + m.coords.1 - mouse.1,
            );
            if pos != last {
                if win.set_position(pos).is_err() {
                    break;
                }
                last = pos;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        // 关闭后快速重开时，旧线程不能清掉新拖动，也不能把窗口重新抢到前台。
        let owned = flag
            .compare_exchange(seq, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if owned && !bubble && win.label()!="input-bubble" && win.is_visible().ok() == Some(true) {
            let _ = win.set_focus();
        }
        if owned {
            if win.label()=="input-bubble" {super::input_bubble::end_drag(win.app_handle());}
            super::extension_windows::set_dragging(win.app_handle(), win.label(), false);
        }
        if owned && bubble {
            if let Some(main) = win.app_handle().get_webview_window("main") {
                let _ = main.emit("bubble-dragging", serde_json::json!({"dragging":false,"owner":&bubble_owner}));
            }
        }
        super::append_log(
            "debug.log",
            &format!("panel-drag-end window={}", win.label()),
        );
    });
    Ok(())
}
