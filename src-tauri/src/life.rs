//! 养成领域：唯一写入者；先落盘再发布内存状态，金币与用量游标一起提交。
use crate::{adapters::state_file::pet_home, life_usage::Usage};
use serde::{Deserialize, Serialize};
use std::os::windows::fs::OpenOptionsExt;
use std::{
    collections::BTreeMap,
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};

static CLI_BUSY: AtomicBool = AtomicBool::new(false);
pub fn set_cli_busy(busy: bool) {
    CLI_BUSY.store(busy, Ordering::Relaxed);
}
fn epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn day() -> String {
    let mut t = windows_sys::Win32::Foundation::SYSTEMTIME::default();
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t);
    }
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Stats {
    food: f64,
    water: f64,
    energy: f64,
    affection: f64,
}
impl Stats {
    fn valid(&self) -> bool {
        [self.food, self.water, self.energy, self.affection]
            .iter()
            .all(|n| n.is_finite() && (0.0..=100.0).contains(n))
    }
    fn clamp(&mut self) {
        self.food = self.food.clamp(0., 100.);
        self.water = self.water.clamp(0., 100.);
        self.energy = self.energy.clamp(0., 100.);
        self.affection = self.affection.clamp(0., 100.);
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Item {
    id: String,
    name: String,
    icon: String,
    price: u64,
    starter: u32,
    effect: Stats,
    line: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct JobRule {
    id: String,
    name: String,
    duration_seconds: f64,
    energy_cost: f64,
    minimum_energy: f64,
    reward: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Rules {
    tokens_per_coin: u64,
    starter_coins: u64,
    initial: Stats,
    decay_per_hour: Stats,
    sleep_energy_per_hour: f64,
    offline_hours_cap: f64,
    offline_need_floor: f64,
    daily_work_coins_cap: u64,
    daily_affection_cap: f64,
    items: Vec<Item>,
    job: JobRule,
}
impl Rules {
    fn valid(&self) -> bool {
        self.tokens_per_coin > 0
            && self.tokens_per_coin <= 1_000_000_000
            && self.starter_coins <= 1_000_000_000
            && self.daily_work_coins_cap <= 1_000_000_000
            && self.initial.valid()
            && self.decay_per_hour.valid()
            && (0.0..=168.0).contains(&self.offline_hours_cap)
            && (0.0..=100.0).contains(&self.offline_need_floor)
            && (0.0..=3600.0).contains(&self.sleep_energy_per_hour)
            && (0.0..=100.0).contains(&self.daily_affection_cap)
            && (10.0..=86400.0).contains(&self.job.duration_seconds)
            && (0.0..=100.0).contains(&self.job.energy_cost)
            && (self.job.energy_cost + 20.0..=100.0).contains(&self.job.minimum_energy)
            && self.job.reward > 0
            && self.job.reward <= self.daily_work_coins_cap
            && !self.items.is_empty()
            && self.items.iter().all(|i| {
                !i.id.is_empty()
                    && i.price > 0
                    && i.price <= 1_000_000_000
                    && i.starter <= 99
                    && i.effect.valid()
            })
            && self
                .items
                .iter()
                .map(|i| &i.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.items.len()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    id: u64,
    rule: JobRule,
    progress: f64,
    ready: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Daily {
    date: String,
    work_coins: u64,
    affection: f64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Transaction {
    at: u64,
    reason: String,
    coins: i64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Save {
    version: u32,
    revision: u64,
    updated_at: u64,
    stats: Stats,
    coins: u64,
    inventory: BTreeMap<String, u32>,
    job: Option<Job>,
    daily: Daily,
    usage: Usage,
    recent: Vec<Transaction>,
    next_job_id: u64,
}
impl Save {
    fn valid(&self, rules: &Rules) -> bool {
        self.version == 1
            && self.stats.valid()
            && self.usage.remainder < rules.tokens_per_coin
            && self.coins <= i64::MAX as u64
            && self.revision < u64::MAX
            && self.next_job_id > 0
            && self.next_job_id < u64::MAX
            && self.inventory.values().all(|n| *n <= 99)
            && self.daily.affection.is_finite()
            && self.daily.affection >= 0.
            && self.job.as_ref().is_none_or(|j| {
                j.progress.is_finite()
                    && j.progress >= 0.
                    && j.rule.duration_seconds.is_finite()
                    && j.rule.duration_seconds > 0.
                    && j.progress <= j.rule.duration_seconds
                    && j.ready == (j.progress >= j.rule.duration_seconds)
                    && j.rule.energy_cost.is_finite()
                    && (0.0..=100.0).contains(&j.rule.energy_cost)
                    && j.rule.reward > 0
                    && j.rule.reward <= i64::MAX as u64
                    && j.id < self.next_job_id
            })
    }
    fn roll_day(&mut self) {
        let today = day();
        // 日期回拨不重置额度；回到未来的新一天才刷新。
        if today > self.daily.date {
            self.daily = Daily {
                date: today,
                work_coins: 0,
                affection: 0.,
            };
        }
    }
    fn affection(&mut self, n: f64, rules: &Rules) {
        let gain = n.min((rules.daily_affection_cap - self.daily.affection).max(0.));
        self.stats.affection += gain;
        self.daily.affection += gain;
    }
    fn log(&mut self, reason: String, coins: i64) {
        self.recent.push(Transaction {
            at: epoch(),
            reason,
            coins,
        });
        if self.recent.len() > 20 {
            self.recent.remove(0);
        }
    }
}
#[derive(Default)]
struct Runtime {
    mode: String,
    job_id: Option<u64>,
    reported: Option<Instant>,
    work_seconds: f64,
    sleep_seconds: f64,
}
pub struct Service {
    // Windows句柄独占同目录锁；进程退出自动释放，不依赖删除锁文件。
    _lease: Option<fs::File>,
    rules: Rules,
    save: Save,
    runtime: Runtime,
    error: Option<String>,
    warning: Option<String>,
    last_tick: Instant,
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    revision: u64,
    stats: Stats,
    coins: u64,
    inventory: BTreeMap<String, u32>,
    job: Option<Job>,
    rules: Rules,
    work_coins_today: u64,
    token_total: u64,
    token_remainder: u64,
    usage_records: u64,
    error: Option<String>,
    warning: Option<String>,
    recent: Vec<Transaction>,
    activity: String,
}
#[derive(Serialize, Clone)]
pub struct Feedback {
    id: u64,
    line: String,
    expires_at: u64,
    action: String,
    item: Option<String>,
}
impl Service {
    fn load() -> Self {
        let (lease, lease_error) = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(pet_home().join("养成存档.lock"))
        {
            Ok(file) => (Some(file), None),
            Err(e) => (
                None,
                Some(format!(
                    "养成存档已被其他实例占用或无法加锁：{e}。请关闭多余桌宠后重启。"
                )),
            ),
        };
        let defaults: Rules =
            serde_json::from_str(include_str!("../../config/养成.json")).expect("内置养成配置错误");
        let path = pet_home().join("config/养成.json");
        let mut warning = None;
        let rules = match fs::read(&path) {
            Ok(b) => match serde_json::from_slice::<Rules>(&b) {
                Ok(r) if r.valid() => r,
                _ => {
                    warning = Some("养成配置无效，本轮使用内置默认值。".into());
                    defaults
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => defaults,
            Err(e) => {
                warning = Some(format!("养成配置读取失败：{e}；本轮使用内置默认值。"));
                defaults
            }
        };
        let mut save = Save {
            version: 1,
            revision: 0,
            updated_at: epoch(),
            stats: rules.initial.clone(),
            coins: rules.starter_coins,
            inventory: rules
                .items
                .iter()
                .map(|i| (i.id.clone(), i.starter))
                .collect(),
            job: None,
            daily: Daily {
                date: day(),
                work_coins: 0,
                affection: 0.,
            },
            usage: Usage::default(),
            recent: vec![],
            next_job_id: 1,
        };
        let mut error = None;
        let mut fresh = false;
        match crate::life_store::load::<Save>(&pet_home(), |s| s.valid(&rules)) {
            Ok(loaded) => {
                if let Some(stored) = loaded.save {
                    save = stored;
                } else {
                    fresh = true;
                }
                if loaded.warning.is_some() {
                    warning = loaded.warning;
                }
            }
            Err(e) => error = Some(e),
        }
        if error.is_none() {
            error = lease_error;
        }
        let mut service = Self {
            _lease: lease,
            rules,
            save,
            runtime: Runtime::default(),
            error,
            warning,
            last_tick: Instant::now(),
        };
        if service.error.is_none() {
            let mut next = service.save.clone();
            let hours = (epoch().saturating_sub(next.updated_at) as f64 / 3600.)
                .min(service.rules.offline_hours_cap);
            let floor = service.rules.offline_need_floor;
            next.stats.food = (next.stats.food - service.rules.decay_per_hour.food * hours)
                .max(next.stats.food.min(floor));
            next.stats.water = (next.stats.water - service.rules.decay_per_hour.water * hours)
                .max(next.stats.water.min(floor));
            next.stats.energy = (next.stats.energy + hours * 6.).min(100.);
            next.roll_day();
            let tokens = next.usage.poll(epoch() * 1000);
            service.credit(&mut next, tokens);
            if fresh {
                next.log(
                    "初次见面的小礼物".into(),
                    service.rules.starter_coins as i64,
                );
            }
            if let Err(e) = service.commit(next) {
                service.error = Some(e);
            }
        }
        service
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.save.revision,
            stats: self.save.stats.clone(),
            coins: self.save.coins,
            inventory: self.save.inventory.clone(),
            job: self.save.job.clone(),
            rules: self.rules.clone(),
            work_coins_today: self.save.daily.work_coins,
            token_total: self.save.usage.total,
            token_remainder: self.save.usage.remainder,
            usage_records: self.save.usage.records,
            error: self.error.clone(),
            warning: self.warning.clone(),
            recent: self.save.recent.clone(),
            activity: if self
                .runtime
                .reported
                .is_some_and(|t| t.elapsed().as_secs() < 3)
                && !CLI_BUSY.load(Ordering::Relaxed)
            {
                self.runtime.mode.clone()
            } else {
                "paused".into()
            },
        }
    }
    fn commit(&mut self, mut next: Save) -> Result<(), String> {
        next.stats.clamp();
        next.updated_at = epoch().max(next.updated_at);
        next.revision = next.revision.checked_add(1).ok_or("存档版本序号超出范围")?;
        if !next.valid(&self.rules) {
            return Err("养成数据校验失败，未保存".into());
        }
        crate::life_store::persist(&pet_home(), &next, |s| s.valid(&self.rules))
            .map_err(|e| format!("存档保存失败：{e}"))?;
        if let Some(warning) = crate::life_store::retire_legacy(&pet_home()) {
            self.warning = Some(warning);
        }
        self.save = next;
        Ok(())
    }
    fn credit(&self, next: &mut Save, tokens: u64) {
        let total = next.usage.remainder.saturating_add(tokens);
        let coins = total / self.rules.tokens_per_coin;
        next.usage.remainder = total % self.rules.tokens_per_coin;
        if coins > 0 {
            next.coins = next.coins.saturating_add(coins).min(i64::MAX as u64);
            next.log("Kimi 实际用量奖励".into(), coins as i64);
        }
    }
    fn tick(&mut self) -> Result<(), String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let elapsed = self.last_tick.elapsed().as_secs_f64();
        let hours = elapsed.min(60.) / 3600.;
        let mut next = self.save.clone();
        next.roll_day();
        next.stats.food -= hours * self.rules.decay_per_hour.food;
        next.stats.water -= hours * self.rules.decay_per_hour.water;
        next.stats.energy -= hours * self.rules.decay_per_hour.energy;
        next.stats.affection -= hours * self.rules.decay_per_hour.affection;
        next.stats.energy += self.runtime.sleep_seconds
            * (self.rules.sleep_energy_per_hour + self.rules.decay_per_hour.energy)
            / 3600.;
        if let Some(job) = &mut next.job {
            if !job.ready && next.stats.energy >= 20. {
                let seconds = self
                    .runtime
                    .work_seconds
                    .min(job.rule.duration_seconds - job.progress);
                job.progress += seconds;
                next.stats.energy -= seconds * job.rule.energy_cost / job.rule.duration_seconds;
                job.ready = job.progress >= job.rule.duration_seconds;
            }
        }
        let tokens = next.usage.poll(epoch() * 1000);
        self.credit(&mut next, tokens);
        self.commit(next)?;
        self.runtime.work_seconds = 0.;
        self.runtime.sleep_seconds = 0.;
        self.last_tick = Instant::now();
        Ok(())
    }
    fn action(&mut self, action: &str, item: Option<&str>) -> Result<Option<String>, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let mut next = self.save.clone();
        next.roll_day();
        let mut feedback = None;
        match action {
            "buy" => {
                let item = self
                    .rules
                    .items
                    .iter()
                    .find(|i| Some(i.id.as_str()) == item)
                    .ok_or("商品不存在")?;
                if next.coins < item.price {
                    return Err("金币不够啦，可以先安排一次整理书架。".into());
                }
                let count = next.inventory.entry(item.id.clone()).or_default();
                if *count >= 99 {
                    return Err("背包里这件物品已经有99份啦。".into());
                }
                *count += 1;
                next.coins -= item.price;
                next.log(format!("购买{}", item.name), -(item.price as i64));
            }
            "use" => {
                let item = self
                    .rules
                    .items
                    .iter()
                    .find(|i| Some(i.id.as_str()) == item)
                    .ok_or("物品不存在")?;
                let count = next.inventory.entry(item.id.clone()).or_default();
                if *count == 0 {
                    return Err("背包里还没有这件物品。".into());
                }
                let s = &next.stats;
                if (item.effect.food == 0. || s.food >= 100.)
                    && (item.effect.water == 0. || s.water >= 100.)
                    && (item.effect.energy == 0. || s.energy >= 100.)
                    && (item.effect.affection == 0.
                        || s.affection >= 100.
                        || next.daily.affection >= self.rules.daily_affection_cap)
                {
                    return Err("现在已经很满足啦，留着下次再用吧。".into());
                }
                *count -= 1;
                next.stats.food += item.effect.food;
                next.stats.water += item.effect.water;
                next.stats.energy += item.effect.energy;
                next.affection(item.effect.affection, &self.rules);
                next.log(format!("使用{}", item.name), 0);
                feedback = Some(item.line.clone());
            }
            "start_job" => {
                if next.job.is_some() {
                    return Err("先完成或领取手头这一份工作吧。".into());
                }
                if next.stats.energy < self.rules.job.minimum_energy {
                    return Err("精力不足，先休息一会儿吧。".into());
                }
                if next.daily.work_coins.saturating_add(self.rules.job.reward)
                    > self.rules.daily_work_coins_cap
                {
                    return Err("今天的打工奖励已经领满啦，明天再来吧。".into());
                }
                next.job = Some(Job {
                    id: next.next_job_id,
                    rule: self.rules.job.clone(),
                    progress: 0.,
                    ready: false,
                });
                next.next_job_id = next.next_job_id.checked_add(1).ok_or("任务序号超出范围")?;
                feedback = Some("我来整理书架。".into());
            }
            "claim_job" => {
                let job = next
                    .job
                    .as_ref()
                    .filter(|j| j.ready)
                    .ok_or("工作还没有完成。")?;
                let reward = job.rule.reward;
                if next.daily.work_coins.saturating_add(reward) > self.rules.daily_work_coins_cap {
                    return Err("今天的打工奖励已满，明天再领取吧。".into());
                }
                next.coins = next
                    .coins
                    .checked_add(reward)
                    .filter(|n| *n <= i64::MAX as u64)
                    .ok_or("金币数量超出范围")?;
                next.daily.work_coins += reward;
                next.job = None;
                next.log("整理书架完成".into(), reward as i64);
                feedback = Some("书架整理好了。".into());
            }
            "cancel_job" => {
                if next.job.as_ref().is_none_or(|j| j.ready) {
                    return Err("当前没有可以取消的工作。".into());
                }
                next.job = None; // 已消耗的精力不退；没有金币结算。
            }
            _ => return Err("未知养成操作".into()),
        }
        self.commit(next)?;
        // 清除上一个任务/睡眠模式的待结算量，避免跨任务继承进度。
        if matches!(action, "start_job" | "claim_job" | "cancel_job") {
            self.runtime.work_seconds = 0.;
            self.runtime.job_id = None;
        }
        Ok(feedback)
    }
}
pub fn install(app: &tauri::AppHandle) {
    app.manage(Mutex::new(Service::load()));
    let handle = app.clone();
    std::thread::spawn(move || {
        // 瞬时 I/O 错误（杀毒软件占用等）不应永久停止结算：连续失败 12 次（约 60 秒）才挂起服务
        let mut consecutive_failures = 0u32;
        let mut lock_failure_logged = false;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let state = handle.state::<Mutex<Service>>();
            let Ok(mut service) = state.lock() else {
                // 锁中毒说明曾 panic 在持锁期间；每次空转前至少留一条日志
                if !lock_failure_logged {
                    lock_failure_logged = true;
                    crate::append_log("debug.log", "life-lock-poisoned, settlement paused");
                }
                continue;
            };
            lock_failure_logged = false;
            if service.error.is_some() {
                continue;
            }
            if let Err(e) = service.tick() {
                consecutive_failures += 1;
                crate::append_log(
                    "debug.log",
                    &format!("life-save-error {e} (consecutive {consecutive_failures})"),
                );
                if consecutive_failures >= 12 {
                    service.error = Some(format!("连续提交失败：{e}"));
                }
            } else {
                consecutive_failures = 0;
            }
            let snapshot = service.snapshot();
            drop(service);
            let _ = handle.emit("life-changed", snapshot);
        }
    });
}
#[tauri::command]
pub fn life_get(state: tauri::State<'_, Mutex<Service>>) -> Result<Snapshot, String> {
    Ok(state.lock().map_err(|_| "养成服务不可用")?.snapshot())
}
#[tauri::command]
pub fn life_action(
    app: tauri::AppHandle,
    state: tauri::State<'_, Mutex<Service>>,
    action: String,
    item: Option<String>,
) -> Result<Snapshot, String> {
    let mut service = state.lock().map_err(|_| "养成服务不可用")?;
    let feedback = service.action(&action, item.as_deref())?;
    let snapshot = service.snapshot();
    drop(service);
    let _ = app.emit("life-changed", &snapshot);
    if let Some(line) = feedback {
        let _ = app.emit(
            "life-feedback",
            Feedback {
                id: snapshot.revision,
                line,
                expires_at: epoch() + 30,
                action: action.clone(),
                item: item.clone(),
            },
        );
    }
    super::chat::queue_life_event(&app, snapshot.revision, action, item);
    Ok(snapshot)
}

/// 提供给聊天的只读、有限状态；不暴露完整日志、配置或存档内容。
pub fn chat_context(app: &tauri::AppHandle) -> serde_json::Value {
    let state = app.state::<Mutex<Service>>();
    let Ok(service) = state.lock() else { return serde_json::json!({"available":false}); };
    let snapshot = service.snapshot();
    serde_json::json!({"available":snapshot.error.is_none(),"revision":snapshot.revision,
        "stats":snapshot.stats,"coins":snapshot.coins,"inventory":snapshot.inventory,
        "job":snapshot.job,"activity":snapshot.activity})
}
#[tauri::command]
pub fn life_mode(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Mutex<Service>>,
    mode: String,
    job_id: Option<u64>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只接受主窗口的活动报告".into());
    }
    if !["job", "sleep", "awake", "paused"].contains(&mode.as_str()) {
        return Err("未知养成活动".into());
    }
    let mut service = state.lock().map_err(|_| "养成服务不可用")?;
    let dt = service
        .runtime
        .reported
        .map(|t| t.elapsed().as_secs_f64())
        .unwrap_or(0.);
    // 只计两次连续报告夹住的时间；休眠/页面挂起不补发打工进度。
    if dt <= 2.5 && service.error.is_none() && !CLI_BUSY.load(Ordering::Relaxed) {
        if mode == "job"
            && service.runtime.mode == "job"
            && service.runtime.job_id == job_id
            && service
                .save
                .job
                .as_ref()
                .is_some_and(|j| Some(j.id) == job_id && !j.ready)
        {
            service.runtime.work_seconds += dt;
        }
        if mode == "sleep" && service.runtime.mode == "sleep" {
            service.runtime.sleep_seconds += dt;
        }
    }
    service.runtime.mode = mode;
    service.runtime.job_id = job_id;
    service.runtime.reported = Some(Instant::now());
    Ok(())
}
