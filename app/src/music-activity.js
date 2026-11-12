// 陪演资格与回应适配，所有出场仍由BehaviorDirector唯一活动槽仲裁。
export class MusicActivity {
  constructor(invoke, log) { this.invoke = invoke; this.log = log; this.snapshot = null; this.visible = false; this.seenAt = 0; this.nextReport = 0; this.reportKey = ""; this.pending = false; }
  update(snapshot, now) {
    if (!snapshot || this.snapshot && snapshot.revision < this.snapshot.revision) return false;
    const before = this.snapshot?.session;
    this.snapshot = snapshot; this.seenAt = now;
    const next = snapshot.session;
    return next?.requested && (before?.id !== next.id || !before?.requested);
  }
  ready(now) { return this.visible && !!this.snapshot?.session?.requested && now - this.seenAt < 3000; }
  behavior() { return { action: "music_flute", musicActive: true, lifeMode: "paused" }; }
  report(now, active, layer) {
    const session = this.snapshot?.session;
    if (!session || this.pending) return;
    const reason = active ? "正在陪你弹" : ({ cli: "她正在响应工作", held: "她被拎起来啦", falling: "等她站稳再弹", sleep: "她在休息", panel: "菜单操作中", "touch-feedback": "稍等她回应一下", "protected-bubble": "等她说完这句话" }[layer] || "稍等一下，马上回来");
    const key = `${session.id}:${active}:${reason}`;
    if (now < this.nextReport && key === this.reportKey) return;
    this.pending = true; this.nextReport = now + 1000; this.reportKey = key;
    this.invoke("music_participation", { id: session.id, granted: active, reason })
      .catch(error => this.log(`music-participation-error ${error}`)).finally(() => { this.pending = false; });
  }
}
