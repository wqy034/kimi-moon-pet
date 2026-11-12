//! 扩展窗口公共层。窗口可见不等于桌宠参与活动；生活窗口始终为passive。
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
};
use tauri::{Emitter, Manager};

struct Spec {
    id: &'static str,
    label: &'static str,
    page: &'static str,
    title: &'static str,
    participation: &'static str,
    size: [f64; 2],
    min: [f64; 2],
}
const SPECS: &[Spec] = &[
    Spec {id:"input",label:"input-bubble",page:"input.html",title:"和她说话",participation:"passive",size:[320.,42.],min:[42.,42.]},
    Spec {
        id: "chat", label: "extension-chat", page: "chat.html", title: "Moon·月 · 交流资料",
        participation: "passive", size: [460., 720.], min: [360., 480.],
    },
    Spec {
        id: "life",
        label: "extension-life",
        page: "life.html",
        title: "Moon·月 · 暗月的小生活",
        participation: "passive",
        size: [380., 600.],
        min: [340., 420.],
    },
    Spec {
        id: "music",
        label: "extension-music",
        page: "music.html",
        title: "Moon·月 · 小小琴盘",
        participation: "requested",
        size: [560., 320.],
        min: [480., 280.],
    },
];
#[derive(Clone, PartialEq, Serialize)]
pub struct WindowState {
    id: String,
    visible: bool,
    focused: bool,
    dragging: bool,
    pointer_inside: bool,
    pinned: bool,
    participation: &'static str,
}
struct Entry {
    state: WindowState,
    ready: bool,
    creating: bool,
    wants_open: bool,
    rect: Option<[i32; 4]>,
}
struct Registry {
    revision: u64,
    entries: BTreeMap<&'static str, Entry>,
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    revision: u64,
    windows: Vec<WindowState>,
    cursor_blocked: bool,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static CURSOR_BLOCKED: AtomicBool = AtomicBool::new(false);
static MUSIC_OPERATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static MUSIC_TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct MusicSizes {
    expanded: bool,
    sizes: [[f64; 2]; 2],
}
static MUSIC_SIZES: Mutex<MusicSizes> = Mutex::new(MusicSizes {
    expanded: false,
    sizes: [[560., 320.], [560., 800.]],
});

// 不持Registry/尺寸锁调用窗口API，也不在此显示窗口。
fn music_size(window: &tauri::WebviewWindow, expanded: bool, ticket: u64) -> Result<(), String> {
    let sf = window.scale_factor().map_err(|e| e.to_string())?;
    let old = window.inner_size().map_err(|e| e.to_string())?;
    let target = {
        let s = MUSIC_SIZES.lock().map_err(|_| "琴盘尺寸不可用")?;
        if s.expanded == expanded {
            return Ok(());
        }
        s.sizes[usize::from(expanded)]
    };
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("找不到琴盘所在屏幕")?;
    let area = monitor.work_area();
    let available = [
        (area.size.width as f64 / sf - 24.).max(100.),
        (area.size.height as f64 / sf - 24.).max(100.),
    ];
    let min = [
        480_f64.min(available[0]),
        (if expanded { 530_f64 } else { 280_f64 }).min(available[1]),
    ];
    let position = window.outer_position().map_err(|e| e.to_string())?;
    if MUSIC_TICKET.load(Ordering::SeqCst) != ticket {
        return Ok(());
    }
    // 通过票校验后才记录旧档尺寸：过期请求（如 reveal 与切换交错）不能覆盖记忆
    MUSIC_SIZES.lock().map_err(|_| "琴盘尺寸不可用")?.sizes[usize::from(!expanded)] =
        [old.width as f64 / sf, old.height as f64 / sf];
    window
        .set_min_size(Some(tauri::LogicalSize::new(min[0], min[1])))
        .map_err(|e| e.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(
            target[0].clamp(min[0], available[0]),
            target[1].clamp(min[1], available[1]),
        ))
        .map_err(|e| e.to_string())?;
    // 即使新请求此时到达，也先记录已经应用的尺寸归属，供下一次切换保存。
    MUSIC_SIZES.lock().map_err(|_| "琴盘尺寸不可用")?.expanded = expanded;
    if MUSIC_TICKET.load(Ordering::SeqCst) != ticket {
        return Ok(());
    }
    let actual = window.outer_size().map_err(|e| e.to_string())?;
    let gap = (12. * sf).round() as i32;
    let left = area.position.x + gap;
    let top = area.position.y + gap;
    let right = area.position.x + area.size.width as i32 - gap;
    let bottom = area.position.y + area.size.height as i32 - gap;
    window
        .set_position(tauri::PhysicalPosition::new(
            position
                .x
                .clamp(left, (right - actual.width as i32).max(left)),
            position
                .y
                .clamp(top, (bottom - actual.height as i32).max(top)),
        ))
        .map_err(|e| e.to_string())?;
    crate::append_log(
        "debug.log",
        &format!(
            "music-layout expanded={expanded} size={}x{}",
            actual.width, actual.height
        ),
    );
    Ok(())
}

#[tauri::command]
pub async fn extension_music_layout(
    window: tauri::WebviewWindow,
    presentation: String,
) -> Result<(), String> {
    if window.label() != "extension-music" {
        return Err("只允许琴盘切换布局".into());
    }
    let expanded = match presentation.as_str() {
        "compact" => false,
        "expanded" => true,
        _ => return Err("未知琴盘布局".into()),
    };
    let ticket = MUSIC_TICKET.fetch_add(1, Ordering::SeqCst) + 1;
    let _operation = MUSIC_OPERATIONS.lock().await;
    let open = registry().lock().map_err(|_| "扩展窗口锁不可用")?.entries["music"].wants_open;
    if !open || MUSIC_TICKET.load(Ordering::SeqCst) != ticket {
        return Ok(());
    }
    music_size(&window, expanded, ticket)
}
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            revision: 0,
            entries: SPECS
                .iter()
                .map(|s| {
                    (
                        s.id,
                        Entry {
                            state: WindowState {
                                id: s.id.into(),
                                visible: false,
                                focused: false,
                                dragging: false,
                                pointer_inside: false,
                                pinned: true,
                                participation: s.participation,
                            },
                            ready: false,
                            creating: false,
                            wants_open: false,
                            rect: None,
                        },
                    )
                })
                .collect(),
        })
    })
}
impl Registry {
    fn snapshot(&self) -> Snapshot {
        let windows: Vec<_> = self.entries.values().map(|e| e.state.clone()).collect();
        let cursor_blocked = windows
            .iter()
            .any(|w| w.visible && (w.focused || w.dragging || w.pointer_inside));
        Snapshot {
            revision: self.revision,
            windows,
            cursor_blocked,
        }
    }
}
fn modify(app: &tauri::AppHandle, change: impl FnOnce(&mut Registry)) {
    let snapshot = {
        let Ok(mut r) = registry().lock() else { return };
        let before = r.snapshot();
        change(&mut r);
        if before.windows == r.snapshot().windows {
            return;
        }
        r.revision += 1;
        let snapshot = r.snapshot();
        CURSOR_BLOCKED.store(snapshot.cursor_blocked, Ordering::SeqCst);
        snapshot
    };
    if snapshot.cursor_blocked {
        super::cursor_carry::cancel();
    }
    let _ = app.emit("extensions-changed", snapshot);
}
pub fn cursor_blocked() -> bool {
    CURSOR_BLOCKED.load(Ordering::SeqCst)
}
pub fn cursor_status() -> (u64, bool) {
    registry()
        .lock()
        .map(|r| (r.revision, cursor_blocked()))
        .unwrap_or((0, true))
}
/// 原生控制还要避开将要经过的扩展矩形，不能只依赖WebView的事件到达时机。
pub fn control_blocked(point: [i32; 2]) -> bool {
    if cursor_blocked() {
        return true;
    }
    registry()
        .lock()
        .map(|r| {
            r.entries
                .values()
                .any(|e| e.state.visible && e.rect.is_some_and(|rect| inside(point, rect)))
        })
        .unwrap_or(true)
}
fn spec(id: &str) -> Result<&'static Spec, String> {
    SPECS
        .iter()
        .find(|s| s.id == id)
        .ok_or("未知扩展窗口".into())
}
fn own(label: &str) -> Result<&'static Spec, String> {
    SPECS
        .iter()
        .find(|s| s.label == label)
        .ok_or("调用者不是已登记的扩展窗口".into())
}
pub fn registered(label: &str) -> bool {
    own(label).is_ok()
}
pub fn component_ready(id:&str)->bool { registry().lock().map(|r|r.entries.get(id).is_some_and(|e|e.ready&&e.wants_open)).unwrap_or(false) }
fn can_read(label: &str) -> Result<(), String> {
    if matches!(label, "main" | "pet-menu") || registered(label) {
        Ok(())
    } else {
        Err("该窗口不能查询扩展状态".into())
    }
}
fn pointer() -> [i32; 2] {
    use device_query::{DeviceQuery, DeviceState};
    let (x, y) = DeviceState::new().get_mouse().coords;
    [x, y]
}
fn inside(p: [i32; 2], rect: [i32; 4]) -> bool {
    p[0] >= rect[0] && p[0] < rect[2] && p[1] >= rect[1] && p[1] < rect[3]
}

