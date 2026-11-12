//! 主窗口原生轮廓：空白处不属于窗口，不依赖光标轮询或合成点击。
//! 先扩展至旧/新两帧的并集，前端呈现后再收紧；不更改鼠标捕获或显隐。
use std::sync::Mutex;
use tauri::Manager;
use windows_sys::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{
        ClientToScreen, CombineRgn, CreateRectRgn, DeleteObject, ExtCreateRegion, SetWindowRgn,
        RDH_RECTANGLES, RGNDATA, RGNDATAHEADER, RGN_OR, XFORM,
    },
    UI::WindowsAndMessaging::{GetClientRect, GetWindowRect},
};

type Rect = [u16; 4];
const WIDTH: u16 = 320;
const HEIGHT: u16 = 360;
const MAX_RECTS: usize = 10000;

#[derive(Default)]
struct Shape {
    session: u64,
    serial: u64,
    current: Vec<Rect>,
    previous: Vec<Rect>,
    preparing: bool,
    geometry: Option<[i32; 4]>,
}

pub fn install(app: &tauri::AppHandle) {
    app.manage(Mutex::new(Shape::default()));
}

// 仅此主窗口可提交轮廓。所有Win32调用在窗口所属线程执行；锁不跨窗口API。
async fn on_main<T: Send + 'static>(
    win: tauri::WebviewWindow,
    op: impl FnOnce(&tauri::WebviewWindow) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    if win.label() != "main" {
        return Err("只有本体窗口可以更新人物轮廓".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let own = win.clone();
    win.run_on_main_thread(move || {
        let _ = tx.send(op(&own));
    })
    .map_err(|_| "人物轮廓队列不可用")?;
    rx.await.map_err(|_| "人物轮廓任务中断")?
}

#[tauri::command]
pub async fn pet_shape_begin(win: tauri::WebviewWindow) -> Result<u64, String> {
    on_main(win, |win| {
        let state = win.state::<Mutex<Shape>>();
        let mut state = state.lock().map_err(|_| "人物轮廓状态不可用")?;
        state.session += 1;
        state.serial = 0;
        // 页面重载后仍保留旧区域，直到首帧扩展/呈现；旧页面请求不能改新区域。
        state.geometry = None;
        Ok(state.session)
    })
    .await
}

fn validate(rects: &[Rect]) -> Result<(), String> {
    if rects.is_empty() || rects.len() > MAX_RECTS {
        return Err("人物轮廓区域数量无效".into());
    }
    let mut last: Option<Rect> = None;
    for &[left, top, right, bottom] in rects {
        if left >= right || top >= bottom || right > WIDTH || bottom > HEIGHT {
            return Err("人物轮廓坐标无效".into());
        }
        // RGNDATA必须按横向带排序，带内不重叠，带间不重叠。
        if let Some([_, y, x2, y2]) = last {
            if top == y {
                if bottom != y2 || left < x2 {
                    return Err("人物轮廓横带无效".into());
                }
            } else if top < y2 {
                return Err("人物轮廓排序无效".into());
            }
        }
        last = Some([left, top, right, bottom]);
    }
    Ok(())
}

#[tauri::command]
pub async fn pet_shape_prepare(
    win: tauri::WebviewWindow,
    session: u64,
    serial: u64,
    rects: Vec<Rect>,
) -> Result<bool, String> {
    validate(&rects)?;
    on_main(win, move |win| {
        let state = win.state::<Mutex<Shape>>();
        {
            let mut s = state.lock().map_err(|_| "人物轮廓状态不可用")?;
            if s.session != session || serial <= s.serial {
                return Ok(false);
            }
            s.serial = serial;
            s.previous = std::mem::replace(&mut s.current, rects);
            s.preparing = true;
            s.geometry = None;
        }
        refresh(win)?;
        Ok(true)
    })
    .await
}

#[tauri::command]
pub async fn pet_shape_commit(
    win: tauri::WebviewWindow,
    session: u64,
    serial: u64,
) -> Result<bool, String> {
    on_main(win, move |win| {
        let state = win.state::<Mutex<Shape>>();
        {
            let mut s = state.lock().map_err(|_| "人物轮廓状态不可用")?;
            if s.session != session || serial != s.serial {
                return Ok(false);
            }
            s.preparing = false;
            s.previous.clear();
            s.geometry = None;
        }
        refresh(win)?;
        Ok(true)
    })
    .await
}

#[tauri::command]
pub async fn pet_shape_reset(win: tauri::WebviewWindow, session: u64) -> Result<bool, String> {
    on_main(win, move |win| {
        let state = win.state::<Mutex<Shape>>();
        {
            let mut s = state.lock().map_err(|_| "人物轮廓状态不可用")?;
            if s.session != session { return Ok(false); }
            s.current.clear(); s.previous.clear(); s.geometry = None;
        }
        let hwnd = win.hwnd().map_err(|_| "人物窗口不可用")?.0;
        if unsafe { SetWindowRgn(hwnd, std::ptr::null_mut(), 1) } == 0 {
            return Err("恢复人物窗口区域失败".into());
        }
        Ok(true)
    }).await
}

// 成功转交SetWindowRgn后不再拥有句柄；失败和临时区域由RAII回收。
struct Region(windows_sys::Win32::Graphics::Gdi::HRGN);
impl Drop for Region {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteObject(self.0) };
        }
    }
}

