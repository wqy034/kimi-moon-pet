// 抱跑场景管理起手、收手和小跑；全程复用同组环抱素材。
const RUN_DISTANCE = 360, RUN_SPEED = 100;
const GRAB_MS = 280, TUCK_MS = 360;
function blendPoint(from, to, progress) {
  const t = Math.max(0, Math.min(1, progress));
  const eased = t * t * (3 - 2 * t);
  return from.map((value, i) => Math.round(value + (to[i] - value) * eased));
}
export class CursorCarry {
  constructor(now, invoke, log) {
    this.invoke = invoke; this.log = log;
    this.active = null; this.id = 0; this.generation = 0;
    this.reason = "disabled";
    this.inFlight = false; this.lastUpdateAt = 0;
    this.endedIds = new Set();
  }
  offer(now, sample, g) {
    if (g.maxX < 10 || g.x < 10 || g.x > g.maxX) { this.reason = "outside-walk-range"; return null; }
    if (!sample?.carry_available || sample.down || !sample.same_monitor ||
        sample.carrying || sample.still_ms < 10000 || now - sample.seenAt > 500) {
      this.reason = !sample?.carry_available ? "waiting-native-idle-or-enabled"
        : "waiting-still-cursor";
      return null;
    }
    const px = sample.x / g.sf, py = sample.y / g.sf;
    const possibilities = [-1, 1].map(direction => {
      const grip = direction < 0 ? g.carryLeft : g.carryRight;
      const goal = direction < 0 ? Math.max(10, g.x - RUN_DISTANCE) : Math.min(g.maxX, g.x + RUN_DISTANCE);
      return { direction, goal, distance: Math.hypot(px - g.x - grip[0], py - g.y - grip[1]) };
    }).filter(v => Math.sign(v.goal - g.x) === v.direction && Math.abs(v.goal - g.x) >= 32 &&
      v.distance <= Math.min(48 * g.scale, 48));
    const offer = possibilities.sort((a, b) => a.distance - b.distance)[0] || null;
    this.reason = offer ? "ready" : "not-near-hand";
    return offer;
  }
  ready(now, sample, geometry) { return !!this.offer(now, sample, geometry); }
  start(now, sample, geometry) {
    const offer = this.offer(now, sample, geometry);
    if (!offer) return false;
    this.generation++;
    this.active = { ...offer, phase: "grab", until: now + 6000, startX: sample.x, startY: sample.y,
      beginPending: false };
    this.id = 0; this.lastUpdateAt = 0; this.reason = "grab";
    return true;
  }
  canReach(sample, g) {
    if (!sample) return false;
    const x = sample.x / g.sf, y = sample.y / g.sf;
    return [g.catchPoint, g.carryLeft, g.carryRight].some(point =>
      point && Math.hypot(x - g.x - point[0], y - g.y - point[1]) <= 48 * g.scale);
  }
  catchApproach(sample, g) {
    const x = sample.x / g.sf, y = sample.y / g.sf;
    const candidates = [g.catchPoint, g.carryLeft, g.carryRight].filter(point =>
      Math.abs(y - g.y - point[1]) <= 48 * g.scale)
      .map(point => x - point[0]).filter(goal => goal >= 10 && goal <= g.maxX);
    return candidates.sort((a, b) => Math.abs(a - g.x) - Math.abs(b - g.x))[0];
  }
  startFromAttention(now, sample, g) {
    if (!sample?.carry_available || sample.down || !sample.same_monitor || sample.carrying ||
        now - sample.seenAt > 500 || !this.canReach(sample, g)) return false;
    const x = sample.x / g.sf, y = sample.y / g.sf;
    const choices = [-1, 1].map(direction => {
      const goal = direction < 0 ? Math.max(10, g.x - RUN_DISTANCE) : Math.min(g.maxX, g.x + RUN_DISTANCE);
      const point = direction < 0 ? g.holdLeft : g.holdRight;
      return { direction, goal, distance: Math.hypot(x - g.x - point[0], y - g.y - point[1]) };
    }).filter(v => Math.sign(v.goal - g.x) === v.direction && Math.abs(v.goal - g.x) >= 32)
      .sort((a, b) => a.distance - b.distance);
    if (!choices.length) return false;
    this.generation++;
    this.active = { ...choices[0], phase: "grab", until: now + 6000,
      startX: sample.x, startY: sample.y, beginPending: false };
    this.id = 0; this.lastUpdateAt = 0;
    return true;
  }
  observe(sample, now) {
    const a = this.active;
    if (!a || a.phase === "release") return;
    if (sample.down || !sample.same_monitor || (!this.id && !a.beginPending &&
        Math.hypot(sample.x - a.startX, sample.y - a.startY) > 2)) this.cancel(now, "user-input");
    // 租约建立后，坐标变化可能是自身移动；原生输入通道决定是否真正接管。
  }
  onEnded(payload, now) {
    this.endedIds.add(payload.id);
    if (this.endedIds.size > 16) this.endedIds.delete(this.endedIds.values().next().value);
    if (payload.id === this.id && this.active?.phase !== "release") this.cancel(now, `native-${payload.reason}`);
  }
  cancel(now, reason) {
    if (!this.active && !this.id) return;
    const id = this.id;
    this.active = null; this.id = 0; this.generation++;
    this.reason = reason;
    if (id) this.invoke("carry_cursor_end", { id }).catch(() => {});
    this.log(`carry-scene end=${reason}`);
  }
  behavior(now, geometry) {
    const a = this.active;
    if (!a) return null;
    if (now >= a.until) { this.cancel(now, a.phase === "release" ? "complete" : "timeout"); return null; }
    if (a.phase === "grab" && this.id && now >= a.grabUntil) {
      a.phase = "tuck"; a.tuckAt = now;
      a.tuckFrom = a.lastPoint || [a.startX, a.startY];
    }
    if (a.phase === "tuck" && now >= a.tuckAt + TUCK_MS) {
      a.phase = "run";
      a.runUntil = now + Math.min(3900, Math.abs(a.goal - geometry.x) / RUN_SPEED * 1000 + 200);
      a.until = a.runUntil + 800;
    }
    if (a.phase === "run" && (Math.abs(geometry.x - a.goal) < 2 || now >= a.runUntil)) {
      a.phase = "release"; a.until = now + 700;
      if (this.id) this.invoke("carry_cursor_end", { id: this.id }).catch(() => {});
    }
    this.reason = a.phase;
    if (a.phase === "release") return { action: "wave", carryPhase: "release" };
    const side = a.direction < 0 ? "left" : "right";
    if (a.phase === "grab" || a.phase === "tuck") {
      return { action: `carry_hold_${side}`, nativeCarry: true, carryPhase: a.phase };
    }
    return { walk: true, walkAction: `carry_run_${side}`, speed: RUN_SPEED,
      cursorTarget: a.goal, nativeCarry: true, carryPhase: "run" };
  }
  sync(now, packet) {
    const a = this.active;
    if (!a || a.phase === "release" || this.inFlight) return;
    const generation = this.generation;
    if (!this.id) {
      if (a.beginPending) return;
      a.beginPending = true;
      // 环抱首帧短暂停留；取得租约后再把原位置光标平滑收到胸前。
      this.invoke("carry_cursor_begin", { point: [a.startX, a.startY], mode: a.mode || "carry" }).then(id => {
        if (!this.active || generation !== this.generation || this.endedIds.has(id)) {
          this.invoke("carry_cursor_end", { id }).catch(() => {});
          if (generation === this.generation) this.cancel(performance.now(), "ended-before-ack");
          return;
        }
        this.id = id;
        this.onAcquired(performance.now(), this.active);
        this.log(`carry-scene acquired=${id}`);
      }).catch(e => { if (generation === this.generation) this.cancel(performance.now(), `begin-failed:${e}`); });
      return;
    }
    if (now - this.lastUpdateAt < 33) return;
    this.lastUpdateAt = now; this.inFlight = true;
    const point = this.cursorPoint(now, packet, a);
    if (generation !== this.generation || !this.id) { this.inFlight = false; return; }
    a.lastPoint = point;
    this.invoke("carry_cursor_update", { id: this.id, windowPos: packet.window_pos, cursorPos: point })
      .catch(e => { if (generation === this.generation) this.cancel(performance.now(), `update-failed:${e}`); })
      .finally(() => { this.inFlight = false; });
  }
  onAcquired(now, a) {
    a.grabAt = now;
    a.grabUntil = now + GRAB_MS;
    a.until = now + 5000;
  }
  cursorPoint(now, packet, a) {
    if (a.phase === "grab") return [a.startX, a.startY];
    if (a.phase === "tuck") return blendPoint(a.tuckFrom, packet.cursor_pos, (now - a.tuckAt) / TUCK_MS);
    return packet.cursor_pos;
  }
}
