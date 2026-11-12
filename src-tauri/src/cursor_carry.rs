//! 光标抱跑原型：显式开关、短租约、输入即释放。绝不发送按键或鼠标点击。
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    Mutex, Once, OnceLock,
};
use std::time::{Duration, Instant};
use tauri::Emitter;
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const MARKER: usize = 0x4d504554;
fn marker() -> usize {
    MARKER ^ (std::process::id() as usize).rotate_left(16)
}
const PROBE_ID_MASK: usize = usize::MAX >> (usize::BITS / 2);
fn probe_prefix() -> usize {
    // 与实际移动标记的最高位不同；低半部只承载探针编号。
    (marker() & !PROBE_ID_MASK) ^ (1usize << (usize::BITS - 1))
}
static ENABLED: AtomicBool = AtomicBool::new(false);
static WORK_BUSY: AtomicBool = AtomicBool::new(false);
static HOOK_READY: AtomicBool = AtomicBool::new(false);
static HOOK_BEAT: AtomicU64 = AtomicU64::new(0);
// 探针仅在申请/持有光标租约时发送，相对零位移且由自己的钩子吞掉。
// 发送编号先于SendInput发布，回调只确认同编号，不比较发送/回声时间戳。
static HOOK_PROBE_SENT: AtomicU64 = AtomicU64::new(0);
static HOOK_PROBE_ECHO: AtomicU64 = AtomicU64::new(0);
static HOOK_CHECK_REQUEST: AtomicU64 = AtomicU64::new(0);
static HOOK_CHECK_OK: AtomicU64 = AtomicU64::new(0);
static HOOK_CHECK_FAILED: AtomicU64 = AtomicU64::new(0);
static HOOK_RETRY: AtomicU64 = AtomicU64::new(0);
static LAST_INPUT: AtomicU64 = AtomicU64::new(0);
static INPUT_EPOCH: AtomicU64 = AtomicU64::new(0);
static ACTIVE: AtomicU64 = AtomicU64::new(0);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static REASON: AtomicU8 = AtomicU8::new(0);
static START_HOOK: Once = Once::new();
static CLOCK: OnceLock<Instant> = OnceLock::new();
static LEASE: Mutex<Option<Lease>> = Mutex::new(None);

