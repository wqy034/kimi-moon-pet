// 仅绘制玩家声部；音符下沿到达亮线的时刻就是该音的目标时刻。
// 固定像素流速，拉高窗口只增加预告范围，不改变临线速度。
const SPEED = 144, HEAD = 14, TOP = 28, BOTTOM = 32;
const COLORS = ["#8cddff", "#d7b4ff", "#ffda86"];
const CODES = ["ZXCVBNM", "ASDFGHJ", "QWERTYU"];

export class MusicFlow {
  constructor() { this.source = null; this.slots = new Map(); }

  arrange(notes) {
    if (this.source === notes) return;
    this.source = notes; this.slots.clear();
    for (let lane = 0; lane < 7; lane++) {
      const column = notes.filter(note => note.player && note.index % 7 === lane).sort((a, b) => a.time - b.time);
      let group = [];
      const flush = () => {
        const octaves = [...new Set(group.map(note => Math.floor(note.index / 7)))].sort();
        for (const note of group) this.slots.set(note, { slot: octaves.indexOf(Math.floor(note.index / 7)), count: octaves.length });
        group = [];
      };
      for (const note of column) {
        // 仅在音符图形相碰时分开八度。同拍跨八度和弦不会遮盖；位置在整轮内固定。
        if (group.length && note.time - group[group.length - 1].time > (HEAD + 3) / SPEED) flush();
        group.push(note);
      }
      flush();
    }
  }

  draw(ctx, width, height, engine, time, flashes, now) {
    this.arrange(engine.displayNotes);
    const laneWidth = width / 7, judgeY = height - BOTTOM, horizon = Math.max(0, (judgeY - TOP) / SPEED);
    const demo = engine.mode === "demo", windowPx = engine.window * SPEED;
    ctx.save();
    for (let lane = 0; lane < 7; lane++) {
      ctx.fillStyle = lane % 2 ? "#1b2439" : "#151d30";
      ctx.fillRect(lane * laneWidth, 0, laneWidth, height);
      ctx.fillStyle = "#b5c8ed25"; ctx.fillRect(lane * laneWidth, TOP, 1, height - TOP);
    }
    ctx.save();
    ctx.beginPath(); ctx.rect(0, TOP, width, height - TOP); ctx.clip();
    // 节拍线从同一个音频时钟投影，帮助分辨节奏；不假设原谱一定是4/4。
    const beatSeconds = 60 / engine.song.bpm;
    ctx.strokeStyle = "#c8d6f019"; ctx.lineWidth = 1;
    for (let beat = Math.max(0, Math.ceil((time - .2) / beatSeconds)); beat * beatSeconds <= time + horizon; beat++) {
      const y = judgeY - (beat * beatSeconds - time) * SPEED;
      ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(width, y); ctx.stroke();
    }
    ctx.fillStyle = "#8cecff1c";
    ctx.fillRect(0, judgeY - windowPx, width, windowPx * 2);
    // 留出灰暗的判定线下方，和上方的可读谱面分离。
    ctx.fillStyle = "#090f1fcc"; ctx.fillRect(0, judgeY, width, height - judgeY);
    for (const note of engine.displayNotes) {
      if (!note.player || !Number.isInteger(note.index)) continue;
      const ahead = note.time - time;
      if (ahead > horizon || ahead < -engine.window - .18 || note.state === "hit") continue;
      const octave = Math.floor(note.index / 7), lane = note.index % 7;
      const layout = this.slots.get(note), count = layout?.count || 1, slot = layout?.slot || 0;
      const cell = laneWidth / count, x = lane * laneWidth + (slot + .5) * cell;
      const noteWidth = Math.max(8, Math.min(66, cell - 7)), y = judgeY - ahead * SPEED;
      const near = Math.abs(ahead) <= engine.window, missed = !demo && ahead < -engine.window;
      ctx.save();
      ctx.globalAlpha = ahead < -engine.window ? .45 : 1;
      ctx.shadowColor = near ? "#d3faff" : COLORS[octave]; ctx.shadowBlur = near ? 10 : 3;
      ctx.fillStyle = near ? "#f5fdff" : missed ? "#d5a1ac" : COLORS[octave];
      ctx.strokeStyle = "#0a1023"; ctx.lineWidth = 3;
      ctx.beginPath(); ctx.roundRect(x - noteWidth / 2, y - HEAD, noteWidth, HEAD, 4); ctx.stroke(); ctx.fill();
      ctx.shadowBlur = 0;
      // 明亮的下沿用于对线，字母直接对应键盘；宽度足够时再加音级数字。
      ctx.fillStyle = "#ffffff"; ctx.fillRect(x - noteWidth / 2 + 3, y - 2, Math.max(2, noteWidth - 6), 2);
      ctx.fillStyle = "#172038"; ctx.textAlign = "center"; ctx.textBaseline = "middle";
      ctx.font = 'bold 11px "Microsoft YaHei",sans-serif';
      const code = CODES[octave][lane];
      ctx.fillText(noteWidth >= 34 ? `${lane + 1} ${code}` : code, x, y - HEAD / 2 - 1);
      ctx.restore();
    }
    ctx.restore();
    // 判定线最后绘制，白色亮芯、青色辉光和深色衬边都不会被落下的音符盖住。
    ctx.strokeStyle = "#050a17"; ctx.lineWidth = 7;
    ctx.beginPath(); ctx.moveTo(0, judgeY); ctx.lineTo(width, judgeY); ctx.stroke();
    ctx.shadowColor = "#6fe9ff"; ctx.shadowBlur = 12; ctx.strokeStyle = "#8eefff"; ctx.lineWidth = 4; ctx.stroke();
    ctx.shadowBlur = 0; ctx.strokeStyle = "#ffffff"; ctx.lineWidth = 2; ctx.stroke();
    for (let lane = 0; lane < 7; lane++) {
      let glow = 0;
      for (let octave = 0; octave < 3; octave++) glow = Math.max(glow, ((flashes.get(octave * 7 + lane) || 0) - now) / 240);
      if (demo && engine.playing && engine.displayNotes.some(note => note.index % 7 === lane && time >= note.time && time < note.time + .12)) glow = 1;
      if (glow > 0) {
        ctx.fillStyle = `rgba(167,245,255,${Math.min(1, glow) * .5})`;
        ctx.fillRect(lane * laneWidth + 2, judgeY - 7, laneWidth - 4, 14);
      }
      ctx.fillStyle = "#a9c1da"; ctx.textAlign = "center"; ctx.textBaseline = "middle";
      ctx.font = 'bold 11px "Microsoft YaHei",sans-serif'; ctx.fillText(String(lane + 1), (lane + .5) * laneWidth, judgeY + 17);
    }
    ctx.restore();
  }
}
