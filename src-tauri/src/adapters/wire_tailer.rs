// wire.jsonl 只读 tail（实现文档 §2.2）。
// 跟踪最近活跃会话的 agents/main/wire.jsonl，提取流式/推理/工具/审批信号。
// 该文件格式无官方稳定性承诺：逐行容错解析，任何字段缺失都只丢弃该条，绝不 panic。

use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

const STREAMING_WINDOW_MS: i64 = 2000;
const REASONING_WINDOW_MS: i64 = 3000;
const FAILED_AFTERGLOW_MS: i64 = 3000;
const FREQ_WINDOW_MS: i64 = 4000;
const TPS_WINDOW_MS: i64 = 2000;
const TOOL_BUBBLE_MS: i64 = 4000;

#[derive(Default)]
pub struct WireSignals {
    delta_until_ms: i64,
    reasoning_until_ms: i64,
    tool_times: Vec<i64>,
    text_chunks: Vec<(i64, usize)>,
    failed_until_ms: i64,
    step_open: bool,
    /// 正在执行的工具类别（tool.call 设，tool.result/新回合清）
    tool_running: Option<&'static str>,
    pub plan_mode: bool,
    pub approval_pending: bool,
    pub fail_count: u32,
    bubble: Option<(u64, String)>,
    bubble_tool_id: Option<String>,
    bubble_until_ms: i64,
    bubble_seq: u64,
    pub parse_ok: bool,
}

impl WireSignals {
    /// 流式输出中：wire 只在 step 边界记录事件，所以用 step.begin/end 判定"模型正在生成"
    pub fn streaming_active(&self, now: i64) -> bool {
        self.step_open || now < self.delta_until_ms
    }
    pub fn reasoning_active(&self, now: i64) -> bool {
        now < self.reasoning_until_ms
    }
    pub fn failed_active(&self, now: i64) -> bool {
        now < self.failed_until_ms
    }
    /// 当前正在执行的工具类别：bash / edit / read / search / agent / other
    pub fn tool_category(&self) -> Option<&'static str> {
        self.tool_running
    }
    pub fn tool_frequency_hz(&mut self, now: i64) -> f64 {
        self.tool_times.retain(|&t| now - t < FREQ_WINDOW_MS);
        self.tool_times.len() as f64 / (FREQ_WINDOW_MS as f64 / 1000.0)
    }
    /// 粗略 TPS：最近窗口内输出字符数 / 2（约等于 token 数）/ 秒
    pub fn tps(&mut self, now: i64) -> f64 {
        self.text_chunks.retain(|(t, _)| now - *t < TPS_WINDOW_MS);
        let chars: usize = self.text_chunks.iter().map(|(_, n)| n).sum();
        chars as f64 / 2.0 / (TPS_WINDOW_MS as f64 / 1000.0)
    }
    pub fn bubble(&self, now: i64) -> Option<&(u64, String)> {
        self.bubble.as_ref().filter(|_| now < self.bubble_until_ms)
    }
    fn set_bubble(&mut self, text: String, tool_id: Option<String>, until_ms: i64) {
        self.bubble_seq += 1;
        self.bubble = Some((self.bubble_seq, text));
        self.bubble_tool_id = tool_id;
        self.bubble_until_ms = until_ms;
    }
    fn finish_tool_bubble(&mut self, tool_id: &str) {
        // 并行工具的较早结果不能撤掉后一个工具的提示；失败提示不归属在途工具。
        if self.bubble_tool_id.as_deref() == Some(tool_id) {
            self.bubble = None;
            self.bubble_tool_id = None;
        }
    }
    fn end_turn(&mut self) {
        self.tool_running = None;
        self.step_open = false;
        self.delta_until_ms = 0;
        self.reasoning_until_ms = 0;
        self.tool_times.clear();
        self.text_chunks.clear();
        self.approval_pending = false;
        self.plan_mode = false;
        if self.bubble_tool_id.take().is_some() {
            self.bubble = None;
        }
    }
}

