// 系统本地时间只影响生活节奏；不重置空闲计时，不主动唤醒角色。
const PERIODS = [
  [6, "late", "夜很深了……我陪你坐一会儿。"],
  [11, "morning", "早。今天慢慢来。"],
  [14, "noon", "中午了……歇一会儿也好。"],
  [18, "afternoon", "下午的光很安静。"],
  [22, "evening", "晚上好。我在这里。"],
  [24, "night", "有点困了……琴先放在这里。"],
];
export class TimeRhythm {
  constructor() {
    this.nextSampleAt = 0;
    this.seen = new Map();
    this.settings = {};
  }
  configure(settings) { this.settings = settings; }
  snapshot(now) {
    if (!this.current || now >= this.nextSampleAt) {
      const date = new Date(), hour = date.getHours();
      const [, period, greeting] = PERIODS.find(([end]) => hour < end);
      const day = `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
      this.current = { night: hour >= 22 || hour < 6, period, greeting, day };
      this.nextSampleAt = now + 1000;
    }
    const s = this.settings, night = this.current.night;
    return { ...this.current,
      lightSleepMs: (night ? s.night_light_sleep_s ?? 300 : s.light_sleep_s ?? 900) * 1000,
      deepSleepMs: (night ? s.night_deep_sleep_s ?? 600 : s.deep_sleep_s ?? 1200) * 1000,
    };
  }
  takeGreeting(snapshot) {
    const { period, day, greeting } = snapshot;
    const key = `pet-greeting-${period}`;
    if (this.seen.get(key) === day) return null;
    try { if (localStorage.getItem(key) === day) return null; } catch (_) { /* 内存去重仍可用 */ }
    this.seen.set(key, day);
    try { localStorage.setItem(key, day); } catch (_) { /* 无存储权限时本次运行去重 */ }
    return greeting;
  }
}
