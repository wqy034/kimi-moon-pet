import { CursorPlay } from "./cursor-play.js";
import { CursorCarry } from "./cursor-carry.js";
import { RopePull } from "./rope-pull.js";
import { DailyLife } from "./daily-life.js";

const WAIT_MS = 45000, COMPLAINT_MS = 2400;
const FIRST_LINE = "……你的光标停了很久。";
const SECOND_LINE = "我在这里。琴也在。";

// 整轮互动的唯一状态与冷却。三个动作播放器不再各自参加调度。
export class CursorAttention {
  constructor(now, invoke, log) {
    this.cursor = new CursorPlay(now);
    this.carry = new CursorCarry(now, invoke, log);
    this.rope = new RopePull(now, invoke, log);
    this.reading = new DailyLife();
    this.active = null;
    this.nextAt = now + 15000;
    this.reason = "startup-rest";
    this.log = log;
  }
  get sample() { return this.cursor.sample; }
  get until() { return this.active?.deadline || 0; }
  get nativeScene() { return this.active?.child || null; }
  get controlsActive() { return this.active?.child === this.rope || this.active?.child === this.carry; }
  ready(now, c) {
    if (c.extensionInteraction) { this.reason = "extension-interaction"; return false; }
    if (this.active) return false;
    if (now < this.nextAt) { this.reason = "cooldown"; return false; }
    if (!c.cursorEnabled && !c.carryEnabled) { this.reason = "disabled"; return false; }
    const p = this.sample;
    if (!this.cursor.ready(now) || (p.input_idle_ms ?? p.still_ms) < 10000) {
      this.reason = "waiting-user-idle"; return false;
    }
    this.reason = "ready";
    return true;
  }
  start(now, c) {
    if (!this.ready(now, c) || !this.cursor.start(now, c.geometry)) return false;
    this.active = { phase: "greet", deadline: now + 300000, phaseAt: now,
      inputRevision: this.sample.input_revision, child: null, announced: false };
    this.reason = "greet";
    this.log("attention phase=greet");
    return true;
  }
  observe(sample, now) {
    const previous = this.sample;
    this.cursor.observe(sample, now);
    const a = this.active;
    if (a) {
      const hasRevision = sample.input_revision !== undefined && a.inputRevision !== undefined;
      const own = this.rope.id || this.carry.id || this.rope.active?.beginPending || this.carry.active?.beginPending;
      const responded = hasRevision ? sample.input_revision !== a.inputRevision
        : !own && previous && (sample.x !== previous.x || sample.y !== previous.y);
      if (responded || sample.down || !sample.same_monitor) {
        this.cancel(now, "user-responded"); return;
      }
    }
    this.carry.observe(sample, now);
    this.rope.observe(sample, now);
  }
  onEnded(payload, now) {
    this.carry.onEnded(payload, now);
    this.rope.onEnded(payload, now);
  }
  cancel(now, reason) {
    if (!this.active) return;
    this.active = null;
    this.cursor.cancel();
    this.carry.cancel(now, reason);
    this.rope.cancel(now, reason);
    this.reading.stop();
    this.nextAt = now + 120000;
    this.reason = reason;
    this.log(`attention end=${reason}`);
  }
  phase(name, now) {
    const a = this.active;
    a.phase = name; a.phaseAt = now; a.announced = false; a.child = null;
    this.reason = name;
    this.log(`attention phase=${name}`);
  }
  wait(now) {
    this.phase("wait", now);
    this.reading.start(now, WAIT_MS);
  }
  startBatch(now, c, reminder) {
    if (!c.carryEnabled || !this.rope.start(now, this.sample, c.geometry)) return false;
    this.phase(reminder ? "reminder-pull" : "pull", now);
    this.active.child = this.rope;
    return true;
  }
  startCarry(now, c) {
    if (!this.carry.startFromAttention(now, this.sample, c.geometry)) return false;
    this.phase("carry", now);
    this.active.child = this.carry;
    return true;
  }
  behavior(now, c) {
    const a = this.active, p = this.sample;
    if (!a) return null;
    if (now >= a.deadline || !p || now - p.seenAt > 500) {
      this.cancel(now, "episode-timeout-or-stale-sensor"); return null;
    }
    if (!c.cursorEnabled && !c.carryEnabled) { this.cancel(now, "disabled"); return null; }
    if (this.controlsActive && !c.carryEnabled) { this.cancel(now, "control-disabled"); return null; }
    if (a.phase === "greet") {
      const b = this.cursor.behavior(now, c.geometry);
      if (b) return { ...b, say: b.announce ? FIRST_LINE : null };
      if (this.cursor.reason !== "finished") { this.cancel(now, "greet-interrupted"); return null; }
      this.cursor.cancel();
      // 四轮拉绳是首轮提醒的一部分；并非直接升级为抱跑。
      if (c.carryEnabled && !this.carry.canReach(p, c.geometry) && this.startBatch(now, c, true)) {
        return this.behavior(now, c);
      }
      this.wait(now);
    }
    if (a.phase === "reminder-pull" || a.phase === "pull" || a.phase === "carry") {
      const child = a.child;
      const b = child.behavior(now, c.geometry);
      if (b) return b;
      if (child.reason !== "complete") { this.cancel(now, child.reason); return null; }
      if (a.phase === "carry") { this.cancel(now, "complete"); return null; }
      if (a.phase === "reminder-pull") this.wait(now);
      else this.phase("pause", now);
    }
    if (a.phase === "wait") {
      if (now - a.phaseAt < WAIT_MS) return this.reading.behavior(now, c.pageMs);
      this.reading.stop();
      this.phase("complaint", now);
    }
    if (a.phase === "complaint") {
      if (now - a.phaseAt < COMPLAINT_MS) {
        const say = !a.announced ? SECOND_LINE : null;
        a.announced = true;
        return { action: "angry", say };
      }
      if (!c.carryEnabled) { this.cancel(now, "reminder-complete"); return null; }
      this.phase("pause", now);
      a.phaseAt = now - 1000;
    }
    if (a.phase === "approach-catch") {
      if (now >= a.arriveBy) { this.cancel(now, "catch-approach-timeout"); return null; }
      if (Math.abs(c.geometry.x - a.goal) >= 2) {
        return { walk: true, speed: 64, cursorTarget: a.goal };
      }
      this.phase("pause", now);
      a.phaseAt = now - 1000;
    }
    if (a.phase === "pause") {
      if (now - a.phaseAt < 1000) return { action: "await" };
      // 每批从当前实测光标位置续拉，既不回原点，也不进入独立长冷却。
      if (!p.carry_available || p.carrying) {
        if (now - a.phaseAt < 2500) return { action: "await" };
        this.cancel(now, "native-not-ready"); return null;
      }
      if (this.carry.canReach(p, c.geometry)) {
        if (this.startCarry(now, c)) return this.behavior(now, c);
        this.cancel(now, "cannot-run-from-here"); return null;
      }
      // 普通招呼会停在鼠标旁留出空隙，升级时再走近到真正可抓的位置。
      const goal = this.carry.catchApproach(p, c.geometry);
      if (Number.isFinite(goal) && Math.abs(goal - c.geometry.x) >= 2) {
        this.phase("approach-catch", now);
        a.goal = goal;
        a.arriveBy = now + Math.abs(goal - c.geometry.x) / 64 * 1000 + 1500;
        return this.behavior(now, c);
      }
      if (this.startBatch(now, c, false)) return this.behavior(now, c);
      this.cancel(now, "unreachable-cursor"); return null;
    }
    return { action: "idle" };
  }
}
