import { JUDGMENTS } from "./music-layout.js";

// 完整谱与玩家分工共用一个音频时钟；自动声部不参与命中率。
export class MusicEngine {
  constructor(audio, song, mode, offsetMs = 0, judgment = "coarse") {
    this.audio = audio; this.song = song; this.mode = mode; this.offset = offsetMs / 1000;
    this.judgment = judgment; this.window = JUDGMENTS[judgment].window / 1000;
    this.duration = song.beats * 60 / song.bpm;
    const timed = (n, id) => ({ ...n, id, time: n.beat * 60 / song.bpm, duration: n.duration * 60 / song.bpm, state: "pending" });
    this.notes = song.notes.filter(n => n.player).map(timed);
    // 示范与练习展示同一份玩家声部；完整合奏只交给下面的音频调度。
    this.displayNotes = this.notes;
    const source = mode === "demo" ? [...song.notes, ...song.accompaniment] : [...song.notes.filter(n => !n.player), ...song.accompaniment];
    // 旋律代奏与伴奏同拍同音只发声一次，保留较长音长。
    const events = new Map();
    for (const note of source) {
      const key = `${note.beat}:${note.pitch}`, old = events.get(key);
      if (!old || old.duration < note.duration) events.set(key, note);
    }
    this.events = [...events.values()].sort((a, b) => a.beat - b.beat || a.pitch - b.pitch).map(timed);
    this.position = 0; this.origin = 0; this.playing = false; this.cursor = 0; this.wrong = 0;
  }
  start() {
    this.origin = this.audio.context.currentTime + .08 + 120 / this.song.bpm - this.position;
    this.segmentStart = performance.now();
    this.cursor = this.events.findIndex(n => n.time >= this.position);
    if (this.cursor < 0) this.cursor = this.events.length;
    this.playing = true;
  }
  rawTime() { return this.playing ? this.audio.now() - this.origin : this.position; }
  get countIn() { return this.playing ? Math.max(0, Math.min(2, Math.ceil((this.origin + this.position - this.audio.now()) * this.song.bpm / 60))) : 0; }
  time() { return Math.max(this.position, Math.min(this.duration, this.rawTime())); }
  pause() { this.position = this.time(); this.playing = false; this.audio.stopAll(); }
  advance() {
    if (!this.playing) return;
    const now = this.audio.context.currentTime;
    while (this.cursor < this.events.length && this.origin + this.events[this.cursor].time <= now + .12) {
      const note = this.events[this.cursor++];
      if (this.origin + note.time >= now - .08 && (this.mode === "demo" || this.audio.assistEnabled)) {
        this.audio.playPitch(note.pitch, this.origin + note.time, note.duration, this.mode !== "demo", note.velocity ?? 96);
      }
    }
    if (this.mode !== "demo") {
      const t = this.rawTime() + this.offset; // miss 标记与 press 判定使用同一微调后时间轴，避免极端微调下画面与判定脱节
      for (const note of this.notes) if (note.state === "pending" && t > note.time + this.window) note.state = "miss";
    }
  }
  press(index, performanceTime) {
    const play = note => this.audio.playPitch(this.song.keys[index].pitch, undefined, undefined, false, note?.velocity ?? 96);
    if (!this.playing || this.mode === "demo" || performanceTime < this.segmentStart) return { voice: play(), hit: false };
    const now = this.audio.fromPerformance(performanceTime) - this.origin + this.offset;
    if (now < this.position - this.window || now > this.duration + this.window) return { voice: play(), hit: false };
    const candidates = this.notes.filter(n => n.index === index && n.state !== "hit" && Math.abs(n.time - now) <= this.window)
      .sort((a, b) => Math.abs(a.time - now) - Math.abs(b.time - now) || a.time - b.time);
    // 一次按下只消费一个音；按原始输入时间匹配，窗口不随谱面密度变化。
    if (candidates.length) { candidates[0].state = "hit"; return { voice: play(candidates[0]), hit: true }; }
    this.wrong++; return { voice: play(), hit: false };
  }
  get hits() { return this.notes.filter(n => n.state === "hit").length; }
  get ended() { return this.playing && this.rawTime() >= this.duration + this.window + .06; }
}
