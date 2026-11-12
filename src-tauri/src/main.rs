#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adapters;
mod state_machine;
mod bubble;
mod pet_menu;
mod panel_drag;
mod cursor_carry;
mod life;
mod life_store;
mod life_usage;
mod extension_windows;
mod music;
mod music_score;
mod chat;
mod chat_persona;
mod chat_store;
mod chat_credentials;
mod chat_profiles;
mod chat_connections;
mod memory;
mod memory_store;
mod memory_model;
mod memory_worker;
mod input_bubble;
mod pet_shape;
mod panel_shape;
mod diagnostics;
mod chat_backup;
mod chat_restore;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// 同一时刻只允许一个拖拽线程
static DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 当前（或最近一次）拖拽线程的序号；begin_drag 复用中线程时返回它
static CURRENT_SEQ: AtomicU64 = AtomicU64::new(0);
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static LOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tauri::command]
fn get_pet_home() -> String {
    let p = adapters::state_file::pet_home();
    let p = std::fs::canonicalize(&p).unwrap_or(p);
    let s = p.to_string_lossy().replace('\\', "/");
    s.strip_prefix("//?/").unwrap_or(&s).to_string()
}

fn append_log(name: &str, line: &str) {
    use std::io::Write as _;
    const LIMIT: u64 = 2 * 1024 * 1024;
    // 不允许日志名变成路径；轮转和写入使用同一把锁，避免并发覆盖。
    if !matches!(name, "debug.log" | "sleep_trigger.log") { return; }
    let Ok(_guard) = LOG_LOCK.lock() else { return };
    let home = adapters::state_file::pet_home();
    let path = home.join(name);
    let first = home.join(format!("{name}.1"));
    let second = home.join(format!("{name}.2"));
    if std::fs::metadata(&path).map(|m| m.len() >= LIMIT).unwrap_or(false) {
        // 无法安全轮转时保留现有文件，不截断、也不继续无限追加。
        if second.exists() && std::fs::remove_file(&second).is_err() { return; }
        if first.exists() && std::fs::rename(&first, &second).is_err() { return; }
        if std::fs::rename(&path, &first).is_err() { return; }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        // 单事件限长并去掉换行，避免异常信息把整份日志挤满或伪装多条事件。
        let text: String = line.chars().take(4096).map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
        let _ = writeln!(f, "{now} {text}");
    }
}

/// 前端睡眠动作触发取证（谁触发睡眠就记谁的账）
#[tauri::command]
fn log_sleep_trigger(reason: &str) {
    append_log("sleep_trigger.log", reason);
}

/// 通用调试日志
#[tauri::command]
fn log_debug(line: &str) {
    append_log("debug.log", line);
}

/// 拎起拖拽：8ms 轮询光标，窗口直接 set_position 跟随（硬绑定，无滞后积累）。
/// anchor 为拎起帧抓褶在窗口内的物理坐标；松手补偿换姿势的偏移，只按真实光标位移结算。
/// 松开左键时 emit pet-released（物理像素：序号 + 终点 + 相对抓取点的位移），
/// 摔落/安置判定由前端做。
/// 若上一次按住的线程还活着（快速连按导致它没观察到松手），复用它并返回其序号。
#[tauri::command]
fn begin_drag(win: tauri::WebviewWindow, anchor: Option<[i32; 2]>) -> u64 {
    cursor_carry::cancel();
    if DRAG_ACTIVE.swap(true, Ordering::SeqCst) {
        return CURRENT_SEQ.load(Ordering::SeqCst);
    }
    let seq = CURRENT_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        use device_query::{DeviceQuery, DeviceState};
        use tauri::Emitter;
        let ds = DeviceState::new();
        let (cx0, cy0) = ds.get_mouse().coords;
        let base = win.outer_position().unwrap_or_default();
        let [grab_x, grab_y] = anchor.unwrap_or([cx0 - base.x, cy0 - base.y]);
        let (offset_x, offset_y) = (cx0 - grab_x - base.x, cy0 - grab_y - base.y);
        append_log("debug.log", &format!("drag-anchor seq={seq} grab={grab_x},{grab_y} offset={offset_x},{offset_y}"));
        let (mut last_x, mut last_y) = (cx0 - grab_x, cy0 - grab_y);
        if ds.get_mouse().button_pressed.get(1).copied().unwrap_or(false) {
            let _ = win.set_position(tauri::PhysicalPosition::new(last_x, last_y));
        }
        let mut moved = false;
        loop {
            let m = ds.get_mouse();
            if !m.button_pressed.get(1).copied().unwrap_or(false) {
                break;
            }
            let (cx, cy) = m.coords;
            let (nx, ny) = (cx - grab_x, cy - grab_y);
            if nx != last_x || ny != last_y {
                let _ = win.set_position(tauri::PhysicalPosition::new(nx, ny));
                if !moved && (cx - cx0).abs() + (cy - cy0).abs() > 6 {
                    moved = true;
                    let _ = win.emit("pet-drag-moved", ());
                }
                last_x = nx;
                last_y = ny;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        last_x -= offset_x;
        last_y -= offset_y;
        let _ = win.set_position(tauri::PhysicalPosition::new(last_x, last_y));
        DRAG_ACTIVE.store(false, Ordering::SeqCst);
        let _ = win.emit(
            "pet-released",
            serde_json::json!({
                "seq": seq,
                "x": last_x,
                "y": last_y,
                "dx": last_x - base.x,
                "dy": last_y - base.y,
            }),
        );
    });
    seq
}

