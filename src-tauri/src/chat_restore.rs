//! 交流备份的延迟恢复。只在持有数据库独占租约的启动阶段替换文件。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}, sync::{Mutex, OnceLock}};

const PLAN: &str = "交流恢复计划.json";
const PLAN_TMP: &str = "交流恢复计划.json.tmp";
const STAGED: &str = "交流恢复待用.sqlite3";
const DB: &str = "聊天记录.sqlite3";
const LIMIT: u64 = 512 * 1024 * 1024;
static RESTORE_LOCK: Mutex<()> = Mutex::new(());
static RESTORED_FILE: OnceLock<String> = OnceLock::new();

#[derive(Serialize)]
pub struct BackupInfo { name: String, bytes: u64, modified: u64 }
#[derive(Serialize)]
pub struct RestoreView { backups: Vec<BackupInfo>, pending: bool, orphan: bool, restored: Option<String> }
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan { version: u32, id: String, sha256: String }

fn regular(path: &Path) -> Result<fs::Metadata, String> {
    let meta=fs::symlink_metadata(path).map_err(|_| "无法读取备份文件")?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len()==0 || meta.len()>LIMIT {
        return Err("备份文件类型或大小不受支持".into());
    }
    Ok(meta)
}
fn verify(path: &Path) -> Result<(), String> {
    regular(path)?;
    let db=rusqlite::Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "无法读取备份数据库")?;
    let version:i64=db.query_row("PRAGMA user_version",[],|r|r.get(0)).map_err(|_| "无法读取备份版本")?;
    if !(1..=crate::chat_store::SCHEMA_VERSION).contains(&version) { return Err("备份数据库版本不受支持".into()); }
    let check:String=db.query_row("PRAGMA integrity_check",[],|r|r.get(0)).map_err(|_| "备份完整性检查失败")?;
    if check!="ok" { return Err("备份完整性检查失败".into()); }
    // 只读验证必需表，避免将结构不完整的SQLite文件当成聊天备份。
    let mut queries=vec![
        "SELECT key,value FROM config LIMIT 0",
        "SELECT id,title,created_at FROM conversations LIMIT 0",
        "SELECT id,conversation_id,request_id,role,source,text,status,created_at,provider,usage,error FROM messages LIMIT 0",
        "SELECT event_id,created_at FROM life_events LIMIT 0",
        "SELECT request_id,created_at FROM auto_requests LIMIT 0",
    ];
    if version>=2 { queries.extend([
        "SELECT id,source_key,kind,user_text,reply,created_at,job_id FROM memory_sources LIMIT 0",
        "SELECT id,epoch,status,attempts,error,provider,usage,created_at FROM memory_jobs LIMIT 0",
        "SELECT id,job_id,status,provider,usage,error,created_at FROM memory_attempts LIMIT 0",
        "SELECT id,kind,topic,text,source_ids,evidence,created_at,updated_at,edited,embedding FROM memory_entries LIMIT 0",
        "SELECT topic,text FROM memory_tombstones LIMIT 0",
    ]); }
    if version>=3 { queries.extend([
        "SELECT id,request_id,connection_id,connection_name,model,status,text,usage,error,created_at FROM chat_attempts LIMIT 0",
        "SELECT route_id FROM memory_jobs LIMIT 0",
    ]); }
    if version>=4 { queries.push("SELECT quarantined,guard_rule FROM memory_entries LIMIT 0"); }
    if version>=5 { queries.extend(["SELECT captured_epoch FROM memory_sources LIMIT 0", "SELECT cutoff_epoch FROM memory_tombstones LIMIT 0"]); }
    if version>=6 { queries.extend(["SELECT blocked FROM memory_sources LIMIT 0", "SELECT archived,archive_reason FROM memory_entries LIMIT 0", "SELECT result FROM memory_jobs LIMIT 0"]); }
    for query in queries { db.prepare(query).map_err(|_| "备份缺少必要的数据结构")?; }
    let mut fk=db.prepare("PRAGMA foreign_key_check").map_err(|_| "备份关联检查失败")?;
    if fk.query([]).map_err(|_| "备份关联检查失败")?.next().map_err(|_| "备份关联检查失败")?.is_some() {
        return Err("备份包含无效的数据关联".into());
    }
    Ok(())
}
fn hash(path: &Path) -> Result<String, String> {
    regular(path)?;
    let mut file=fs::File::open(path).map_err(|_| "无法读取恢复文件")?;
    let mut hash=Sha256::new();let mut buf=[0u8;65536];
    loop { let n=file.read(&mut buf).map_err(|_| "读取恢复文件失败")?;if n==0 {break;}hash.update(&buf[..n]); }
    Ok(format!("{:x}",hash.finalize()))
}
fn read_plan(home: &Path) -> Result<Option<Plan>, String> {
    let path=home.join(PLAN);
    let meta=match fs::symlink_metadata(&path) {
        Ok(m)=>m, Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(None),
        Err(_)=>return Err("无法读取恢复计划".into()),
    };
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len()>4096 { return Err("恢复计划文件无效".into()); }
    let text=fs::read(&path).map_err(|_| "无法读取恢复计划")?;
    let plan:Plan=serde_json::from_slice(&text).map_err(|_| "恢复计划格式无效")?;
    if plan.version!=1 || plan.id.len()!=32 || !plan.id.bytes().all(|b|b.is_ascii_hexdigit()) || plan.sha256.len()!=64 || !plan.sha256.bytes().all(|b|b.is_ascii_hexdigit()) {
        return Err("恢复计划内容无效".into());
    }
    Ok(Some(plan))
}
fn before(home:&Path,plan:&Plan)->PathBuf {home.join("交流备份").join(format!("恢复前交流记忆-{}.sqlite3",plan.id))}

