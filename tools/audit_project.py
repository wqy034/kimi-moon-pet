"""离线检查源码语法、运行资源与可见边界；不操作窗口、不访问网络。"""
import ast
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]

def main():
    tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode("utf-8").split("\0")
    files = [ROOT / name for name in tracked if name and (ROOT / name).is_file()]
    counts = {"python": 0, "javascript": 0, "json": 0}
    for file in files:
        if file.suffix == ".py":
            ast.parse(file.read_text(encoding="utf-8-sig"), filename=str(file))
            counts["python"] += 1
        elif file.suffix == ".js":
            subprocess.run(["node", "--check", str(file)], check=True, cwd=ROOT)
            counts["javascript"] += 1
        elif file.suffix == ".json":
            json.loads(file.read_text(encoding="utf-8-sig"))
            counts["json"] += 1
    actions = json.loads((ROOT / "config/动画.json").read_text(encoding="utf-8"))["actions"]
    bounds = json.loads((ROOT / "config/可见边界.json").read_text(encoding="utf-8"))
    frames = set()
    for name, action in actions.items():
        assert action["frames"] and action["frame_ms"] > 0, name
        assert action["anchor"] in ("feet_center", "body_center"), name
        if "cursor_pull_y" in action:
            assert len(action["cursor_pull_y"]) == len(action["frames"]) + 1, name
        for frame in action["frames"]:
            key = f'{action["dir"]}/{frame}'
            if key in frames:
                continue
            with Image.open(ROOT / "帧文件" / f"{key}.png") as image:
                assert image.size == (320, 320), key
                actual = image.convert("RGBA").getchannel("A").point(lambda a: 255 if a >= 16 else 0).getbbox()
                assert actual and list(actual) == bounds.get(key), f"可见边界不一致: {key}"
            frames.add(key)
    for prop in ("睡觉_Zzz.png", "鼠标指针.png"):
        with Image.open(ROOT / "道具" / prop) as image:
            image.verify()
    audio_dir = ROOT / "app/audio/piano"
    manifest = json.loads((audio_dir / "manifest.json").read_text(encoding="utf-8"))
    samples = manifest["samples"]
    for sample in samples:
        assert Path(sample["file"]).name == sample["file"], sample["file"]
        assert hashlib.sha256((audio_dir / sample["file"]).read_bytes()).hexdigest() == sample["sha256"], sample["file"]
    pitches = [48 + oct * 12 + step for oct in range(3) for step in (0, 2, 4, 5, 7, 9, 11)]
    assert all(min(abs(note - sample["midi"]) for sample in samples) <= 1 for note in pitches)
    songs = list((ROOT / "config/曲库").glob("*.json"))
    song_ids = set()
    for file in songs:
        song = json.loads(file.read_text(encoding="utf-8"))
        assert song["id"] not in song_ids, file
        song_ids.add(song["id"])
        assert 40 <= song["bpm"] <= 200 and 1 <= song["beats"] <= 1000, file
        assert song["beats"] * 60 / song["bpm"] <= 600 and 4 <= len(song["notes"]) <= 2000, file
        version = song.get("formatVersion", 1)
        assert version in (1, 2), file
        for note in song["notes"] + song.get("accompaniment", []):
            if version == 1:
                assert note["key"] in range(7) and note["oct"] in range(3), file
            else:
                assert isinstance(note["pitch"], int) and note["pitch"] in range(128), file
                assert note.get("lyre21", "auto") in ("auto", "player", "assist"), file
            assert math.isfinite(note["beat"]) and note["beat"] >= 0 and .05 <= note["duration"] <= 16, file
            assert note["beat"] + note["duration"] <= song["beats"], file
            assert isinstance(note.get("velocity", 96), int) and 1 <= note.get("velocity", 96) <= 127, file
        assert all(b["beat"] >= a["beat"] for a, b in zip(song["notes"], song["notes"][1:])), file
        # 和弦、密谱与分工由Rust运行时同源的examples/audit_music.rs精确校验。
    print(json.dumps({**counts, "actions": len(actions), "unique_runtime_frames": len(frames),
                      "piano_samples": len(samples), "music_pitches": len(pitches), "songs": len(songs),
                      "result": "PASS"}, ensure_ascii=False))

if __name__ == "__main__":
    main()