/// 视线跟随：100ms 轮询光标（device_query 没有事件回调，只能轮询）。
/// 只在鼠标"正在动"时算八向 emit；静止超过 2s 发一次空串复位，
/// 前端收到后把帧还给眨眼循环，避免注视帧和眨眼抢状态。
/// 近距离（120px 内）不切换，避免在她脸上晃动时抖动。
fn gaze_watch(win: tauri::WebviewWindow) {
    use device_query::{DeviceQuery, DeviceState};
    use tauri::{Emitter, Manager};
    let ds = DeviceState::new();
    let mut last = "";
    let mut last_pos = (i32::MIN, i32::MIN);
    let mut moved_at = std::time::Instant::now();
    let mut last_input = String::new();
    let mut input_at = std::time::Instant::now();
    loop {
        let mouse = ds.get_mouse();
        let (cx, cy) = mouse.coords;
        extension_windows::observe(win.app_handle(), [cx, cy]);
        let now = std::time::Instant::now();
        let input_revision = cursor_carry::input_revision();
        if input_revision != last_input {
            last_input = input_revision.clone();
            input_at = now;
        }
        if (cx, cy) != last_pos {
            last_pos = (cx, cy);
            moved_at = now;
        }
        let same_monitor = win.current_monitor().ok().flatten().map(|m| {
            let area = m.work_area();
            cx >= area.position.x && cy >= area.position.y
                && cx < area.position.x + area.size.width as i32
                && cy < area.position.y + area.size.height as i32
        }).unwrap_or(false);
        let (extension_revision, extension_blocked) = extension_windows::cursor_status();
        let _ = win.emit("pet-cursor", serde_json::json!({
            "extension_revision": extension_revision, "extension_blocked": extension_blocked,
            "x": cx, "y": cy, "still_ms": now.duration_since(moved_at).as_millis() as u64,
            "down": mouse.button_pressed.iter().any(|pressed| *pressed), "same_monitor": same_monitor,
            "carry_available": cursor_carry::available(), "carrying": cursor_carry::active(),
            "input_revision": input_revision, "input_idle_ms": now.duration_since(input_at).as_millis() as u64
        }));
        if now.duration_since(moved_at).as_millis() > 2000 {
            if !last.is_empty() {
                last = "";
                let _ = win.emit("pet-gaze", "");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }
        if let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) {
            let dx = cx as f64 - (pos.x as f64 + size.width as f64 / 2.0);
            let dy = cy as f64 - (pos.y as f64 + size.height as f64 / 2.0);
            if dx.hypot(dy) >= 120.0 {
                // 屏幕坐标 y 向下：0=右，顺时针 1=右下 2=下 3=左下 4=左 5=左上 6=上 7=右上
                let oct = (dy.atan2(dx) / (std::f64::consts::PI / 4.0)).round() as i32;
                let dir = ["右", "右下", "下", "左下", "左", "左上", "上", "右上"]
                    [oct.rem_euclid(8) as usize];
                if dir != last {
                    last = dir;
                    let _ = win.emit("pet-gaze", dir);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// 设置文件默认值
fn default_settings() -> serde_json::Value {
    serde_json::json!({
        "scale": 1.0,
        "panel_style": "light",
        "wander": true,
        "gaze": true,
        "daily_speech": true,
        "cursor_play": true,
        "cursor_carry": false,
        "light_sleep_s": 900,
        "deep_sleep_s": 1200,
        "night_light_sleep_s": 300,
        "night_deep_sleep_s": 600,
        "sleep_defaults_version": 2
    })
}

#[tauri::command]
fn load_settings() -> serde_json::Value {
    let path = adapters::state_file::pet_home().join("config").join("设置.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .and_then(|mut s: serde_json::Value| {
            // 只迁移未标版本的旧默认组合；自定义值及之后主动保存的2/5分钟保留。
            if s.get("sleep_defaults_version").is_none()
                && s["light_sleep_s"].as_f64() == Some(120.0)
                && s["deep_sleep_s"].as_f64() == Some(300.0) {
                s["light_sleep_s"] = serde_json::json!(900);
                s["deep_sleep_s"] = serde_json::json!(1200);
            }
            validate_settings(s).ok()
        })
        .unwrap_or_else(default_settings)
}

fn validate_settings(settings: serde_json::Value) -> Result<serde_json::Value, String> {
    if !settings.is_object() { return Err("设置必须是对象".into()); }
    let mut value = default_settings();
    for key in ["scale", "panel_style", "wander", "gaze", "daily_speech", "cursor_play", "cursor_carry", "light_sleep_s", "deep_sleep_s", "night_light_sleep_s", "night_deep_sleep_s"] {
        if let Some(v) = settings.get(key) { value[key] = v.clone(); }
    }
    let scale = value["scale"].as_f64().ok_or("体型缩放必须是数字")?;
    if !(0.5..=2.0).contains(&scale) { return Err("体型缩放应在 0.5～2.0 之间".into()); }
    if !matches!(value["panel_style"].as_str(), Some("light" | "clear")) { return Err("未知面板背景样式".into()); }
    for key in ["wander", "gaze", "daily_speech", "cursor_play", "cursor_carry"] {
        if !value[key].is_boolean() { return Err(format!("{key} 必须是开关值")); }
    }
    for (light_key, deep_key) in [("light_sleep_s", "deep_sleep_s"), ("night_light_sleep_s", "night_deep_sleep_s")] {
        let light = value[light_key].as_f64().ok_or("浅睡时间必须是数字")?;
        let deep = value[deep_key].as_f64().ok_or("熟睡时间必须是数字")?;
        if !(60.0..=1800.0).contains(&light) { return Err("浅睡时间应在 1～30 分钟之间".into()); }
        if !(120.0..=3600.0).contains(&deep) || deep < light + 60.0 {
            return Err("熟睡时间应在 2～60 分钟之间，且至少比浅睡晚 1 分钟".into());
        }
    }
    Ok(value)
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: serde_json::Value) -> Result<(), String> {
    use tauri::Emitter;
    let _guard = SETTINGS_LOCK.lock().map_err(|_| "设置锁不可用")?;
    let settings = validate_settings(settings)?;
    let dir = adapters::state_file::pet_home().join("config");
    let tmp = dir.join("设置.json.tmp");
    let path = dir.join("设置.json");
    let s = serde_json::to_string_pretty(&settings).map_err(|e| format!("设置序列化失败：{e}"))?;
    std::fs::write(&tmp, s).map_err(|e| format!("写入设置失败：{e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换设置失败：{e}"))?;
    cursor_carry::set_enabled(settings["cursor_carry"] == true);
    app.emit("settings-changed", settings).map_err(|e| format!("设置已保存，但即时应用失败，请重启桌宠：{e}"))?;
    Ok(())
}

#[tauri::command]
fn preview_settings(app: tauri::AppHandle, settings: serde_json::Value) -> Result<(), String> {
    use tauri::{Emitter, Manager};
    let _guard = SETTINGS_LOCK.lock().map_err(|_| "设置锁不可用")?;
    if !pet_menu::preferences_active() || app.get_webview_window("pet-menu").and_then(|w| w.is_visible().ok()) != Some(true) { return Ok(()); }
    let settings = validate_settings(settings)?;
    // 未保存的开启预览不能获得真实光标控制权；关闭预览立即释放。
    cursor_carry::set_enabled(settings["cursor_carry"] == true && load_settings()["cursor_carry"] == true);
    app.emit("settings-changed", settings).map_err(|e| e.to_string())
}

/// 额度查询：读本地 OAuth token，GET api.kimi.com/coding/v1/usages。
/// 非公开接口，任何一步失败都返回 Err，前端兜底显示。
#[tauri::command]
async fn query_usage() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(query_usage_inner).await
        .map_err(|e| format!("额度查询任务失败：{e}"))?
}

fn query_usage_inner() -> Result<serde_json::Value, String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "找不到用户目录".to_string())?;
    let cred_path = std::path::PathBuf::from(home)
        .join(".kimi-code")
        .join("credentials")
        .join("kimi-code.json");
    let text = std::fs::read_to_string(&cred_path).map_err(|e| format!("读取凭证失败: {e}"))?;
    let cred: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("凭证解析失败: {e}"))?;
    let token = cred["access_token"]
        .as_str()
        .ok_or_else(|| "凭证里没有 access_token".to_string())?;
    if let Some(exp) = cred["expires_at"].as_i64() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        // expires_at 单位随版本可能是秒或毫秒，按量级判断
        let exp_ms = if exp < 100_000_000_000 { exp * 1000 } else { exp };
        if now_ms >= exp_ms {
            return Err("登录已过期，在 CLI 里随便跑一条命令刷新后再试".to_string());
        }
    }
    let resp = ureq::get("https://api.kimi.com/coding/v1/usages")
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| format!("请求失败: {e}"))?;
    let body: serde_json::Value = resp.into_json().map_err(|e| format!("响应解析失败: {e}"))?;
    Ok(serde_json::json!({
        "limit_5h": body["usages"]["limit_5h"],
        "limit_7d": body["usages"]["limit_7d"],
        "usage": body["usage"],
    }))
}