fn view() -> Result<RestoreView,String> {
    let home=crate::adapters::state_file::pet_home();
    let pending=read_plan(&home)?.is_some();
    let orphan=!pending && (home.join(STAGED).exists() || home.join(PLAN_TMP).exists());
    let mut backups=Vec::new();
    match fs::read_dir(home.join("交流备份")) {
        Ok(entries)=>for entry in entries {
            let entry=entry.map_err(|_| "无法读取备份目录")?;
            let name=entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("交流记忆-") || name.starts_with("升级前交流记忆-") || name.starts_with("恢复前交流记忆-")) || !name.ends_with(".sqlite3") {continue;}
            if let Ok(meta)=fs::symlink_metadata(entry.path()) {
                if meta.file_type().is_file() && !meta.file_type().is_symlink() && meta.len()>0 {
                    let modified=meta.modified().ok().and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d|d.as_secs()).unwrap_or(0);
                    backups.push(BackupInfo{name,bytes:meta.len(),modified});
                }
            }
        },
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
        Err(_)=>return Err("无法读取备份目录".into()),
    }
    backups.sort_by(|a,b| b.modified.cmp(&a.modified).then_with(||b.name.cmp(&a.name)));
    Ok(RestoreView {backups,pending,orphan,restored:RESTORED_FILE.get().cloned()})
}
#[tauri::command]
pub async fn chat_restore_list(window:tauri::WebviewWindow)->Result<RestoreView,String> {
    crate::chat::own(&window)?;
    tauri::async_runtime::spawn_blocking(view).await.map_err(|_| "读取备份列表失败")?
}
#[tauri::command]
pub async fn chat_restore_schedule(window:tauri::WebviewWindow,name:String)->Result<(),String> {
    crate::chat::own(&window)?;
    tauri::async_runtime::spawn_blocking(move||schedule(&name)).await.map_err(|_| "准备恢复任务中断")?
}
fn schedule(name:&str)->Result<(),String> {
    let _guard=RESTORE_LOCK.lock().map_err(|_| "恢复服务不可用")?;
    let home=crate::adapters::state_file::pet_home();
    if read_plan(&home)?.is_some() || home.join(STAGED).exists() || home.join(PLAN_TMP).exists() {return Err("已有待恢复文件，请先取消或完成恢复".into())}
    let directory=home.join("交流备份");
    if !view()?.backups.iter().any(|b| b.name==name) {return Err("备份不存在，请重新选择".into());}
    let source=directory.join(name);
    regular(&source)?;
    let real_dir=fs::canonicalize(&directory).map_err(|_| "无法读取备份目录")?;
    let real_source=fs::canonicalize(&source).map_err(|_| "无法读取备份文件")?;
    if real_source.parent()!=Some(real_dir.as_path()) { return Err("备份路径无效".into()); }
    let staged=home.join(STAGED);
    let mut out=fs::OpenOptions::new().write(true).create_new(true).open(&staged).map_err(|_| "无法创建待恢复文件")?;
    let result=(|| {
        let source=fs::File::open(real_source).map_err(|_| "无法读取备份文件")?;
        let copied=std::io::copy(&mut source.take(LIMIT+1),&mut out).map_err(|_| "复制备份失败")?;
        if copied>LIMIT {return Err("备份文件过大".into());}
        out.sync_all().map_err(|_| "保存待恢复文件失败")?;
        drop(out);
        verify(&staged)?;
        let plan=Plan{version:1,id:crate::chat_store::id()?,sha256:hash(&staged)?};
        let data=serde_json::to_vec(&plan).map_err(|_| "无法生成恢复计划")?;
        let plan_path=home.join(PLAN);let temp_path=home.join(PLAN_TMP);
        let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&temp_path).map_err(|_| "无法保存恢复计划")?;
        if file.write_all(&data).and_then(|_|file.sync_all()).is_err() {
            drop(file);let _=fs::remove_file(&temp_path);return Err("无法写入恢复计划".into());
        }
        drop(file);
        if fs::rename(&temp_path,&plan_path).is_err() {let _=fs::remove_file(&temp_path);return Err("无法提交恢复计划".into());}
        Ok(())
    })();
    if result.is_err() {let _=fs::remove_file(&staged);}
    result
}
#[tauri::command]
pub async fn chat_restore_cancel(window:tauri::WebviewWindow)->Result<(),String> {
    crate::chat::own(&window)?;
    tauri::async_runtime::spawn_blocking(cancel).await.map_err(|_| "取消恢复任务中断")?
}
fn cancel()->Result<(),String> {
    let _guard=RESTORE_LOCK.lock().map_err(|_| "恢复服务不可用")?;
    let home=crate::adapters::state_file::pet_home();
    if let Some(plan)=read_plan(&home)? {
        if before(&home,&plan).exists() {return Err("恢复已经开始，请退出并重新启动以完成恢复".into());}
        fs::remove_file(home.join(PLAN)).map_err(|_| "取消恢复失败")?;
    }
    let staged=home.join(STAGED);
    if staged.exists() {fs::remove_file(staged).map_err(|_| "清理待恢复文件失败")?;}
    let temp=home.join(PLAN_TMP);
    if temp.exists() {fs::remove_file(temp).map_err(|_| "清理临时恢复计划失败")?;}
    Ok(())
}

