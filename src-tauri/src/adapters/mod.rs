// 适配层主循环：每 500ms 汇总 hooks 状态 + wire 信号 → 通用状态 → 推给前端。

pub mod state_file;
pub mod wire_tailer;

use crate::state_machine;
use serde_json::json;
use tauri::{AppHandle, Emitter};

pub fn run_loop(handle: AppHandle) {
    let mut tailer = wire_tailer::WireTailer::new();
    let mut transient: Option<state_machine::Transient> = None;
    let mut seen_event_at: i64 = 0;
    let mut last_hook = state_file::HookState::empty();

    loop {
        let now = now_ms();
        // 读取失败（文件替换瞬间的共享锁等）时沿用上一份完好的状态，杜绝单拍抖动成 sleeping
        let fresh = state_file::read();
        let fresh_read_ok = fresh.read_ok;
        if fresh.read_ok {
            last_hook = fresh;
        }
        let hook = &last_hook;
        let hb_timeout = state_file::heartbeat_timeout_s();

        // 新 hook 事件 → 瞬态（celebrating / greeting / failed）
        if hook.last_event_at_ms > seen_event_at {
            seen_event_at = hook.last_event_at_ms;
            // 重启时只消费新鲜事件，不能把磁盘里的旧Stop/Interrupt重新演一遍。
            if (0..=3000).contains(&(now - hook.last_event_at_ms)) {
                if ["Stop", "Interrupt", "SessionEnd"].contains(&hook.last_event.as_str()) {
                    tailer.end_turn();
                }
                if let Some(t) = state_machine::event_transient(&hook.last_event, now) {
                    transient = Some(t);
                }
            }
        }
        // 先消费hook收尾，再读wire增量；同拍到达的新回合工具不能被旧Stop清掉。
        tailer.poll(now);
        let wire = &mut tailer.signals;
        // wire 自己维护失败截止时间，不在每拍重复延长。
        let state = if wire.failed_active(now) {
            state_machine::FAILED
        } else { match &transient {
            Some(t) if now < t.until_ms => t.state,
            _ => {
                transient = None;
                state_machine::continuous_state(now, hook, wire, hb_timeout)
            }
        }};

        crate::cursor_carry::set_busy(state != state_machine::IDLE && state != state_machine::SLEEPING);
        crate::life::set_cli_busy(state != state_machine::IDLE && state != state_machine::SLEEPING);
        let payload = json!({
            "state": state,
            "diagnostics": {
                "heartbeat_age_s": hook.heartbeat_age_s(now),
                "heartbeat_timeout_s": hb_timeout,
                "hook_read_ok": fresh_read_ok,
                "last_event": hook.last_event,
                "tool_category": wire.tool_category(),
            },
            "tps": wire.tps(now),
            "fail_count": wire.fail_count,
            "bubble": wire.bubble(now).map(|(id, text)| json!({ "id": id, "text": text })),
        });
        let _ = handle.emit("pet-state", payload);

        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