// 复用现有100ms真实光标采样，不由WebView上报hover，避免漏掉mouseleave。
pub fn observe(app: &tauri::AppHandle, point: [i32; 2]) {
    for s in SPECS {
        let observation = app.get_webview_window(s.label).map(|w| {
            let visible = w.is_visible().unwrap_or(false);
            let rect = if visible {
                w.outer_position()
                    .ok()
                    .zip(w.outer_size().ok())
                    .map(|(p, z)| [p.x, p.y, p.x + z.width as i32, p.y + z.height as i32])
            } else {
                None
            };
            (visible, visible && w.is_focused().unwrap_or(false), rect)
        });
        modify(app, |r| {
            let e = r.entries.get_mut(s.id).unwrap();
            let (visible, focused, rect) = observation.unwrap_or((false, false, None));
            e.state.visible = visible;
            e.state.focused = focused;
            e.rect = rect;
            e.state.pointer_inside = rect.is_some_and(|rect| inside(point, rect));
            if !visible {
                e.state.dragging = false;
            }
        });
    }
}
pub fn set_dragging(app: &tauri::AppHandle, label: &str, dragging: bool) {
    let Ok(s) = own(label) else { return };
    modify(app, |r| {
        let e = r.entries.get_mut(s.id).unwrap();
        e.state.dragging = dragging && e.state.visible;
    });
}