fn region(rects: &[Rect], geometry: [i32; 4]) -> Result<Region, String> {
    if rects.is_empty() {
        let r = Region(unsafe { CreateRectRgn(0, 0, 0, 0) });
        return if r.0.is_null() { Err("创建空轮廓失败".into()) } else { Ok(r) };
    }
    // Vec<u32>保证RGNDATA/RECT所需的4字节对齐，避免把Vec<u8>强转为结构体。
    let bytes = std::mem::size_of::<RGNDATAHEADER>() + rects.len() * std::mem::size_of::<RECT>();
    let mut data = vec![0u32; bytes / 4];
    let ptr = data.as_mut_ptr().cast::<RGNDATA>();
    unsafe {
        (*ptr).rdh = RGNDATAHEADER {
            dwSize: std::mem::size_of::<RGNDATAHEADER>() as u32,
            iType: RDH_RECTANGLES,
            nCount: rects.len() as u32,
            nRgnSize: (rects.len() * std::mem::size_of::<RECT>()) as u32,
            rcBound: RECT { left: 0, top: 0, right: WIDTH.into(), bottom: HEIGHT.into() },
        };
        let out = data.as_mut_ptr().add(std::mem::size_of::<RGNDATAHEADER>() / 4).cast::<RECT>();
        for (i, &[left, top, right, bottom]) in rects.iter().enumerate() {
            out.add(i).write(RECT { left: left.into(), top: top.into(), right: right.into(), bottom: bottom.into() });
        }
    }
    let transform = XFORM {
        eM11: geometry[0] as f32 / WIDTH as f32,
        eM12: 0., eM21: 0.,
        eM22: geometry[1] as f32 / HEIGHT as f32,
        eDx: geometry[2] as f32, eDy: geometry[3] as f32,
    };
    let r = Region(unsafe { ExtCreateRegion(&transform, bytes as u32, ptr) });
    if r.0.is_null() { Err("创建人物轮廓失败".into()) } else { Ok(r) }
}

fn refresh(win: &tauri::WebviewWindow) -> Result<(), String> {
    let hwnd = win.hwnd().map_err(|_| "人物窗口不可用")?.0;
    let mut client = RECT::default();
    let mut outer = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        if GetClientRect(hwnd, &mut client) == 0 || GetWindowRect(hwnd, &mut outer) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
            return Err("读取人物窗口几何失败".into());
        }
    }
    let geometry = [client.right, client.bottom, origin.x - outer.left, origin.y - outer.top];
    if geometry[0] <= 0 || geometry[1] <= 0 { return Ok(()); }
    let state = win.state::<Mutex<Shape>>();
    let (current, previous) = {
        let mut s = state.lock().map_err(|_| "人物轮廓状态不可用")?;
        if s.current.is_empty() || s.geometry == Some(geometry) { return Ok(()); }
        // 先记尺寸防止SetWindowRgn的窗口消息重入递归；窗口API前释放锁。
        s.geometry = Some(geometry);
        (s.current.clone(), if s.preparing { s.previous.clone() } else { vec![] })
    };
    let result = (|| {
        let mut next = region(&current, geometry)?;
        if !previous.is_empty() {
            let old = region(&previous, geometry)?;
            if unsafe { CombineRgn(next.0, next.0, old.0, RGN_OR) } == 0 {
                return Err("合并人物轮廓失败".into());
            }
        }
        if unsafe { SetWindowRgn(hwnd, next.0, 1) } == 0 {
            return Err("应用人物轮廓失败".into());
        }
        next.0 = std::ptr::null_mut();
        Ok(())
    })();
    if result.is_err() {
        state.lock().map_err(|_| "人物轮廓状态不可用")?.geometry = None;
    }
    result
}

pub fn window_event(win: &tauri::Window, event: &tauri::WindowEvent) {
    if win.label() != "main" || !matches!(event, tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }) {
        return;
    }
    // 不在窗口事件内同步重设区域，避免原生窗口事件重入。
    if let Some(webview) = win.app_handle().get_webview_window("main") {
        let own = webview.clone();
        let _ = webview.run_on_main_thread(move || {
            if refresh(&own).is_err() {
                crate::append_log("debug.log", "pet-shape stage=resize-failed");
            }
        });
    }
}
