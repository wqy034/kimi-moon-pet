// 养成动作适配器。出场资格、互斥和冷却由BehaviorDirector统一管理。
export class LifeActivity {
  constructor() { this.snapshot = null; this.pending = null; this.lastFeedback = 0; this.current = null; }
  update(snapshot) {
    if (!this.snapshot || snapshot.revision >= this.snapshot.revision) this.snapshot = snapshot;
  }
  feedback(message) {
    if (!message || message.id <= this.lastFeedback || message.expires_at * 1000 < Date.now()) return false;
    this.lastFeedback = message.id;
    this.pending = message;
    return true;
  }
  ready() { return !!this.pending && Date.now() < this.pending.expires_at * 1000; }
  jobReady() { return !this.snapshot?.error && !!this.snapshot?.job && !this.snapshot.job.ready && this.snapshot.stats.energy >= 20; }
  needsReady() {
    const s = this.snapshot;
    return !s?.error && s && (s.stats.food < 25 || s.stats.water < 25 || s.stats.energy < 25);
  }
  start(id) {
    if (id === "care") { this.current = this.pending; this.pending = null; }
  }
  behavior(id, started = false) {
    if (id === "care") {
      const message = this.current;
      const itemActions = { water: "care_drink", bread: "care_eat_bread", cake: "care_eat_cake" };
      const action = message?.action === "use" ? (Object.hasOwn(itemActions, message.item) ? itemActions[message.item] : "await")
        : message?.action === "start_job" ? "life_books" : "await";
      return { action, lifeSay: started ? message?.line : null };
    }
    if (id === "job") return { action: "life_books", lifeMode: "job" };
    const s = this.snapshot?.stats;
    const line = s?.water < 25 ? "有一点口渴了……" : s?.food < 25 ? "有点饿了。" : "有一点困了……";
    return { action: s?.energy < 25 ? "yawn" : "await", lifeSay: started ? line : null, lifeReminder: true };
  }
}