/// 由Store::open在已取得独占数据库租约后、打开SQLite之前调用。
pub(crate) fn apply(home:&Path)->Result<(),String> {
    let _guard=RESTORE_LOCK.lock().map_err(|_| "恢复服务不可用")?;
    let Some(plan)=read_plan(home)? else {return Ok(())};
    let staged=home.join(STAGED);let current=home.join(DB);let previous=before(home,&plan);
    if staged.exists() {
        verify(&staged)?;
        if hash(&staged)?!=plan.sha256 {return Err("待恢复备份校验不符，原数据保留".into());}
        if !previous.exists() {
            if !current.exists() {return Err("原交流数据库不存在，已停止恢复".into())}
            let current_meta=fs::symlink_metadata(&current).map_err(|_| "无法检查当前交流数据库")?;
            if !current_meta.file_type().is_file() || current_meta.file_type().is_symlink() {
                return Err("当前交流数据库不是普通文件，已停止恢复".into());
            }
            for suffix in ["-journal","-wal","-shm"] {
                if home.join(format!("{DB}{suffix}")).exists() {return Err("发现未清理的数据库事务文件，已停止恢复；请保留所有文件检查".into());}
            }
            fs::rename(&current,&previous).map_err(|_| "无法保留恢复前数据库")?;
        } else if current.exists() {
            return Err("恢复状态存在冲突，原文件已保留".into())
        }
        if fs::rename(&staged,&current).is_err() {
            if !current.exists() {let _=fs::rename(&previous,&current);}
            return Err("应用备份失败，已保留恢复前文件".into())
        }
    } else if !previous.exists() || !current.exists() || hash(&current)?!=plan.sha256 {
        return Err("恢复状态不完整，原文件已保留".into())
    }
    // 若上次在替换后中断，哈希确认目标是预定备份，再清理计划。
    if hash(&current)?!=plan.sha256 {return Err("恢复文件校验不符，原文件已保留".into())}
    fs::remove_file(home.join(PLAN)).map_err(|_| "无法完成恢复计划，请保留文件并重启")?;
    if let Some(name)=previous.file_name().and_then(|n|n.to_str()) { let _=RESTORED_FILE.set(name.to_owned()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(path: &Path, marker: &str) {
        let db = rusqlite::Connection::open(path).unwrap();
        db.execute_batch("CREATE TABLE config(key TEXT, value TEXT);
            CREATE TABLE conversations(id TEXT, title TEXT, created_at INTEGER);
            CREATE TABLE messages(id INTEGER, conversation_id TEXT, request_id TEXT, role TEXT, source TEXT, text TEXT, status TEXT, created_at INTEGER, provider TEXT, usage TEXT, error TEXT);
            CREATE TABLE life_events(event_id TEXT, created_at INTEGER);
            CREATE TABLE auto_requests(request_id TEXT, created_at INTEGER);
            PRAGMA user_version=1;").unwrap();
        db.execute("INSERT INTO config VALUES ('marker', ?1)", [marker]).unwrap();
    }

    fn marker(path: &Path) -> String {
        let db = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        db.query_row("SELECT value FROM config WHERE key='marker'", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn schema6_backup_requires_lifecycle_columns() {
        let path = std::env::temp_dir().join(format!("moon-backup-fixture-{}.sqlite3", crate::chat_store::id().unwrap()));
        {
            let mut db = rusqlite::Connection::open(&path).unwrap();
            crate::chat_store::upgrade_schema(&mut db, 0).unwrap();
        }
        verify(&path).unwrap();
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("ALTER TABLE memory_jobs DROP COLUMN result").unwrap();
        }
        assert!(verify(&path).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn restore_recovers_interrupted_steps_and_preserves_original() {
        for scenario in ["normal", "after_original_moved", "after_replacement", "bad_hash", "sidecar"] {
            let home = std::env::temp_dir().join(format!("moon-restore-test-{}", crate::chat_store::id().unwrap()));
            fs::create_dir_all(home.join("交流备份")).unwrap();
            let current = home.join(DB);
            let staged = home.join(STAGED);
            database(&current, "original");
            database(&staged, "backup");
            let mut plan = Plan { version: 1, id: crate::chat_store::id().unwrap(), sha256: hash(&staged).unwrap() };
            let previous = before(&home, &plan);
            if scenario == "bad_hash" { plan.sha256 = "0".repeat(64); }
            fs::write(home.join(PLAN), serde_json::to_vec(&plan).unwrap()).unwrap();
            match scenario {
                "after_original_moved" => fs::rename(&current, &previous).unwrap(),
                "after_replacement" => { fs::rename(&current, &previous).unwrap(); fs::rename(&staged, &current).unwrap(); },
                "sidecar" => fs::write(home.join(format!("{DB}-wal")), b"test").unwrap(),
                _ => {}
            }
            let result = apply(&home);
            if scenario == "bad_hash" || scenario == "sidecar" {
                assert!(result.is_err(), "{scenario}");
                if scenario == "sidecar" { fs::remove_file(home.join(format!("{DB}-wal"))).unwrap(); }
                assert_eq!(marker(&current), "original");
                assert!(staged.exists());
                assert!(home.join(PLAN).exists());
            } else {
                result.unwrap();
                assert_eq!(marker(&current), "backup");
                assert_eq!(marker(&previous), "original");
                assert!(!home.join(PLAN).exists());
            }
            fs::remove_dir_all(&home).unwrap();
        }
    }
}
