// 阅读播放器：只处理一轮静读/翻页，不决定何时开始，也不参与全局竞争。
const randomMs = (min, max) => min + Math.random() * (max - min);
export class DailyLife {
  constructor() {
    this.readUntil = 0;
    this.nextPageAt = 0;
    this.pageUntil = 0;
  }
  isReading(now) { return now < this.readUntil; }
  stop() {
    this.readUntil = 0;
    this.pageUntil = 0;
  }
  start(now, duration) {
    this.readUntil = now + duration;
    this.nextPageAt = now + randomMs(2000, 3500);
    this.pageUntil = 0;
  }
  pauseForTouch(now, duration) {
    if (!this.isReading(now)) return;
    this.readUntil += duration;
    // 翻页被打断后静读收尾，不在回应结束时从半页突然续播。
    this.pageUntil = 0;
    this.nextPageAt = now + duration + randomMs(2000, 3500);
  }
  behavior(now, pageMs) {
    if (!this.isReading(now)) return null;
    if (now >= this.nextPageAt && now + pageMs < this.readUntil) {
      this.pageUntil = now + pageMs;
      this.nextPageAt = this.pageUntil + randomMs(3000, 5000);
    }
    return { action: now < this.pageUntil ? "daily_page" : "daily_read" };
  }
}
