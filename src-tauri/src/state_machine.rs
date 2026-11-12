// 通用状态机（实现文档 §3）：输入 hooks/wire 信号，输出通用状态。
// 规则按 状态映射.json 的顺序语义求值，事件态带 afterglow 回落。

use crate::adapters::state_file::HookState;
use crate::adapters::wire_tailer::WireSignals;

pub const IDLE: &str = "idle";
pub const STREAMING: &str = "streaming";
pub const THINKING: &str = "thinking";
pub const WORKING: &str = "working";
pub const WRITING: &str = "writing";
pub const SEARCHING: &str = "searching";
pub const REVIEWING: &str = "reviewing";
pub const WAITING: &str = "waiting";
pub const FAILED: &str = "failed";
pub const ANGRY: &str = "angry";
pub const CELEBRATING: &str = "celebrating";
pub const GREETING: &str = "greeting";
pub const SLEEPING: &str = "sleeping";

/// 事件态（transient）：某事件触发后占据状态机直到 afterglow 到期
pub struct Transient {
    pub state: &'static str,
    pub until_ms: i64,
}

/// 事件 → 瞬态。返回 None 表示该事件不产生瞬态。
pub fn event_transient(event: &str, now: i64) -> Option<Transient> {
    let (state, afterglow_ms) = match event {
        "Stop" => (CELEBRATING, 2500),
        "Interrupt" => (ANGRY, 2500),
        "TurnStarted" => (GREETING, 2000),
        _ => return None,
    };
    Some(Transient { state, until_ms: now + afterglow_ms })
}

/// 持续态求值：顺序即优先级（对应 状态映射.json 的"先命中先生效"）。
pub fn continuous_state(now: i64, hook: &HookState, wire: &mut WireSignals, heartbeat_timeout_s: i64) -> &'static str {
    if !hook.session_alive { return SLEEPING; }
    let hb_age_s = hook.heartbeat_age_s(now);
    match hb_age_s {
        // 从未收到心跳 / 心跳超时 → sleeping（CLI 未运行或已退出）
        None => return SLEEPING,
        Some(age) if age >= heartbeat_timeout_s => return SLEEPING,
        _ => {}
    }
    if wire.approval_pending {
        return WAITING;
    }
    if wire.plan_mode {
        return REVIEWING;
    }
    // 工具正在跑：按工具类别细分（常用指令 → 动作对号入座）
    match wire.tool_category() {
        Some("edit") => return WRITING,
        Some("search") => return SEARCHING,
        Some("read") => return REVIEWING,
        Some(_) => return WORKING,
        None => {}
    }
    if wire.reasoning_active(now) {
        return THINKING;
    }
    if wire.tool_frequency_hz(now) > 0.5 {
        return WORKING;
    }
    if wire.streaming_active(now) {
        return STREAMING;
    }
    IDLE
}
