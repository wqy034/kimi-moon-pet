//! 统一谱面：原始音高不因21键简化而丢弃，布局只决定演奏分工。
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
fn one() -> u32 {
    1
}
fn c_key() -> String {
    "C".into()
}
fn lyre() -> String {
    "lyre21".into()
}
fn auto_role() -> String {
    "auto".into()
}
fn two() -> usize {
    2
}
fn three() -> i8 {
    3
}
fn default_velocity() -> u8 {
    96
}
fn normal_velocity(value: &u8) -> bool {
    *value == 96
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSong {
    #[serde(default = "one")]
    format_version: u32,
    id: String,
    version: u32,
    title: String,
    bpm: f64,
    beats: f64,
    #[serde(default)]
    credit: String,
    #[serde(default = "c_key")]
    key: String,
    #[serde(default = "lyre")]
    layout: String,
    #[serde(default)]
    min_gap_ms: Option<f64>,
    #[serde(default)]
    lyre21: Arrangement,
    notes: Vec<RawNote>,
    #[serde(default)]
    accompaniment: Vec<RawNote>,
}
#[derive(Deserialize)]
struct RawNote {
    #[serde(default)]
    pitch: Option<u8>,
    #[serde(default)]
    key: Option<u8>,
    #[serde(default)]
    oct: Option<u8>,
    beat: f64,
    duration: f64,
    #[serde(default = "default_velocity")]
    velocity: u8,
    #[serde(default = "auto_role")]
    lyre21: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Arrangement {
    #[serde(default = "three")]
    base_octave: i8,
    #[serde(default = "two")]
    max_chord: usize,
}
impl Default for Arrangement {
    fn default() -> Self {
        Self {
            base_octave: 3,
            max_chord: 2,
        }
    }
}
#[derive(Clone, Serialize)]
pub struct Key {
    pub pitch: u8,
    pub label: String,
}
#[derive(Clone, Serialize)]
pub struct Note {
    pub pitch: u8,
    pub beat: f64,
    pub duration: f64,
    pub index: Option<usize>,
    pub player: bool,
    // 默认力度不序列化，保证已有谱面的分工摘要不因新增可选字段改变。
    #[serde(skip_serializing_if = "normal_velocity")]
    pub velocity: u8,
}
#[derive(Clone, Serialize)]
pub struct Song {
    pub id: String,
    pub version: u32,
    pub title: String,
    pub bpm: f64,
    pub beats: f64,
    pub credit: String,
    pub key: String,
    pub key_label: String,
    pub keys: Vec<Key>,
    pub notes: Vec<Note>,
    pub accompaniment: Vec<Note>,
    pub player_count: usize,
    pub assist_count: usize,
    pub difficulty: String,
    pub recommended: String,
    pub min_gap_ms: Option<f64>,
    pub arrangement: String,
    pub legacy_compatible: bool,
}
impl Song {
    pub fn seconds(&self) -> f64 {
        self.beats * 60. / self.bpm
    }
    pub fn legacy_key(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
    pub fn record_key(&self) -> String {
        format!("{}|lyre21|{}", self.legacy_key(), self.arrangement)
    }
}
pub fn judgment_rank(value: &str) -> Option<u8> {
    match value {
        "coarse" => Some(1),
        "strict" => Some(2),
        "ultra" => Some(3),
        _ => None,
    }
}
fn key_signature(value: &str) -> Option<(u8, String)> {
    let minor = value.ends_with('m');
    let name = if minor {
        &value[..value.len() - 1]
    } else {
        value
    };
    let pc: u8 = match name {
        "C" => 0,
        "C#" | "Db" => 1,
        "D" => 2,
        "D#" | "Eb" => 3,
        "E" | "Fb" => 4,
        "F" | "E#" => 5,
        "F#" | "Gb" => 6,
        "G" => 7,
        "G#" | "Ab" => 8,
        "A" => 9,
        "A#" | "Bb" => 10,
        "B" | "Cb" => 11,
        _ => return None,
    };
    // 自然小调使用la为主音的唱名：Am与C大调共用键位，变化音仍完整保留并代奏。
    let do_pc = if minor { (pc + 3) % 12 } else { pc };
    let names = [
        "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
    ];
    let label = if minor {
        format!("{value} · 1={}，6={name}", names[do_pc as usize])
    } else {
        format!("{value} 大调 · 1={name}")
    };
    Some((do_pc, label))
}
pub fn decode(bytes: &[u8]) -> Result<Song, String> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let raw: RawSong = serde_json::from_slice(bytes).map_err(|e| format!("JSON字段错误：{e}"))?;
    if ![1, 2].contains(&raw.format_version) {
        return Err("不支持的formatVersion".into());
    }
    if raw.id.is_empty()
        || raw.id.len() > 64
        || !raw
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        || raw.version == 0
        || raw.title.trim().is_empty()
        || raw.title.chars().count() > 60
        || !(40.0..=200.).contains(&raw.bpm)
        || !(1.0..=1000.).contains(&raw.beats)
        || raw.beats * 60. / raw.bpm > 600.
        || !(4..=2000).contains(&raw.notes.len())
        || raw.accompaniment.len() > 4000
    {
        return Err("曲目信息、长度或音符数量超出范围".into());
    }
    if raw.layout != "lyre21" {
        return Err("本轮仅开放lyre21布局，原谱可保留完整和弦".into());
    }
    if raw.format_version == 1 && raw.key != "C" {
        return Err("旧谱固定为C调，随调键位请使用formatVersion=2".into());
    }
    if raw.min_gap_ms.is_some_and(|g| !(1.0..=2000.).contains(&g)) {
        return Err("minGapMs须为1～2000，省略则按实际节奏读取".into());
    }
    if !(-1..=7).contains(&raw.lyre21.base_octave) || !(1..=7).contains(&raw.lyre21.max_chord) {
        return Err("lyre21的baseOctave或maxChord无效".into());
    }
    let (do_pc, key_label) = key_signature(&raw.key).ok_or("无法识别的调性")?;
    let base = (i16::from(raw.lyre21.base_octave) + 1) * 12 + i16::from(do_pc);
    if base < 0 || base + 35 > 127 {
        return Err("21键布局超出MIDI音域，请调整baseOctave".into());
    }
    let keys: Vec<Key> = (0..21)
        .map(|i| Key {
            pitch: (base + (i / 7) as i16 * 12 + i16::from(MAJOR[i % 7])) as u8,
            label: (i % 7 + 1).to_string(),
        })
        .collect();
    let parse_notes = |source: &[RawNote], backing: bool| -> Result<Vec<Note>, String> {
        let mut seen = BTreeSet::new();
        let mut last = -1.;
        source
            .iter()
            .map(|n| {
                if !n.beat.is_finite()
                    || n.beat < 0.
                    || !(0.05..=16.).contains(&n.duration)
                    || n.beat + n.duration > raw.beats
                    || n.beat < last
                    || !(1..=127).contains(&n.velocity)
                {
                    return Err(
                        "音符必须按拍位排序，时长0.05～16拍且不越曲尾，velocity为1～127".into(),
                    );
                }
                last = n.beat;
                let pitch = if raw.format_version == 1 {
                    match (n.key, n.oct, n.pitch) {
                        (Some(k @ 0..=6), Some(o @ 0..=2), None) => 48 + o * 12 + MAJOR[k as usize],
                        _ => return Err("旧谱音符需要key=0～6、oct=0～2".into()),
                    }
                } else {
                    match (n.pitch, n.key, n.oct) {
                        (Some(p @ 0..=127), None, None) => p,
                        _ => return Err("v2的pitch使用标准MIDI整数0～127，不混用key/oct".into()),
                    }
                };
                if !seen.insert((if n.beat == 0. { 0 } else { n.beat.to_bits() }, pitch))
                    && !backing
                {
                    return Err("同一拍位不能重复同一音高".into());
                }
                if !["auto", "player", "assist"].contains(&n.lyre21.as_str()) {
                    return Err("音符lyre21必须为auto/player/assist".into());
                }
                if backing && n.lyre21 == "player" {
                    return Err("伴奏轨不能声明player音符".into());
                }
                let index = keys.iter().position(|k| k.pitch == pitch);
                if !backing && n.lyre21 == "player" && index.is_none() {
                    return Err("指定给玩家的音不在21键布局内，请改分工或baseOctave".into());
                }
                Ok(Note {
                    pitch,
                    beat: if n.beat == 0. { 0. } else { n.beat },
                    duration: n.duration,
                    index,
                    player: !backing && index.is_some() && n.lyre21 != "assist",
                    velocity: n.velocity,
                })
            })
            .collect()
    };
    let mut notes = parse_notes(&raw.notes, false).map_err(|e| format!("旋律：{e}"))?;
    let mut backing: BTreeMap<(u64, u8), Note> = BTreeMap::new();
    for n in parse_notes(&raw.accompaniment, true).map_err(|e| format!("伴奏：{e}"))? {
        backing
            .entry((n.beat.to_bits(), n.pitch))
            .and_modify(|old| {
                old.duration = old.duration.max(n.duration);
                old.velocity = old.velocity.max(n.velocity);
            })
            .or_insert(n);
    }
    let accompaniment: Vec<Note> = backing.into_values().collect();
    // 资源保护只限制不具备实际演奏意义的极端事件洪峰，不量化节奏。
    let mut events: Vec<&Note> = notes.iter().chain(&accompaniment).collect();
    events.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    if events
        .windows(65)
        .any(|w| (w[64].beat - w[0].beat) * 60. / raw.bpm < 0.05)
    {
        return Err("50ms内音符超过64个，请检查重复轨或异常密度".into());
    }
    // 同拍组保留明确指定的音，自动分配优先高音；其余自动交给她，不删原谱。
    let mut start = 0;
    while start < notes.len() {
        let end = (start + 1..notes.len())
            .find(|&i| notes[i].beat != notes[start].beat)
            .unwrap_or(notes.len());
        let forced = (start..end)
            .filter(|&i| raw.notes[i].lyre21 == "player")
            .count();
        if forced > raw.lyre21.max_chord {
            return Err("同拍指定player音数超过maxChord".into());
        }
        let mut auto: Vec<usize> = (start..end)
            .filter(|&i| notes[i].player && raw.notes[i].lyre21 == "auto")
            .collect();
        auto.sort_by_key(|&i| std::cmp::Reverse(notes[i].pitch));
        for &i in auto.iter().skip(raw.lyre21.max_chord - forced) {
            notes[i].player = false;
        }
        start = end;
    }
    let player_notes: Vec<&Note> = notes.iter().filter(|n| n.player).collect();
    let mut gap: Option<f64> = None;
    let mut max_chord = 0;
    let mut count = 0;
    let mut prior: Option<f64> = None;
    for n in &player_notes {
        if let Some(p) = prior {
            if n.beat == p {
                count += 1;
            } else {
                let ms = (n.beat - p) * 60000. / raw.bpm;
                if ms < 1. {
                    return Err("不同时点至少相隔1ms；和弦请写相同beat".into());
                }
                gap = Some(gap.map_or(ms, |g| g.min(ms)));
                count = 1;
            }
        } else {
            count = 1;
        }
        max_chord = max_chord.max(count);
        prior = Some(n.beat);
    }
    if raw
        .min_gap_ms
        .is_some_and(|g| gap.is_some_and(|actual| actual + 0.0001 < g))
    {
        return Err("实际玩家音符间距小于谱面声明的minGapMs".into());
    }
    let difficulty = if gap.is_some_and(|g| g < 125. - 0.0001) || max_chord >= 4 {
        "expert"
    } else if gap.is_some_and(|g| g < 250. - 0.0001) || max_chord >= 3 {
        "red"
    } else {
        "green"
    };
    let recommended = match difficulty {
        "expert" => "ultra",
        "red" => "strict",
        _ => "coarse",
    };
    // FNV-1a为稳定身份摘要（非安全用途），覆盖音高、键位、时点、分工及伴奏。
    let identity = serde_json::to_vec(&(&keys, &notes, &accompaniment, raw.bpm, raw.beats))
        .map_err(|e| e.to_string())?;
    let hash = identity.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    let legacy_compatible = raw.key == "C"
        && raw.lyre21.base_octave == 3
        && notes.iter().all(|n| n.player)
        && max_chord == 1
        && gap.is_none_or(|g| g >= 250. - 0.0001);
    let player_count = player_notes.len();
    Ok(Song {
        id: raw.id,
        version: raw.version,
        title: raw.title,
        bpm: raw.bpm,
        beats: raw.beats,
        credit: raw.credit,
        key: raw.key,
        key_label,
        keys,
        player_count,
        assist_count: notes.len() - player_count + accompaniment.len(),
        notes,
        accompaniment,
        difficulty: difficulty.into(),
        recommended: recommended.into(),
        min_gap_ms: gap,
        arrangement: format!("auto1-{hash:016x}"),
        legacy_compatible,
    })
}