pub struct WireTailer {
    path: Option<PathBuf>,
    offset: u64,
    stale_checks: u32,
    pending: Vec<u8>,
    skip_partial_line: bool,
    // toolCallId → (工具名, 简述)：tool.result 不带工具名，失败判定需回溯 tool.call
    pending_tools: std::collections::HashMap<String, (String, String)>,
    pub signals: WireSignals,
}

impl WireTailer {
    pub fn new() -> Self {
        Self { path: None, offset: 0, stale_checks: 0, pending: Vec::new(), skip_partial_line: false, pending_tools: std::collections::HashMap::new(), signals: WireSignals::default() }
    }

    fn reset_stream(&mut self) {
        self.pending.clear();
        self.pending_tools.clear();
        self.skip_partial_line = false;
        // 气泡编号跨会话递增，避免前端把新会话气泡误判为已显示。
        let bubble_seq = self.signals.bubble_seq;
        self.signals = WireSignals::default();
        self.signals.bubble_seq = bubble_seq;
    }

    pub fn end_turn(&mut self) {
        self.pending_tools.clear();
        self.signals.end_turn();
    }

    pub fn poll(&mut self, now: i64) {
        // 每 20 tick（10s）检查一次是否有更新的会话值得切换
        self.stale_checks += 1;
        if self.path.is_none() || self.stale_checks >= 20 {
            self.stale_checks = 0;
            if let Some(p) = latest_wire_path() {
                let dominated = match &self.path {
                    None => true,
                    // 滞后切换：候选文件必须明显更新（mtime 新 10s 以上）才换，防乒乓
                    Some(cur) if *cur != p => {
                        let cur_mtime = std::fs::metadata(cur).and_then(|m| m.modified()).ok();
                        let new_mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
                        match (cur_mtime, new_mtime) {
                            (Some(c), Some(n)) => {
                                n.duration_since(c).map(|d| d.as_secs() > 10).unwrap_or(false)
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                };
                if dominated {
                    // 关键：挂载新文件时跳到末尾，只消费增量，绝不重放历史
                    let Ok(mut file) = std::fs::File::open(&p) else { return };
                    let Ok(meta) = file.metadata() else { return };
                    let len = meta.len();
                    let mut last = [0u8; 1];
                    if len > 0 && (file.seek(SeekFrom::Start(len - 1)).is_err() || file.read_exact(&mut last).is_err()) {
                        return;
                    }
                    self.reset_stream();
                    self.skip_partial_line = len > 0 && last[0] != b'\n';
                    self.path = Some(p);
                    self.offset = len;
                }
            }
        }
        let Some(path) = self.path.clone() else { return };
        self.tail_file(&path, now);
    }

    fn tail_file(&mut self, path: &PathBuf, now: i64) {
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    self.path = None;
                    self.reset_stream();
                }
                return;
            }
        };
        let Ok(meta) = file.metadata() else { return };
        if meta.len() < self.offset {
            self.offset = 0; // 文件被截断/轮换
            self.reset_stream();
        }
        if meta.len() == self.offset {
            return;
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() { return; }
        let mut bytes = Vec::new();
        // 每拍最多读取 1MiB，避免大量新增内容阻塞状态推送。
        let limit = (meta.len() - self.offset).min(1024 * 1024);
        if file.take(limit).read_to_end(&mut bytes).is_err() { return; }
        self.offset += bytes.len() as u64;
        self.pending.extend_from_slice(&bytes);
        let mut consumed = 0;
        while let Some(end) = self.pending[consumed..].iter().position(|&b| b == b'\n') {
            let end = consumed + end;
            if self.skip_partial_line {
                self.skip_partial_line = false;
            } else {
                let line = self.pending[consumed..end].to_vec();
                if let Ok(text) = std::str::from_utf8(&line) {
                    self.parse_line(text, now);
                }
            }
            consumed = end + 1;
        }
        self.pending.drain(..consumed);
        // 异常的超长未结束行只丢弃该行，后续换行后恢复解析。
        if self.pending.len() > 8 * 1024 * 1024 {
            self.pending.clear();
            self.skip_partial_line = true;
        }
    }

    fn parse_line(&mut self, line: &str, now: i64) {
        let Ok(rec) = serde_json::from_str::<Value>(line) else { return };
        self.signals.parse_ok = true;
        let ty = rec.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "context.append_loop_event" => {
                let Some(ev) = rec.get("event") else { return };
                let ev_ty = ev.get("type").and_then(Value::as_str).unwrap_or("");
                match ev_ty {
                    "content.part" => {
                        let part = ev.get("part").cloned().unwrap_or(Value::Null);
                        match part.get("type").and_then(Value::as_str) {
                            Some("think") => self.signals.reasoning_until_ms = now + REASONING_WINDOW_MS,
                            Some("text") => {
                                self.signals.delta_until_ms = now + STREAMING_WINDOW_MS;
                                let n = part.get("text").and_then(Value::as_str).map(str::len).unwrap_or(0);
                                if n > 0 {
                                    self.signals.text_chunks.push((now, n));
                                }
                            }
                            _ => {}
                        }
                    }
                    "tool.call" => {
                        self.signals.tool_times.push(now);
                        let name = ev.get("name").and_then(Value::as_str).unwrap_or("");
                        self.signals.tool_running = Some(tool_category_of(name));
                        let args = ev.get("args").cloned().unwrap_or(Value::Null);
                        let bubble = tool_call_bubble(name, &args);
                        if let Some(id) = ev.get("toolCallId").and_then(Value::as_str) {
                            if self.pending_tools.len() > 64 {
                                self.pending_tools.clear();
                            }
                            // 失败气泡用的简述：Bash 取命令前 30 字，其余取工具名
                            let label = if name == "Bash" {
                                args.get("command")
                                    .and_then(Value::as_str)
                                    .map(|c| c.trim().chars().take(30).collect::<String>())
                                    .filter(|c| !c.is_empty())
                                    .unwrap_or_else(|| "命令".to_string())
                            } else {
                                name.to_string()
                            };
                            self.pending_tools.insert(id.to_string(), (name.to_string(), label));
                        }
                        let id = ev.get("toolCallId").and_then(Value::as_str).unwrap_or("");
                        self.signals.set_bubble(bubble, Some(id.to_owned()), now + TOOL_BUBBLE_MS);
                    }
                    "step.begin" => self.signals.step_open = true,
                    "step.end" => self.signals.step_open = false,
                    "tool.result" => {
                        self.signals.tool_running = None;
                        self.signals.finish_tool_bubble(ev.get("toolCallId").and_then(Value::as_str).unwrap_or(""));
                        let tool = ev
                            .get("toolCallId")
                            .and_then(Value::as_str)
                            .and_then(|id| self.pending_tools.remove(id));
                        if let Some((cmd, code)) = extract_failure(ev, tool.as_ref()) {
                            self.signals.failed_until_ms = now + FAILED_AFTERGLOW_MS;
                            self.signals.fail_count += 1;
                            let text = match code {
                                Some(code) => format!("{cmd} 挂了（exit {code}）"),
                                None => format!("{cmd} 执行失败"),
                            };
                            self.signals.set_bubble(text, None, now + FAILED_AFTERGLOW_MS);
                        }
                    }
                    _ => {}
                }
            }
            "plan_mode.enter" => self.signals.plan_mode = true,
            "plan_mode.exit" => self.signals.plan_mode = false,
            "interaction.request" => self.signals.approval_pending = true,
            "interaction.resolved" => self.signals.approval_pending = false,
            "agent.turn.started" => {
                self.end_turn();
                self.signals.fail_count = 0;
                self.signals.failed_until_ms = 0;
                self.signals.bubble = None;
            }
            "agent.turn.ended" | "turn.ended" | "prompt.completed" => self.end_turn(),
            _ => {}
        }
    }
}

