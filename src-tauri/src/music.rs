//! 琴盘领域：曲库、单个演奏会话与通关事实。音符调度/输入判定留在琴盘WebView。
use crate::adapters::state_file::pet_home;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::windows::fs::OpenOptionsExt,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};

use crate::music_score::{self, Song};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Record {
    sight: u8,
    mole: u8,
}
#[derive(Clone, Serialize, Deserialize)]
struct Save {
    version: u32,
    records: BTreeMap<String, Record>,
    #[serde(default)]
    legacy: BTreeMap<String, LegacyRecord>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct LegacyRecord {
    sight: bool,
    mole: bool,
}
#[derive(Deserialize)]
struct LegacySave {
    version: u32,
    records: BTreeMap<String, LegacyRecord>,
}
struct Session {
    id: u64,
    mode: String,
    judgment: String,
    song: Option<Song>,
    phase: String,
    position: f64,
    beat_at: Instant,
    started: Instant,
    granted: bool,
    grant_at: Instant,
    reason: String,
}
#[derive(Clone, Serialize)]
pub struct SessionView {
    id: u64,
    mode: String,
    title: String,
    phase: String,
    requested: bool,
    granted: bool,
    reason: String,
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    revision: u64,
    session: Option<SessionView>,
}
#[derive(Serialize)]
pub struct Catalog {
    songs: Vec<Song>,
    records: BTreeMap<String, Record>,
    weights: BTreeMap<String, u32>,
    warnings: Vec<String>,
    state: Snapshot,
}
#[derive(Serialize)]
pub struct StartReply {
    state: Snapshot,
    song: Option<Song>,
    judgment: String,
}
#[derive(Serialize)]
pub struct Outcome {
    passed: bool,
    saved: bool,
    hits: usize,
    wrong: usize,
    total: usize,
    judgment: String,
    arrangement: String,
}
pub struct Service {
    revision: u64,
    sequence: u64,
    session: Option<Session>,
    save: Save,
    error: Option<String>,
    _lease: Option<fs::File>,
}
fn authorize(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "extension-music" {
        return Err("仅琴盘窗口可以操作演奏会话".into());
    }
    Ok(())
}
fn catalog() -> (Vec<Song>, Vec<String>) {
    let mut songs = Vec::new();
    let mut warnings = Vec::new();
    if let Ok(entries) = fs::read_dir(pet_home().join("config/曲库")) {
        let mut files: Vec<_> = entries
            .flatten()
            .filter(|e| {
                e.file_type().is_ok_and(|t| t.is_file())
                    && e.path().extension().is_some_and(|s| s == "json")
            })
            .collect();
        files.sort_by_key(|e| e.file_name());
        for file in files.into_iter().take(200) {
            let valid = file.metadata().is_ok_and(|m| m.len() <= 512 * 1024);
            let decoded = if valid {
                fs::read(file.path())
                    .map_err(|e| e.to_string())
                    .and_then(|b| music_score::decode(&b))
            } else {
                Err("文件大于512KiB".into())
            };
            match decoded {
                Ok(song) if !songs.iter().any(|old: &Song| old.id == song.id) => songs.push(song),
                Ok(_) => warnings.push(format!(
                    "已跳过重名曲谱：{}",
                    file.file_name().to_string_lossy()
                )),
                Err(reason) => warnings.push(format!(
                    "已跳过曲谱{}：{reason}",
                    file.file_name().to_string_lossy()
                )),
            }
        }
    }
    if songs.is_empty() {
        warnings.push("曲库为空，请检查config/曲库内的JSON曲谱。".into());
    }
    (songs, warnings)
}
impl Service {
    fn load() -> Self {
        let mut service = Self {
            revision: 0,
            sequence: 0,
            session: None,
            save: Save {
                version: 2,
                records: BTreeMap::new(),
                legacy: BTreeMap::new(),
            },
            error: None,
            _lease: None,
        };
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(pet_home().join("琴盘记录.lock"))
        {
            Ok(file) => service._lease = Some(file),
            Err(e) => service.error = Some(format!("琴盘记录无法加锁，仍可弹奏，暂不能保存：{e}")),
        }
        match fs::read(pet_home().join("琴盘记录.json")) {
            Ok(bytes) => {
                let current = serde_json::from_slice::<Save>(&bytes).ok().filter(|v| {
                    v.version == 2
                        && v.records.len() <= 10000
                        && v.records.values().all(|r| r.sight <= 3 && r.mole <= 3)
                });
                if let Some(save) = current {
                    service.save = save;
                } else if let Some(old) = serde_json::from_slice::<LegacySave>(&bytes)
                    .ok()
                    .filter(|v| v.version == 1 && v.records.len() <= 10000)
                {
                    service.save.legacy = old.records;
                } else {
                    service.error =
                        Some("琴盘记录损坏或版本不兼容，保留原文件，暂不保存新记录。".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => service.error = Some(format!("琴盘记录读取失败：{e}")),
        }
        service
    }
    fn view(&self) -> Snapshot {
        Snapshot {
            revision: self.revision,
            session: self.session.as_ref().map(|s| SessionView {
                id: s.id,
                mode: s.mode.clone(),
                title: s
                    .song
                    .as_ref()
                    .map(|s| s.title.clone())
                    .unwrap_or("自由弹奏".into()),
                phase: s.phase.clone(),
                requested: s.phase == "playing" && s.beat_at.elapsed().as_secs_f64() < 5.,
                granted: s.phase == "playing"
                    && s.beat_at.elapsed().as_secs_f64() < 5.
                    && s.granted
                    && s.grant_at.elapsed().as_secs_f64() < 3.,
                reason: s.reason.clone(),
            }),
        }
    }
    fn publish(&mut self, app: &tauri::AppHandle) -> Snapshot {
        self.revision += 1;
        let view = self.view();
        let _ = app.emit("music-session", &view);
        view
    }
    fn record(&self, song: &Song) -> Record {
        let mut r = self
            .save
            .records
            .get(&song.record_key())
            .cloned()
            .unwrap_or_default();
        if song.legacy_compatible {
            if let Some(old) = self.save.legacy.get(&song.legacy_key()) {
                r.sight = r.sight.max(u8::from(old.sight));
                r.mole = r.mole.max(u8::from(old.mole));
            }
        }
        r
    }
    fn write_record(&mut self, song: &Song, mode: &str, rank: u8) -> Result<(), String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let mut next = self.save.clone();
        let r = next
            .records
            .entry(song.record_key())
            .or_insert_with(|| self.record(song));
        if mode == "sight" {
            r.sight = r.sight.max(rank);
        } else {
            r.mole = r.mole.max(rank);
        }
        let bytes = serde_json::to_vec_pretty(&next).map_err(|e| e.to_string())?;
        let temp = pet_home().join("琴盘记录.json.tmp");
        let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(temp, pet_home().join("琴盘记录.json")).map_err(|e| e.to_string())?;
        self.save = next;
        Ok(())
    }
}
pub fn install(app: &tauri::AppHandle) {
    app.manage(Mutex::new(Service::load()));
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let state = app.state::<Mutex<Service>>();
        let Ok(mut s) = state.lock() else { continue };
        let had_session = s.session.is_some();
        if s.session
            .as_ref()
            .is_some_and(|v| v.beat_at.elapsed().as_secs_f64() > 5.)
        {
            s.session = None;
        }
        if let Some(v) = &mut s.session {
            if v.grant_at.elapsed().as_secs_f64() > 3. {
                v.granted = false;
                v.reason = "等待桌宠回应".into();
            }
        }
        if had_session {
            s.publish(&app);
        }
    });
}
pub fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "extension-music" {
        return;
    }
    match event {
        tauri::WindowEvent::Destroyed | tauri::WindowEvent::CloseRequested { .. } => {
            stop_window(window.app_handle())
        }
        tauri::WindowEvent::Focused(false) => {
            let state = window.state::<Mutex<Service>>();
            let Ok(mut service) = state.lock() else {
                return;
            };
            if let Some(s) = &mut service.session {
                if s.mode != "demo" {
                    s.phase = "paused".into();
                    s.granted = false;
                    s.reason = "切换窗口已暂停".into();
                }
            }
            service.publish(window.app_handle());
        }
        _ => {}
    }
}
pub fn stop_window(app: &tauri::AppHandle) {
    let state = app.state::<Mutex<Service>>();
    if let Ok(mut s) = state.lock() {
        s.session = None;
        s.publish(app);
    };
}
#[tauri::command]
pub fn music_get(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Mutex<Service>>,
) -> Result<Catalog, String> {
    if !matches!(window.label(), "main" | "extension-music") {
        return Err("该窗口不能读取琴盘数据".into());
    }
    let s = state.lock().map_err(|_| "琴盘锁不可用")?;
    let (songs, mut warnings) = catalog();
    if let Some(e) = &s.error {
        warnings.push(e.clone());
    }
    let weights = songs
        .iter()
        .map(|song| {
            let r = s.record(song);
            (
                song.id.clone(),
                if r.sight > 0 || r.mole > 0 { 3 } else { 1 },
            )
        })
        .collect();
    Ok(Catalog {
        weights,
        records: songs
            .iter()
            .map(|song| (song.record_key(), s.record(song)))
            .collect(),
        warnings,
        state: s.view(),
        songs,
    })
}
#[tauri::command]
pub fn music_start(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Mutex<Service>>,
    mode: String,
    song_id: Option<String>,
    judgment: Option<String>,
) -> Result<StartReply, String> {
    authorize(&window)?;
    if window.is_visible().ok() != Some(true) {
        return Err("请先打开琴盘窗口".into());
    }
    if !["free", "demo", "sight", "mole"].contains(&mode.as_str()) {
        return Err("未知琴盘模式".into());
    }
    let song = if mode == "free" {
        None
    } else {
        Some(
            catalog()
                .0
                .into_iter()
                .find(|s| Some(s.id.as_str()) == song_id.as_deref())
                .ok_or("曲目不存在")?,
        )
    };
    let judgment = judgment.unwrap_or_else(|| "coarse".into());
    music_score::judgment_rank(&judgment).ok_or("未知判定档位")?;
    if matches!(mode.as_str(), "sight" | "mole") && song.as_ref().is_none_or(|s| s.player_count < 4)
    {
        return Err("玩家音符不足4个，本曲可听她示范，或调整21键分工".into());
    }
    let mut s = state.lock().map_err(|_| "琴盘锁不可用")?;
    s.sequence = s.sequence.checked_add(1).ok_or("演奏会话序号超出范围")?;
    let selected = song.clone();
    s.session = Some(Session {
        id: s.sequence,
        mode,
        judgment: judgment.clone(),
        song,
        phase: "playing".into(),
        position: 0.,
        beat_at: Instant::now(),
        started: Instant::now(),
        granted: false,
        grant_at: Instant::now(),
        reason: "等待桌宠回应".into(),
    });
    Ok(StartReply {
        state: s.publish(window.app_handle()),
        song: selected,
        judgment,
    })
}
#[tauri::command]
pub fn music_beat(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Mutex<Service>>,
    id: u64,
    phase: String,
    position: f64,
) -> Result<Snapshot, String> {
    authorize(&window)?;
    if !["playing", "paused"].contains(&phase.as_str()) || !position.is_finite() || position < 0. {
        return Err("无效的演奏状态".into());
    }
    let mut service = state.lock().map_err(|_| "琴盘锁不可用")?;
    let s = service
        .session
        .as_mut()
        .filter(|s| s.id == id)
        .ok_or("演奏会话已经结束")?;
    if window.is_visible().ok() != Some(true) {
        return Err("琴盘窗口未显示".into());
    }
    let limit = s.song.as_ref().map(Song::seconds).unwrap_or(1800.);
    if position > limit + 0.5 {
        return Err("演奏位置超出曲目范围".into());
    }
    s.phase = if s.mode != "demo" && window.is_focused().ok() != Some(true) {
        "paused".into()
    } else {
        phase
    };
    s.position = position;
    s.beat_at = Instant::now();
    Ok(service.publish(window.app_handle()))
}
#[tauri::command]
pub fn music_participation(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Mutex<Service>>,
    id: u64,
    granted: bool,
    reason: String,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只有桌宠调度器可以回应陪演".into());
    }
    let mut service = state.lock().map_err(|_| "琴盘锁不可用")?;
    if let Some(s) = service.session.as_mut().filter(|s| s.id == id) {
        let reason: String = reason.chars().take(80).collect();
        let changed = s.granted != granted || s.reason != reason;
        s.granted = granted;
        s.reason = reason;
        s.grant_at = Instant::now();
        if changed {
            service.publish(window.app_handle());
        }
    }
    Ok(())
}
#[tauri::command]
pub fn music_stop(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Mutex<Service>>,
    id: u64,
) -> Result<(), String> {
    authorize(&window)?;
    let mut s = state.lock().map_err(|_| "琴盘锁不可用")?;
    if s.session.as_ref().is_some_and(|v| v.id == id) {
        s.session = None;
        s.publish(window.app_handle());
    }
    Ok(())
}
#[tauri::command]
pub fn music_finish(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Mutex<Service>>,
    id: u64,
    hits: usize,
    wrong: usize,
) -> Result<Outcome, String> {
    authorize(&window)?;
    let mut service = state.lock().map_err(|_| "琴盘锁不可用")?;
    let s = service
        .session
        .as_ref()
        .filter(|s| s.id == id)
        .ok_or("演奏会话已经结束")?;
    let song = s.song.clone().ok_or("自由弹奏不记录通关")?;
    if !["sight", "mole"].contains(&s.mode.as_str())
        || hits > song.player_count
        || wrong > 100000
        || s.phase != "playing"
        || s.beat_at.elapsed().as_secs_f64() > 5.
        || s.position + 0.15 < song.seconds()
        || s.started.elapsed().as_secs_f64() + 0.15 < song.seconds()
    {
        return Err("尚未完成一轮有效练习".into());
    }
    let total = song.player_count;
    let passed = total >= 4 && hits * 10 >= total * 9 && wrong <= 2.max(total / 10);
    let mode = s.mode.clone();
    let judgment = s.judgment.clone();
    let rank = music_score::judgment_rank(&judgment).ok_or("无效判定档位")?;
    if passed {
        // 写盘失败也要清会话并广播，否则前端崩溃时会话要等 5 秒心跳超时才回收
        if let Err(e) = service.write_record(&song, &mode, rank) {
            service.session = None;
            service.publish(window.app_handle());
            return Err(e);
        }
    }
    service.session = None;
    service.publish(window.app_handle());
    if passed {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = window.app_handle().emit("music-cleared", serde_json::json!({"session_id": id, "song_id": song.id, "version": song.version, "title": song.title, "mode": mode, "layout": "lyre21", "arrangement": song.arrangement, "judgment": judgment, "at": at}));
    }
    Ok(Outcome {
        passed,
        saved: passed,
        hits,
        wrong,
        total,
        judgment,
        arrangement: song.arrangement,
    })
}
