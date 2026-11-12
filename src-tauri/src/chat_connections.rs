//! 设置页专用：连接管理、脱敏凭据和模型目录。查询不会跨服务商。
use crate::{
    chat::{self, Service},
    chat_credentials as credentials,
    chat_profiles::{self, Draft, Preset, Profile, ProfileView},
};
use serde::Serialize;
use serde_json::Value;
use std::{sync::Mutex, time::Duration};
use tauri::{Emitter, Manager};
use tokio::sync::watch;

#[derive(Serialize)]
pub struct State {
    revision: u64,
    selected: String,
    auto_fallback: bool,
    interactions: bool,
    profiles: Vec<ProfileView>,
    presets: Vec<Preset>,
}
fn snapshot(s: &Service) -> State {
    State {
        revision: s.connections_revision,
        selected: s.provider.clone(),
        auto_fallback: s.auto_fallback,
        interactions: s.interactions,
        profiles: s.profiles.iter().map(Profile::view).collect(),
        presets: chat_profiles::presets(),
    }
}
pub(crate) fn publish(app: &tauri::AppHandle, s: &Service) {
    let _ = app.emit_to("extension-chat", "chat-connections", &snapshot(s));
}
#[tauri::command]
pub fn chat_connections(window: tauri::WebviewWindow) -> Result<State, String> {
    chat::own(&window)?;
    let state = window.app_handle().state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    s.store()?;
    Ok(snapshot(&s))
}
#[tauri::command]
pub fn chat_connection_save(
    window: tauri::WebviewWindow,
    draft: Draft,
    key: Option<String>,
    primary: bool,
    revision: u64,
) -> Result<State, String> {
    chat::own(&window)?;
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    s.store()?;
    if s.fault.is_some() {
        return Err("聊天配置或数据库异常，请先处理现有错误".into());
    }
    if s.connections_revision != revision {
        return Err("连接配置已变化，请刷新后再保存".into());
    }
    let id = match &draft.id {
        Some(id) => {
            s.profile(id)?;
            id.clone()
        }
        None => chat_profiles::fresh_id()?,
    };
    let previous = s.profiles.iter().find(|p| p.id == id).cloned();
    if previous.is_none() && s.profiles.len() >= 16 {
        return Err("最多保存16组API连接".into());
    }
    let mut profile = Profile::from_draft(draft, id.clone())?;
    let key = key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    if previous
        .as_ref()
        .is_some_and(|p| p.base_url != profile.base_url)
        && credentials::exists(&id)
        && key.is_none()
    {
        return Err("修改API地址后请重新填写Key，避免旧凭据被发送到其他接口".into());
    }
    if let Some(old) = &previous {
        if old.base_url == profile.base_url && key.is_none() {
            profile.models = old.models.clone();
            profile.models_at = old.models_at;
            profile.key_hint = old.key_hint.clone();
        }
    }
    s.invalidate()?;
    s.publish(app, false, "", "");
    if let Some(key) = key {
        credentials::save(&id, &profile.base_url, key)?;
        profile.key_hint = credentials::mask(key);
    }
    let mut profiles = s.profiles.clone();
    if let Some(index) = profiles.iter().position(|p| p.id == id) {
        profiles[index] = profile;
    } else {
        profiles.push(profile);
    }
    let selected = if primary || s.provider.is_empty() {
        id
    } else {
        s.provider.clone()
    };
    chat_profiles::persist(
        s.store()?,
        &profiles,
        &selected,
        s.auto_fallback,
        s.interactions,
    )?;
    s.profiles = profiles;
    s.provider = selected;
    s.connections_revision += 1;
    s.revision += 1;
    s.publish(app, false, "", "");
    publish(app, &s);
    crate::memory::publish(app, &mut s);
    Ok(snapshot(&s))
}
#[tauri::command]
pub fn chat_connection_action(
    window: tauri::WebviewWindow,
    id: String,
    operation: String,
    revision: u64,
) -> Result<State, String> {
    chat::own(&window)?;
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    if s.connections_revision != revision {
        return Err("连接配置已变化，请查看最新内容后再操作".into());
    }
    let index = s
        .profiles
        .iter()
        .position(|p| p.id == id)
        .ok_or("该连接已不存在")?;
    let mut profiles = s.profiles.clone();
    let mut selected = s.provider.clone();
    match operation.as_str() {
        "primary" => selected = id.clone(),
        "fallback" => profiles[index].fallback = !profiles[index].fallback,
        "up" if index > 0 => profiles.swap(index, index - 1),
        "down" if index + 1 < profiles.len() => profiles.swap(index, index + 1),
        "up" | "down" => return Ok(snapshot(&s)),
        "delete" => {
            profiles.remove(index);
            if selected == id {
                selected = profiles.first().map(|p| p.id.clone()).unwrap_or_default();
            }
        }
        _ => return Err("未知连接操作".into()),
    }
    s.invalidate()?;
    s.publish(app, false, "", "");
    chat_profiles::persist(
        s.store()?,
        &profiles,
        &selected,
        s.auto_fallback,
        s.interactions,
    )?;
    s.profiles = profiles;
    s.provider = selected;
    s.connections_revision += 1;
    s.revision += 1;
    s.publish(app, false, "", "");
    publish(app, &s);
    crate::memory::publish(app, &mut s);
    if operation == "delete" {
        credentials::remove(&id)?;
    }
    Ok(snapshot(&s))
}
#[tauri::command]
pub async fn chat_models(
    window: tauri::WebviewWindow,
    id: String,
    revision: u64,
) -> Result<State, String> {
    chat::own(&window)?;
    let (profile, mut key, generation, check_id, mut cancel) = {
        let state = window.app_handle().state::<Mutex<Service>>();
        let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
        if revision != s.connections_revision {
            return Err("连接配置已变化，请刷新后再查询".into());
        }
        let profile = s.profile(&id)?.clone();
        let key = credentials::load(&id, &profile.base_url)?;
        if let Some(old) = s.check_cancel.take() {
            let _ = old.send(true);
        }
        let (tx, rx) = watch::channel(false);
        s.check_cancel = Some(tx);
        let check_id = crate::chat_store::id()?;
        s.check_id = check_id.clone();
        (profile, key, s.generation, check_id, rx)
    };
    let request = async {
        let mut response = chat::client()?
            .get(format!("{}/models", profile.base_url))
            .bearer_auth(&key)
            .send()
            .await
            .map_err(|_| "模型查询连接失败，请检查API地址和网络")?;
        if response.status().as_u16() == 404 {
            return Err("该接口未提供模型查询，可直接手填模型ID并保存".into());
        }
        if !response.status().is_success() {
            return Err(chat::http_error(response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "模型列表传输中断")? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("模型列表过大，请手填模型ID".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&bytes).map_err(|_| "模型列表格式无效")?;
        let data = body["data"]
            .as_array()
            .ok_or("接口没有返回OpenAI兼容的模型列表，可手填模型ID")?;
        let mut models: Vec<String> = data
            .iter()
            .filter_map(|m| m["id"].as_str())
            .filter(|m| chat_profiles::valid_model(m))
            .take(2000)
            .map(str::to_owned)
            .collect();
        models.sort();
        models.dedup();
        if models.is_empty() {
            return Err("接口未返回可用模型ID，可手填模型后保存".into());
        }
        Ok(models)
    };
    let result: Result<Vec<String>, String> = tokio::select! {biased;_=cancel.changed()=>Err("模型查询已取消".into()),value=tokio::time::timeout(Duration::from_secs(15),request)=>value.unwrap_or_else(|_|Err("模型查询超过15秒，请检查连接后重试".into()))};
    key.clear();
    let state = window.app_handle().state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    if s.generation != generation || s.connections_revision != revision || s.check_id != check_id {
        return Err("模型查询已过期，未保存迟到列表".into());
    }
    s.check_cancel = None;
    let models = result?;
    let mut profiles = s.profiles.clone();
    let profile = profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("连接已删除")?;
    profile.models = models;
    profile.models_at = crate::chat_store::now();
    chat_profiles::persist(
        s.store()?,
        &profiles,
        &s.provider,
        s.auto_fallback,
        s.interactions,
    )?;
    s.profiles = profiles;
    s.connections_revision += 1;
    publish(window.app_handle(), &s);
    Ok(snapshot(&s))
}
