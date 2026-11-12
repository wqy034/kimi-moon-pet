//! Key只向设置页提供首尾各四位；DPAPI密文同时绑定连接ID与Base URL。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
};
fn path(provider: &str) -> Result<PathBuf, String> {
    if !crate::chat_profiles::valid_id(provider) {
        return Err("无效的凭据编号".into());
    }
    Ok(crate::adapters::state_file::pet_home().join(format!("聊天凭据-{provider}.dat")))
}
fn transform(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| "凭据过长")?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // Windows分配output；复制后必须用LocalFree释放，禁止记录任何输入输出内容。
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err("Windows凭据加解密失败；更换电脑或账户后请重新填写Key".into());
        }
        if output.pbData.is_null() || output.cbData == 0 {
            if !output.pbData.is_null() {
                LocalFree(output.pbData.cast());
            }
            return Err("凭据内容为空，请重新填写Key".into());
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(result)
    }
}
pub fn exists(provider: &str) -> bool {
    path(provider).is_ok_and(|p| p.is_file())
}
#[derive(Serialize, Deserialize)]
struct BoundKey {
    version: u32,
    connection_id: String,
    base_url: String,
    key: String,
}
pub fn mask(key: &str) -> String {
    let chars: Vec<_> = key.chars().collect();
    if chars.len() <= 8 {
        return "••••••••".into();
    }
    format!(
        "{}••••{}",
        chars[..4].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}
pub fn save(provider: &str, base_url: &str, key: &str) -> Result<(), String> {
    if key.is_empty()
        || key.len() > 4096
        || key.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("Key不能为空、不能包含空白或控制字符且不能超过4096字节".into());
    }
    let payload = serde_json::to_vec(&BoundKey {
        version: 1,
        connection_id: provider.into(),
        base_url: base_url.into(),
        key: key.into(),
    })
    .map_err(|_| "凭据编码失败")?;
    let encrypted = transform(&payload, true)?;
    let destination = path(provider)?;
    let temporary = destination.with_extension("dat.tmp");
    std::fs::write(&temporary, encrypted).map_err(|_| "凭据保存失败")?;
    std::fs::rename(temporary, destination).map_err(|_| "凭据替换失败")?;
    Ok(())
}
pub fn load(provider: &str, base_url: &str) -> Result<String, String> {
    let encrypted = std::fs::read(path(provider)?).map_err(|_| "请先填写并保存该连接的Key")?;
    let raw = String::from_utf8(transform(&encrypted, false)?)
        .map_err(|_| "凭据内容无效，请重新填写Key")?;
    if raw.starts_with('{') {
        let bound: BoundKey =
            serde_json::from_str(&raw).map_err(|_| "凭据版本无效，请重新填写Key")?;
        if bound.version != 1 || bound.connection_id != provider || bound.base_url != base_url {
            return Err("API地址与凭据绑定不符，请重新填写Key".into());
        }
        return Ok(bound.key);
    }
    let legacy = match provider {
        "deepseek" => "https://api.deepseek.com",
        "siliconflow" => "https://api.siliconflow.cn/v1",
        _ => "",
    };
    if legacy.is_empty() || legacy != base_url {
        return Err("修改API地址后需要重新填写Key".into());
    }
    Ok(raw)
}
pub fn hint(provider: &str, base_url: &str) -> Result<String, String> {
    let key = load(provider, base_url)?;
    Ok(mask(&key))
}
pub fn remove(provider: &str) -> Result<(), String> {
    match std::fs::remove_file(path(provider)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("连接已移除，但加密凭据文件清理失败".into()),
    }
}
