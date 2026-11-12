#!/usr/bin/env python3
"""月之少女桌宠 · Kimi Code hook 适配器

Kimi Code 通过 config.toml 的 [[hooks]] 调用本脚本：
    py hook.py <event_name>
事件 payload 从 stdin 读入 JSON，合并进 state.json（与引擎共享的状态文件）。

约束（见实现文档 §2.1 / §10）：
- 轻量：<300ms，不做任何网络/重 IO
- 幂等：同一事件重复触发结果一致
- 容错：任何异常都不允许以非零退出码回传给 CLI（fail-open 之上的双保险）
- 原子写：临时文件 + os.replace
"""

from contextlib import contextmanager
import json
import os
import sys
import time
from pathlib import Path

STATE_PATH = Path(__file__).resolve().parent.parent / "state.json"
HEARTBEAT_EVENT = "SessionHeartbeat"

EVENT_ALIASES = {
    "session_start": "SessionStart",
    "session_end": "SessionEnd",
    "turn_started": "TurnStarted",
    "stop": "Stop",
    "interrupt": "Interrupt",
    "task_started": "TaskStarted",
    "subagent_start": "SubagentStart",
    "subagent_stop": "SubagentStop",
    "heartbeat": "SessionHeartbeat",
}


def now_ms() -> int:
    return int(time.time() * 1000)


def load_state() -> dict:
    try:
        value = json.loads(STATE_PATH.read_text(encoding="utf-8"))
        return value if isinstance(value, dict) else {}
    except Exception:
        return {}


@contextmanager
def state_lock():
    # 锁覆盖读取、合并、替换，避免心跳与工具hook并发丢事件。
    # 最多等待200ms，失败仍由main容错，不阻塞CLI。
    with STATE_PATH.with_suffix(".lock").open("a+b") as lock:
        lock.seek(0, os.SEEK_END)
        if lock.tell() == 0:
            lock.write(b"0")
            lock.flush()
        deadline = time.monotonic() + .2
        while True:
            try:
                lock.seek(0)
                if os.name == "nt":
                    import msvcrt
                    msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except OSError:
                if time.monotonic() >= deadline:
                    raise TimeoutError("状态锁等待超时")
                time.sleep(.01)
        try:
            yield
        finally:
            lock.seek(0)
            if os.name == "nt":
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)


def save_state(state: dict) -> None:
    tmp = STATE_PATH.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(state, ensure_ascii=False, indent=1), encoding="utf-8")
    os.replace(tmp, STATE_PATH)


def apply_event(state: dict, event: str, payload: dict) -> dict:
    ts = now_ms()
    state["version"] = 1
    state["updated_at_ms"] = ts
    state["last_event"] = {"name": event, "at_ms": ts}

    if payload.get("session_id"):
        state["session_id"] = payload["session_id"]
    if payload.get("cwd"):
        state["cwd"] = payload["cwd"]

    if event == "SessionStart":
        state["model"] = payload.get("model")
        state["profile"] = payload.get("profile")
        state["session_alive"] = True
        state["turn_active"] = False
        state["running_tasks"] = 0
        state["subagents"] = 0
        state["fail_count_turn"] = 0
    elif event == "SessionEnd":
        state["session_alive"] = False
        state["turn_active"] = False
        state["running_tasks"] = 0
        state["subagents"] = 0
    elif event == "TurnStarted":
        state["turn_active"] = True
        state["fail_count_turn"] = 0  # 失败计数按 turn 重置（文档 §6）
        state["turn_origin"] = payload.get("origin_kind")
        prompt = payload.get("prompt")
        if isinstance(prompt, str):
            state["last_prompt"] = prompt[:200]
    elif event == "Stop":
        state["turn_active"] = False
    elif event == "Interrupt":
        state["turn_active"] = False
        state["interrupt_reason"] = payload.get("reason")
    elif event == "TaskStarted":
        state["running_tasks"] = int(state.get("running_tasks") or 0) + 1
    elif event == "SubagentStart":
        state["subagents"] = int(state.get("subagents") or 0) + 1
    elif event == "SubagentStop":
        state["subagents"] = max(0, int(state.get("subagents") or 0) - 1)
    elif event == HEARTBEAT_EVENT:
        state["heartbeat_at_ms"] = ts
        state["uptime_ms"] = payload.get("uptime_ms")
        state["session_alive"] = True

    return state


def main() -> int:
    try:
        alias = sys.argv[1] if len(sys.argv) > 1 else ""
        event = EVENT_ALIASES.get(alias, alias)
        raw = sys.stdin.read()
        try:
            payload = json.loads(raw) if raw.strip() else {}
        except Exception:
            payload = {}
        if not isinstance(payload, dict):
            payload = {}

        with state_lock():
            state = apply_event(load_state(), event, payload)
            save_state(state)
    except Exception:
        pass  # 绝不把异常回传给 CLI
    return 0


if __name__ == "__main__":
    sys.exit(main())
