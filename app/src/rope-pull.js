import { CursorCarry } from "./cursor-carry.js";

// 复用租约代际、输入取消和冷却；不自行安排出场。
export class RopePull extends CursorCarry {
  offer(now, sample, g) {
    if (g.maxX < 10 || g.x < 10 || g.x > g.maxX) { this.reason = "outside-walk-range"; return null; }
    if (!sample?.carry_available || sample.down || !sample.same_monitor ||
        sample.carrying || now - sample.seenAt > 500) {
      this.reason = "waiting-native-idle";
      return null;
    }
    const px = sample.x / g.sf, py = sample.y / g.sf;
    const maxDown = Math.min(136, g.y + g.catchPoint[1] - py);
    const goal = px - g.ropeGripX;
    if (maxDown < 1 || goal < 10 || goal > g.maxX) { this.reason = "not-above-hand-or-outside-projection"; return null; }
    this.reason = "ready";
    return { mode: "rope", goal, maxDown, sf: g.sf };
  }
  start(now, sample, g) {
    const offer = this.offer(now, sample, g);
    if (!offer) return false;
    this.generation++;
    const travelMs = Math.abs(offer.goal - g.x) / 64 * 1000;
    this.active = { ...offer, phase: "approach", started: now,
      until: now + travelMs + 6000, arriveBy: now + travelMs + 1900,
      startX: sample.x, startY: sample.y, beginPending: false };
    this.id = 0; this.lastUpdateAt = 0;
    return true;
  }
  onAcquired(now, a) {
    a.phase = "pull";
    a.until = now + 5500; // 四轮共4.8秒，余量留给首帧和释放；高处模式原生上限6秒。
  }
  observe(sample, now) {
    if (this.active && !this.id && !this.active.beginPending && !sample.carry_available) {
      this.cancel(now, "rope-native-idle-lost");
      return;
    }
    super.observe(sample, now);
  }
  behavior(now, g) {
    const a = this.active;
    if (!a) return null;
    if (now >= a.until) { this.cancel(now, a.phase === "release" ? "complete" : "rope-timeout"); return null; }
    if (a.phase === "approach") {
      if (now - a.started < 400) return { action: "await", carryPhase: "rope-look" };
      if (Math.abs(g.x - a.goal) >= 2) {
        if (now >= a.arriveBy) { this.cancel(now, "rope-approach-timeout"); return null; }
        return { walk: true, speed: 64, cursorTarget: a.goal, carryPhase: "rope-approach" };
      }
      // 统一互动允许累积拉到手边；窗口不随光标升降。
      a.maxDown = Math.min(a.maxDown, g.y + g.catchPoint[1] - a.startY / g.sf);
      if (a.maxDown < 1) { this.cancel(now, "rope-too-close"); return null; }
      a.phase = "grab";
      a.until = now + 6000;
    }
    if (a.phase === "pull" && a.completed) {
      a.phase = "release"; a.until = now + 700;
      if (this.id) this.invoke("carry_cursor_end", { id: this.id }).catch(() => {});
    }
    this.reason = a.phase;
    if (a.phase === "release") return { action: "wave", carryPhase: "rope-release" };
    return { action: a.phase === "pull" ? "rope_pull" : "rope_hold",
      nativeCarry: true, carryPhase: `rope-${a.phase}` };
  }
  cursorPoint(now, packet, a) {
    if (a.phase !== "pull" || packet.action !== "rope_pull") return [a.startX, a.startY];
    const curve = packet.pullCurve;
    if (!Array.isArray(curve) || curve.length < 2 || !curve.every(Number.isFinite) ||
        !Number.isFinite(packet.animationElapsed) || !(packet.frameMs > 0)) {
      this.cancel(now, "rope-missing-motion");
      return [a.startX, a.startY];
    }
    const cycles = Math.max(1, Math.min(4, packet.pullCycles || 4));
    const count = curve.length - 1, duration = count * packet.frameMs;
    const elapsed = Math.max(0, Math.min(packet.animationElapsed, duration * cycles));
    const cycle = Math.min(cycles - 1, Math.floor(elapsed / duration));
    if (a.lastCycle !== cycle) {
      a.lastCycle = cycle;
      this.log(`rope-scene cycle=${cycle + 1} maxDown=${Math.round(a.maxDown)}`);
    }
    const frame = Math.min(count, (elapsed - cycle * duration) / packet.frameMs);
    const index = Math.min(count - 1, Math.floor(frame)), t = frame - index;
    const eased = t * t * (3 - 2 * t);
    const net = curve.at(-1);
    const peak = Math.max(...curve) + Math.max(0, net) * (cycles - 1);
    const gain = Math.min(1, a.maxDown / Math.max(1, peak));
    const delta = (cycle * net + curve[index] + (curve[index + 1] - curve[index]) * eased) * gain;
    if (elapsed >= duration * cycles) a.completed = true;
    return [a.startX, Math.round(a.startY + Math.max(0, Math.min(a.maxDown, delta)) * a.sf)];
  }
}
