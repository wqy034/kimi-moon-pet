//! 文字聊天独立领域：Rust拥有请求、凭据、记录和生活事件的授权入口。
use crate::{
    chat_credentials as credentials,
    chat_profiles::{self, Failure, Profile},
    chat_store::{self, Store},
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::sync::watch;

pub(crate) fn own(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "extension-chat" {
        Ok(())
    } else {
        Err("该窗口不能访问聊天记录或凭据".into())
    }
}
#[derive(Serialize)]
pub struct Snapshot {
    view: String,
    revision: u64,
    generation: u64,
    history_epoch: u64,
    current: String,
    provider: String,
    pub(crate) interactions: bool,
    configured: bool,
    retryable: bool,
    database_path: String,
    error: Option<String>,
    conversations: Vec<chat_store::Conversation>,
    messages: Vec<chat_store::Message>,
    history_has_older: bool,
    active: Option<ActiveView>,
    routing_status: String,
}
#[derive(Serialize)]
struct ActiveView {
    request_id: String,
    message_id: i64,
    source: String,
    text: String,
    connection: String,
    model: String,
}
impl Active {
    fn review(&mut self) -> Result<bool, String> {
        if self.reviewed { return Ok(self.filtered); }
        let (text, rule) = crate::chat_persona::policy()?.review(&self.text);
        self.filtered = rule.is_some();
        if let Some(rule) = rule { crate::chat_persona::log(rule); }
        self.text = text;
        // 先校验完整响应，再取生活回应的一句，避免后半段越界被截断掩盖。
        if self.source == "life" {
            if let Some((at, c)) = self.text.char_indices().find(|(_, c)| ['。','！','？','!','?','\n'].contains(c)) {
                self.text.truncate(at + c.len_utf8());
            }
        }
        self.reviewed = true;
        Ok(self.filtered)
    }
}
pub(crate) struct Active {
    request_id: String,
    message_id: i64,
    source: String,
    text: String,
    usage: Option<Value>,
    cancel: watch::Sender<bool>,
    attempt_id: String,
    connection: String,
    model: String,
    reviewed: bool,
    filtered: bool,
}
pub struct Service {
    pub(crate) store: Result<Store, String>,
    current: String,
    pub(crate) provider: String,
    pub(crate) interactions: bool,
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) history_epoch: u64,
    pub(crate) active: Option<Active>,
    activity: String,
    activity_at: u64,
    pub(crate) fault: Option<String>,
    pub(crate) check_cancel: Option<watch::Sender<bool>>,
    pub(crate) check_id: String,
    view: String,
    last_request: String,
    pub(crate) memory_revision: u64,
    pub(crate) memory_error: Option<String>,
    pub(crate) memory_cancel: Option<watch::Sender<bool>>,
    pub(crate) memory_flush: bool,
    pub(crate) last_user_at: Instant,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) auto_fallback: bool,
    pub(crate) connections_revision: u64,
    routing_status: String,
}
impl Service {
    pub(crate) fn store(&self) -> Result<&Store, String> {
        self.store.as_ref().map_err(Clone::clone)
    }
    pub(crate) fn profile(&self, id: &str) -> Result<&Profile, String> {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| "该API连接不存在，请重新选择".into())
    }
    pub(crate) fn routes(&self) -> Vec<Profile> {
        let mut routes = Vec::new();
        if let Ok(primary) = self.profile(&self.provider) {
            routes.push(primary.clone());
        }
        if self.auto_fallback {
            routes.extend(
                self.profiles
                    .iter()
                    .filter(|p| p.id != self.provider && p.fallback)
                    .cloned(),
            );
        }
        routes
    }
    pub(crate) fn configured(&self) -> bool {
        self.routes().iter().any(Profile::configured)
    }
    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        let (conversations, messages, history_has_older) = if let Ok(store) = &self.store {
            let (messages, has_older) = if self.view == "history" {
                let page=store.history_page(None)?; (page.messages,page.has_older)
            } else { (store.last_message(&self.current)?.into_iter().collect(),false) };
            (store.conversations()?, messages, has_older)
        } else {
            (vec![], vec![], false)
        };
        Ok(Snapshot {
            view: self.view.clone(),
            revision: self.revision,
            generation: self.generation,
            history_epoch: self.history_epoch,
            current: self.current.clone(),
            provider: self.provider.clone(),
            interactions: self.interactions,
            configured: self.configured(),
            retryable: self.input_state()["retryable"].as_bool().unwrap_or(false),
            database_path: crate::adapters::state_file::pet_home()
                .join("聊天记录.sqlite3")
                .to_string_lossy()
                .into(),
            error: self.store.as_ref().err().cloned().or(self.fault.clone()),
            conversations,
            messages,
            history_has_older,
            active: self.active.as_ref().map(|a| ActiveView {
                request_id: a.request_id.clone(),
                message_id: a.message_id,
                source: a.source.clone(),
                // 未经过整条校验的网络缓冲不能进入任何窗口快照。
                text: String::new(),
                connection: a.connection.clone(),
                model: a.model.clone(),
            }),
            routing_status: self.routing_status.clone(),
        })
    }
    fn activity(&self, completed: bool, text: &str, source: &str) -> Value {
        json!({"revision":self.revision,"generation":self.generation,"request_id":self.active.as_ref().map(|a|a.request_id.as_str()).unwrap_or(&self.last_request),"active":self.active.is_some(),"completed":completed,"source":source,"text":text,"expires_at":chat_store::now()+125})
    }
    pub(crate) fn publish(
        &self,
        app: &tauri::AppHandle,
        completed: bool,
        text: &str,
        source: &str,
    ) {
        let _ = app.emit_to("input-bubble", "input-chat", self.input_state());
        if crate::extension_windows::component_ready("chat") {
            match self.snapshot() {
                Ok(snapshot) => {
                    let _ = app.emit_to("extension-chat", "chat-changed", &snapshot);
                }
                Err(_) => {
                    let _ = app.emit_to(
                        "extension-chat",
                        "chat-storage-error",
                        "聊天数据库读取失败，原文件保留，请退出后备份检查",
                    );
                }
            }
        }
        let _ = app.emit_to(
            "main",
            "chat-activity",
            self.activity(completed, text, source),
        );
    }
    fn input_state(&self) -> Value {
        let last = self
            .store()
            .and_then(|store| store.last_message(&self.current))
            .ok()
            .flatten();
        let retryable = last.as_ref().is_some_and(|last| {
            last.role == "assistant"
                && last.source == "user"
                && ["failed", "cancelled", "interrupted"].contains(&last.status.as_str())
        });
        let reply_error = if retryable {
            last.as_ref().and_then(|m| m.error.as_deref())
        } else {
            None
        };
        json!({"revision":self.revision,"generation":self.generation,"configured":self.configured(),"error":self.store.as_ref().err().or(self.fault.as_ref()),"active":self.active.as_ref().map(|a|json!({"request_id":a.request_id,"source":a.source})),"retryable":retryable,"reply_error":reply_error})
    }
    pub(crate) fn invalidate(&mut self) -> Result<(), String> {
        crate::memory::cancel(self);
        self.generation += 1;
        self.revision += 1;
        if let Some(cancel) = self.check_cancel.take() {
            let _ = cancel.send(true);
        }
        if let Some(mut a) = self.active.take() {
            a.review()?;
            crate::append_log(
                "debug.log",
                &format!(
                    "chat-request id={} source={} stage=cancelled",
                    a.request_id, a.source
                ),
            );
            let _ = a.cancel.send(true);
            if !a.attempt_id.is_empty() {
                self.store()?.attempt_finish(
                    &a.attempt_id,
                    &a.text,
                    "cancelled",
                    &a.usage,
                    Some("已停止"),
                )?;
            }
            if let Err(e) =
                self.store()?
                    .finish(a.message_id, &a.text, "cancelled", &a.usage, Some("已停止"))
            {
                self.fault = Some(e.clone());
                return Err(e);
            }
        }
        Ok(())
    }
}
pub fn install(app: &tauri::AppHandle) {
    let store = Store::open();
    let mut service = Service {
        store,
        current: String::new(),
        provider: "deepseek".into(),
        interactions: true,
        revision: 1,
        generation: 1,
        history_epoch: 1,
        active: None,
        activity: "unknown".into(),
        activity_at: 0,
        fault: None,
        check_cancel: None,
        check_id: String::new(),
        view: "settings".into(),
        last_request: String::new(),
        memory_revision: 1,
        memory_error: None,
        memory_cancel: None,
        memory_flush: false,
        last_user_at: Instant::now(),
        profiles: vec![],
        auto_fallback: false,
        connections_revision: 1,
        routing_status: String::new(),
    };
    let loaded = (|| -> Result<_, String> {
        let store = service.store()?;
        store.discard_input_config()?;
        let p = store
            .config("provider")?
            .unwrap_or_else(|| "deepseek".into());
        let profiles = chat_profiles::load(store)?;
        if !p.is_empty() && !profiles.iter().any(|profile| profile.id == p) {
            return Err("主连接引用无效，原配置保留".into());
        }
        Ok((
            store.current()?,
            p,
            store.config("interactions")?.as_deref() != Some("false"),
            profiles,
            store.config("auto_fallback")?.as_deref() == Some("true"),
        ))
    })();
    match loaded {
        Ok((c, p, i, profiles, auto_fallback)) => {
            service.current = c;
            service.provider = p;
            service.interactions = i;
            service.profiles = profiles;
            service.auto_fallback = auto_fallback;
        }
        Err(e) => service.fault = Some(e),
    }
    crate::append_log(
        "debug.log",
        &format!(
            "chat-installed available={} supported_schema={}",
            service.store.is_ok() && service.fault.is_none(),
            crate::chat_store::SCHEMA_VERSION
        ),
    );
    app.manage(Mutex::new(service));
}
pub fn open_view(app: &tauri::AppHandle, view: &str) -> Result<(), String> {
    if !["memory", "history", "settings"].contains(&view) {
        return Err("未知交流视图".into());
    }
    {
        let state = app.state::<Mutex<Service>>();
        let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
        s.view = view.into();
        s.revision += 1;
        if let Ok(snapshot) = s.snapshot() {
            let _ = app.emit_to("extension-chat", "chat-changed", &snapshot);
        }
    }
    crate::extension_windows::open(app, "chat")
}
pub fn cancel_check(app: &tauri::AppHandle) {
    let state = app.state::<Mutex<Service>>();
    if let Ok(mut s) = state.lock() {
        if let Some(cancel) = s.check_cancel.take() {
            let _ = cancel.send(true);
        }
    };
}
#[tauri::command]
pub fn chat_input_state(window: tauri::WebviewWindow) -> Result<Value, String> {
    crate::input_bubble::own(&window)?;
    let state = window.app_handle().state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    Ok(s.input_state())
}
#[tauri::command]
pub async fn chat_view(window: tauri::WebviewWindow, view: String) -> Result<(), String> {
    if !["input-bubble", "extension-chat", "pet-menu"].contains(&window.label()) {
        return Err("无效的交流入口".into());
    }
    if view == "input" {
        return crate::input_bubble::open(window.app_handle(), true);
    }
    open_view(window.app_handle(), &view)
}
#[tauri::command]
pub fn chat_state(window: tauri::WebviewWindow) -> Result<Snapshot, String> {
    own(&window)?;
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    s.snapshot()
}
#[tauri::command]
pub fn chat_history_page(window: tauri::WebviewWindow, before: i64, history_epoch: u64) -> Result<chat_store::HistoryPage, String> {
    own(&window)?;
    if before <= 0 { return Err("历史记录位置无效".into()); }
    let state = window.app_handle().state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    if s.history_epoch != history_epoch { return Err("交流记录已变化，请重新查看".into()); }
    s.store()?.history_page(Some(before))
}
#[tauri::command]
pub fn chat_activity_state(window: tauri::WebviewWindow) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("只接受桌宠读取表达状态".into());
    }
    let state = window.app_handle().state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    Ok(s.activity(false, "", ""))
}
#[tauri::command]
pub fn chat_pet_context(window: tauri::WebviewWindow, activity: String) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只接受桌宠上报活动".into());
    }
    // 只接受已登记的语义，不接受用户脚本、路径或任意提示词。
    if ![
        "idle",
        "read",
        "walk",
        "sleep",
        "sleep_light",
        "job",
        "music",
        "care",
        "cli",
        "held",
        "falling",
        "touch",
        "attention",
        "chat",
    ]
    .contains(&activity.as_str())
    {
        return Err("未知桌宠活动".into());
    }
    let state = window.app_handle().state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    s.activity = activity;
    s.activity_at = chat_store::now();
    Ok(())
}
#[tauri::command]
pub fn chat_configure(
    window: tauri::WebviewWindow,
    selected: String,
    interactions: bool,
    auto_fallback: bool,
    revision: u64,
) -> Result<Snapshot, String> {
    own(&window)?;
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    s.store()?;
    if revision != s.connections_revision {
        return Err("连接配置已变化，请刷新后重试".into());
    }
    if !selected.is_empty() || !s.profiles.is_empty() {
        s.profile(&selected)?;
    }
    s.invalidate()?;
    // 在发出取消事件之后保存，保存失败也必须把旧生成的撤销通知出去。
    s.publish(app, false, "", "");
    chat_profiles::persist(
        s.store()?,
        &s.profiles,
        &selected,
        auto_fallback,
        interactions,
    )?;
    s.provider = selected;
    s.interactions = interactions;
    s.auto_fallback = auto_fallback;
    s.connections_revision += 1;
    crate::chat_connections::publish(app, &s);
    s.revision += 1;
    s.publish(app, false, "", "");
    s.snapshot()
}
#[tauri::command]
pub fn chat_stop(
    window: tauri::WebviewWindow,
    request_id: Option<String>,
) -> Result<Value, String> {
    if !["input-bubble", "bubble", "extension-chat"].contains(&window.label()) {
        return Err("无效的停止入口".into());
    }
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    // 旧回复上的停止事件不得撤销已经开始的新请求。
    if window.label() == "bubble"
        && !s
            .active
            .as_ref()
            .is_some_and(|a| Some(a.request_id.as_str()) == request_id.as_deref())
    {
        return Ok(s.input_state());
    }
    s.invalidate()?;
    s.publish(app, false, "", "");
    Ok(s.input_state())
}
#[tauri::command]
pub fn chat_conversation(
    window: tauri::WebviewWindow,
    operation: String,
) -> Result<Snapshot, String> {
    own(&window)?;
    if operation != "clear" {
        return Err("未知对话操作".into());
    }
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    s.invalidate()?;
    s.publish(app, false, "", "");
    let current = s.store.as_mut().map_err(|e| e.clone())?.clear()?;
    s.history_epoch += 1;
    s.current = current;
    s.memory_flush = false;
    s.memory_error = None;
    crate::memory::publish(app, &mut s);
    s.revision += 1;
    s.publish(app, false, "", "");
    drop(s);
    let _ = app.emit_to("input-bubble", "input-reset", ());
    chat_state(window)
}
struct Work {
    conversation_id: String,
    request_id: String,
    generation: u64,
    provider: String,
    key: String,
    messages: Vec<Value>,
    input: String,
    source: String,
    cancel: watch::Receiver<bool>,
    memory_epoch: u64,
    memory_enabled: bool,
    routes: Vec<Profile>,
    attempt_id: String,
    route_index: usize,
}
fn begin(
    app: &tauri::AppHandle,
    input: String,
    source: &str,
    expected_generation: Option<u64>,
) -> Result<(), String> {
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    if expected_generation.is_some_and(|g| g != s.generation) {
        return if source == "user" {
            Err("交流状态已更新，请重新发送".into())
        } else {
            Ok(())
        };
    }
    if let Some(error) = &s.fault {
        return Err(error.clone());
    }
    if source == "life"
        && (s.active.is_some()
            || s.check_cancel.is_some()
            || s.memory_cancel.is_some()
            || !s.interactions
            || !s.store()?.event_budget()?)
    {
        return Ok(());
    }
    if source == "user" {
        if s.active.as_ref().is_some_and(|a| a.source == "user") {
            return Err("请先停止当前回复".into());
        }
        s.invalidate()?;
        s.last_user_at = Instant::now();
        s.publish(app, false, "", "");
    }
    let memory_epoch = s.store()?.memory_epoch()?;
    let memory_enabled = s.store()?.memory_enabled()?;
    let routes = s.routes();
    if !routes.iter().any(Profile::configured) {
        return Err("请先保存主连接或已启用备选的Key与模型".into());
    }
    let policy = crate::chat_persona::policy()?;
    let life = crate::life::chat_context(app);
    let activity = if chat_store::now().saturating_sub(s.activity_at) <= 5 {
        s.activity.as_str()
    } else {
        "unknown"
    };
    let messages = policy.messages(s.store()?.context(&s.current)?, &input, source, &life.to_string(), activity);
    if source == "user" {
        if let Some(rule) = policy.input_rule(&input) { crate::chat_persona::log(rule); }
    }
    let request = chat_store::id()?;
    let current = s.current.clone();
    let selected = s.provider.clone();
    let message = s
        .store
        .as_mut()
        .map_err(|e| e.clone())?
        .start(&current, &request, source, &input, &selected)?;
    let (tx, rx) = watch::channel(false);
    s.last_request = request.clone();
    s.active = Some(Active {
        request_id: request.clone(),
        message_id: message,
        source: source.into(),
        text: String::new(),
        usage: None,
        cancel: tx,
        attempt_id: String::new(),
        connection: String::new(),
        model: String::new(),
        reviewed: false,
        filtered: false,
    });
    s.revision += 1;
    let work = Work {
        conversation_id: current,
        request_id: request,
        generation: s.generation,
        provider: selected,
        key: String::new(),
        messages,
        input,
        source: source.into(),
        cancel: rx,
        memory_epoch,
        memory_enabled,
        routes,
        attempt_id: String::new(),
        route_index: 0,
    };
    crate::append_log(
        "debug.log",
        &format!(
            "chat-request id={} generation={} source={} stage=start",
            work.request_id, work.generation, work.source
        ),
    );
    s.publish(app, false, "", source);
    drop(s);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        run(handle, work).await;
    });
    Ok(())
}
#[tauri::command]
pub fn chat_send(
    window: tauri::WebviewWindow,
    text: String,
    generation: u64,
) -> Result<Value, String> {
    crate::input_bubble::own(&window)?;
    let input = text.trim();
    if input.is_empty() || input.chars().count() > 4000 {
        return Err("请输入1～4000字的消息".into());
    }
    begin(window.app_handle(), input.into(), "user", Some(generation))?;
    chat_input_state(window)
}
#[tauri::command]
pub fn chat_retry(window: tauri::WebviewWindow) -> Result<Value, String> {
    own(&window)?;
    let (input, generation) = {
        let state = window.app_handle().state::<Mutex<Service>>();
        let s = state.lock().map_err(|_| "聊天服务不可用")?;
        let last = s.store()?.last_message(&s.current)?.ok_or("没有可重试的回复")?;
        if last.role != "assistant"
            || last.source != "user"
            || !["failed", "cancelled", "interrupted"].contains(&last.status.as_str())
        {
            return Err("仅可重试最后一条未完成的用户回复".into());
        }
        let input = s.store()?.user_text(&s.current, &last.request_id)?.ok_or("找不到原始输入")?;
        (input, s.generation)
    };
    begin(window.app_handle(), input, "user", Some(generation))?;
    let state = window.app_handle().state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    Ok(s.input_state())
}
pub(crate) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "网络客户端初始化失败".into())
}
pub(crate) fn http_error(status: u16) -> String {
    match status {
        401 | 403 => "凭据无效或没有所选模型权限".into(),
        402 => "服务商余额不足".into(),
        404 => "服务商模型或接口不可用".into(),
        429 => "服务商限流或额度不足，请稍后手动重试".into(),
        500..=599 => "服务商暂时不可用，请稍后手动重试".into(),
        _ => format!("服务商返回HTTP {status}，请检查配置"),
    }
}
fn update(
    app: &tauri::AppHandle,
    work: &Work,
    delta: &str,
    usage: Option<Value>,
) -> Result<(), String> {
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    if s.generation != work.generation || s.current != work.conversation_id {
        return Err("请求已撤销".into());
    }
    let Some(a) = s
        .active
        .as_mut()
        .filter(|a| a.request_id == work.request_id && a.attempt_id == work.attempt_id)
    else {
        return Err("请求已撤销".into());
    };
    if a.text.len() + delta.len() > 65536 {
        return Err("回复超过首版显示上限".into());
    }
    a.text.push_str(delta);
    if !delta.is_empty() { a.reviewed = false; }
    if usage.is_some() {
        a.usage = usage;
    }
    // 网络仍流式接收；整条结算前不向气泡、历史或快照发布正文。
    Ok(())
}
// SSE按字节积累完整行，避免UTF-8跨网络包切分；支持CRLF、注释、空包和多行data。
#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
    data: String,
    done: bool,
    finished: bool,
}
impl Sse {
    fn event(&mut self, app: &tauri::AppHandle, work: &Work) -> Result<(), Failure> {
        if self.data.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.data);
        if data.trim() == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let v: Value = serde_json::from_str(&data).map_err(|_| "服务商流式数据格式无效")?;
        if !v["error"].is_null() {
            let code = v["error"]["code"]
                .as_u64()
                .or_else(|| v["error"]["code"].as_str().and_then(|s| s.parse().ok()));
            return Err(match code.filter(|n| (100..=599).contains(n)) {
                Some(code) => Failure::http(code as u16),
                None => Failure::permanent("服务商在流式回复中返回错误"),
            });
        }
        let choice = &v["choices"][0];
        let delta = choice["delta"]["content"].as_str().unwrap_or("");
        update(
            app,
            work,
            delta,
            v.get("usage").filter(|v| v.is_object()).cloned(),
        )?;
        if let Some(reason) = choice["finish_reason"].as_str() {
            if reason != "stop" {
                return Err(if reason == "length" {
                    "回复达到长度上限，可手动重试"
                } else {
                    "服务商未正常完成文字回复"
                }
                .into());
            }
            self.finished = true;
        }
        Ok(())
    }
    fn push(&mut self, chunk: &[u8], app: &tauri::AppHandle, work: &Work) -> Result<(), Failure> {
        self.pending.extend_from_slice(chunk);
        if self.pending.len() > 1024 * 1024 {
            return Err("服务商单个数据包过大".into());
        }
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let bytes: Vec<_> = self.pending.drain(..=end).collect();
            let line = std::str::from_utf8(&bytes[..bytes.len() - 1])
                .map_err(|_| "流式文本编码无效")?
                .trim_end_matches('\r');
            if line.is_empty() {
                self.event(app, work)?;
            } else if let Some(data) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data.strip_prefix(' ').unwrap_or(data));
                if self.data.len() > 1024 * 1024 {
                    return Err("服务商单条事件过大".into());
                }
            }
            if self.done {
                break;
            }
        }
        Ok(())
    }
}
async fn stream(app: &tauri::AppHandle, work: &Work) -> Result<(), Failure> {
    let profile = &work.routes[work.route_index];
    let body = profile.body(
        &work.messages,
        true,
        if work.source == "life" { 128 } else { 1024 },
        false,
    );
    let request = client()
        .map_err(Failure::permanent)?
        .post(format!("{}/chat/completions", profile.base_url))
        .bearer_auth(&work.key)
        .header("X-Request-ID", &work.request_id)
        .header("X-Conversation-ID", &work.conversation_id)
        .json(&body)
        .send();
    let mut response = tokio::time::timeout(Duration::from_secs(30), request)
        .await
        .map_err(|_| Failure::transient("等待服务商响应超过30秒"))?
        .map_err(|_| Failure::transient("请求失败，请检查网络或代理设置"))?;
    if !response.status().is_success() {
        return Err(Failure::http(response.status().as_u16()));
    }
    let mut sse = Sse::default();
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(30), response.chunk())
            .await
            .map_err(|_| Failure::transient("等待回复超过30秒"))?
            .map_err(|_| Failure::transient("回复连接中断"))?;
        match chunk {
            Some(bytes) => {
                sse.push(&bytes, app, work)?;
                if sse.done {
                    break;
                }
            }
            None => {
                // 允许最后一条data没有换行；仍必须有正常的完成标记。
                if !sse.pending.is_empty() {
                    sse.push(b"\n", app, work)?;
                }
                sse.event(app, work)?;
                break;
            }
        }
    }
    if !sse.finished {
        return Err(Failure::transient(
            "回复流提前结束，内容未完整保存为有效上下文",
        ));
    }
    Ok(())
}
fn start_attempt(app: &tauri::AppHandle, work: &mut Work) -> Result<(), Failure> {
    let profile = &work.routes[work.route_index];
    let state = app.state::<Mutex<Service>>();
    let mut s = state
        .lock()
        .map_err(|_| Failure::permanent("聊天服务不可用"))?;
    if s.generation != work.generation
        || s.current != work.conversation_id
        || !s
            .active
            .as_ref()
            .is_some_and(|a| a.request_id == work.request_id)
    {
        return Err(Failure::permanent("请求已撤销"));
    }
    work.attempt_id = chat_store::id().map_err(Failure::permanent)?;
    work.provider = profile.id.clone();
    let message = s.active.as_ref().unwrap().message_id;
    s.store()
        .and_then(|store| store.attempt_start(&work.attempt_id, &work.request_id, profile))
        .map_err(Failure::permanent)?;
    s.store()
        .and_then(|store| store.message_provider(message, &profile.id))
        .map_err(Failure::permanent)?;
    let a = s.active.as_mut().unwrap();
    a.attempt_id = work.attempt_id.clone();
    a.connection = profile.name.clone();
    a.model = profile.model.clone();
    a.usage = None;
    s.routing_status = format!(
        "{}{} · {}",
        if work.route_index == 0 {
            "正在使用 "
        } else {
            "已切换备选 "
        },
        profile.name,
        profile.model
    );
    s.revision += 1;
    s.publish(app, false, "", &work.source);
    drop(s);
    if !chat_profiles::valid_model(&profile.model) {
        return Err(Failure::transient("该连接尚未设置模型"));
    }
    work.key = credentials::load(&profile.id, &profile.base_url).map_err(Failure::transient)?;
    Ok(())
}
async fn routed_stream(app: &tauri::AppHandle, work: &mut Work) -> Result<(), String> {
    for index in 0..work.routes.len() {
        if *work.cancel.borrow() {
            return Err("请求已撤销".into());
        }
        work.route_index = index;
        let mut result = match start_attempt(app, work) {
            Ok(()) => stream(app, work).await,
            Err(error) => Err(error),
        };
        if result.is_ok() {
            let state = app.state::<Mutex<Service>>();
            let s = state.lock().map_err(|_| "聊天服务不可用")?;
            if s.active
                .as_ref()
                .is_some_and(|a| a.request_id == work.request_id && a.text.trim().is_empty())
            {
                result = Err(Failure::transient("服务商返回空回复"));
            }
        }
        work.key.clear();
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                let state = app.state::<Mutex<Service>>();
                let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
                if s.generation != work.generation
                    || !s
                        .active
                        .as_ref()
                        .is_some_and(|a| a.request_id == work.request_id)
                {
                    return Err("请求已撤销".into());
                }
                let has_text = !s.active.as_ref().unwrap().text.is_empty();
                s.active.as_mut().unwrap().review()?;
                let a = s.active.as_ref().unwrap();
                s.store()?.attempt_finish(
                    &work.attempt_id,
                    &a.text,
                    "failed",
                    &a.usage,
                    Some(&error.message),
                )?;
                if !error.fallback || has_text || index + 1 == work.routes.len() {
                    return Err(error.message);
                }
                s.routing_status = format!(
                    "{}：{}，正在尝试下一组备选",
                    work.routes[index].name, error.message
                );
                s.revision += 1;
                s.publish(app, false, "", &work.source);
            }
        }
    }
    Err("没有可用API连接".into())
}
async fn run(app: tauri::AppHandle, mut work: Work) {
    if work.source == "user" && work.memory_enabled {
        let query = crate::memory_model::query(&app, work.input.clone()).await;
        let memory = (|| -> Result<String, String> {
            let state = app.state::<Mutex<Service>>();
            let s = state.lock().map_err(|_| "聊天服务不可用")?;
            if s.generation != work.generation || s.store()?.memory_epoch()? != work.memory_epoch {
                return Ok(String::new());
            }
            s.store()?.memory_recall(&work.input, query.as_deref())
        })();
        match memory {
            Ok(prefix) => {
                if !prefix.is_empty() {
                    if let Err(error) = crate::chat_persona::policy().and_then(|p| p.attach_memory(&mut work.messages, &prefix)) { crate::memory::set_error(&app, error); }
                }
            }
            Err(error) => crate::memory::set_error(&app, error),
        }
    }
    let mut cancel = work.cancel.clone();
    if *cancel.borrow() {
        return;
    }
    let result = tokio::select! {
        biased;
        _=cancel.changed()=>Err("请求已撤销".into()),
        result=tokio::time::timeout(Duration::from_secs(120),routed_stream(&app,&mut work))=>result.unwrap_or_else(|_|Err("本次回复超过120秒，请手动重试".into()))
    };
    // 尽早释放Key；不输出网络错误原文，避免错误对象带请求信息。
    work.key.clear();
    let state = app.state::<Mutex<Service>>();
    let Ok(mut s) = state.lock() else { return };
    if s.generation != work.generation
        || s.current != work.conversation_id
        || !s
            .active
            .as_ref()
            .is_some_and(|a| a.request_id == work.request_id)
    {
        return;
    }
    let mut a = s.active.take().unwrap();
    let filtered = match a.review() {
        Ok(filtered) => filtered,
        Err(error) => { s.fault = Some(error); a.text.clear(); false }
    };
    let result = if result.is_ok() && a.text.trim().is_empty() {
        Err("服务商返回空回复".into())
    } else {
        result
    };
    let status = if result.is_err() { "failed" } else if filtered { "filtered" } else { "complete" };
    let error = result.as_ref().err().map(String::as_str);
    let saved = s.store().and_then(|store| {
        if !a.attempt_id.is_empty() {
            store.attempt_finish(&a.attempt_id, &a.text, status, &a.usage, error)?;
        }
        store.finish(a.message_id, &a.text, status, &a.usage, error)
    });
    if let Err(e) = saved {
        s.fault = Some(e);
    }
    s.revision += 1;
    let completed = result.is_ok() && s.fault.is_none();
    s.routing_status = format!(
        "{} · {} · {}",
        a.connection,
        a.model,
        if completed { "已完成" } else { "未完成" }
    );
    crate::append_log(
        "debug.log",
        &format!(
            "chat-request id={} generation={} source={} stage={}",
            work.request_id,
            work.generation,
            work.source,
            if completed { "complete" } else { "failed" }
        ),
    );
    s.publish(&app, completed, &a.text, &a.source);
    if completed && !filtered && work.source == "user" {
        let captured = s
            .store
            .as_mut()
            .map_err(|e| e.clone())
            .and_then(Store::memory_capture);
        if let Err(error) = captured {
            s.memory_error = Some(error);
        }
        crate::memory::publish(&app, &mut s);
    }
}
pub fn queue_life_event(
    app: &tauri::AppHandle,
    revision: u64,
    action: String,
    item: Option<String>,
) {
    if !["use", "start_job", "claim_job"].contains(&action.as_str()) {
        return;
    }
    let (generation, conversation) = {
        let state = app.state::<Mutex<Service>>();
        let Ok(s) = state.lock() else { return };
        (s.generation, s.current.clone())
    };
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        life_event(
            &handle,
            revision,
            &action,
            item.as_deref(),
            generation,
            &conversation,
        )
    });
}
fn life_event(
    app: &tauri::AppHandle,
    revision: u64,
    action: &str,
    item: Option<&str>,
    generation: u64,
    conversation: &str,
) {
    if !["use", "start_job", "claim_job"].contains(&action) {
        return;
    }
    let item_name = match item {
        Some("bread") => "面包",
        Some("water") => "水",
        Some("cake") => "蛋糕",
        _ => "背包中的物品",
    };
    let input = match action {
        "use" => format!("用户成功给桌宠使用了物品：{item_name}。"),
        "start_job" => "用户安排了整理书架，任务已经成功创建，尚未完成。".into(),
        _ => "用户成功领取了已完成整理书架任务的奖励，金币已按本地规则结算。".into(),
    };
    let state = app.state::<Mutex<Service>>();
    let Ok(mut s) = state.lock() else { return };
    if s.current != conversation {
        return;
    }
    // 即使关闭开关或正在聊天也消费事件，不在重新开启时补发。
    match s
        .store
        .as_mut()
        .map_err(|e| e.clone())
        .and_then(|store| store.accept_event(revision, &input))
    {
        Ok(true) => crate::memory::publish(app, &mut s),
        Ok(false) => return,
        Err(error) => {
            s.memory_error = Some(error);
            crate::memory::publish(app, &mut s);
            return;
        }
    }
    let Ok(store) = s.store() else { return };
    if generation != s.generation
        || !s.interactions
        || s.active.is_some()
        || !s.configured()
        || store.event_budget() != Ok(true)
    {
        return;
    }
    drop(s);
    // begin内部再次检查独占；生活事件调用在原生事务成功之后，绝不依赖前端广播触发。
    if let Err(error) = begin(app, input, "life", Some(generation)) {
        crate::append_log("debug.log", &format!("chat-life-event skipped: {error}"));
    }
}
