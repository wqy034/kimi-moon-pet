//! 只读实际用量。消费游标、去重键和金币由life存入同一份事务存档。
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Checkpoint {
    offset: u64,
    anchor: Vec<u8>,
    skip_line: bool,
    #[serde(default)]
    step_usage: u64,
    #[serde(default)]
    message_id: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub initialized: bool,
    pub since_ms: u64,
    pub total: u64,
    pub remainder: u64,
    files: BTreeMap<String, Checkpoint>,
    seen: BTreeSet<String>,
    pub records: u64,
}

fn roots() -> Vec<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let mut roots = Vec::new();
    for (env, folder) in [("KIMI_CODE_HOME", ".kimi-code"), ("KIMI_HOME", ".kimi")] {
        if let Some(p) = std::env::var_os(env) {
            roots.push(PathBuf::from(p).join("sessions"));
        } else if let Some(h) = &home {
            roots.push(h.join(folder).join("sessions"));
        }
    }
    roots.sort();
    roots.dedup();
    roots
}
fn collect(dir: &Path, depth: u8, out: &mut BTreeSet<PathBuf>) {
    if depth == 0 {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                collect(&e.path(), depth - 1, out);
            } else if t.is_file() && e.file_name() == "wire.jsonl" {
                out.insert(e.path());
            }
        }
    }
}
fn anchor(file: &mut fs::File, offset: u64) -> std::io::Result<Vec<u8>> {
    let n = offset.min(64);
    file.seek(SeekFrom::Start(offset - n))?;
    let mut bytes = vec![0; n as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn baseline(file: &mut fs::File, len: u64) -> std::io::Result<Checkpoint> {
    let bytes = anchor(file, len)?;
    Ok(Checkpoint {
        offset: len,
        skip_line: bytes.last().is_some_and(|&b| b != b'\n'),
        anchor: bytes,
        ..Checkpoint::default()
    })
}
fn fingerprint(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}
fn sum(usage: &Value, camel: bool) -> Option<u64> {
    let keys = if camel {
        [
            "inputOther",
            "output",
            "inputCacheRead",
            "inputCacheCreation",
        ]
    } else {
        [
            "input_other",
            "output",
            "input_cache_read",
            "input_cache_creation",
        ]
    };
    // 不接受字符串、负数、浮点数，也不把context_tokens当消费。
    let input = usage.get(keys[0])?.as_u64()?;
    let output = usage.get(keys[1])?.as_u64()?;
    let mut total = input.checked_add(output)?;
    for key in &keys[2..] {
        total = total.checked_add(match usage.get(*key) {
            None => 0,
            Some(v) => v.as_u64()?,
        })?;
    }
    (total <= 1_000_000_000).then_some(total)
}
fn record(v: &Value, since: u64) -> Option<(String, u64)> {
    if v.get("type")?.as_str()? == "usage.record" {
        // Code的session记录是累计值，只认turn增量；不从message.meta再计一次。
        if v.get("usageScope")?.as_str()? != "turn" || v.get("time")?.as_u64()? < since {
            return None;
        }
        let n = sum(v.get("usage")?, true)?;
        return Some((fingerprint(serde_json::to_string(v).ok()?.as_bytes()), n));
    }
    None
}
fn official_record(v: &Value, cp: &mut Checkpoint, since: u64) -> Option<(String, u64)> {
    let message = v.get("message")?;
    let kind = message.get("type")?.as_str()?;
    if matches!(kind, "TurnBegin" | "StepBegin") {
        cp.step_usage = 0;
        cp.message_id.clear();
    }
    if kind != "StatusUpdate" {
        return None;
    }
    let stamp = v.get("timestamp")?.as_f64()? * 1000.;
    if !stamp.is_finite() || stamp < since as f64 {
        return None;
    }
    let payload = message.get("payload")?;
    let n = sum(payload.get("token_usage")?, false)?;
    if let Some(id) = payload.get("message_id").and_then(Value::as_str) {
        if cp.message_id != id {
            cp.step_usage = 0;
            cp.message_id = id.into();
        }
    }
    let delta = n.saturating_sub(cp.step_usage);
    cp.step_usage = cp.step_usage.max(n);
    Some((
        fingerprint(serde_json::to_string(v).ok()?.as_bytes()),
        delta,
    ))
}
impl Usage {
    pub fn poll(&mut self, now_ms: u64) -> u64 {
        let mut paths = BTreeSet::new();
        for root in roots() {
            collect(&root, 6, &mut paths);
        }
        let first = !self.initialized;
        if first {
            self.since_ms = now_ms;
        }
        let mut earned_tokens: u64 = 0;
        let mut budget = 8 * 1024 * 1024_u64;
        for path in paths {
            let Ok(mut file) = fs::File::open(&path) else {
                continue;
            };
            let Ok(meta) = file.metadata() else { continue };
            let len = meta.len();
            let key = path.to_string_lossy().to_string();
            if first {
                if let Ok(cp) = baseline(&mut file, len) {
                    self.files.insert(key, cp);
                }
                continue;
            }
            let cp = self.files.entry(key).or_default();
            // 换文件/截断/重写不回放；重新以当前EOF为基线。
            if cp.offset > len || anchor(&mut file, cp.offset).ok().as_ref() != Some(&cp.anchor) {
                if let Ok(new) = baseline(&mut file, len) {
                    *cp = new;
                }
                continue;
            }
            if budget == 0 || cp.offset == len {
                continue;
            }
            if file.seek(SeekFrom::Start(cp.offset)).is_err() {
                continue;
            }
            let limit = budget.min(2 * 1024 * 1024);
            let mut bytes = Vec::new();
            if (&mut file).take(limit).read_to_end(&mut bytes).is_err() {
                continue;
            }
            budget = budget.saturating_sub(bytes.len() as u64);
            let mut used = 0;
            for line in bytes.split_inclusive(|b| *b == b'\n') {
                if line.last() != Some(&b'\n') {
                    break;
                }
                used += line.len();
                if cp.skip_line {
                    cp.skip_line = false;
                    continue;
                }
                if line.len() > 1024 * 1024 {
                    continue;
                }
                if let Ok(v) = serde_json::from_slice::<Value>(line) {
                    if let Some((id, n)) =
                        record(&v, self.since_ms).or_else(|| official_record(&v, cp, self.since_ms))
                    {
                        if n > 0 && self.seen.insert(id) {
                            earned_tokens = earned_tokens.saturating_add(n);
                            self.records += 1;
                        }
                    }
                }
            }
            // 超长单行分块丢弃，避免永远卡在同一处；普通半行留待下次。
            if used == 0 && bytes.len() as u64 == limit {
                used = bytes.len();
                cp.skip_line = true;
            }
            cp.offset += used as u64;
            if let Ok(a) = anchor(&mut file, cp.offset) {
                cp.anchor = a;
            }
        }
        self.initialized = true;
        self.total = self.total.saturating_add(earned_tokens);
        earned_tokens
    }
}
