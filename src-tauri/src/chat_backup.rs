//! 用户主动创建交流数据库快照；不读取凭据，不覆盖现有文件。
use std::{fs, sync::Mutex};
use tauri::Manager;

#[tauri::command]
pub async fn backup_chat(window: tauri::WebviewWindow) -> Result<String, String> {
    crate::chat::own(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || save(&app))
        .await.map_err(|_| "交流备份任务中断".to_owned())?
}

fn save(app: &tauri::AppHandle) -> Result<String, String> {
    let state = app.state::<Mutex<crate::chat::Service>>();
    let service = state.lock().map_err(|_| "聊天服务不可用")?;
    let store = service.store()?;
    snapshot(&store.db, crate::chat_store::SCHEMA_VERSION, "交流记忆")
}

/// 供手动备份与迁移前备份共用；仅使用调用者已经独占的SQLite连接。
pub(crate) fn snapshot(db: &rusqlite::Connection, expected_version: i64, prefix: &str) -> Result<String, String> {
    let dir = crate::adapters::state_file::pet_home().join("交流备份");
    fs::create_dir_all(&dir).map_err(|_| "无法创建交流备份目录")?;
    let name = format!("{prefix}-{}-{}.sqlite3", crate::chat_store::now(), crate::chat_store::id()?);
    let path = dir.join(&name);
    // 先独占创建空文件占位；SQLite的VACUUM INTO允许目标为空文件。
    fs::OpenOptions::new().write(true).create_new(true).open(&path)
        .map_err(|_| "无法创建交流备份文件")?;
    let result = (|| {
        let filename = path.to_str().ok_or("备份路径无效")?;
        // SQLite按一致性快照生成备份，不复制正在使用的数据库文件。
        db.execute("VACUUM INTO ?1", [filename]).map_err(|_| "创建交流备份失败，原记录未改动")?;
        let backup = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "无法检查交流备份")?;
        let check: String = backup.query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|_| "交流备份完整性检查失败")?;
        if check != "ok" { return Err("交流备份完整性检查失败".into()); }
        let version: i64 = backup.query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| "无法检查交流备份版本")?;
        if version != expected_version { return Err("交流备份版本不符合原数据库".into()); }
        Ok(())
    })();
    if let Err(error) = result {
        // 路径由本次独占创建，失败时只移除这一份未完成的备份。
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok(name)
}
