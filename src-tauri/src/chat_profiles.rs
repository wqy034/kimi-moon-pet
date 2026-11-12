//! 多连接配置不含Key；凭据按连接ID与准确Base URL绑定。
use crate::{
    chat_credentials as credentials,
    chat_store::{self, Store},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    pub fallback: bool,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub models_at: u64,
    #[serde(default)]
    pub key_hint: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    pub fallback: bool,
}
#[derive(Serialize)]
pub struct ProfileView {
    #[serde(flatten)]
    pub profile: Profile,
    pub configured: bool,
}
#[derive(Serialize)]
pub struct Preset {
    pub kind: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub model: &'static str,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    profiles: Vec<Profile>,
}
pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            kind: "deepseek",
            name: "DeepSeek 官方",
            base_url: "https://api.deepseek.com",
            model: "deepseek-flash",
        },
        Preset {
            kind: "siliconflow",
            name: "硅基流动",
            base_url: "https://api.siliconflow.cn/v1",
            model: "deepseek-ai/DeepSeek-V4.1-Flash",
        },
        Preset {
            kind: "openai",
            name: "OpenAI",
            base_url: "https://api.openai.com/v1",
            model: "",
        },
        Preset {
            kind: "gemini",
            name: "Google Gemini（兼容接口）",
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            model: "",
        },
        Preset {
            kind: "dashscope",
            name: "阿里云百炼 / 通义千问",
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            model: "",
        },
        Preset {
            kind: "moonshot",
            name: "月之暗面 Kimi",
            base_url: "https://api.moonshot.cn/v1",
            model: "",
        },
        Preset {
            kind: "zhipu",
            name: "智谱 GLM",
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            model: "",
        },
        Preset {
            kind: "openrouter",
            name: "OpenRouter",
            base_url: "https://openrouter.ai/api/v1",
            model: "",
        },
        Preset {
            kind: "custom",
            name: "自定义 OpenAI 兼容接口",
            base_url: "",
            model: "",
        },
    ]
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn base_url(value: &str) -> Result<String, String> {
    if value.len() > 2048 {
        return Err("API地址过长".into());
    }
    let value = value.trim().trim_end_matches('/');
    let value = value
        .strip_suffix("/chat/completions")
        .or_else(|| value.strip_suffix("/models"))
        .unwrap_or(value);
    let url = reqwest::Url::parse(value).map_err(|_| "请输入完整的API Base URL")?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if url.host_str().is_none()
        || (!matches!(url.scheme(), "https") && !(url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("API地址须为HTTPS（本机可用HTTP），不能含账号、密码、查询参数或片段".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
pub fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.chars().count() <= 200
        && !model.chars().any(|c| c.is_control() || c.is_whitespace())
}
impl Profile {
    pub fn from_draft(draft: Draft, id: String) -> Result<Self, String> {
        let p = Self {
            id,
            name: draft.name.trim().into(),
            kind: draft.kind,
            base_url: base_url(&draft.base_url)?,
            model: draft.model.trim().into(),
            fallback: draft.fallback,
            models: vec![],
            models_at: 0,
            key_hint: String::new(),
        };
        p.validate()?;
        Ok(p)
    }
    fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id)
            || self.name.is_empty()
            || self.name.chars().count() > 60
            || self.name.chars().any(char::is_control)
            || !presets().iter().any(|p| p.kind == self.kind)
            || (!self.model.is_empty() && !valid_model(&self.model))
            || self.models.len() > 2000
            || self.models.iter().any(|m| !valid_model(m))
            || self.key_hint.chars().count() > 32
        {
            return Err("连接配置格式无效，请检查名称、类型和模型ID".into());
        }
        if base_url(&self.base_url)? != self.base_url {
            return Err("保存的API地址格式无效，原配置保留".into());
        }
        Ok(())
    }
    pub fn configured(&self) -> bool {
        credentials::exists(&self.id) && valid_model(&self.model)
    }
    pub fn view(&self) -> ProfileView {
        ProfileView {
            profile: self.clone(),
            configured: credentials::exists(&self.id),
        }
    }
    pub fn body(&self, messages: &[Value], stream: bool, limit: u32, json_mode: bool) -> Value {
        let mut body = json!({"model":self.model,"messages":messages,"stream":stream});
        if self.kind == "openai" {
            body["max_completion_tokens"] = json!(limit);
        } else {
            body["max_tokens"] = json!(limit);
        }
        match self.kind.as_str() {
            "deepseek" | "moonshot" | "zhipu" => body["thinking"] = json!({"type":"disabled"}),
            "siliconflow" | "dashscope" => body["enable_thinking"] = json!(false),
            "gemini" => body["reasoning_effort"] = json!("low"),
            _ => {}
        }
        if stream
            && matches!(
                self.kind.as_str(),
                "deepseek" | "openai" | "gemini" | "dashscope" | "openrouter"
            )
        {
            body["stream_options"] = json!({"include_usage":true});
        }
        if json_mode && self.kind != "custom" {
            body["response_format"] = json!({"type":"json_object"});
        }
        body
    }
}
pub fn load(store: &Store) -> Result<Vec<Profile>, String> {
    if let Some(json) = store.config("api_profiles_v1")? {
        let saved: Saved =
            serde_json::from_str(&json).map_err(|_| "多连接配置无法读取，原数据保留")?;
        if saved.version != 1 || saved.profiles.len() > 16 {
            return Err("多连接配置版本未知或数量超限，原数据保留".into());
        }
        let mut ids = std::collections::HashSet::new();
        for profile in &saved.profiles {
            profile.validate()?;
            if !ids.insert(&profile.id) {
                return Err("连接编号重复，原数据保留".into());
            }
        }
        return Ok(saved.profiles);
    }
    // 保留原有两个ID与DPAPI文件，迁移时不改动旧凭据，不主动查询网络。
    let profiles: Vec<_> = presets()
        .into_iter()
        .take(2)
        .map(|p| Profile {
            id: p.kind.into(),
            name: p.name.into(),
            kind: p.kind.into(),
            base_url: p.base_url.into(),
            model: p.model.into(),
            fallback: false,
            models: vec![],
            models_at: 0,
            key_hint: credentials::hint(p.kind, p.base_url).unwrap_or_default(),
        })
        .collect();
    save(store, &profiles)?;
    Ok(profiles)
}
pub fn save(store: &Store, profiles: &[Profile]) -> Result<(), String> {
    if profiles.len() > 16 {
        return Err("最多保存16组API连接".into());
    }
    let json = serde_json::to_string(&Saved {
        version: 1,
        profiles: profiles.to_vec(),
    })
    .map_err(|_| "连接配置编码失败")?;
    store.set("api_profiles_v1", &json)
}
pub fn persist(
    store: &Store,
    profiles: &[Profile],
    selected: &str,
    auto_fallback: bool,
    interactions: bool,
) -> Result<(), String> {
    if profiles.len() > 16 || (!selected.is_empty() && !profiles.iter().any(|p| p.id == selected)) {
        return Err("主连接不存在或连接数量超限".into());
    }
    let json = serde_json::to_string(&Saved {
        version: 1,
        profiles: profiles.to_vec(),
    })
    .map_err(|_| "连接配置编码失败")?;
    let tx = store
        .db
        .unchecked_transaction()
        .map_err(chat_store::db_error)?;
    for (key, value) in [
        ("api_profiles_v1", json.as_str()),
        ("provider", selected),
        (
            "auto_fallback",
            if auto_fallback { "true" } else { "false" },
        ),
        ("interactions", if interactions { "true" } else { "false" }),
    ] {
        tx.execute(
            "INSERT INTO config VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![key, value],
        )
        .map_err(chat_store::db_error)?;
    }
    tx.commit().map_err(chat_store::db_error)
}
pub fn fresh_id() -> Result<String, String> {
    chat_store::id()
}

pub struct Failure {
    pub message: String,
    pub fallback: bool,
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::permanent(message)
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::permanent(message)
    }
}
impl Failure {
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fallback: false,
        }
    }
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fallback: true,
        }
    }
    pub fn http(status: u16) -> Self {
        Self {
            message: crate::chat::http_error(status),
            fallback: matches!(status,401..=404|408|409|425|429|500..=599),
        }
    }
}
