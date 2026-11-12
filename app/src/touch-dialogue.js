// 本地触摸台词：不调用模型，不保存用户操作记录。
const lines = {
  lift: ["……要去哪里？", "我自己会走。", "轻一点，发饰会歪。"],
  place: ["嗯，这里也不错。", "站稳了。", "这里离你近一点。"],
  land: ["……落地了。", "下次轻一点。", "裙摆乱了……"],
  trip1: ["等一下……有点晕。", "让我缓一会儿。", "地面好像转了一圈。"],
  trip2: ["……这次摔得有点重。", "先让我坐一会儿。", "下次放轻一点。"],
  reading: ["看到一半呢……", "这一页还没看完。", "要一起看看吗？"],
  wake: ["嗯……我醒了。", "让我睁开眼睛。", "……再缓一会儿。"],
  comfort: ["刚才那一下……有点疼。", "我没事，坐一会儿就好。", "下次轻一点。"],
  repeat: ["我听见了。", "等等……头发乱了。", "轻一点。"],
  protest: ["先让我缓一会儿。", "先停一下。", "你还在戳……"],
  busy_head: ["等一下，我还看到一半。", "嗯……我还在忙。", "这页快看完了。"],
  busy_face: ["脸要被你戳歪了。", "在呢……等我忙完这一点。", "差一点就想到了。"],
  busy_body: ["怎么了？还要一小会儿。", "我在忙，也看见你了。", "你先坐一会儿。"],
  head: ["……再摸一下也可以。", "轻一点，发饰会歪。", "嗯，今天的手很温柔。", "我收下了。"],
  face: ["脸要被你戳歪了。", "嗯？", "被发现我在发呆了……", "我在听。"],
  body: ["怎么了？", "我在这里。", "要一起发会儿呆吗？", "今天想做什么？"],
  reunion: ["你来了。", "我还在这里。", "刚才那页，还没翻过去。"],
};

export class TouchDialogue {
  constructor() {
    this.lastSpokenAt = -Infinity;
    this.lastTouchAt = null;
    this.recent = [];
  }

  respond({ now, region, sleeping, busy, recentFall, reading, count, event }) {
    const reunion = this.lastTouchAt !== null && now - this.lastTouchAt > 180000;
    this.lastTouchAt = now;
    // 持续快速点击仍有动作反馈，台词最多每 900ms 更新一次。
    if (!event && now - this.lastSpokenAt < 900) return null;
    const key = event || (sleeping ? "wake" : count >= 4 ? "protest" : count >= 2 ? "repeat"
      : recentFall ? "comfort" : busy ? `busy_${region}` : reading ? "reading" : reunion ? "reunion" : region);
    const pool = lines[key] || lines.body;
    const available = pool.filter(text => !this.recent.includes(text));
    const choices = available.length ? available : pool.filter(text => text !== this.recent.at(-1));
    const text = choices[Math.floor(Math.random() * choices.length)] || pool[0];
    this.recent.push(text);
    this.recent = this.recent.slice(-4);
    this.lastSpokenAt = now;
    return text;
  }
}
