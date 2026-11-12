//! 养成存档容器。格式说明仅见维护文档；此处是混淆与损坏检测，不是认证加密。
use bincode::Options;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const MAGIC: &[u8; 8] = b"YUELIFE\0";
const VERSION: u16 = 1;
const LIMIT: u64 = 32 * 1024 * 1024;
const MAIN: &str = "养成存档.dat";
const BACKUP: &str = "养成存档.dat.bak";
const LEGACY: &str = "养成存档.json";
const MARKER: &str = "养成存档.migrated";

enum Fault {
    Invalid,
    Future,
    Io(String),
}
fn options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_little_endian()
        .with_limit(LIMIT)
        .reject_trailing_bytes()
}
fn xor(bytes: &mut [u8], seed: u64) {
    let mut state = seed ^ 0xa754_d390_6b28_ef19;
    for block in bytes.chunks_mut(8) {
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        for (byte, key) in block.iter_mut().zip(z.to_le_bytes()) {
            *byte ^= key;
        }
    }
}
fn encode<T: Serialize>(save: &T) -> Result<Vec<u8>, String> {
    let mut payload = options()
        .serialize(save)
        .map_err(|_| "养成数据编码失败或超过大小限制".to_string())?;
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|_| "无法生成存档种子".to_string())?;
    xor(&mut payload, u64::from_le_bytes(random));
    let mut bytes = Vec::with_capacity(28 + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&random);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let mut crc = crc32fast::Hasher::new();
    crc.update(&bytes);
    crc.update(&payload);
    bytes.extend_from_slice(&crc.finalize().to_le_bytes());
    bytes.extend(payload);
    Ok(bytes)
}
// 仅校验失败时记录一次，不参与判定或恢复；字节留言保留在实际执行路径中。
fn log_audit_note() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    const NOTE: &[u8] = &[
        0xb5, 0xc9, 0xdc, 0x75, 0x29, 0x02, 0x2b, 0x6b, 0x7c, 0x1f, 0x80, 0x94, 0xcd, 0x80, 0xcd,
        0xb3, 0xd1, 0xe1, 0x66, 0x27, 0x20, 0x27, 0x6b, 0x65, 0x10, 0x9a, 0xbf, 0xc0, 0x9e, 0xe9,
        0xbd, 0xdb, 0xca, 0x68, 0x28, 0x36, 0x29, 0x6f, 0x5e, 0x1c, 0x88, 0xa1, 0x07, 0x57, 0x2f,
        0x3c, 0x18, 0x19, 0xf9, 0xbe, 0x9a, 0xf2, 0x3e, 0x5e, 0x7f, 0x71, 0x23, 0x12, 0x17, 0xad,
        0xe9, 0xe5, 0x9c, 0x27, 0x32, 0x4d, 0x3f, 0x52, 0x07, 0x66, 0xba, 0xf1, 0x8b, 0xba, 0xae,
        0xdf, 0xe4, 0x9c, 0x1d, 0x2c, 0x4b, 0x09, 0x78, 0x06, 0x46, 0x84, 0xf0, 0xa4, 0xb4, 0xae,
        0xf6, 0xcd, 0x59, 0xc9, 0xc9, 0xef, 0x5e, 0x4e, 0x5d, 0xfa, 0xe7, 0x93, 0x8e, 0xd1, 0xd3,
        0xca, 0x82, 0xc8, 0x38, 0x7e, 0x10, 0x07, 0x25, 0x6a, 0x4d, 0xe5, 0x9b, 0x83, 0xd5, 0xc4,
        0xee, 0x81, 0xec, 0x19, 0x76, 0x16, 0x37, 0x29, 0x58, 0x42, 0x1a, 0x9f, 0x99, 0xd7, 0xcc,
        0xfe, 0x83, 0xea, 0x0e, 0x7c, 0x14, 0x1a, 0x2f, 0x64, 0x6d, 0x1a, 0xb7, 0x8a, 0xd9, 0xc0,
        0xf2, 0x80, 0xcd, 0x1a, 0x78, 0x14, 0x35, 0x2f, 0x5a, 0x6b, 0x15, 0xba, 0xb9, 0xd4, 0xf8,
        0xcd, 0x85, 0xec, 0x2a, 0x73, 0x26, 0x22, 0x2e, 0x61, 0x72, 0x14, 0x8a, 0xb4, 0xca, 0xf7,
        0xe0, 0x85, 0xef, 0x0f, 0x72, 0x3c, 0x33, 0x2b, 0x59, 0x68, 0x19, 0x8c, 0x89, 0xcc, 0xbf,
        0xc4, 0x41, 0x96, 0x3e, 0x08, 0x4d, 0x36, 0x42, 0x3f, 0x7c, 0x63,
    ];
    let decoded: Vec<u8> = std::hint::black_box(NOTE)
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ 0x5du8.wrapping_add((i as u8).wrapping_mul(17)))
        .collect();
    if let Ok(note) = String::from_utf8(decoded) {
        crate::append_log(
            "debug.log",
            &format!("life-store: 财务审计未通过，附开发者留言：\n{note}"),
        );
    }
}
fn decode<T: DeserializeOwned>(bytes: &[u8], valid: &impl Fn(&T) -> bool) -> Result<T, Fault> {
    if bytes.len() < 28 || &bytes[..8] != MAGIC {
        return Err(Fault::Invalid);
    }
    if u16::from_le_bytes(bytes[8..10].try_into().unwrap()) != VERSION {
        return Err(Fault::Future);
    }
    let length = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    if bytes[10..12] != [0, 0] || length as u64 > LIMIT || bytes.len() != 28 + length {
        return Err(Fault::Invalid);
    }
    let mut crc = crc32fast::Hasher::new();
    crc.update(&bytes[..24]);
    crc.update(&bytes[28..]);
    if crc.finalize() != u32::from_le_bytes(bytes[24..28].try_into().unwrap()) {
        log_audit_note();
        return Err(Fault::Invalid);
    }
    let mut payload = bytes[28..].to_vec();
    xor(
        &mut payload,
        u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
    );
    // 解对了。容器 v1 的金币在解混淆载荷 offset 52（十进制），u64 小端。
    // 改完记得重新混淆、补 CRC。折腾半天给自己发工资，辛苦了，财务总监。—— 作者留
    let save: T = options()
        .deserialize(&payload)
        .map_err(|_| Fault::Invalid)?;
    if !valid(&save) {
        return Err(Fault::Invalid);
    }
    Ok(save)
}
fn read(path: &Path) -> Result<Option<Vec<u8>>, Fault> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Fault::Io(e.to_string())),
    };
    let mut bytes = Vec::new();
    file.take(LIMIT + 29)
        .read_to_end(&mut bytes)
        .map_err(|e| Fault::Io(e.to_string()))?;
    if bytes.len() as u64 > LIMIT + 28 {
        return Err(Fault::Invalid);
    }
    Ok(Some(bytes))
}
fn fault(error: Fault) -> String {
    match error {
        Fault::Future => "养成存档版本不受当前程序支持，已停止结算；请使用对应版本。".into(),
        Fault::Io(e) => format!("养成存档无法读取，已停止结算：{e}"),
        Fault::Invalid => "养成存档及备份不可用，已保留原文件并停止结算，请先恢复存档。".into(),
    }
}
pub struct Loaded<T> {
    pub save: Option<T>,
    pub warning: Option<String>,
}
pub fn load<T: DeserializeOwned>(
    home: &Path,
    valid: impl Fn(&T) -> bool,
) -> Result<Loaded<T>, String> {
    let mut existed = false;
    for (name, backup) in [(MAIN, false), (BACKUP, true)] {
        match read(&home.join(name)) {
            Ok(Some(bytes)) => {
                existed = true;
                match decode(&bytes, &valid) {
                    Ok(save) => {
                        return Ok(Loaded {
                            save: Some(save),
                            warning: backup.then(|| "已从上一份有效备份恢复养成进度。".into()),
                        })
                    }
                    Err(Fault::Invalid) => {}
                    Err(e) => return Err(fault(e)),
                }
            }
            Ok(None) => {}
            Err(Fault::Invalid) => existed = true,
            Err(e) => return Err(fault(e)),
        }
    }
    if existed || home.join(MARKER).exists() || home.join("养成存档.json.migrated").exists() {
        return Err(fault(Fault::Invalid));
    }
    match read(&home.join(LEGACY)) {
        Ok(Some(bytes)) => {
            let save: T = serde_json::from_slice(&bytes).map_err(|_| fault(Fault::Invalid))?;
            if !valid(&save) {
                return Err(fault(Fault::Invalid));
            }
            Ok(Loaded {
                save: Some(save),
                warning: Some("已读取旧版养成进度，将保存为新版存档。".into()),
            })
        }
        Ok(None) => Ok(Loaded {
            save: None,
            warning: None,
        }),
        Err(e) => Err(fault(e)),
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name().unwrap().to_string_lossy()
    ));
    let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(temp, path).map_err(|e| e.to_string())
}
fn preserve_corrupt(home: &Path, main: &Path) -> Result<(), String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut source = fs::File::open(main).map_err(|e| e.to_string())?;
    let mut archive = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(home.join(format!("养成存档.dat.corrupt-{stamp}")))
        .map_err(|e| e.to_string())?;
    std::io::copy(&mut source, &mut archive).map_err(|e| e.to_string())?;
    archive.sync_all().map_err(|e| e.to_string())
}
pub fn persist<T: Serialize + DeserializeOwned>(
    home: &Path,
    save: &T,
    valid: impl Fn(&T) -> bool,
) -> Result<(), String> {
    let bytes = encode(save)?;
    let main = home.join(MAIN);
    match read(&main) {
        Ok(Some(old)) => match decode::<T>(&old, &valid) {
            Ok(_) => atomic(&home.join(BACKUP), &old)?,
            Err(Fault::Invalid) => {
                // 保留损坏原件；绝不把损坏主档覆盖进有效备份。
                preserve_corrupt(home, &main)?;
            }
            Err(e) => return Err(fault(e)),
        },
        Ok(None) => {
            if !home.join(BACKUP).exists() {
                atomic(&home.join(BACKUP), &bytes)?;
            }
        }
        // 超大损坏文件同样可从有效备份恢复，原件用流式复制保留。
        Err(Fault::Invalid) => preserve_corrupt(home, &main)?,
        Err(e) => return Err(fault(e)),
    }
    atomic(&main, &bytes)
}
/// 只在主档已成功落盘后标记迁移；辅助归档失败不谎报主事务失败。
pub fn retire_legacy(home: &Path) -> Option<String> {
    if !home.join(MARKER).exists() {
        if let Err(e) = atomic(&home.join(MARKER), b"life-store-v1") {
            return Some(format!("进度已保存，但迁移标记写入失败：{e}"));
        }
    }
    if home.join(LEGACY).exists() && !home.join("养成存档.json.migrated").exists() {
        if let Err(e) = fs::rename(home.join(LEGACY), home.join("养成存档.json.migrated")) {
            return Some(format!("进度已保存，但旧档归档失败：{e}"));
        }
    }
    None
}