fn restore_settings(app: &tauri::AppHandle) {
    use tauri::Emitter;
    if let Ok(_guard) = SETTINGS_LOCK.lock() {
        let settings = load_settings();
        cursor_carry::set_enabled(settings["cursor_carry"] == true);
        let _ = app.emit("settings-changed", settings);
    }
}

/// 右键菜单：独立主题面板，在当前光标位置弹出。
#[tauri::command]
async fn show_pet_menu(win: tauri::WebviewWindow) -> Result<(), String> {
    use tauri::Manager;
    pet_menu::show(win.app_handle(), false)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_pet_home,
            begin_drag,
            log_sleep_trigger,
            log_debug,
            load_settings,
            save_settings,
            preview_settings,
            show_pet_menu,
            query_usage,
            cursor_carry::carry_cursor_begin, cursor_carry::carry_cursor_update, cursor_carry::carry_cursor_end,
            bubble::set_bubble, bubble::bubble_content, bubble::bubble_ready, bubble::drag_bubble, bubble::bubble_anchor,
            pet_menu::menu_ready, pet_menu::menu_action, pet_menu::menu_view, pet_menu::menu_preferences,
            extension_windows::extension_open, extension_windows::extension_ready,
            extension_windows::extension_close, extension_windows::extension_state, extension_windows::extension_pin,
            extension_windows::extension_music_layout,
            music::music_get, music::music_start, music::music_beat, music::music_stop, music::music_finish, music::music_participation,
            life::life_get, life::life_action, life::life_mode,
            chat::chat_state, chat::chat_configure, chat::chat_send, chat::chat_retry,
            chat::chat_stop, chat::chat_conversation, chat::chat_history_page,
            chat_connections::chat_connections, chat_connections::chat_connection_save,
            chat_connections::chat_connection_action, chat_connections::chat_models,
            chat::chat_pet_context, chat::chat_activity_state,
            chat::chat_input_state, chat::chat_view,
            memory::memory_state, memory::memory_update, memory::memory_sources,
            input_bubble::input_state, input_bubble::input_mode, input_bubble::input_editing,
            input_bubble::input_toggle, input_bubble::input_gesture,
            bubble::bubble_hover,
            bubble::bubble_dismiss,
            panel_drag::begin_panel_drag,
            pet_shape::pet_shape_begin, pet_shape::pet_shape_prepare,
            pet_shape::pet_shape_commit, pet_shape::pet_shape_reset, panel_shape::panel_shape,
            diagnostics::export_diagnostics, chat_backup::backup_chat,
            chat_restore::chat_restore_list, chat_restore::chat_restore_schedule, chat_restore::chat_restore_cancel
        ])
        .on_window_event(|window, event| {
            use tauri::Manager;
            pet_shape::window_event(window, event);
            music::window_event(window, event);
            extension_windows::window_event(window, event);
            if window.label()=="input-bubble" && matches!(event,tauri::WindowEvent::Focused(false)) {input_bubble::focus_lost(window.app_handle());}
            if window.label() == "pet-menu" && matches!(event, tauri::WindowEvent::Focused(false)) {
                if panel_drag::active("pet-menu") { return; }
                let _ = window.hide();
                pet_menu::cancel_preview(window.app_handle());
                use tauri::Emitter;
                let _ = window.emit("menu-closed", ());
                let _ = window.app_handle().emit("pet-menu-visible", false);
            }
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) {
                bubble::position(window.app_handle());
            }
        })
        .setup(|app| {
            use tauri::Manager;
            let home = std::fs::canonicalize(adapters::state_file::pet_home())?;
            for folder in ["config", "帧文件", "道具"] {
                app.asset_protocol_scope().allow_directory(home.join(folder), true)?;
            }
            life::install(app.handle());
            pet_shape::install(app.handle());
            chat::install(app.handle());
            memory::install(app.handle());
            input_bubble::install(app.handle());
            music::install(app.handle());
            cursor_carry::set_enabled(load_settings()["cursor_carry"] == true);
            app.on_menu_event(|app, ev| match ev.id().0.as_str() {
                "menu_input" => {let _=input_bubble::open(app,true);},
                "menu_settings" => { let _ = pet_menu::show(app, true); },
                "menu_usage" => {
                    use tauri::Emitter;
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.emit("show-usage", ());
                    }
                }
                "menu_quit" => app.exit(0),
                "menu_chat_settings" | "menu_chat_history" => {
                    let view=if ev.id().0=="menu_chat_settings" {"settings"} else {"history"};
                    let handle=app.clone();
                    tauri::async_runtime::spawn(async move {if chat::open_view(&handle,view).is_err(){append_log("debug.log","chat-view stage=failed");}});
                },
                _ => {}
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || adapters::run_loop(handle));
            // 系统托盘：设置/退出的入口备份（右键宠物本体之外的第二通道）
            {
                use tauri::menu::{MenuBuilder, MenuItemBuilder};
                use tauri::tray::TrayIconBuilder;
                let tray_menu = MenuBuilder::new(app)
                    .item(&MenuItemBuilder::with_id("menu_chat_settings", "交流设置").build(app)?)
                    .item(&MenuItemBuilder::with_id("menu_usage", "星光额度").build(app)?)
                    .item(&MenuItemBuilder::with_id("menu_settings", "偏好设置").build(app)?)
                    .separator()
                    .item(&MenuItemBuilder::with_id("menu_quit", "退出桌宠").build(app)?)
                    .build()?;
                let icon_path = adapters::state_file::pet_home()
                    .join("帧文件")
                    .join("待机眨眼")
                    .join("01_睁眼.png");
                match std::fs::read(&icon_path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| tauri::image::Image::from_bytes(&b).map_err(|e| e.to_string()))
                {
                    Ok(img) => {
                        let _ = TrayIconBuilder::new()
                            .menu(&tray_menu)
                            .tooltip("Moon·月")
                            .icon(img)
                            .build(app);
                    }
                    Err(e) => append_log("debug.log", &format!("tray_icon_err {e}")),
                }
            }
            // 初始位置：主屏右下角（物理像素）。JS 启动时会再按工作区微调一次
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(Some(mon)) = win.primary_monitor() {
                    let sf = mon.scale_factor();
                    let ms = mon.size();
                    let w = (320.0 * sf) as i32;
                    let h = (360.0 * sf) as i32;
                    let x = ms.width as i32 - w - (40.0 * sf) as i32;
                    let y = ms.height as i32 - h;
                    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
                }
                std::thread::spawn(move || gaze_watch(win));
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running pet-engine");
}