#[derive(Clone)]
struct Lease {
    id: u64,
    born: u64,
    updated: u64,
    origin: [i32; 2],
    origin_win: [i32; 2],
    target_win: [i32; 2],
    target: [i32; 2],
    expected: [i32; 2],
    sent_win: [i32; 2],
    area: [i32; 4],
    scale: f64,
    epoch: u64,
    has_update: bool,
    rope: bool,
    rope_floor: i32,
}
fn now() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}
fn stop(reason: u8) {
    REASON.store(reason, Ordering::SeqCst);
    ACTIVE.store(0, Ordering::SeqCst);
}
pub fn cancel() {
    stop(2);
}
pub fn set_busy(busy: bool) {
    WORK_BUSY.store(busy, Ordering::SeqCst);
    if busy {
        cancel();
    }
}
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst) != 0
}
pub fn available() -> bool {
    ENABLED.load(Ordering::SeqCst)
        && !WORK_BUSY.load(Ordering::SeqCst)
        && !super::extension_windows::cursor_blocked()
        && !super::DRAG_ACTIVE.load(Ordering::SeqCst)
        && HOOK_READY.load(Ordering::SeqCst)
        && !active()
        && now().saturating_sub(HOOK_BEAT.load(Ordering::SeqCst)) < 500
        && now().saturating_sub(LAST_INPUT.load(Ordering::SeqCst)) >= 10000
}
// 只暴露输入变化编号，不暴露按键内容；本程序标记的移动不计入用户回应。
pub fn input_revision() -> String {
    if ENABLED.load(Ordering::SeqCst) && HOOK_READY.load(Ordering::SeqCst)
        && now().saturating_sub(HOOK_BEAT.load(Ordering::SeqCst)) < 500 {
        return format!("hook:{}", INPUT_EPOCH.load(Ordering::SeqCst));
    }
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if unsafe { GetLastInputInfo(&mut info) } != 0 {
        format!("session:{}", info.dwTime)
    } else {
        "unavailable".into()
    }
}
fn external_input() {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    LAST_INPUT.store(now(), Ordering::SeqCst);
    INPUT_EPOCH.fetch_add(1, Ordering::SeqCst);
    stop(1);
}
unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = &*(l as *const MSLLHOOKSTRUCT);
        if event.flags & LLMHF_INJECTED != 0
            && event.dwExtraInfo & !PROBE_ID_MASK == probe_prefix()
        {
            let id = (event.dwExtraInfo & PROBE_ID_MASK) as u64;
            if id != 0 && id == HOOK_PROBE_SENT.load(Ordering::SeqCst) {
                HOOK_PROBE_ECHO.store(id, Ordering::SeqCst);
            }
            // 包括迟到探针：不当成真实输入，不继续投递到其他应用。
            return 1;
        }
        if event.flags & LLMHF_INJECTED == 0 || event.dwExtraInfo != marker() {
            external_input();
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, w, l)
}
unsafe extern "system" fn key_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        external_input();
    }
    CallNextHookEx(std::ptr::null_mut(), code, w, l)
}
fn install_hooks() -> Option<(HHOOK, HHOOK)> {
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0);
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), module, 0);
        if mouse.is_null() || keyboard.is_null() {
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
            if !keyboard.is_null() {
                UnhookWindowsHookEx(keyboard);
            }
            super::append_log("debug.log", "cursor-carry hooks-unavailable");
            return None;
        }
        HOOK_PROBE_SENT.store(0, Ordering::SeqCst);
        HOOK_PROBE_ECHO.store(0, Ordering::SeqCst);
        HOOK_READY.store(true, Ordering::SeqCst);
        Some((mouse, keyboard))
    }
}
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
    if !enabled {
        cancel();
        return;
    }
    // 每次重新启用都重置“10 秒无输入”闸门，而不是只在首次装钩子时
    LAST_INPUT.store(now(), Ordering::SeqCst);
    START_HOOK.call_once(|| {
        std::thread::spawn(|| unsafe {
            let mut hooks = install_hooks();
            let mut probe: Option<(u64, u64, u64)> = None; // 编号、发送时刻、申请编号
            let mut probe_seq = 0u64;
            let mut last_probe_at = 0u64;
            let mut misses = 0u8;
            let mut message: MSG = std::mem::zeroed();
            loop {
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if message.message == WM_QUIT {
                        HOOK_READY.store(false, Ordering::SeqCst);
                        cancel();
                        if let Some((mouse, keyboard)) = hooks {
                            UnhookWindowsHookEx(mouse);
                            UnhookWindowsHookEx(keyboard);
                        }
                        return;
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                HOOK_BEAT.store(now(), Ordering::SeqCst);
                if ENABLED.load(Ordering::SeqCst) {
                    if hooks.is_some() {
                        let at = now();
                        if let Some((id, sent_at, check)) = probe {
                            if HOOK_PROBE_ECHO.load(Ordering::SeqCst) == id {
                                HOOK_CHECK_OK.fetch_max(check, Ordering::SeqCst);
                                misses = 0;
                                probe = None;
                            } else if at.saturating_sub(sent_at) >= 500 {
                                misses += 1;
                                probe = None;
                            }
                        }
                        let requested = HOOK_CHECK_REQUEST.load(Ordering::SeqCst);
                        let waiting = requested > HOOK_CHECK_OK.load(Ordering::SeqCst)
                            && requested > HOOK_CHECK_FAILED.load(Ordering::SeqCst);
                        let mut send_failed = false;
                        if misses < 3 && probe.is_none() && (waiting || active() || misses > 0)
                            && (waiting || at.saturating_sub(last_probe_at) >= 500)
                        {
                            probe_seq = probe_seq.wrapping_add(1) & PROBE_ID_MASK as u64;
                            if probe_seq == 0 { probe_seq = 1; }
                            HOOK_PROBE_ECHO.store(0, Ordering::SeqCst);
                            HOOK_PROBE_SENT.store(probe_seq, Ordering::SeqCst);
                            if send_probe(probe_seq) {
                                probe = Some((probe_seq, at, requested));
                                last_probe_at = at;
                            } else {
                                send_failed = true;
                            }
                        }
                        if send_failed || misses >= 3 {
                            // 发送失败不冒充“钩子被摘除”；两者都先放弃控制，稍后重装。
                            super::append_log("debug.log", if send_failed {
                                "cursor-carry probe-send-failed, control paused"
                            } else { "cursor-carry probe-unconfirmed, reinstall pending" });
                            HOOK_CHECK_FAILED.fetch_max(requested, Ordering::SeqCst);
                            HOOK_PROBE_SENT.store(0, Ordering::SeqCst);
                            HOOK_READY.store(false, Ordering::SeqCst);
                            cancel();
                            if let Some((mouse, keyboard)) = hooks.take() {
                                UnhookWindowsHookEx(mouse);
                                UnhookWindowsHookEx(keyboard);
                            }
                            probe = None;
                            HOOK_RETRY.store(at, Ordering::SeqCst);
                        }
                    } else if now().saturating_sub(HOOK_RETRY.load(Ordering::SeqCst)) >= 5000 {
                        HOOK_RETRY.store(now(), Ordering::SeqCst);
                        hooks = install_hooks();
                        if hooks.is_some() {
                            probe = None;
                            misses = 0;
                            super::append_log("debug.log", "cursor-carry hooks-reinstalled");
                        }
                    }
                } else {
                    probe = None;
                    misses = 0;
                    HOOK_PROBE_SENT.store(0, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(8));
            }
        });
    });
}
async fn verify_hook() -> Result<(), String> {
    let check = HOOK_CHECK_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
    let started = Instant::now();
    loop {
        if !ENABLED.load(Ordering::SeqCst) || !HOOK_READY.load(Ordering::SeqCst)
            || HOOK_CHECK_FAILED.load(Ordering::SeqCst) >= check
        {
            HOOK_CHECK_FAILED.fetch_max(check, Ordering::SeqCst);
            return Err("光标监听暂不可确认，本次不接管光标".into());
        }
        if HOOK_CHECK_OK.load(Ordering::SeqCst) >= check { return Ok(()); }
        if started.elapsed() >= Duration::from_millis(300) {
            HOOK_CHECK_FAILED.fetch_max(check, Ordering::SeqCst);
            return Err("光标监听确认超时，本次不接管光标".into());
        }
        tokio::time::sleep(Duration::from_millis(8)).await;
    }
}
fn inside(point: [i32; 2], area: [i32; 4]) -> bool {
    point[0] >= area[0] + 8
        && point[0] < area[2] - 8
        && point[1] >= area[1] + 8
        && point[1] < area[3] - 8
}
fn distance(a: [i32; 2], b: [i32; 2]) -> f64 {
    (a[0] as f64 - b[0] as f64).hypot(a[1] as f64 - b[1] as f64)
}
#[tauri::command]
pub async fn carry_cursor_begin(win: tauri::WebviewWindow, point: [i32; 2], mode: Option<String>) -> Result<u64, String> {
    let rope = match mode.as_deref().unwrap_or("carry") {
        "carry" => false,
        "rope" => true,
        _ => return Err("未知光标交互模式".into()),
    };
    if win.label() != "main" || !available() || super::load_settings()["cursor_carry"] != true {
        return Err("抱跑未启用或用户尚未空闲".into());
    }
    let epoch = INPUT_EPOCH.load(Ordering::SeqCst);
    verify_hook().await?;
    if !available() || INPUT_EPOCH.load(Ordering::SeqCst) != epoch {
        return Err("用户已接管或光标交互已让位".into());
    }
    // 窗口/光标信息采集全部是阻塞 API，必须在拿 LEASE 锁之前完成，避免推迟 update
    unsafe {
        // 有任何键处于按下状态时，不取得控制权。
        if (1..=254).any(|key| GetAsyncKeyState(key) as u16 & 0x8000 != 0) {
            return Err("用户正在按键".into());
        }
    }
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) } == 0 {
        return Err("读取光标失败".into());
    }
    if super::extension_windows::control_blocked([cursor.x, cursor.y]) {
        return Err("正在操作扩展窗口".into());
    }
    let position = win.outer_position().map_err(|e| e.to_string())?;
    let monitor = win
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("缺少屏幕")?;
    let area = monitor.work_area();
    let area = [
        area.position.x,
        area.position.y,
        area.position.x + area.size.width as i32,
        area.position.y + area.size.height as i32,
    ];
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let origin = [cursor.x, cursor.y];
    if !inside(origin, area) || !inside(point, area) || distance(origin, point) > 64.0 * scale {
        return Err("光标不在手边".into());
    }
    let size = win.outer_size().map_err(|e| e.to_string())?;
    let rope_floor = position.y + (size.height as f64 * 0.56) as i32;
    if rope && (origin[0] < position.x + (16.0 * scale) as i32
        || origin[0] > position.x + size.width as i32 - (16.0 * scale) as i32
        || origin[1] >= rope_floor)
    {
        return Err("高处拉绳需先走到同屏光标下方".into());
    }
    let mut slot = LEASE.lock().map_err(|_| "抱跑锁不可用")?;
    if slot.is_some() {
        return Err("上次抱跑尚未释放".into());
    }
    let id = SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
    REASON.store(0, Ordering::SeqCst);
    ACTIVE.store(id, Ordering::SeqCst);
    if INPUT_EPOCH.load(Ordering::SeqCst) != epoch || !ENABLED.load(Ordering::SeqCst) {
        cancel();
        return Err("用户已接管".into());
    }
    *slot = Some(Lease {
        id,
        born: now(),
        updated: now(),
        origin,
        origin_win: [position.x, position.y],
        target_win: [position.x, position.y],
        sent_win: [position.x, position.y],
        target: origin,
        expected: origin,
        area,
        scale,
        epoch,
        has_update: false,
        rope,
        rope_floor,
    });
    drop(slot);
    super::append_log("debug.log", &format!("cursor-carry begin={id} mode={}", if rope { "rope" } else { "carry" }));
    std::thread::spawn(move || {
        // run 万一 panic，guard 兜底释放租约和 ACTIVE，否则抱跑会永久报“尚未释放”（fail-closed，不影响鼠标）
        struct Guard(u64, tauri::WebviewWindow);
        impl Drop for Guard {
            fn drop(&mut self) {
                let cleared = {
                    let mut slot = LEASE.lock().unwrap_or_else(|e| e.into_inner());
                    if slot.as_ref().is_some_and(|v| v.id == self.0) {
                        *slot = None;
                        true
                    } else {
                        false
                    }
                };
                if ACTIVE.load(Ordering::SeqCst) == self.0 {
                    ACTIVE.store(0, Ordering::SeqCst);
                }
                if cleared {
                    let reason = REASON.load(Ordering::SeqCst);
                    super::append_log(
                        "debug.log",
                        &format!("cursor-carry end={} reason={} (guard)", self.0, reason),
                    );
                    let _ = self
                        .1
                        .emit("cursor-carry-ended", serde_json::json!({ "id": self.0, "reason": reason }));
                }
            }
        }
        let _guard = Guard(id, win.clone());
        run(win, id);
    });
    Ok(id)
}
#[tauri::command]
pub fn carry_cursor_update(
    win: tauri::WebviewWindow,
    id: u64,
    window_pos: [i32; 2],
    cursor_pos: [i32; 2],
) -> Result<(), String> {
    if win.label() != "main" {
        return Err("非主窗口不得操作抱跑".into());
    }
    if super::extension_windows::control_blocked(cursor_pos) {
        cancel(); return Err("扩展窗口保护已释放光标".into());
    }
    if ACTIVE.load(Ordering::SeqCst) != id || !ENABLED.load(Ordering::SeqCst) {
        return Err("抱跑已释放".into());
    }
    let mut slot = LEASE.lock().map_err(|_| "抱跑锁不可用")?;
    let lease = slot.as_mut().filter(|v| v.id == id).ok_or("抱跑已失效")?;
    let outside_mode = if lease.rope {
        distance(window_pos, lease.origin_win) > 4.0 * lease.scale
            || (cursor_pos[0] - lease.origin[0]).abs() as f64 > 2.0 * lease.scale
            || cursor_pos[1] < lease.origin[1] - (2.0 * lease.scale) as i32
            || cursor_pos[1] > lease.origin[1] + (180.0 * lease.scale) as i32
            || cursor_pos[1] > lease.rope_floor
    } else {
        distance(window_pos, lease.origin_win) > 420.0 * lease.scale
            || distance(cursor_pos, lease.origin) > 520.0 * lease.scale
    };
    if !inside(cursor_pos, lease.area) || outside_mode
        || (window_pos[1] - lease.origin_win[1]).abs() > (2.0 * lease.scale) as i32
        || distance(cursor_pos, lease.expected) > 80.0 * lease.scale
    {
        stop(3);
        return Err("抱跑目标超出原型范围".into());
    }
    lease.target_win = window_pos;
    lease.target = cursor_pos;
    lease.updated = now();
    lease.has_update = true;
    Ok(())
}
#[tauri::command]
pub fn carry_cursor_end(win: tauri::WebviewWindow, id: u64) {
    if win.label() != "main" {
        return;
    }
    if ACTIVE.load(Ordering::SeqCst) == id {
        stop(2);
    }
}
fn run(win: tauri::WebviewWindow, id: u64) {
    loop {
        let snapshot = LEASE
            .lock()
            .ok()
            .and_then(|v| v.as_ref().filter(|v| v.id == id).cloned());
        let Some(lease) = snapshot else { break };
        if ACTIVE.load(Ordering::SeqCst) != id {
            break;
        }
        if super::extension_windows::control_blocked(lease.target) { cancel(); break; }
        if !ENABLED.load(Ordering::SeqCst) || INPUT_EPOCH.load(Ordering::SeqCst) != lease.epoch {
            stop(1);
            break;
        }
        if now() - lease.born > if lease.rope { 6000 } else { 5000 }
            || now() - lease.updated > 350
            || now() - HOOK_BEAT.load(Ordering::SeqCst) > 500
            || !HOOK_READY.load(Ordering::SeqCst)
        {
            stop(4);
            break;
        }
        if lease.has_update {
            let mut actual = POINT::default();
            if unsafe { GetCursorPos(&mut actual) } == 0
                || distance([actual.x, actual.y], lease.expected) > 16.0 * lease.scale
            {
                stop(1);
                break;
            }
            if super::extension_windows::control_blocked([actual.x, actual.y]) { cancel(); break; }
            if lease.target_win != lease.sent_win
                && win
                    .set_position(tauri::PhysicalPosition::new(
                        lease.target_win[0],
                        lease.target_win[1],
                    ))
                    .is_err()
            {
                stop(3);
                break;
            }
            let delta = distance(lease.expected, lease.target);
            let fraction = (12.0 * lease.scale / delta.max(1.0)).min(1.0);
            let next = [
                (lease.expected[0] as f64 + (lease.target[0] - lease.expected[0]) as f64 * fraction)
                    .round() as i32,
                (lease.expected[1] as f64 + (lease.target[1] - lease.expected[1]) as f64 * fraction)
                    .round() as i32,
            ];
            if ACTIVE.load(Ordering::SeqCst) != id
                || INPUT_EPOCH.load(Ordering::SeqCst) != lease.epoch
            {
                break;
            }
            if super::extension_windows::control_blocked(next) { cancel(); break; }
            if next != lease.expected && !send_move(next) {
                stop(5);
                break;
            }
            if let Ok(mut slot) = LEASE.lock() {
                if let Some(v) = slot.as_mut().filter(|v| v.id == id) {
                    v.expected = next;
                    v.sent_win = lease.target_win;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    if let Ok(mut slot) = LEASE.lock() {
        if slot.as_ref().is_some_and(|v| v.id == id) {
            *slot = None;
        }
    }
    if ACTIVE.load(Ordering::SeqCst) == id {
        ACTIVE.store(0, Ordering::SeqCst);
    }
    let reason = REASON.load(Ordering::SeqCst);
    super::append_log(
        "debug.log",
        &format!("cursor-carry end={id} reason={reason}"),
    );
    let _ = win.emit(
        "cursor-carry-ended",
        serde_json::json!({ "id": id, "reason": reason }),
    );
}
fn send_probe(id: u64) -> bool {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                // 相对零位移：不采样位置，不使用ABSOLUTE，不回写旧坐标。
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: probe_prefix() | (id as usize & PROBE_ID_MASK),
            },
        },
    };
    unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1 }
}
fn send_move(point: [i32; 2]) -> bool {
    unsafe {
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if width <= 1 || height <= 1 {
            return false;
        }
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: (((point[0] - left) as f64 * 65535.0 / (width - 1) as f64).round() as i32)
                        .clamp(0, 65535),
                    dy: (((point[1] - top) as f64 * 65535.0 / (height - 1) as f64).round() as i32)
                        .clamp(0, 65535),
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: marker(),
                },
            },
        };
        SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1
    }
}
