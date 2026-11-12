//! 聊天专用数据库；不接触冻结的养成容器。DELETE journal保证正常关闭后可单文件搬迁。
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
pub const SCHEMA_VERSION: i64 = 6;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成聊天请求编号")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn db_error(_: rusqlite::Error) -> String {
    "聊天数据库读写失败，请退出后备份数据库并查看维护说明；原文件未清空".into()
}

#[derive(Serialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at: u64,
}
#[derive(Serialize)]
pub struct Message {
    pub id: i64,
    pub request_id: String,
    pub role: String,
    pub source: String,
    pub text: String,
    pub status: String,
    pub created_at: u64,
    pub provider: String,
    pub usage: Option<Value>,
    pub error: Option<String>,
    pub attempts: Vec<Attempt>,
}
#[derive(Serialize)]
pub struct Attempt {
    pub connection_name: String,
    pub model: String,
    pub status: String,
    pub usage: Option<Value>,
    pub error: Option<String>,
    pub created_at: u64,
}
#[derive(Serialize)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub has_older: bool,
}
pub struct Store {
    pub(crate) db: Connection,
    _lease: std::fs::File,
}
impl Store {
    pub fn open() -> Result<Self, String> {
        crate::chat_persona::policy()?;
        use std::os::windows::fs::OpenOptionsExt;
        let lease = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(crate::adapters::state_file::pet_home().join("聊天记录.lock"))
            .map_err(|_| "聊天记录被其他实例占用或无法加锁，请关闭多余桌宠后重启")?;
        crate::chat_restore::apply(&crate::adapters::state_file::pet_home())?;
        let mut db =
            Connection::open(crate::adapters::state_file::pet_home().join("聊天记录.sqlite3"))
                .map_err(db_error)?;
        db.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(db_error)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if version > SCHEMA_VERSION {
            return Err("聊天数据库版本高于当前程序，请使用新版程序；原文件未改写".into());
        }
        // 不在未知版本数据库上执行journal切换或迁移。
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;").map_err(db_error)?;
        if version > 0 && version < SCHEMA_VERSION {
            crate::chat_backup::snapshot(&db, version, "升级前交流记忆")
                .map_err(|e| format!("升级前备份未完成：{e}；数据库未迁移"))?;
        }
        upgrade_schema(&mut db,version)?;
        db.execute("CREATE INDEX IF NOT EXISTS messages_request_lookup ON messages(conversation_id,request_id,role,id)", []).map_err(db_error)?;
        let mut store = Self { db, _lease: lease };
        store.memory_screen_existing()?;
        store.recover_interrupted()?;
        Ok(store)
    }
    pub(crate) fn recover_interrupted(&self) -> Result<(), String> {
        self
            .db
            .execute(
                "UPDATE memory_jobs SET status='queued' WHERE status='running'",
                [],
            )
            .map_err(db_error)?;
        self
            .db
            .execute(
                "UPDATE memory_attempts SET status='interrupted' WHERE status='running'",
                [],
            )
            .map_err(db_error)?;
        self.db.execute("UPDATE messages SET status='interrupted',error='上次运行结束时回复尚未完成' WHERE status='streaming'", []).map_err(db_error)?;
        self.db.execute("UPDATE chat_attempts SET status='interrupted',error='上次运行结束时请求尚未完成' WHERE status='streaming'",[]).map_err(db_error)?;
        Ok(())
    }
    pub fn config(&self, key: &str) -> Result<Option<String>, String> {
        self.db
            .query_row("SELECT value FROM config WHERE key=?", [key], |r| r.get(0))
            .optional()
            .map_err(db_error)
    }
    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.db.execute("INSERT INTO config VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(db_error)?;
        Ok(())
    }
    pub fn current(&self) -> Result<String, String> {
        if let Some(current) = self.config("current")? {
            if self
                .db
                .query_row("SELECT 1 FROM conversations WHERE id=?", [&current], |_| {
                    Ok(())
                })
                .optional()
                .map_err(db_error)?
                .is_some()
            {
                return Ok(current);
            }
            return Err("聊天当前对话引用损坏，原数据保留".into());
        }
        self.new_conversation()
    }
    pub fn new_conversation(&self) -> Result<String, String> {
        let current = id()?;
        self.db
            .execute(
                "INSERT INTO conversations VALUES(?1,'新的对话',?2)",
                params![current, now()],
            )
            .map_err(db_error)?;
        self.set("current", &current)?;
        Ok(current)
    }
    pub fn conversations(&self) -> Result<Vec<Conversation>, String> {
        self.db
            .prepare(
                "SELECT id,title,created_at FROM conversations ORDER BY created_at DESC,rowid DESC",
            )
            .map_err(db_error)?
            .query_map([], |r| {
                Ok(Conversation {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn last_message(&self, current: &str) -> Result<Option<Message>, String> {
        self.db.query_row("SELECT id,request_id,role,source,text,status,created_at,provider,usage,error FROM messages WHERE conversation_id=?1 ORDER BY id DESC LIMIT 1",[current],|r| {
            let usage: Option<String> = r.get(8)?;
            Ok(Message{id:r.get(0)?,request_id:r.get(1)?,role:r.get(2)?,source:r.get(3)?,text:r.get(4)?,status:r.get(5)?,created_at:r.get(6)?,provider:r.get(7)?,usage:usage.and_then(|v|serde_json::from_str(&v).ok()),error:r.get(9)?,attempts:vec![]})
        }).optional().map_err(db_error)
    }
    pub fn user_text(&self, current: &str, request: &str) -> Result<Option<String>, String> {
        self.db.query_row("SELECT text FROM messages WHERE conversation_id=?1 AND request_id=?2 AND role='user' ORDER BY id LIMIT 1",params![current,request],|r|r.get(0)).optional().map_err(db_error)
    }
    pub fn history_page(&self, before: Option<i64>) -> Result<HistoryPage, String> {
        let before = before.unwrap_or(i64::MAX);
        let mut messages:Vec<Message>=self.db.prepare("SELECT id,request_id,role,source,text,status,created_at,provider,usage,error FROM messages WHERE id<?1 ORDER BY id DESC LIMIT 101").map_err(db_error)?
            .query_map([before], |r| {
                let usage: Option<String> = r.get(8)?;
                Ok(Message{id:r.get(0)?,request_id:r.get(1)?,role:r.get(2)?,source:r.get(3)?,text:r.get(4)?,status:r.get(5)?,created_at:r.get(6)?,provider:r.get(7)?,usage:usage.and_then(|v|serde_json::from_str(&v).ok()),error:r.get(9)?,attempts:vec![]})
            }).map_err(db_error)?.collect::<Result<Vec<_>,_>>().map_err(db_error)?;
        let has_older = messages.len() > 100;
        if has_older { messages.pop(); }
        messages.reverse();
        let mut attempts = self.attempts_before(before)?;
        for message in &mut messages {
            if message.role == "assistant" {
                message.attempts = attempts.remove(&message.request_id).unwrap_or_default();
            }
        }
        Ok(HistoryPage { messages, has_older })
    }
    pub fn attempt_start(
        &self,
        id: &str,
        request: &str,
        profile: &crate::chat_profiles::Profile,
    ) -> Result<(), String> {
        self.db.execute("INSERT INTO chat_attempts(id,request_id,connection_id,connection_name,model,status,created_at) VALUES(?1,?2,?3,?4,?5,'streaming',?6)",params![id,request,profile.id,profile.name,profile.model,now()]).map_err(db_error)?;
        Ok(())
    }
    pub fn attempt_finish(
        &self,
        id: &str,
        text: &str,
        status: &str,
        usage: &Option<Value>,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.db.execute("UPDATE chat_attempts SET text=?2,status=?3,usage=?4,error=?5 WHERE id=?1 AND status='streaming'",params![id,text,status,usage.as_ref().map(Value::to_string),error]).map_err(db_error)?;
        Ok(())
    }
    pub fn message_provider(&self, id: i64, provider: &str) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE messages SET provider=?2 WHERE id=?1",
                params![id, provider],
            )
            .map_err(db_error)?;
        Ok(())
    }
    fn attempts_before(&self, before: i64) -> Result<std::collections::HashMap<String, Vec<Attempt>>, String> {
        let mut map = std::collections::HashMap::<String, Vec<Attempt>>::new();
        let mut stmt=self.db.prepare("SELECT request_id,connection_name,model,status,usage,error,created_at FROM chat_attempts WHERE request_id IN (SELECT request_id FROM messages WHERE id<?1 ORDER BY id DESC LIMIT 101) ORDER BY rowid").map_err(db_error)?;
        let rows = stmt
            .query_map([before], |r| {
                let usage: Option<String> = r.get(4)?;
                Ok((
                    r.get::<_, String>(0)?,
                    Attempt {
                        connection_name: r.get(1)?,
                        model: r.get(2)?,
                        status: r.get(3)?,
                        usage: usage.and_then(|s| serde_json::from_str(&s).ok()),
                        error: r.get(5)?,
                        created_at: r.get(6)?,
                    },
                ))
            })
            .map_err(db_error)?;
        for row in rows {
            let (id, attempt) = row.map_err(db_error)?;
            map.entry(id).or_default().push(attempt);
        }
        Ok(map)
    }
    pub fn context(&self, current: &str) -> Result<Vec<Value>, String> {
        let mut statement = self.db.prepare("SELECT request_id,text FROM messages WHERE conversation_id=?1 AND role='assistant' AND status='complete' AND source='user' ORDER BY id DESC").map_err(db_error)?;
        let candidates = statement.query_map([current], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(db_error)?;
        let mut pairs = Vec::new();
        let mut size = 0;
        let policy = crate::chat_persona::policy()?;
        for candidate in candidates {
            let (request, assistant) = candidate.map_err(db_error)?;
            let Some(user) = self.user_text(current, &request)? else {
                continue;
            };
            if policy.memory_rule(&user).is_some() || policy.output_rule(&assistant).is_some() {
                continue;
            }
            let chars = user.chars().count() + assistant.chars().count();
            if pairs.len() == 12 || size + chars > 12000 {
                break;
            }
            size += chars;
            pairs.push((user, assistant));
        }
        Ok(pairs
            .into_iter()
            .rev()
            .flat_map(|(u, a)| {
                [
                    json!({"role":"user","content":u}),
                    json!({"role":"assistant","content":a}),
                ]
            })
            .collect())
    }
    pub fn start(
        &mut self,
        conversation: &str,
        request: &str,
        source: &str,
        input: &str,
        provider: &str,
    ) -> Result<i64, String> {
        let tx = self.db.transaction().map_err(db_error)?;
        let role = if source == "user" { "user" } else { "event" };
        tx.execute("INSERT INTO messages(conversation_id,request_id,role,source,text,status,created_at,provider) VALUES(?1,?2,?3,?4,?5,'complete',?6,?7)",params![conversation,request,role,source,input,now(),provider]).map_err(db_error)?;
        if source == "user" {
            tx.execute(
                "UPDATE conversations SET title=?2 WHERE id=?1 AND title='新的对话'",
                params![conversation, input.chars().take(24).collect::<String>()],
            )
            .map_err(db_error)?;
        }
        tx.execute("INSERT INTO messages(conversation_id,request_id,role,source,text,status,created_at,provider) VALUES(?1,?2,'assistant',?3,'','streaming',?4,?5)",params![conversation,request,source,now(),provider]).map_err(db_error)?;
        let message = tx.last_insert_rowid();
        if source == "life" {
            tx.execute(
                "INSERT INTO auto_requests VALUES(?1,?2)",
                params![request, now()],
            )
            .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(message)
    }
    pub fn finish(
        &self,
        message: i64,
        text: &str,
        status: &str,
        usage: &Option<Value>,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE messages SET text=?2,status=?3,usage=?4,error=?5 WHERE id=?1",
                params![
                    message,
                    text,
                    status,
                    usage.as_ref().map(Value::to_string),
                    error
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn discard_input_config(&self) -> Result<(), String> {
        self.db
            .execute("DELETE FROM config WHERE key='input_component_v1'", [])
            .map_err(db_error)?;
        Ok(())
    }
    pub fn clear(&mut self) -> Result<String, String> {
        let current = id()?;
        let tx = self.db.transaction().map_err(db_error)?;
        // 输入组件不再持久化文字，移除旧版遗留项，不解析或恢复旧草稿。
        tx.execute("DELETE FROM config WHERE key='input_component_v1'", [])
            .map_err(db_error)?;
        tx.execute_batch("DELETE FROM memory_entries; DELETE FROM memory_attempts; DELETE FROM memory_jobs; DELETE FROM memory_sources; DELETE FROM memory_tombstones; DELETE FROM config WHERE key IN ('memory_forget_cutoff','memory_legacy_message_cutoff'); DELETE FROM chat_attempts; DELETE FROM messages; DELETE FROM conversations;
            UPDATE config SET value='0' WHERE key='memory_cursor';
            UPDATE config SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT) WHERE key='memory_epoch';")
            .map_err(db_error)?;
        tx.execute(
            "INSERT INTO conversations VALUES(?1,'新的对话',?2)",
            params![current, now()],
        )
        .map_err(db_error)?;
        tx.execute("INSERT INTO config VALUES('current',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&current]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(current)
    }
    pub fn accept_event(&mut self, revision: u64, text: &str) -> Result<bool, String> {
        let enabled = self.memory_enabled()?;
        let tx = self.db.transaction().map_err(db_error)?;
        let key = format!("life:{revision}");
        let accepted = tx
            .execute(
                "INSERT OR IGNORE INTO life_events VALUES(?1,?2)",
                params![key, now()],
            )
            .map_err(db_error)?
            == 1;
        if accepted && enabled {
            tx.execute("INSERT OR IGNORE INTO memory_sources(source_key,kind,user_text,reply,created_at,captured_epoch) VALUES(?1,'life',?2,'',?3,(SELECT CAST(value AS INTEGER) FROM config WHERE key='memory_epoch'))",params![key,text,now()]).map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(accepted)
    }
    pub fn event_budget(&self) -> Result<bool, String> {
        let (count, last): (u64, u64) = self
            .db
            .query_row(
                "SELECT count(*),coalesce(max(created_at),0) FROM auto_requests WHERE created_at>?",
                [now().saturating_sub(3600)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_error)?;
        Ok(count < 12 && now().saturating_sub(last) >= 60)
    }
}

/// 仅变更已知旧版结构；调用者在文件数据库上必须先完成升级前备份。
pub(crate) fn upgrade_schema(db: &mut Connection, version: i64) -> Result<(), String> {
    if !(0..=SCHEMA_VERSION).contains(&version) { return Err("聊天数据库版本不受支持，原文件保留".into()); }
        if version == 0 {
            let tx = db.transaction().map_err(db_error)?;
            tx.execute_batch("CREATE TABLE conversations(id TEXT PRIMARY KEY,title TEXT NOT NULL,created_at INTEGER NOT NULL);
                CREATE TABLE messages(id INTEGER PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id),request_id TEXT NOT NULL,role TEXT NOT NULL,source TEXT NOT NULL,text TEXT NOT NULL,status TEXT NOT NULL,created_at INTEGER NOT NULL,provider TEXT NOT NULL,usage TEXT,error TEXT);
                CREATE INDEX messages_conversation ON messages(conversation_id,id);
                CREATE TABLE config(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                CREATE TABLE life_events(event_id TEXT PRIMARY KEY,created_at INTEGER NOT NULL);
                CREATE TABLE auto_requests(request_id TEXT PRIMARY KEY,created_at INTEGER NOT NULL);
                PRAGMA user_version=1;").map_err(db_error)?;
            tx.commit().map_err(db_error)?;
        }
        if version < 2 {
            crate::memory_store::migrate(db)?;
        }
        if version < 3 {
            let tx = db.transaction().map_err(db_error)?;
            tx.execute_batch("CREATE TABLE chat_attempts(id TEXT PRIMARY KEY,request_id TEXT NOT NULL,connection_id TEXT NOT NULL,connection_name TEXT NOT NULL,model TEXT NOT NULL,status TEXT NOT NULL,text TEXT NOT NULL DEFAULT '',usage TEXT,error TEXT,created_at INTEGER NOT NULL);
                CREATE INDEX chat_attempt_request ON chat_attempts(request_id,created_at);
                ALTER TABLE memory_jobs ADD COLUMN route_id TEXT;
                UPDATE memory_jobs SET route_id=provider;
                PRAGMA user_version=3;").map_err(db_error)?;
            tx.commit().map_err(db_error)?;
        }
        if version < 4 {
            let tx = db.transaction().map_err(db_error)?;
            tx.execute_batch("ALTER TABLE memory_entries ADD COLUMN quarantined INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE memory_entries ADD COLUMN guard_rule TEXT;
                DROP INDEX memory_fact_topic;
                CREATE UNIQUE INDEX memory_fact_topic ON memory_entries(topic) WHERE kind='fact' AND quarantined=0;
                PRAGMA user_version=4;").map_err(db_error)?;
            tx.commit().map_err(db_error)?;
        }
        if version < 5 {
            let epoch: String = db.query_row("SELECT value FROM config WHERE key='memory_epoch'", [], |r| r.get(0))
                .map_err(|_| "记忆代次缺失，已保留原数据库与升级前备份")?;
            let epoch: i64 = epoch.parse().map_err(|_| "记忆代次损坏，已保留原数据库与升级前备份")?;
            if epoch < 0 || epoch == i64::MAX { return Err("记忆代次超出范围，已保留原数据库与升级前备份".into()); }
            let tx = db.transaction().map_err(db_error)?;
            tx.execute_batch("ALTER TABLE memory_sources ADD COLUMN captured_epoch INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE memory_tombstones ADD COLUMN cutoff_epoch INTEGER NOT NULL DEFAULT 0;
                UPDATE memory_tombstones SET cutoff_epoch=(SELECT CAST(value AS INTEGER) FROM config WHERE key='memory_epoch');
                INSERT INTO config(key,value) SELECT 'memory_legacy_message_cutoff',CAST(coalesce(max(id),0) AS TEXT) FROM messages WHERE 1
                    ON CONFLICT(key) DO UPDATE SET value=excluded.value;
                INSERT INTO config(key,value) SELECT 'memory_forget_cutoff',value FROM config
                    WHERE key='memory_epoch' AND EXISTS(SELECT 1 FROM memory_tombstones)
                    ON CONFLICT(key) DO UPDATE SET value=excluded.value;
                UPDATE config SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT) WHERE key='memory_epoch';
                PRAGMA user_version=5;").map_err(db_error)?;
            tx.commit().map_err(db_error)?;
        }
        if version < 6 { crate::memory_store::migrate_v6(db)?; }
    Ok(())
}

#[cfg(test)]
impl Store {
    pub(crate) fn test_database(db: Connection) -> Self {
        Self {db,_lease:std::fs::File::open(std::env::current_exe().unwrap()).unwrap()}
    }
}