/// 工具名 → 类别（决定她做什么动作）
fn tool_category_of(name: &str) -> &'static str {
    match name {
        "Bash" | "TaskOutput" | "TaskStop" => "bash",
        "Edit" | "Write" | "NotebookEdit" => "edit",
        "Read" | "Grep" | "Glob" | "ReadMediaFile" => "read",
        "WebSearch" | "FetchURL" => "search",
        "Agent" | "AgentSwarm" => "agent",
        _ => "other",
    }
}

fn tool_call_bubble(name: &str, args: &Value) -> String {
    let short = |s: &str, n: usize| -> String {
        let s = s.trim();
        if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() }
    };
    let basename = |p: &str| p.replace('\\', "/").rsplit('/').next().unwrap_or(p).to_string();
    match name {
        "Bash" => format!("正在跑 `{}`…", short(args.get("command").and_then(Value::as_str).unwrap_or(""), 30)),
        "Read" => format!("在读 {}", basename(args.get("path").and_then(Value::as_str).unwrap_or(""))),
        "Edit" | "Write" => format!("在改 {}", basename(args.get("path").and_then(Value::as_str).unwrap_or(""))),
        "Grep" => format!("在搜 `{}`", short(args.get("pattern").and_then(Value::as_str).unwrap_or(""), 20)),
        "Glob" => format!("在找 `{}`", short(args.get("pattern").and_then(Value::as_str).unwrap_or(""), 20)),
        "WebSearch" => format!("在查 `{}`", short(args.get("query").and_then(Value::as_str).unwrap_or(""), 20)),
        "FetchURL" => "在看网页…".to_string(),
        "" => "在调用工具…".to_string(),
        other => format!("在用 {other}"),
    }
}

