import { DailyLife } from "./daily-life.js";
import { CursorAttention } from "./cursor-attention.js";
import { LifeActivity } from "./life-activity.js";
import { MusicActivity } from "./music-activity.js";
import { ChatActivity } from "./chat-activity.js";

const between = ([min, max]) => min + Math.random() * (max - min);
const ACTIVITIES = [
  { id: "care", manual: true, weight: 0, duration: [4200, 4200], cooldown: [0, 0], eligible: c => c.careReady },
  { id: "music", manual: true, weight: 0, duration: [0, 0], cooldown: [0, 0], eligible: c => c.musicReady },
  { id: "job", manual: true, weight: 0, duration: [15000, 25000], cooldown: [0, 0], eligible: c => c.jobReady },
  { id: "chat", manual: true, weight: 0, duration: [0, 0], cooldown: [0, 0], eligible: c => c.chatReady },
  { id: "needs", weight: .8, duration: [4200, 4200], cooldown: [900000, 1200000], eligible: c => c.dailySpeech && c.needsReady },
  { id: "idle", weight: 3, duration: [6000, 15000], cooldown: [0, 0], eligible: () => true },
  { id: "read", weight: 2.2, duration: [12000, 20000], cooldown: [50000, 90000], eligible: () => true },
  { id: "walk", weight: 1.5, duration: [2000, 4000], cooldown: [35000, 65000], eligible: c => c.wander && !c.attentionReady },
  { id: "yawn", weight: .7, duration: [1800, 1800], cooldown: [80000, 130000], eligible: c => c.idleMs >= 25000 },
  { id: "attention", weight: 10, duration: [0, 0], cooldown: [0, 0], eligible: c => c.attentionReady },
];

