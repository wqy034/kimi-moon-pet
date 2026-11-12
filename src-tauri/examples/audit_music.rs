//! 离线资源检查，复用运行时解析器；不创建窗口、不播放、不模拟输入。
#[path = "../src/music_score.rs"]
mod music_score;
use std::{collections::BTreeSet, fs, path::PathBuf};

fn main() {
    let folder = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../config/曲库"));
    let mut paths: Vec<_> = fs::read_dir(&folder)
        .expect("曲库目录不存在")
        .flatten()
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_file())
                && e.path().extension().is_some_and(|s| s == "json")
        })
        .map(|e| e.path())
        .collect();
    paths.sort();
    let mut ids = BTreeSet::new();
    let mut failed = 0;
    for path in paths {
        let result = fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| music_score::decode(&b));
        match result {
            Ok(song) if ids.insert(song.id.clone()) => {
                let rank = music_score::judgment_rank(&song.recommended).expect("推荐判定无效");
                println!(
                    "{}",
                    serde_json::json!({"id":song.id,"seconds":song.seconds(),"player":song.player_count,"assist":song.assist_count,"key":song.key,"difficulty":song.difficulty,"recommended_rank":rank,"record":song.record_key()})
                );
            }
            Ok(_) => {
                eprintln!("{}: 重复id", path.display());
                failed += 1;
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                failed += 1;
            }
        }
    }
    println!("songs={} invalid={failed}", ids.len());
    if failed > 0 || ids.is_empty() {
        std::process::exit(1);
    }
}
