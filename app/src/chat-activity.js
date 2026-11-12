// 表达只向Director申请已有动作；网络状态不会直接切换播放器。
export class ChatActivity {
  constructor() { this.revision = -1; this.active = false; this.until = 0; this.completed = false; }
  update(message, now) {
    if (!message || message.revision <= this.revision) return false;
    this.revision = message.revision;
    this.active = message.active === true;
    this.completed = message.completed === true;
    this.until = this.active ? now + Math.max(0, Math.min(125000, message.expires_at * 1000 - Date.now())) : this.completed ? now + 2500 : 0;
    return true;
  }
  ready(now) { return now < this.until; }
  behavior(now) { return { action: this.active && this.ready(now) ? "think" : this.completed && now < this.until ? "await" : "idle" }; }
}
