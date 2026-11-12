//! 有来源的记忆浏览、纠错、删除与本地向量索引。
use crate::{
    chat::Service,
    memory_model,
    memory_store::{Entry, JobView, Source},
};
use serde::Serialize;
use std::{sync::Mutex, time::Duration};
use tauri::{Emitter, Manager};

#[derive(Serialize)]
pub struct Snapshot {
    revision: u64,
    enabled: bool,
    model_status: String,
    entries: Vec<Entry>,
    jobs: Vec<JobView>,
    waiting: u64,
    error: Option<String>,
    connection_names: std::collections::HashMap<String, String>,
}
fn own(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "extension-chat" {
        Ok(())
    } else {
        Err("该窗口不能浏览或修改长期记忆".into())
    }
}
fn snapshot(app: &tauri::AppHandle, s: &Service) -> Result<Snapshot, String> {
    let store = s.store()?;
    let state = app.state::<memory_model::Embeddings>();
    let status = state
        .status
        .lock()
        .map_err(|_| "本地模型状态不可用")?
        .clone();
    let waiting = store
        .db
        .query_row(
            "SELECT count(*) FROM memory_sources WHERE job_id IS NULL AND blocked=0",
            [],
            |r| r.get(0),
        )
        .map_err(crate::chat_store::db_error)?;
    Ok(Snapshot {
        revision: s.memory_revision,
        enabled: store.memory_enabled()?,
        model_status: status,
        entries: store.memory_entries()?,
        jobs: store.memory_jobs()?,
        waiting,
        error: s.memory_error.clone(),
        connection_names: s
            .profiles
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect(),
    })
}
pub(crate) fn publish(app: &tauri::AppHandle, s: &mut Service) {
    s.memory_revision += 1;
    match snapshot(app, s) {
        Ok(view) => {
            let _ = app.emit_to("extension-chat", "memory-changed", &view);
        }
        Err(_) => {
            let _ = app.emit_to(
                "extension-chat",
                "memory-error",
                "记忆数据库读取失败，原数据保留，请退出后备份检查",
            );
        }
    }
}
pub fn cancel(s: &mut Service) {
    if let Some(tx) = s.memory_cancel.take() {
        let _ = tx.send(true);
    }
}
pub fn install(app: &tauri::AppHandle) {
    memory_model::install(app);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let ready = {
                let state = handle.state::<Mutex<Service>>();
                state.lock().ok().is_some_and(|s| {
                    s.fault.is_none() && s.store().and_then(|db| db.memory_enabled()) == Ok(true)
                })
            };
            if ready {
                memory_model::load(&handle, false);
                index(&handle).await;
                crate::memory_worker::tick(&handle).await;
            }
        }
    });
}
#[tauri::command]
pub fn memory_state(window: tauri::WebviewWindow) -> Result<Snapshot, String> {
    own(&window)?;
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    snapshot(app, &s)
}
#[tauri::command]
pub fn memory_sources(window: tauri::WebviewWindow, id: i64) -> Result<Vec<Source>, String> {
    own(&window)?;
    let state = window.app_handle().state::<Mutex<Service>>();
    let s = state.lock().map_err(|_| "聊天服务不可用")?;
    s.store()?.memory_sources(id)
}
#[tauri::command]
pub fn memory_update(
    window: tauri::WebviewWindow,
    operation: String,
    id: Option<i64>,
    text: Option<String>,
    revision: u64,
) -> Result<Snapshot, String> {
    own(&window)?;
    let app = window.app_handle();
    let state = app.state::<Mutex<Service>>();
    let mut s = state.lock().map_err(|_| "聊天服务不可用")?;
    if revision != s.memory_revision {
        return Err("记忆刚刚更新，请查看最新内容后再操作".into());
    }
    if operation == "model" {
        drop(s);
        memory_model::load(app, true);
        return memory_state(window);
    }
    if !["enable", "disable", "edit", "delete_entry", "delete", "delete_original", "retry", "flush"].contains(&operation.as_str()) {
        return Err("未知记忆操作".into());
    }
    if operation == "flush" {
        if !s.store()?.memory_enabled()? {
            return Err("请先开启记忆".into());
        }
        if !s.configured() {
            return Err("请先在交流设置中填写Key".into());
        }
        s.memory_flush = true;
    } else {
        cancel(&mut s);
        if ["edit", "delete_entry", "delete", "delete_original", "disable"].contains(&operation.as_str()) {
            s.invalidate()?;
            s.publish(app, false, "", "");
        }
        s.store.as_mut().map_err(|e| e.clone())?.memory_mutate(
            &operation,
            id.unwrap_or(0),
            text.as_deref().unwrap_or(""),
        )?;
        if operation == "delete_original" {
            s.history_epoch += 1;
            s.publish(app, false, "", "");
        }
        s.memory_error = None;
    }
    publish(app, &mut s);
    let view = snapshot(app, &s)?;
    drop(s);
    if operation == "enable" {
        memory_model::load(app, false);
    }
    Ok(view)
}
pub(crate) fn set_error(app: &tauri::AppHandle, error: String) {
    let state = app.state::<Mutex<Service>>();
    if let Ok(mut s) = state.lock() {
        s.memory_error = Some(error);
        publish(app, &mut s);
    };
}
async fn index(app: &tauri::AppHandle) {
    let data = (|| -> Result<_, String> {
        let state = app.state::<Mutex<Service>>();
        let s = state.lock().map_err(|_| "聊天服务不可用")?;
        Ok((s.store()?.memory_epoch()?, s.store()?.memory_unindexed()?))
    })();
    let (epoch, entries) = match data {
        Ok(v) => v,
        Err(e) => {
            set_error(app, e);
            return;
        }
    };
    if entries.is_empty() {
        return;
    }
    let handle = app.clone();
    let items = entries.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<memory_model::Embeddings>();
        let mut lock = state.model.try_lock().ok()?;
        let model = lock.as_mut()?;
        let texts: Vec<_> = items.iter().map(|(_, text)| text.as_str()).collect();
        model.embed(texts, Some(4)).ok()
    })
    .await;
    let Ok(Some(vectors)) = result else {
        return;
    };
    let state = app.state::<Mutex<Service>>();
    let Ok(mut s) = state.lock() else {
        return;
    };
    let saved = (|| -> Result<(), String> {
        let store = s.store()?;
        if store.memory_epoch()? != epoch || !store.memory_enabled()? {
            return Ok(());
        }
        for ((id, text), vector) in entries.iter().zip(vectors) {
            store.memory_index(*id, text, &vector)?;
        }
        Ok(())
    })();
    if let Err(e) = saved {
        s.memory_error = Some(e);
    }
    publish(app, &mut s);
}