/// 仅信任Bash结果自身的结构化失败标记；stdout中的字样不能决定成败。
/// 退出码只作为展示信息，从结构化字段或独立末行提取；缺失时不编造。
fn extract_failure(ev: &Value, tool: Option<&(String, String)>) -> Option<(String, Option<i64>)> {
    let (name, label) = tool?;
    if name != "Bash" {
        return None;
    }
    let result = ev.get("result")?;
    if result.get("isError").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let code = result.get("exitCode").or_else(|| result.get("exit_code")).and_then(Value::as_i64)
        .or_else(|| {
            let last = result.get("output")?.as_str()?.lines().rev().find(|line| !line.trim().is_empty())?.trim();
            ["Command failed with exit code: ", "exit code: ", "Exit code: ", "Process exited with code "]
                .iter().find_map(|prefix| last.strip_prefix(prefix)?.parse::<i64>().ok())
        }).filter(|code| *code != 0);
    Some((format!("`{label}`"), code))
}

/// 最近活跃的 main agent wire.jsonl：遍历 sessions/*/​*/agents/main/wire.jsonl 取 mtime 最新。
fn latest_wire_path() -> Option<PathBuf> {
    let home = std::env::var_os("KIMI_CODE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|u| PathBuf::from(u).join(".kimi-code")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".kimi-code")))?;
    let sessions = home.join("sessions");
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for wd in std::fs::read_dir(&sessions).ok()? {
        let Ok(wd) = wd else { continue };
        for sid in std::fs::read_dir(wd.path()).ok().into_iter().flatten() {
            let Ok(sid) = sid else { continue };
            let p = sid.path().join("agents").join("main").join("wire.jsonl");
            let Ok(meta) = p.metadata() else { continue };
            let Ok(mtime) = meta.modified() else { continue };
            if best.as_ref().is_none_or(|(_, t)| mtime > *t) {
                best = Some((p, mtime));
            }
        }
    }
    best.map(|(p, _)| p)
}