fn place(app: &tauri::AppHandle, s: &Spec) -> Result<(), String> {
    let window = app.get_webview_window(s.label).ok_or("扩展窗口不存在")?;
    let main = app.get_webview_window("main").ok_or("桌宠窗口不存在")?;
    let monitor = main
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("找不到桌宠所在屏幕")?;
    let area = monitor.work_area();
    let sf = monitor.scale_factor();
    let gap = (12. * sf).round() as i32;
    let aw = (area.size.width as f64 / sf - 24.).max(100.);
    let ah = (area.size.height as f64 / sf - 24.).max(100.);
    let current_sf = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let width = (size.width as f64 / current_sf).min(aw);
    let height = (size.height as f64 / current_sf).min(ah);
    window
        .set_min_size(Some(tauri::LogicalSize::new(
            s.min[0].min(aw),
            s.min[1].min(ah),
        )))
        .map_err(|e| e.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    let pet = super::bubble::visible_rect(app).ok_or("无法读取人物位置")?;
    let bw = (width * sf).round() as i32;
    let bh = (height * sf).round() as i32;
    let left = area.position.x + gap;
    let top = area.position.y + gap;
    let right = area.position.x + area.size.width as i32 - gap;
    let bottom = area.position.y + area.size.height as i32 - gap;
    let x = if pet[2] + gap + bw <= right {
        pet[2] + gap
    } else if pet[0] - gap - bw >= left {
        pet[0] - gap - bw
    } else if right - pet[2] >= pet[0] - left {
        pet[2] + gap
    } else {
        pet[0] - gap - bw
    };
    let y = pet[3] - bh;
    window
        .set_position(tauri::PhysicalPosition::new(
            x.clamp(left, (right - bw).max(left)),
            y.clamp(top, (bottom - bh).max(top)),
        ))
        .map_err(|e| e.to_string())?;
    // 进入另一DPI屏幕后，用最终物理尺寸再限制一次，避免逻辑尺寸换算越界。
    let z = window.outer_size().map_err(|e| e.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(
            x.clamp(left, (right - z.width as i32).max(left)),
            y.clamp(top, (bottom - z.height as i32).max(top)),
        ))
        .map_err(|e| e.to_string())
}
fn reveal(app: &tauri::AppHandle, s: &Spec) -> Result<(), String> {
    let show = registry()
        .lock()
        .map_err(|_| "扩展窗口锁不可用")?
        .entries
        .get(s.id)
        .is_some_and(|e| e.ready && e.wants_open);
    if !show {
        return Ok(());
    }
    if s.id=="input" {return super::input_bubble::reveal(app);}
    let window = app.get_webview_window(s.label).ok_or("扩展窗口不存在")?;
    if !window.is_visible().map_err(|e| e.to_string())? {
        if s.id == "music" {
            let ticket = MUSIC_TICKET.fetch_add(1, Ordering::SeqCst) + 1;
            music_size(&window, false, ticket)?;
        }
        place(app, s)?;
    }
    window.show().map_err(|e| e.to_string())?;
    // close可能与异步创建交错；关闭意图不能被迟到的ready重新打开。
    if !registry().lock().map_err(|_| "扩展窗口锁不可用")?.entries[s.id].wants_open {
        window.hide().map_err(|e| e.to_string())?;
        return Ok(());
    }
    window.set_focus().map_err(|e| e.to_string())?;
    observe(app, pointer());
    window
        .emit("extension-opened", s.id)
        .map_err(|e| e.to_string())
}
pub fn open(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    if id=="input" {return super::input_bubble::open(app,true);}
    open_component(app,id)
}
pub fn open_component(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let s = spec(id)?;
    super::cursor_carry::cancel();
    let create = {
        let mut r = registry().lock().map_err(|_| "扩展窗口锁不可用")?;
        let e = r.entries.get_mut(s.id).unwrap();
        e.wants_open = true;
        if app.get_webview_window(s.label).is_some() {
            false
        } else if e.creating {
            return Ok(());
        } else {
            e.creating = true;
            e.ready = false;
            true
        }
    };
    if create {
        if let Err(error) =
            tauri::WebviewWindowBuilder::new(app, s.label, tauri::WebviewUrl::App(s.page.into()))
                .title(s.title)
                .inner_size(s.size[0], s.size[1])
                .min_inner_size(s.min[0], s.min[1])
                .transparent(true)
                .decorations(false)
                .resizable(s.id != "input")
                .maximizable(false)
                .minimizable(false)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .visible(false)
                .build()
        {
            modify(app, |r| {
                let e = r.entries.get_mut(s.id).unwrap();
                e.creating = false;
                e.wants_open = false;
            });
            return Err(error.to_string());
        }
        crate::append_log("debug.log", &format!("extension-created id={}", s.id));
    }
    reveal(app, s)
}
fn hide(app: &tauri::AppHandle, s: &Spec) -> Result<(), String> {
    if s.id == "chat" { super::chat::cancel_check(app); }
    if s.id == "input" {super::input_bubble::hide(app);}
    if s.id == "music" {
        MUSIC_TICKET.fetch_add(1, Ordering::SeqCst);
        super::music::stop_window(app);
    }
    modify(app, |r| {
        let e = r.entries.get_mut(s.id).unwrap();
        e.wants_open = false;
        e.state.visible = false;
        e.state.focused = false;
        e.state.dragging = false;
        e.state.pointer_inside = false;
        e.rect = None;
    });
    super::panel_drag::cancel(s.label);
    if let Some(w) = app.get_webview_window(s.label) {
        w.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let Ok(s) = own(window.label()) else { return };
    let app = window.app_handle();
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = hide(app, s);
        }
        tauri::WindowEvent::Destroyed => {
            if s.id == "chat" { super::chat::cancel_check(app); }
            super::panel_drag::cancel(s.label);
            modify(app, |r| {
                let e = r.entries.get_mut(s.id).unwrap();
                e.ready = false;
                e.creating = false;
                e.wants_open = false;
                e.rect = None;
                e.state.visible = false;
                e.state.focused = false;
                e.state.dragging = false;
                e.state.pointer_inside = false;
                e.state.pinned = true;
            });
        }
        tauri::WindowEvent::Focused(focused) => modify(app, |r| {
            let e = r.entries.get_mut(s.id).unwrap();
            e.state.focused = *focused && e.state.visible;
        }),
        _ => {}
    }
}
#[tauri::command]
pub async fn extension_open(window: tauri::WebviewWindow, kind: String) -> Result<(), String> {
    if !matches!(window.label(), "main" | "pet-menu") {
        return Err("只接受桌宠入口打开扩展".into());
    }
    open(window.app_handle(), &kind)
}
#[tauri::command]
pub fn extension_state(window: tauri::WebviewWindow) -> Result<Snapshot, String> {
    can_read(window.label())?;
    Ok(registry()
        .lock()
        .map_err(|_| "扩展窗口锁不可用")?
        .snapshot())
}
#[tauri::command]
pub async fn extension_ready(window: tauri::WebviewWindow) -> Result<Snapshot, String> {
    let s = own(window.label())?;
    let app = window.app_handle();
    modify(app, |r| {
        let e = r.entries.get_mut(s.id).unwrap();
        e.ready = true;
        e.creating = false;
    });
    reveal(app, s)?;
    extension_state(window)
}
#[tauri::command]
pub fn extension_close(window: tauri::WebviewWindow) -> Result<(), String> {
    hide(window.app_handle(), own(window.label())?)
}
#[tauri::command]
pub fn extension_pin(window: tauri::WebviewWindow, pinned: bool) -> Result<Snapshot, String> {
    let s = own(window.label())?;
    window
        .set_always_on_top(pinned)
        .map_err(|e| e.to_string())?;
    modify(window.app_handle(), |r| {
        r.entries.get_mut(s.id).unwrap().state.pinned = pinned
    });
    extension_state(window)
}
