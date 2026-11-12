//! 导出固定字段的诊断摘要；不复制原始日志或用户数据。
use std::io::{Read, Seek, SeekFrom, Write};

#[tauri::command]
pub async fn export_diagnostics(win: tauri::WebviewWindow) -> Result<String, String> {
    if win.label() != "extension-chat" { return Err("请从交流设置导出诊断摘要".into()); }
    tauri::async_runtime::spawn_blocking(export).await.map_err(|_| "诊断任务中断".to_owned())?
}

fn export() -> Result<String, String> {
    const SAMPLE: u64 = 256 * 1024;
    let home = crate::adapters::state_file::pet_home();
    let mut counts = [0u64; 8];
    let mut read_files = 0;
    let mut failed_files = 0;
    // 仅统计固定事件类别和结果，不导出任何日志原文、坐标、路径或编号。
    for name in ["debug.log", "debug.log.1", "debug.log.2"] {
        let path = home.join(name);
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => { failed_files += 1; continue; }
        };
        let bytes = (|| -> std::io::Result<Vec<u8>> {
            let size = file.metadata()?.len();
            file.seek(SeekFrom::Start(size.saturating_sub(SAMPLE)))?;
            let mut bytes = Vec::new();
            file.take(SAMPLE).read_to_end(&mut bytes)?;
            Ok(bytes)
        })();
        let Ok(bytes) = bytes else { failed_files += 1; continue };
        read_files += 1;
        for line in String::from_utf8_lossy(&bytes).lines() {
            let Some((timestamp, event)) = line.split_once(' ') else { continue };
            if !timestamp.bytes().all(|c| c.is_ascii_digit()) { continue; }
            for (i, prefix) in ["pet-shape ", "panel-shape ", "input-ui ", "input-native ", "extension-", "chat-guard ", "cursor-carry ", "life-lock-"] .iter().enumerate() {
                if event.starts_with(prefix) { counts[i] += 1; break; }
            }
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_| "系统时间无效")?.as_millis();
    let body = format!("Moon·月诊断摘要\n版本：{}\n系统：{} / {}\n生成时间（Unix毫秒）：{}\n日志采样：{}份，每份最多末尾256 KiB；读取失败{}份\n事件数量（仅采样范围）：\n人物轮廓：{}\n面板轮廓：{}\n输入界面：{}\n输入窗口：{}\n扩展窗口：{}\n交流规则：{}\n光标互动：{}\n养成存储锁：{}\n\n摘要仅含固定类别计数，不含日志正文、路径、坐标、聊天内容、凭据或个人存档。\n", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, now, read_files, failed_files, counts[0],counts[1],counts[2],counts[3],counts[4],counts[5],counts[6],counts[7]);
    for i in 0..100 {
        let name=format!("诊断摘要-{now}-{i}.txt");
        match std::fs::OpenOptions::new().write(true).create_new(true).open(home.join(&name)) {
            Ok(mut f) => { f.write_all(body.as_bytes()).map_err(|_| "写入诊断摘要失败")?; return Ok(name); }
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("无法创建诊断摘要，请检查程序目录是否可写".into()),
        }
    }
    Err("无法分配诊断文件名".into())
}