export class BehaviorDirector {
  constructor(now, log = () => {}, invoke = () => Promise.reject("缺少原生通道")) {
    this.reading = new DailyLife();
    this.life = new LifeActivity();
    this.music = new MusicActivity(invoke, log);
    this.chat = new ChatActivity();
    this.attention = new CursorAttention(now, invoke, log);
    // 兼容现有诊断入口；生命周期和资格只由attention持有。
    this.cursor = this.attention.cursor;
    this.carry = this.attention.carry;
    this.rope = this.attention.rope;
    this.active = null;
    this.nextDecisionAt = now + between([10000, 16000]);
    this.cooldowns = new Map();
    this.history = [];
    this.cursorReadySince = null;
    this.reason = "startup-rest";
    this.layer = "";
    this.log = log;
  }
  isReading(now) { return this.active?.id === "read" && this.reading.isReading(now); }
  pauseForTouch(now, duration) {
    if (this.isReading(now)) {
      this.reading.pauseForTouch(now, duration);
      this.active.until = this.reading.readUntil;
    } else this.interrupt(now, "touch");
  }
  observeCursor(sample, now) {
    this.attention.observe(sample, now);
    if (this.active?.id === "attention" && !this.attention.active) this.finish(now, this.attention.reason);
  }
  finish(now, reason) {
    if (!this.active) return;
    const activity = this.active;
    const spec = ACTIVITIES.find(a => a.id === activity.id);
    this.cooldowns.set(activity.id, now + between(spec.cooldown));
    this.active = null;
    this.reading.stop();
    this.attention.cancel(now, reason);
    this.nextDecisionAt = now + between([8000, 18000]);
    this.log(`director end=${activity.id} reason=${reason}`);
  }
  interrupt(now, reason = "external-interaction") {
    this.finish(now, reason);
    this.nextDecisionAt = Math.max(this.nextDecisionAt, now + 8000);
    this.cursorReadySince = null;
  }
  route(layer, now, behavior, preserveRead = false) {
    this.cursorReadySince = null;
    if (!(preserveRead && this.active?.id === "read") && this.active) this.interrupt(now, layer);
    if (this.layer !== layer) this.log(`director layer=${layer}`);
    this.layer = layer;
    this.reason = layer;
    return behavior;
  }
  step(now, c) {
    const musicReady = this.music.ready(now);
    const conversation = c.conversation === true || this.chat.ready(now);
    // 扩展窗口只屏蔽光标骚扰，不占用日常/打工活动槽，也不持续重置睡眠计时。
    if ((c.extensionInteraction || conversation) && this.active?.id === "attention") this.finish(now, "extension-interaction");
    if (c.held) return this.route("held", now, { action: c.heldAction });
    if (c.falling) return this.route("falling", now, { action: "falling" });
    if (c.override) return this.route("touch-feedback", now, c.override, !conversation);
    if (c.menu || c.scaling) return this.route("panel", now, c.busy ? c.work : { action: "idle" });
    if (c.busy) return this.route("cli", now, c.work);
    // 完整互动期间延后自动入睡，避免刚拉到一半就睡着；整轮自身有5分钟上限。
    if (c.idleMs >= c.lightSleepMs && !this.attention.active && !musicReady && !conversation) return this.route("sleep", now,
      { action: c.idleMs >= c.deepSleepMs ? "sleep" : "sleep_light" });
    if (c.pressing) return this.route("pressing", now, { action: "idle" }, !conversation);
    if (c.protectedBubble && !(c.lifeBubble && ["care", "needs"].includes(this.active?.id))) return this.route("protected-bubble", now, { action: "idle" }, !conversation);
    this.layer = "autonomous";

    const context = { ...c, musicReady, chatReady: conversation, careReady: this.life.ready(), jobReady: this.life.jobReady(), needsReady: this.life.needsReady() };
    // 用户安排的反馈/工作也登记在同一个活动表，不额外启动动作定时器。
    const manual = this.active?.id === "care" && now < this.active.until
      ? ACTIVITIES.find(a => a.id === "care") : ACTIVITIES.find(a => a.manual && a.eligible(context));
    if (manual && this.active && this.active.id !== manual.id) this.finish(now, "life-request");
    if (this.active?.id === "job" && !context.jobReady) this.finish(now, "job-paused-or-complete");
    if (this.active?.id === "music" && !musicReady) this.finish(now, "music-paused-or-complete");
    if (this.active?.id === "chat" && !context.chatReady) this.finish(now, "chat-finished");
    const attentionReady = !manual && this.attention.ready(now, { ...c, extensionInteraction: c.extensionInteraction || conversation });
    if (attentionReady) {
      if (this.cursorReadySince === null) {
        this.cursorReadySince = now;
        if (this.active?.id === "idle") this.finish(now, "cursor-opportunity");
        this.nextDecisionAt = Math.min(this.nextDecisionAt, now + 1000);
      }
    } else this.cursorReadySince = null;
    if (this.active?.id === "walk" && !c.wander) this.finish(now, "disabled");
    if (this.active?.id === "read") this.active.until = this.reading.readUntil;
    if (this.active?.id === "attention" && this.attention.active) this.active.until = this.attention.until;
    if (this.active?.id === "attention" && !this.attention.active) this.finish(now, this.attention.reason);
    if (this.active?.id === "music") this.active.until = now + 2000;
    if (this.active?.id === "chat") this.active.until = Math.max(now + 1000, this.chat.until);
    if (this.active && now >= this.active.until) this.finish(now, "complete");
    if (!this.active && !manual && (now < this.nextDecisionAt || c.idleMs < 8000)) {
      this.reason = "quiet-rest"; return { action: "idle" };
    }
    let started = false;
    if (!this.active) {
      context.attentionReady = attentionReady;
      const candidates = (manual ? [manual] : ACTIVITIES.filter(a => !a.manual && a.eligible(context) && now >= (this.cooldowns.get(a.id) || 0)))
        .map(a => ({ ...a, effectiveWeight: (a.manual ? 1 : a.weight) * (a.id === "yawn" && c.night ? 3 : 1) *
          (this.history.at(-1) === a.id ? .2 : this.history.includes(a.id) ? .65 : 1) *
          (a.id === "attention" && now - this.cursorReadySince >= 20000 ? 3 : 1) }));
      let ticket = Math.random() * candidates.reduce((sum, a) => sum + a.effectiveWeight, 0);
      const selected = candidates.find(a => (ticket -= a.effectiveWeight) <= 0) || candidates[0];
      const duration = selected.id === "yawn" ? c.yawnMs : between(selected.duration);
      this.active = { id: selected.id, until: now + duration, direction: Math.random() < .5 ? -1 : 1 };
      if (selected.id === "music") this.active.until = now + 2000;
      if (selected.id === "chat") this.active.until = Math.max(now + 1000, this.chat.until);
      this.life.start(selected.id);
      if (selected.id === "read") this.reading.start(now, duration);
      if (selected.id === "attention") {
        if (!this.attention.start(now, c)) { this.finish(now, "attention-no-longer-ready"); return { action: "idle" }; }
        this.active.until = this.attention.until;
      }
      this.history = [...this.history, selected.id].slice(-3);
      this.log(`director choose=${selected.id} candidates=${candidates.map(a => a.id).join(",")} duration=${Math.round(this.active.until - now)}`);
      started = true;
    }
    const a = this.active;
    this.reason = a.id;
    switch (a.id) {
      case "chat": return this.chat.behavior(now);
      case "music": return this.music.behavior();
      case "care":
      case "job":
      case "needs": return this.life.behavior(a.id, started);
      case "read": return { ...this.reading.behavior(now, c.pageMs), startedRead: started };
      case "walk": return { walk: true, speed: 40, startDirection: started ? a.direction : null };
      case "yawn": return { action: "yawn" };
      case "attention": {
        const behavior = this.attention.behavior(now, c);
        if (!behavior) { this.finish(now, this.attention.reason); return { action: "idle" }; }
        this.active.until = this.attention.until;
        this.reason = `attention:${this.attention.reason}`;
        return behavior;
      }
      default: return { action: "idle" };
    }
  }
}
