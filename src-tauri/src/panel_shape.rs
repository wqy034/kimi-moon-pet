//! 扩展窗口透明留白：仅使用各窗口自身提交的可见面板和缩放手柄矩形。
use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::{CreateRectRgn, CombineRgn, DeleteObject, SetWindowRgn, RGN_OR}, UI::WindowsAndMessaging::GetClientRect};

#[tauri::command]
pub async fn panel_shape(win: tauri::WebviewWindow, viewport: [f64; 2], rects: Vec<[f64; 4]>) -> Result<(), String> {
    if !matches!(win.label(), "bubble" | "pet-menu" | "input-bubble" | "extension-life" | "extension-music" | "extension-chat") {
        return Err("当前窗口不能设置面板区域".into());
    }
    if viewport.iter().any(|v| !v.is_finite() || *v < 1. || *v > 10000.) || rects.is_empty() || rects.len() > 16 || rects.iter().flatten().any(|v| !v.is_finite() || v.abs() > 20000.) {
        return Err("面板区域参数无效".into());
    }
    let (tx,rx) = tokio::sync::oneshot::channel();
    let own=win.clone();
    win.run_on_main_thread(move || {
        let result=(|| {
            let hwnd=own.hwnd().map_err(|_| "窗口不可用")?.0;
            let mut client=RECT::default();
            if unsafe { GetClientRect(hwnd,&mut client) } == 0 || client.right <= 0 || client.bottom <= 0 { return Err("读取窗口尺寸失败"); }
            let sx=client.right as f64/viewport[0]; let sy=client.bottom as f64/viewport[1];
            let total=unsafe { CreateRectRgn(0,0,0,0) };
            if total.is_null() { return Err("创建窗口区域失败"); }
            let mut good=false;
            for [x1,y1,x2,y2] in rects {
                let x1=(x1*sx).floor().clamp(0.,client.right as f64) as i32;
                let y1=(y1*sy).floor().clamp(0.,client.bottom as f64) as i32;
                let x2=(x2*sx).ceil().clamp(0.,client.right as f64) as i32;
                let y2=(y2*sy).ceil().clamp(0.,client.bottom as f64) as i32;
                if x2<=x1 || y2<=y1 { continue; }
                let item=unsafe { CreateRectRgn(x1,y1,x2,y2) };
                if item.is_null() { unsafe { DeleteObject(total) }; return Err("创建面板区域失败"); }
                let combined=unsafe { CombineRgn(total,total,item,RGN_OR) };
                unsafe { DeleteObject(item) };
                if combined==0 { unsafe { DeleteObject(total) }; return Err("合并面板区域失败"); }
                good=true;
            }
            if !good { unsafe { DeleteObject(total) }; return Err("面板区域为空"); }
            if unsafe { SetWindowRgn(hwnd,total,1) } == 0 { unsafe { DeleteObject(total) }; return Err("应用面板区域失败"); }
            Ok(())
        })();
        if result.is_err() {
            // 失败时撤销旧区域，避免窗口尺寸变化后继续裁切控件。
            if let Ok(hwnd)=own.hwnd() { unsafe { SetWindowRgn(hwnd.0,std::ptr::null_mut(),1) }; }
        }
        let _=tx.send(result.map_err(str::to_owned));
    }).map_err(|_| "窗口区域队列不可用")?;
    rx.await.map_err(|_| "窗口区域任务中断")?
}
