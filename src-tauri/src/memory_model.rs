//! 本地中文向量模型。只下载公共权重，不向模型站点发送聊天文本。
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::{io::Read, path::Path, time::Duration};
use tauri::Manager;

pub struct Embeddings {
    pub model: Mutex<Option<TextEmbedding>>,
    pub status: Mutex<String>,
    loading: AtomicBool,
}
pub fn install(app: &tauri::AppHandle) {
    app.manage(Embeddings {
        model: Mutex::new(None),
        status: Mutex::new("尚未加载".into()),
        loading: AtomicBool::new(false),
    });
}
pub fn load(app: &tauri::AppHandle, retry: bool) {
    let model = app.state::<Embeddings>();
    if model.loading.swap(true, Ordering::AcqRel) {
        return;
    }
    let status = model
        .status
        .lock()
        .ok()
        .map(|s| s.clone())
        .unwrap_or_default();
    if status == "本地中文检索已就绪" || (!retry && status.starts_with("加载失败")) {
        model.loading.store(false, Ordering::Release);
        return;
    }
    if let Ok(mut s) = model.status.lock() {
        *s = "正在准备本地中文模型（首次需要下载，可继续聊天）".into();
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cache = crate::adapters::state_file::pet_home()
            .join("模型缓存")
            .join("memory-bge-small-zh-v1.5");
        let result = std::panic::catch_unwind(|| prepare(&cache));
        let state = handle.state::<Embeddings>();
        let status = match result {
            Ok(Ok(engine)) => {
                if let Ok(mut model) = state.model.lock() {
                    *model = Some(engine);
                }
                "本地中文检索已就绪".to_owned()
            }
            Ok(Err(error)) => format!("加载失败：{error}；聊天与事实仍可用，可在设置页重试"),
            _ => "加载失败：本地模型初始化异常；聊天与事实仍可用，可在设置页重试".into(),
        };
        if let Ok(mut s) = state.status.lock() {
            *s = status.clone();
        }
        state.loading.store(false, Ordering::Release);
        crate::append_log("debug.log", &format!("memory-model status={status}"));
        let service = handle.state::<Mutex<crate::chat::Service>>();
        if let Ok(mut s) = service.lock() {
            crate::memory::publish(&handle, &mut s);
        };
    });
}
// 固定公共模型版本、长度与SHA256；下载不读取HF_TOKEN/HF_HOME，不携带聊天Key。
const REVISION: &str = "75c43b069aac4d136ba6bc1122f995fedcfd2781";
fn prepare(cache: &Path) -> Result<TextEmbedding, String> {
    std::fs::create_dir_all(cache).map_err(|_| "无法创建本地模型目录")?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .timeout(Duration::from_secs(300))
        .redirects(5)
        .build();
    let files = [
        (
            "model.onnx",
            "onnx/model.onnx",
            94851877,
            "69a0b846f4f116b5e6aabf9546ea6754d02264f3211a13a1bd69b31b8040749a",
        ),
        (
            "config.json",
            "config.json",
            716,
            "d4193ead3a810fd694fa8a31d7fc72fbaebc0668b603e398734bf2f6538ff42f",
        ),
        (
            "tokenizer.json",
            "tokenizer.json",
            439125,
            "48cea5d44424912a6fd1ea647bf4fe50b55ab8b1e5879c3275f80e339e8fae26",
        ),
        (
            "tokenizer_config.json",
            "tokenizer_config.json",
            367,
            "e6f3b96db926a37d4039995fbf5ad17de158dfb8f6343d607e4dbaad18d75f5a",
        ),
        (
            "special_tokens_map.json",
            "special_tokens_map.json",
            125,
            "b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3",
        ),
    ];
    let mut blobs = Vec::new();
    for (name, remote, size, hash) in files {
        let path = cache.join(name);
        let cached = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() == size)
            .and_then(|_| std::fs::read(&path).ok())
            .filter(|bytes| format!("{:x}", Sha256::digest(bytes)) == hash);
        let bytes = if let Some(bytes) = cached {
            bytes
        } else {
            let response = agent
                .get(&format!(
                    "https://huggingface.co/Xenova/bge-small-zh-v1.5/resolve/{REVISION}/{remote}"
                ))
                .call()
                .map_err(|_| "公共模型下载失败")?;
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(size + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "模型下载中断")?;
            if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(&bytes)) != hash {
                return Err("模型文件校验失败".into());
            }
            let temporary = cache.join(format!("{name}.download"));
            std::fs::write(&temporary, &bytes).map_err(|_| "模型缓存写入失败")?;
            std::fs::rename(&temporary, &path).map_err(|_| "模型缓存替换失败")?;
            bytes
        };
        blobs.push(bytes);
    }
    let tokenizer = TokenizerFiles {
        special_tokens_map_file: blobs.pop().unwrap(),
        tokenizer_config_file: blobs.pop().unwrap(),
        tokenizer_file: blobs.pop().unwrap(),
        config_file: blobs.pop().unwrap(),
    };
    let model =
        UserDefinedEmbeddingModel::new(blobs.pop().unwrap(), tokenizer).with_pooling(Pooling::Cls);
    TextEmbedding::try_new_from_user_defined(
        model,
        InitOptionsUserDefined::new()
            .with_max_length(512)
            .with_intra_threads(2),
    )
    .map_err(|_| "模型初始化失败".into())
}
/// 本地推理不持有聊天锁；模型繁忙、加载中或超时均跳过，不拖住发送。
pub async fn query(app: &tauri::AppHandle, text: String) -> Option<Vec<f32>> {
    let handle = app.clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tauri::async_runtime::spawn_blocking(move || {
            let state = handle.state::<Embeddings>();
            let mut engine = state.model.try_lock().ok()?;
            let engine = engine.as_mut()?;
            let chars: Vec<_> = text.chars().collect();
            let chunks: Vec<String> = chars
                .chunks(350)
                .map(|part| {
                    format!(
                        "为这个句子生成表示以用于检索相关文章：{}",
                        part.iter().collect::<String>()
                    )
                })
                .collect();
            let vectors = engine.embed(chunks, Some(4)).ok()?;
            let mut avg = vec![0.0; 512];
            for vector in &vectors {
                if vector.len() != 512 {
                    return None;
                }
                for (i, v) in vector.iter().enumerate() {
                    avg[i] += v;
                }
            }
            if vectors.is_empty() {
                None
            } else {
                Some(avg)
            }
        }),
    )
    .await
    .ok()?
    .ok()?
}
