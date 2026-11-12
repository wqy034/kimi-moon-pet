// 只观察真实鼠标；所有移动都作用于桌宠窗口。
export class CursorPlay {
  constructor(now) {
    this.sample = null;
    this.active = null;
    this.waitMs = 8000 + Math.random() * 4000;
    this.reason = "waiting-sensor";
  }
  observe(sample, now) {
    const moved = !this.sample || sample.x !== this.sample.x || sample.y !== this.sample.y;
    this.sample = { ...sample, seenAt: now };
    if (moved) {
      this.waitMs = 8000 + Math.random() * 4000;
    }
    if (moved || sample.down || !sample.same_monitor) this.cancel();
  }
  cancel() { this.active = null; }
  ready(now) {
    const p = this.sample;
    if (!p || now - p.seenAt > 500 || p.down || !p.same_monitor) {
      this.reason = !p ? "no-sensor" : now - p.seenAt > 500
        ? "stale-sensor" : p.down ? "mouse-button-down" : "other-monitor-or-taskbar";
      return false;
    }
    if (p.still_ms < this.waitMs) { this.reason = "cursor-moving-or-waiting"; return false; }
    this.reason = "ready";
    return true;
  }
  start(now, geometry) {
    if (!this.ready(now)) return false;
    const p = this.sample;
    const { x, y, width, sf, maxX } = geometry;
    const px = p.x / sf, py = p.y / sf;
    const projected = py < y - 36;
    let target;
    if (projected) {
      // 高处光标：沿当前基线走到其水平投影，不爬升。
      target = Math.max(10, Math.min(maxX, px - width / 2));
    } else {
      // 同高度时把整个窗口停在光标旁边，避免遮住实际点击位置。
      const candidates = [px - width - 28, px + 28].filter(value => value >= 10 && value <= maxX);
      target = candidates.sort((a, b) => Math.abs(a - x) - Math.abs(b - x))[0] ?? x;
    }
    const speed = 64;
    const travelMs = Math.abs(target - x) / speed * 1000;
    const arriveBy = now + 600 + travelMs + 1500;
    this.active = { target, started: now, gestureAt: 0, announced: false, speed,
      projected, arriveBy, until: arriveBy + 2800 };
    return true;
  }
  behavior(now, geometry) {
    if (!this.active) return null;
    const p = this.sample;
    if (!p || now - p.seenAt > 500 || p.down || !p.same_monitor) { this.cancel(); return null; }
    const a = this.active;
    if (now >= a.until) { this.reason = "finished"; this.cancel(); return null; }
    if (now - a.started < 600) return { action: "await", cursorPhase: "look" };
    if (!a.gestureAt && Math.abs(geometry.x - a.target) < 2) {
      a.gestureAt = now;
      a.until = now + 2800;
    }
    if (a.gestureAt) {
      this.reason = "waving";
      const announce = !a.announced;
      // 整轮提醒与恶作剧的冷却由CursorAttention统一持有。
      a.announced = true;
      return { action: "wave", announce, cursorPhase: "greet",
        say: announce ? "……你的光标停了很久。" : null };
    }
    if (now >= a.arriveBy) { this.reason = "approach-timeout"; this.cancel(); return null; }
    this.reason = a.projected ? "walking-under-cursor" : "walking-beside-cursor";
    return { walk: true, speed: a.speed, cursorTarget: a.target, cursorPhase: "approach" };
  }
}
