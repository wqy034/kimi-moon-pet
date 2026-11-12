// state.json 读取：hook.py 写入的共享状态文件。
// 引擎只读；读取失败一律容错返回默认（文档 §10）。

use serde_json::Value;
use std::path::PathBuf;

pub fn pet_home() -> PathBuf {
    #[cfg(debug_assertions)]
    { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..") }
    #[cfg(not(debug_assertions))]
    {
        // 便携版：与当前工作目录无关，素材和可写配置放在exe旁边。
        std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub struct HookState {
    pub last_event: String,
    pub last_event_at_ms: i64,
    pub heartbeat_at_ms: Option<i64>,
    pub session_alive: bool,
    pub read_ok: bool,
}

impl HookState {
    pub fn empty() -> Self {
        Self {
            last_event: String::new(),
            last_event_at_ms: 0,
            heartbeat_at_ms: None,
            session_alive: false,
            read_ok: false,
        }
    }

    pub fn heartbeat_age_s(&self, now_ms: i64) -> Option<i64> {
        self.heartbeat_at_ms.map(|t| (now_ms - t) / 1000)
    }
}

pub fn read() -> HookState {
    let path = pet_home().join("state.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return HookState::empty();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return HookState::empty();
    };
    if !v.is_object() { return HookState::empty(); }
    let st = HookState {
        last_event: v
            .get("last_event")
            .and_then(|e| e.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        last_event_at_ms: v
            .get("last_event")
            .and_then(|e| e.get("at_ms"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        heartbeat_at_ms: v.get("heartbeat_at_ms").and_then(Value::as_i64),
        session_alive: v
            .get("session_alive")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        read_ok: true,
    };
    st
}

/// 从 config/状态映射.json 读心跳超时（秒），读不到用 90。
pub fn heartbeat_timeout_s() -> i64 {
    let path = pet_home().join("config").join("状态映射.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("heartbeat_timeout_seconds").and_then(Value::as_i64))
        .unwrap_or(90)
}
