// 月之少女 · 前端渲染器 + 动画播放器 + 本地交互
// 架构：窗口即角色（320×360 透明小窗口）。位置 = OS 窗口位置，
// 拖拽由 Rust 线程轮询光标直接 set_position（硬绑定无漂移），
// 走路/摔落由本文件驱动 setPosition。页面元素从不做布局位移。

import { TouchDialogue } from "./touch-dialogue.js";
import { BehaviorDirector } from "./behavior-director.js";
import { TimeRhythm } from "./time-rhythm.js";
import { PetShape } from "./pet-shape.js";

const TAU = window.__TAURI__;
const appWin = TAU.window.getCurrentWindow();
const canvas = document.getElementById("stage");
const drawing = document.createElement("canvas");
drawing.width = 320; drawing.height = 360;
const ctx = drawing.getContext("2d", { willReadFrequently: true });
const petShape = new PetShape(ctx, canvas, (command,args) => TAU.core.invoke(command,args),
  sprite => { drawnSprite = sprite; trackBubbleAnchor(sprite); }, logDebug);

const BOX_W = 320, BOX_H = 360;      // 画布内部坐标（固定）
canvas.width = BOX_W;
canvas.height = BOX_H;
const FEET_IN_BOX = BOX_H - 24;      // 脚底在盒子里的 y（画布内部）

// 屏幕几何（随缩放设置变化）
let S = 1;                 // 体型缩放
let winW = 320, winH = 360;   // 窗口 CSS 尺寸
let feetOff = 336;         // 脚底距窗口顶 = FEET_IN_BOX * S
let modelH = 280;          // 身高阈值 = 280 * S：拎起一个身高内松手=摔回基线，超过=搬家
let wanderOn = true;       // 待机闲逛开关

function logDebug(line) {
  TAU.core.invoke("log_debug", { line }).catch(() => {});
}

// ---------- 窗口位置（CSS px，提交时换算物理像素） ----------

let SF = 1;          // scaleFactor：物理 = CSS * SF
let winX = 0, winY = 0; // 窗口左上角（CSS px）
let baseY = 0;       // 脚下基线对应的窗口 y（她当前"站在"的高度）
let floorY = 0;      // 脚底贴工作区底部时的窗口 y

let lastSentX = -1, lastSentY = -1, lastSentT = 0;
let commitErrLogged = false;
function commitPos(now, force = false) {
  if (!force && now - lastSentT < 33) return; // 30Hz 上限，走路不需要更高
  if (Math.abs(winX - lastSentX) < 0.5 && Math.abs(winY - lastSentY) < 0.5) return;
  lastSentX = winX; lastSentY = winY; lastSentT = now;
  appWin.setPosition(new TAU.window.PhysicalPosition(
    Math.round(winX * SF), Math.round(winY * SF)
  )).catch((e) => {
    // 失败不能保留“已发送”坐标，否则静止时永远不再尝试定位。
    lastSentX = lastSentY = -Infinity;
    if (!commitErrLogged) { commitErrLogged = true; logDebug("setPosition_err " + e); }
  });
}

// ---------- 资源加载 ----------
// 素材/配置文件含中文路径，一律走 Tauri asset 协议（convertFileSrc + 绝对路径）。

let PET_HOME = "";
function fileUrl(rel) {
  return TAU.core.convertFileSrc(`${PET_HOME}/${rel}`);
}
function frameUrl(dir, name) {
  return fileUrl(`帧文件/${dir}/${name}.png`);
}

const imageCache = new Map();
const hitMasks = new WeakMap();
let drawnSprite = null;
// 对实际画出的精灵做逆变换，缩放、呼吸和动作偏移后仍能正确命中。
function touchRegion(event) {
  if (!drawnSprite) return null;
  const rect = canvas.getBoundingClientRect();
  if (!rect.width || !rect.height) return null;
  const { img, transform, ox, oy, action } = drawnSprite;
  const point = new DOMPoint((event.clientX - rect.left) * BOX_W / rect.width,
    (event.clientY - rect.top) * BOX_H / rect.height).matrixTransform(transform.inverse());
  const x = Math.floor(point.x - ox), y = Math.floor(point.y - oy);
  if (x < 0 || y < 0 || x >= img.width || y >= img.height) return null;
  if (!hitMasks.has(img)) {
    const probe = document.createElement("canvas");
    probe.width = img.width; probe.height = img.height;
    const probeCtx = probe.getContext("2d", { willReadFrequently: true });
    try {
      probeCtx.drawImage(img, 0, 0);
      hitMasks.set(img, probeCtx.getImageData(0, 0, img.width, img.height).data);
    } catch {
      // asset 协议若限制像素读取，保守使用人物中央区域，不命中空白边缘。
      hitMasks.set(img, null);
      logDebug(`touch-mask-unavailable action=${action}`);
    }
  }
  const pixels = hitMasks.get(img);
  if (pixels ? pixels[(y * img.width + x) * 4 + 3] < 24 : x < 80 || x > 240 || y < 30 || y > 310) return null;
  // 卧姿、举牌等构图不套站姿脸部范围，只使用情境回应。
  const standing = ["idle", "gaze", "walk_left", "walk_right", "wave", "poked", "await", "think", "reading", "daily_read", "daily_page", "writing", "review", "sad", "angry", "yawn"].includes(action);
  if (standing && x >= 110 && x <= 210) {
    if (y >= 30 && y < 77) return "head";
    if (y >= 77 && y < 120) return "face";
  }
  return "body";
}
function loadImage(src) {
  if (imageCache.has(src)) return imageCache.get(src);
  const img = new Image();
  img.crossOrigin = "anonymous"; // asset协议返回CORS头，允许读取像素用于人物轮廓与触摸判断。
  img.onload = () => { lastFrameKey = ""; }; // 静态动作先画了兜底图时，原帧就绪也要重绘
  img.onerror = () => logDebug(`image-load-err ${src}`);
  img.src = src;
  imageCache.set(src, img);
  return img;
}

let ACTIONS = {};
const spriteBounds = new Map();
let anchorTarget = null, anchorSmooth = null, anchorSent = null;
let anchorTime = 0, anchorSentAt = -Infinity, anchorBusy = false;
function trackBubbleAnchor(sprite) {
  const box = spriteBounds.get(sprite.img.src) || [0, 0, sprite.img.width, sprite.img.height];
  const points = [[box[0], box[1]], [box[2], box[1]], [box[0], box[3]], [box[2], box[3]]]
    .map(([x, y]) => new DOMPoint(x + sprite.ox, y + sprite.oy).matrixTransform(sprite.transform));
  anchorTarget = [Math.min(...points.map(p => p.x)), Math.min(...points.map(p => p.y)),
    Math.max(...points.map(p => p.x)), Math.max(...points.map(p => p.y))];
}
function updateBubbleAnchor(now) {
  if (!anchorTarget) return;
  const alpha = 1 - Math.exp(-Math.min(100, now - anchorTime) / 100);
  anchorTime = now;
  if (!anchorSmooth) anchorSmooth = [...anchorTarget];
  else anchorSmooth = anchorTarget.map((value, i) => {
    // 起身时顶部立即向上避让，趴下时平滑跟近，避免过渡中盖住人物。
    if ((i < 2 && value < anchorSmooth[i]) || (i >= 2 && value > anchorSmooth[i])) return value;
    return anchorSmooth[i] + (value - anchorSmooth[i]) * alpha;
  });
  const bounds = anchorSmooth.map(Math.round);
  const rising = anchorSent && bounds[1] < anchorSent[1] - 4;
  if (anchorBusy || (!rising && now - anchorSentAt < 80)) return;
  if (anchorSent && bounds.every((value, i) => Math.abs(value - anchorSent[i]) < 2)) return;
  anchorBusy = true;
  anchorSentAt = now;
  TAU.core.invoke("bubble_anchor", { bounds }).then(() => { anchorSent = bounds; })
    .catch(e => logDebug(`bubble-anchor-err ${e}`)).finally(() => { anchorBusy = false; });
}
async function loadConfig() {
  PET_HOME = await TAU.core.invoke("get_pet_home");
  const res = await fetch(fileUrl("config/动画.json"));
  const data = await res.json();
  ACTIONS = data.actions;
  try {
    const response = await fetch(fileUrl("config/可见边界.json"));
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    for (const [key, box] of Object.entries(await response.json())) {
      if (Array.isArray(box) && box.length === 4 && box.every(Number.isFinite)) {
        spriteBounds.set(new URL(fileUrl(`帧文件/${key}.png`), location.href).href, box);
      }
    }
  } catch (e) { logDebug(`sprite-bounds-err ${e}`); }
  for (const a of Object.values(ACTIONS)) {
    for (const f of a.frames) loadImage(frameUrl(a.dir, f));
  }
  loadImage(fileUrl("道具/睡觉_Zzz.png"));
  loadImage(fileUrl("道具/鼠标指针.png"));
}

// ---------- 动画播放器 ----------

class Player {
  constructor() {
    this.actionName = null;
    this.startedAt = 0;
    this.replayAt = 0;
  }
  play(name, now, force = false) {
    if (!force && this.actionName === name) return;
    this.actionName = name;
    this.startedAt = now;
    this.replayAt = 0;
  }
  /** 返回 {img, meta, progress, done, idx} */
  frame(now) {
    const a = ACTIONS[this.actionName];
    if (!a) return null;
    const seq = a.frames;
    const elapsed = now - this.startedAt;
    const totalMs = seq.length * a.frame_ms;
    let idx, progress;
    if (a.loop) {
      idx = Math.floor(elapsed / a.frame_ms) % seq.length;
      progress = (elapsed % totalMs) / totalMs;
    } else {
      const holdMs = a.hold_ms || 0;
      if (elapsed < totalMs) {
        idx = Math.floor(elapsed / a.frame_ms);
        progress = elapsed / totalMs;
      } else if (elapsed < totalMs + holdMs) {
        idx = seq.length - 1;
        progress = 1;
      } else {
        // 播完：保持最后一帧（若有 replay_gap_ms 则由外层决定何时重播）
        const img = imageCache.get(frameUrl(a.dir, seq[seq.length - 1]));
        return { meta: a, img, progress: 1, done: true, idx: seq.length - 1 };
      }
    }
    const img = imageCache.get(frameUrl(a.dir, seq[idx]));
    return { meta: a, img, progress, done: false, idx };
  }
}

// ---------- 程序动效 ----------

const TAU2 = Math.PI * 2;

function applyEffects(meta, progress, now, cx, baseYInBox) {
  let x = cx, y = baseYInBox, sx = 1, sy = 1, rot = 0;

  if (meta.squash) {
    const sinceStart = now - player.startedAt;
    const rb = meta.squash.rebound_ms;
    if (sinceStart < rb) {
      const t = sinceStart / rb;
      sy = 1 - (1 - meta.squash.scale_y) * Math.cos(t * Math.PI);
    }
  }
  if (meta.shake) {
    x += meta.shake.amplitude_px * Math.sin(TAU2 * meta.shake.hz * now / 1000);
  }
  if (meta.swing_deg) {
    rot = (meta.swing_deg * Math.PI / 180) * Math.sin(TAU2 * 1.6 * now / 1000);
  }
  if (meta.y_offset_curve === "parabola_up_40px") {
    y -= 40 * 4 * progress * (1 - progress);
  }
  if (meta.breathe_pct) {
    const s = 1 + (meta.breathe_pct / 100) * Math.sin(TAU2 * 0.35 * now / 1000);
    sx = s; sy = s;
  }
  return { x, y, sx, sy, rot };
}

function drawZzz(now) {
  const img = imageCache.get(fileUrl("道具/睡觉_Zzz.png"));
  if (!img || !img.complete) return;
  const cycle = (now % 3000) / 3000;
  const zx = BOX_W / 2 + 70, zy = BOX_H / 2 - 40 - cycle * 40;
  ctx.globalAlpha = 1 - cycle * 0.8;
  const s = 0.8 + cycle * 0.4;
  ctx.drawImage(img, zx, zy, img.width * s * 0.5, img.height * s * 0.5);
  ctx.globalAlpha = 1;
}

// ---------- 状态 → 动作/行为（实现文档 §6）----------

const persistent = {
  idle:       { action: "idle" },
  streaming:  { action: "writing" }, // 纯文本输出：伏案写回复
  thinking:   { action: "think" },
  working:    { action: "think" },   // 跑命令/杂务：沉思（走路只留给桌面闲逛）
  writing:    { action: "writing" }, // 正在改文件：伏案写入
  searching:  { action: "reading" }, // 搜索/看网页：翻书
  reviewing:  { action: "review" },  // plan 模式/读文件：审视
  waiting:    { action: "await" },
  failed:     { action: "sad" },     // 失败用失落回应；具体原因由气泡说明
  angry:      { action: "angry" },   // 被打断：叉腰生气
  sleeping:   { action: "sleep" },
  celebrating:{ action: "jump" },
  greeting:   { action: "wave" },
};

// ---------- 主循环 ----------

const player = new Player();
// 未收到后台状态前保持中性待机，不能把“尚未初始化”当作 CLI 离线。
let petState = { state: "idle", tps: 0 };
let pendingState = null, pendingSince = 0; // 持续态防抖：新状态需稳定 1s 才切换
const STATE_DEBOUNCE_MS = 1000;
let walkDir = 1;
let lastTs = performance.now();
let overrideUntil = 0;       // poked/trip 等本地单发动作的占用截止时间
let overrideAction = null;
function startOverride(action, now, duration) {
  overrideAction = action;
  overrideUntil = now + duration;
  player.play(action, now, true);
}
function clearOverride() {
  overrideAction = null;
  overrideUntil = 0;
}
let pokeCount = 0, pokeWindowStart = 0; // 3 秒窗口内的连戳计数
let pressWasSleeping = false; // 按下瞬间她是否在睡（pointerdown 会重置 idleSince，得提前记）
let pressWasReading = false;
let held = false;
let idleSince = performance.now(); // 进入 idle 的起始时刻（交互或状态切换会重置）
let falling = null; // { vy, targetY } 重力摔落
let fallCount = 0, lastFallAt = 0; // 连续摔落计数（1 分钟窗口）
let gazeDir = "下"; // 视线方向（Rust pet-gaze 事件更新）
let gazeActive = false; // 鼠标静止 2s 后 Rust 发空串复位，注视帧让位给眨眼
let gazeEnabled = true; // 设置页"视线跟随鼠标"开关
let lastDraw = 0, lastFrameKey = "";
let lastLoggedAction = "";

// 空闲节奏：站立眨眼 → 浅睡（趴睡无 Zzz）→ 熟睡打呼（阈值可在设置页调）
const timeRhythm = new TimeRhythm();

// 统一行为仲裁器拥有全部自主活动与冷却；主文件只提供当前环境快照。
const director = new BehaviorDirector(performance.now(), logDebug, (command, args) => TAU.core.invoke(command, args));
let dailySpeechOn = true, menuOpen = false, cursorPlayOn = true;
let cursorCarryOn = false;
let extensionRevision = -1, extensionInteraction = false;
let inputEditing=false, inputPresenceRevision=-1, wasChatEngaged=false;
function acceptExtensions(snapshot) {
  if (!snapshot || !Number.isFinite(snapshot.revision) || snapshot.revision < extensionRevision) return;
  extensionRevision = snapshot.revision;
  extensionInteraction = snapshot.cursor_blocked === true;
  if (snapshot.windows) director.music.visible = snapshot.windows.some(item => item.id === "music" && item.visible);
}
let nextCursorDiagnosticAt = 0;
let nextLifeReportAt = 0, lifeReportPending = false, lastLifeMode = "";
let nextChatReportAt = 0, chatReportPending = false;
let nextDailySpeechAt = performance.now() + 15000;
let lastDailyLine = "";

function announceDailyReading(now) {
  if (!dailySpeechOn || menuOpen || chatEngaged(now) || now < nextDailySpeechAt || now < bubbleLockUntil) return;
  const options = ["趁现在，接着看几页。", "这段故事……还挺有意思的。", "我在旁边看会儿书，陪着你。"].filter(text => text !== lastDailyLine);
  lastDailyLine = options[Math.floor(Math.random() * options.length)];
  nextDailySpeechAt = now + 90000 + Math.random() * 60000;
  showBubble(`daily-${++bubbleRevision}`, lastDailyLine, 4200, true, null, "daily");
}

function currentBehavior(now) {
  const conversation=chatEngaged(now);
  if(conversation!==wasChatEngaged){wasChatEngaged=conversation;idleSince=now;logDebug(`conversation active=${conversation} editing=${inputEditing} generating=${director.chat.active}`);}
  if (now >= overrideUntil) clearOverride();
  const page = ACTIONS.daily_page, yawn = ACTIONS.yawn;
  const leftGrip = ACTIONS.walk_left.cursor_grip || [85, 192];
  const rightGrip = ACTIONS.walk_right.cursor_grip || [235, 192];
  const holdLeft = ACTIONS.carry_hold_left.cursor_grip;
  const holdRight = ACTIONS.carry_hold_right.cursor_grip;
  const work = persistent[petState.state] || persistent.idle;
  const rhythm = timeRhythm.snapshot(now);
  const b = director.step(now, {
    held, heldAction: flapping ? heldMoveAction : "held_idle", falling: !!falling,
    override: now < overrideUntil ? { action: overrideAction } : null,
    menu: menuOpen, scaling: scaleBusy, pressing: !!pressT,
    busy: !["idle", "sleeping"].includes(petState.state), work: { ...work },
    idleMs: now - idleSince, lightSleepMs: rhythm.lightSleepMs, deepSleepMs: rhythm.deepSleepMs,
    night: rhythm.night, dailySpeech: dailySpeechOn&&!conversation, extensionInteraction, conversation,
    protectedBubble: bubbleLockUntil === Infinity || (now < bubbleLockUntil && !["daily", "chat"].includes(bubbleKind)),
    lifeBubble: bubbleKind === "life" && bubbleLockUntil !== Infinity,
    wander: wanderOn, cursorEnabled: cursorPlayOn, carryEnabled: cursorCarryOn,
    pageMs: page.frames.length * page.frame_ms,
    yawnMs: yawn.frames.length * yawn.frame_ms + (yawn.hold_ms || 0),
    geometry: { x: winX, y: winY, width: winW, height: winH, sf: SF, scale: S,
      maxX: screen.availWidth - winW - 10,
      carryLeft: [leftGrip[0] * S, (FEET_IN_BOX - 320 + leftGrip[1]) * S],
      carryRight: [rightGrip[0] * S, (FEET_IN_BOX - 320 + rightGrip[1]) * S],
      ropeGripX: (ACTIONS.rope_hold?.cursor_grip?.[0] || 144) * S,
      catchPoint: [(ACTIONS.rope_hold?.cursor_grip?.[0] || 144) * S,
        (FEET_IN_BOX - 320 + (ACTIONS.rope_hold?.cursor_grip?.[1] || 136)) * S],
      holdLeft: [holdLeft[0] * S, (FEET_IN_BOX - 320 + holdLeft[1]) * S],
      holdRight: [holdRight[0] * S, (FEET_IN_BOX - 320 + holdRight[1]) * S] },
  });
  director.music.report(now, b.musicActive === true, director.layer);
  if (now >= nextChatReportAt && !chatReportPending) {
    nextChatReportAt = now + 2000; chatReportPending = true;
    const activity = held ? "held" : falling ? "falling" : director.layer === "cli" ? "cli" :
      ["sleep", "sleep_light"].includes(b.action) ? b.action : director.layer === "touch-feedback" ? "touch" :
      ["idle", "read", "walk", "job", "music", "care", "attention", "chat"].includes(director.active?.id) ? director.active.id : "idle";
    TAU.core.invoke("chat_pet_context", { activity }).catch(() => {}).finally(() => { chatReportPending = false; });
  }
  const lifeMode = b.lifeMode || (["sleep", "sleep_light"].includes(b.action) ? "sleep" :
    director.layer === "autonomous" ? "awake" : "paused");
  if ((now >= nextLifeReportAt || lifeMode !== lastLifeMode) && !lifeReportPending) {
    nextLifeReportAt = now + 1000;
    lastLifeMode = lifeMode;
    lifeReportPending = true;
    TAU.core.invoke("life_mode", { mode: lifeMode, jobId: director.life.snapshot?.job?.id || null })
      .catch(error => logDebug(`life-mode-error ${error}`))
      .finally(() => { lifeReportPending = false; });
  }
  if (b.lifeSay && (!b.lifeReminder || (dailySpeechOn&&!conversation))) {
    showBubble(`life-${++bubbleRevision}`, b.lifeSay, 4200, true, null, "life");
  }
  if (b.startDirection) walkDir = b.startDirection;
  // 只借用安全的站立/静读时机说话，不额外占用活动槽或抢光标互动对白。
  if (dailySpeechOn && !conversation && director.layer === "autonomous" &&
      (!director.active || ["idle", "read"].includes(director.active.id)) &&
      now - idleSince >= 8000 && now >= nextDailySpeechAt && now >= bubbleLockUntil) {
    const greeting = timeRhythm.takeGreeting(rhythm);
    if (greeting) {
      nextDailySpeechAt = now + 90000 + Math.random() * 60000;
      showBubble(`time-${++bubbleRevision}`, greeting, 5200, true, null, "daily");
    }
  }
  if (b.startedRead) announceDailyReading(now);
  if (b.say && dailySpeechOn && !conversation) {
    const point = director.cursor.sample;
    showBubble(`cursor-${++bubbleRevision}`, b.say, 4200, true, null, "daily",
      point ? [point.x, point.y] : null);
  }
  if ((cursorPlayOn || cursorCarryOn) && now >= nextCursorDiagnosticAt) {
    nextCursorDiagnosticAt = now + 15000;
    logDebug(`director-check layer=${director.layer} activity=${director.reason} attention=${director.attention.reason} cursor=${director.cursor.reason} carry=${cursorCarryOn ? director.carry.reason : "disabled"} rope=${cursorCarryOn ? director.rope.reason : "disabled"}`);
  }
  return b;
}

function tick(now) {
  const dt = Math.min(0.1, (now - lastTs) / 1000);
  lastTs = now;
  if (pendingSettings && !held && !falling && !scaleBusy) {
    const settings = pendingSettings;
    pendingSettings = null;
    applySettings(settings);
  }

  // 释放看门狗：松手后 Rust 的 pet-released 迟迟不到（丢失/竞态），强制结算
  if (held && awaitingReleaseT && !releaseChecking && now - awaitingReleaseT > 600) {
    logDebug(`release-watchdog seq=${myDragSeq}`);
    releaseChecking = true;
    const generation = dragGeneration;
    appWin.outerPosition().then((pos) => {
      if (held && generation === dragGeneration) {
        finishRelease(pos.x, pos.y, pos.y - Math.round(grabWinY * SF));
      }
    }).catch((e) => {
      logDebug(`release-position-err ${e}`);
      if (held && generation === dragGeneration) {
        finishRelease(winX * SF, winY * SF, (winY - grabWinY) * SF);
      }
    }).finally(() => {
      if (generation === dragGeneration) releaseChecking = false;
    });
  }

  const b = currentBehavior(now);
  let action = b.action;

  // 行走：移 OS 窗口，在当前基线上水平走，到屏幕边缘折返
  if (b.walk && !held && !falling) {
    const speed = b.speed * (petState.state === "streaming" ? (1 + Math.min(petState.tps || 0, 60) / 30) : 1);
    if (Number.isFinite(b.cursorTarget)) {
      walkDir = b.cursorTarget >= winX ? 1 : -1;
      winX += walkDir * Math.min(Math.abs(b.cursorTarget - winX), speed * dt);
    } else winX += walkDir * speed * dt;
    winY = baseY;
    const maxX = screen.availWidth - winW - 10;
    if (winX > maxX) { winX = maxX; walkDir = -1; }
    if (winX < 10) { winX = 10; walkDir = 1; }
    action = b.walkAction || (walkDir > 0 ? "walk_right" : "walk_left");
  }

  // 重力摔落：窗口 y 加速下坠到目标基线；落地动作按 1 分钟内的连续摔落次数升级：
  // 第 1 次摔坐 → 第 2 次摔懵 → 第 3 次起泪眼；超过 1 分钟没摔则重新计数
  if (falling) {
    falling.vy += 3000 * dt;
    winY += falling.vy * dt;
    if (winY >= falling.targetY) {
      winY = falling.targetY;
      baseY = winY;
      falling = null;
      if (now - lastFallAt > 60000) fallCount = 0;
      fallCount++;
      lastFallAt = now;
      const landing = fallCount === 1 ? "land" : fallCount === 2 ? "trip1" : "trip2";
      const a = ACTIONS[landing];
      startOverride(landing, now, a.frames.length * a.frame_ms + (a.hold_ms || 0));
      showTouchDialogue({ now, event: landing });
      action = player.actionName; // 防止本 tick 末尾的 play(action) 把落地动作冲掉
    }
  }

  if (!held && !b.nativeCarry) commitPos(now); // 拎起/抱跑期间位置由Rust独占

  // 睡眠触发取证：谁让她睡的就记谁的账
  if ((action === "sleep" || action === "sleep_light") && player.actionName !== action) {
    const idleFor = ((now - idleSince) / 1000).toFixed(0);
    TAU.core.invoke("log_sleep_trigger", {
      reason: `action=${action} engineState=${petState.state} idleFor=${idleFor}s held=${held} override=${now < overrideUntil} evtCount=${evtCount} pending=${pendingState} prevAction=${player.actionName}`,
    }).catch(() => {});
  }
  player.play(action, now);
  if (action !== lastLoggedAction) {
    lastLoggedAction = action;
    logDebug(`action=${action} held=${held} falling=${!!falling} flapping=${flapping}`);
  }

  const f = player.frame(now);

  // 单发动作的重播/收尾（必须在脏检查之前，否则待机眨眼永远等不到重播）
  if (f && f.done) {
    const gap = f.meta.replay_gap_ms;
    if (gap) {
      if (!player.replayAt) {
        player.replayAt = now + gap[0] + Math.random() * (gap[1] - gap[0]);
      }
      if (now >= player.replayAt) {
        player.replayAt = 0;
        player.play(player.actionName, now, true);
      }
    }
  }

  // 省电：静止动作只在切帧时重绘；动效场景锁 30fps
  if (f) {
    const fx = held || falling || b.walk || b.nativeCarry || f.meta.squash || f.meta.shake || f.meta.swing_deg ||
               f.meta.y_offset_curve || f.meta.breathe_pct || f.meta.zzz;
    // 待机眨眼播完后的静止期：换成注视帧（视线跟光标），并计入重绘键
    let img = f.img;
    let gazeKey = "";
    if (gazeEnabled && gazeActive && player.actionName === "idle" && f.done) {
      const g = imageCache.get(frameUrl("注视八向", gazeDir));
      if (g && g.complete && g.naturalWidth) { img = g; gazeKey = ":" + gazeDir; }
    }
    // 缺帧兜底：帧文件缺失时退回待机睁眼，绝不隐形
    if (!img || !img.complete || !img.naturalWidth) {
      img = imageCache.get(frameUrl("待机眨眼", "01_睁眼"));
    }
    const frameKey = JSON.stringify([player.actionName, f.idx, gazeKey]);
    const dirty = fx ? (now - lastDraw >= 33) : (frameKey !== lastFrameKey);
    if (dirty && img && img.complete && img.naturalWidth) {
      lastDraw = now;
      lastFrameKey = frameKey;

      ctx.clearRect(0, 0, BOX_W, BOX_H);
      const bodyCenter = f.meta.anchor === "body_center";
      const cx = BOX_W / 2;
      const bob = b.walk && !b.nativeCarry ? 1.5 * Math.sin(TAU2 * 1.5 * now / 1000) : 0; // 抱跑帧自带起伏，不再额外浮动抓点
      const baseYInBox = (bodyCenter ? BOX_H / 2 + 30 : FEET_IN_BOX) + bob;
      const { x, y, sx, sy, rot } = applyEffects(f.meta, f.progress, now, cx, baseYInBox);

      ctx.save();
      const grip = held && bodyCenter ? f.meta.grab_point : null;
      ctx.translate(grip ? x + grip[0] - img.width / 2 : x,
                    grip ? y + grip[1] - img.height / 2 : y);
      ctx.rotate(rot);
      ctx.scale(sx, sy);
      const nextSprite = { img, action: player.actionName, transform: ctx.getTransform(),
        animationElapsed: now - player.startedAt, frameMs: f.meta.frame_ms,
        pullCurve: f.meta.cursor_pull_y, pullCycles: f.meta.cursor_pull_cycles,
        ox: grip ? -grip[0] : -img.width / 2,
        oy: grip ? -grip[1] : bodyCenter ? -img.height / 2 : -img.height,
        carryGrip: f.meta.cursor_grips?.[f.idx] || f.meta.cursor_grip };
      if (grip) ctx.drawImage(img, -grip[0], -grip[1]);
      else if (bodyCenter) ctx.drawImage(img, -img.width / 2, -img.height / 2);
      else ctx.drawImage(img, -img.width / 2, -img.height);
      ctx.restore();

      if (f.meta.zzz) drawZzz(now);
      petShape.offer(nextSprite);
    }
  }

  updateBubbleAnchor(now);
  if (b.nativeCarry && drawnSprite?.carryGrip) {
    const sprite = drawnSprite;
    const hand = new DOMPoint(sprite.carryGrip[0] + sprite.ox, sprite.carryGrip[1] + sprite.oy)
      .matrixTransform(sprite.transform);
    const scene = director.attention.nativeScene;
    scene?.sync(now, { logicalX: winX, action: sprite.action,
      animationElapsed: sprite.animationElapsed, frameMs: sprite.frameMs,
      pullCurve: sprite.pullCurve, pullCycles: sprite.pullCycles,
      window_pos: [Math.round(winX * SF), Math.round(winY * SF)],
      cursor_pos: [Math.round((winX + hand.x * S) * SF), Math.round((winY + hand.y * S) * SF)] });
  }
  requestAnimationFrame(tick);
}

// ---------- 气泡 ----------

let lastBubbleId = 0, bubbleTimer = 0, bubbleLockUntil = 0, bubbleOwner="";
// CLI通知只消费一次；普通卡片隐藏/触摸/聊天不能重置这个游标。
let lastCliBubbleId = 0, cliBubbleOwner = "";
let bubbleKind = "status";
let bubbleRevision = Date.now() * 1000, bubbleUpdates = Promise.resolve();
let speechRevision=-1,speechGeneration=-1,speechHovered=false,speechRemaining=0,speechStreaming=false,speechHiddenRequest=null;
let pendingSpeech=null,speechDeadline=0,deferredUsage=null;
function chatEngaged(now){return inputEditing||director.chat.ready(now)||!!(pendingSpeech&&pendingSpeech.request_id!==speechHiddenRequest&&now<speechDeadline);}
function clearCurrentBubble(){
  deferredUsage=null;clearTimeout(bubbleTimer);bubbleLockUntil=0;lastBubbleId=0;bubbleOwner="";bubbleKind="status";speechHovered=false;updateBubble(null);
}
function hideSpeech(){pendingSpeech=null;speechDeadline=0;if(bubbleKind==="chat")clearCurrentBubble();}
function armSpeechTimer(ms){
  clearTimeout(bubbleTimer);speechRemaining=ms;
  speechDeadline=speechHovered?Number.MAX_SAFE_INTEGER:performance.now()+ms;
  bubbleLockUntil=speechDeadline;
  const owner=bubbleOwner;
  if(!speechHovered)bubbleTimer=setTimeout(()=>{if(bubbleKind==="chat"&&bubbleOwner===owner)hideSpeech();},ms);
}
function renderPendingSpeech(){
  const now=performance.now(),message=pendingSpeech;
  if(!message||message.request_id===speechHiddenRequest||now>=speechDeadline)return;
  if(menuOpen||bubbleLockUntil===Infinity||(bubbleKind!=="chat"&&now<bubbleLockUntil))return;
  const owner=`speech-${message.request_id}`;
  bubbleKind="chat";lastBubbleId=owner;bubbleOwner=owner;speechStreaming=message.streaming===true;
  updateBubble({revision:++bubbleRevision,owner,kind:"chat",text:message.text,request_id:message.request_id,streaming:speechStreaming});
  armSpeechTimer(speechStreaming?125000:Math.max(1000,speechDeadline-now));
}
function showChatSpeech(message){
  if(message.revision<speechRevision||message.generation<speechGeneration)return;
  speechRevision=message.revision;speechGeneration=message.generation;
  if(!message.text||message.request_id===speechHiddenRequest)return;
  const same=pendingSpeech?.request_id===message.request_id;
  if(!same){speechHovered=false;logDebug(`speech owner=${message.request_id} stage=received streaming=${message.streaming===true}`);}
  const wasStreaming=pendingSpeech?.streaming;
  pendingSpeech=message;
  if(message.streaming||!same||wasStreaming)speechDeadline=performance.now()+(message.streaming?125000:Math.max(10000,Math.min(60000,[...message.text].length*180)));
  renderPendingSpeech();
}
function updateBubble(content) {
  bubbleUpdates = bubbleUpdates.then(() => TAU.core.invoke("set_bubble", { content }))
    .catch((e) => logDebug(`bubble_err ${e}`));
}
// 所有原生台词共用此入口：自主台词让位交流，显式反馈临时占用后恢复尚未过期的回复。
function showBubble(id,text,ms=4000,lock=false,usage=null,kind="status",avoidPoint=null){
  if(!text||(kind==="daily"&&chatEngaged(performance.now())))return;
  if(bubbleLockUntil===Infinity)return;
  if(!lock&&performance.now()<bubbleLockUntil)return;
  if(id===lastBubbleId)return;
  if(bubbleKind==="chat"&&speechHovered)speechDeadline=performance.now()+speechRemaining;
  clearTimeout(bubbleTimer);speechHovered=false;lastBubbleId=id;bubbleKind=kind;
  bubbleLockUntil=lock?performance.now()+ms:0;
  const owner=`local-${++bubbleRevision}`;bubbleOwner=owner;
  updateBubble({revision:bubbleRevision,owner,text,usage,kind,avoid_point:avoidPoint});
  bubbleTimer=setTimeout(()=>{
    if(bubbleOwner!==owner||bubbleKind!==kind)return;
    clearCurrentBubble();renderPendingSpeech();
  },ms);
}
function acceptCliBubble(bubble){
  if(!bubble){
    if(cliBubbleOwner&&bubbleOwner===cliBubbleOwner&&bubbleKind==="status"&&performance.now()>=bubbleLockUntil){
      clearCurrentBubble();renderPendingSpeech();
    }
    return;
  }
  if(!Number.isSafeInteger(bubble.id)||bubble.id<=lastCliBubbleId)return;
  lastCliBubbleId=bubble.id;
  showBubble(bubble.id,bubble.text);
  if(bubbleKind==="status"&&lastBubbleId===bubble.id)cliBubbleOwner=bubbleOwner;
}

const touchDialogue = new TouchDialogue();
function showTouchDialogue(context) {
  // 手动额度/拖动卡片的锁不能被触摸顶掉；本地对话可以更新本地对话。
  if (bubbleLockUntil === Infinity || (performance.now() < bubbleLockUntil && !["touch", "daily", "life", "chat"].includes(bubbleKind))) return;
  const text = touchDialogue.respond(context);
  if (!text) return;
  showBubble(`touch-${++bubbleRevision}`, text, 4200, true, null, "touch");
}
function dismissTouchDialogue() {
  if (!["touch", "daily", "life"].includes(bubbleKind) || bubbleLockUntil === Infinity) return;
  clearCurrentBubble();renderPendingSpeech();
}

// ---------- 额度查询（右键菜单"额度" → show-usage 事件）----------

let usageBusy = false;
function showUsageResult(...args){
  if(bubbleLockUntil===Infinity){deferredUsage={owner:bubbleOwner,apply:()=>showBubble(...args)};return;}
  showBubble(...args);
}
async function showUsage() {
  if (usageBusy || bubbleLockUntil === Infinity) return;
  usageBusy = true;
  showBubble("usage-loading", "查询额度中…", 10000, true);
  try {
    const u = await TAU.core.invoke("query_usage");
    const pct = (r) => r != null && Number.isFinite(Number(r)) ? Math.round(Number(r) * 100) : null;
    const hm = (iso) => {
      if (!iso) return "未知时间";
      const d = new Date(iso);
      return isNaN(d) ? "?" : `${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
    };
    const md = (iso) => {
      if (!iso) return "未知时间";
      const d = new Date(iso);
      return isNaN(d) ? "?" : `${d.getMonth() + 1}/${d.getDate()}`;
    };
    const l5 = u.limit_5h || {}, l7 = u.limit_7d || {};
    // 注意：接口里的 usage 字段只是 7 天窗口的另一种口径，不是月度/赠送额度
    // （月度/赠送要走 kimi.com 网页 token + GetSubscriptionStats，暂未接入）
    showUsageResult("usage",
      `5h窗口 已用${pct(l5.used_ratio)}%（${hm(l5.reset_time)}重置）\n` +
      `7天窗口 已用${pct(l7.used_ratio)}%（${md(l7.reset_time)}重置）`,
      8000, true, [
        { label: "5 小时窗口", percent: pct(l5.used_ratio), reset: hm(l5.reset_time) },
        { label: "7 天窗口", percent: pct(l7.used_ratio), reset: md(l7.reset_time) },
      ]);
  } catch (e) {
    showUsageResult("usage-err", `额度查询失败：${e}`, 6000, true);
  }
  usageBusy = false;
}

// ---------- 本地交互：戳 / 拎 / 摔 ----------
// 窗口即角色，普通命中检测，无穿透 hack。
// - 按下 220ms 或移动 >8px → 拎起：begin_drag，Rust 线程接管窗口移动
// - 拎起后动过 → 一直扑腾到松手；没动过 → 悬空帧
// - 松手（Rust pet-released，按 seq 认领，陈旧事件丢弃；600ms 看门狗兜底）：
//   拎起高度 ≤ 一个身高 → 重力摔回原基线 + 平地摔；超过 → 搬家，就地定新基线
// - 短击（没拎起、<300ms）：分区轻触 + 本地对白，连戳仍按原阶梯抗议

let pressT = 0, pressX = 0, pressY = 0;
let pressRegion = "body";
let flapping = false;
let heldMoveAction = "held_move"; // 每轮选一次单帧姿势，保持到松手
let liftTimer = 0;
let liftDialogueTimer = 0;
let grabWinX = 0, grabWinY = 0;   // 拎起那一刻的窗口位置（基线）
let myDragSeq = 0;                // 本轮拎起认领的拖拽线程序号
let awaitingReleaseT = 0;         // 已松手、等待 pet-released 的起始时刻（0=无）
let dragGeneration = 0, dragStarting = false, releaseChecking = false;
const earlyReleases = new Map();

function finishRelease(x, y, dy) {
  clearTimeout(liftTimer);
  pressT = 0; // 原生松手可能先于或替代网页pointerup到达
  clearTimeout(liftDialogueTimer);
  dismissTouchDialogue();
  held = false;
  flapping = false;
  awaitingReleaseT = 0;
  dragStarting = false;
  releaseChecking = false;
  earlyReleases.clear();
  idleSince = performance.now();
  settleRelease(x, y, dy);
}

async function startLift() {
  cancelPendingTouch();
  if (held) return;
  if (scaleBusy) {
    clearTimeout(liftTimer);
    liftTimer = setTimeout(() => { if (pressT) startLift(); }, 40);
    return;
  }
  clearTimeout(liftTimer);
  clearTimeout(liftDialogueTimer);
  dismissTouchDialogue();
  director.interrupt(performance.now());
  clearOverride();
  held = true;
  const generation = ++dragGeneration;
  heldMoveAction = Math.random() < 0.5 ? "held_move" : "held_move_alt";
  dragStarting = true;
  releaseChecking = false;
  myDragSeq = 0;
  earlyReleases.clear();
  flapping = false;
  falling = null;
  awaitingReleaseT = 0;
  grabWinX = winX;
  grabWinY = winY;
  // 同步一次真实窗口位置（上轮拖拽若异常收场，JS 坐标可能是陈旧的）
  try {
    const pos = await appWin.outerPosition();
    if (!held || generation !== dragGeneration) return;
    winX = pos.x / SF;
    winY = pos.y / SF;
  } catch {}
  if (!held || generation !== dragGeneration) return;
  grabWinX = winX;
  grabWinY = winY;
  try {
    const meta = ACTIONS.held_idle;
    const grip = meta.grab_point;
    const anchor = grip ? [Math.round(grip[0] * S * SF),
      Math.round((BOX_H / 2 + 30 - BOX_W / 2 + grip[1]) * S * SF)] : null;
    const seq = await TAU.core.invoke("begin_drag", { anchor });
    if (!held || generation !== dragGeneration) return;
    myDragSeq = seq;
    dragStarting = false;
    logDebug(`lift seq=${myDragSeq} at ${winX | 0},${winY | 0}`);
    const release = earlyReleases.get(seq);
    earlyReleases.clear();
    if (release) finishRelease(release.x, release.y, release.dy);
    else {
      // 拎着静止600ms才回应；异步回调只能服务本轮拖拽。
      liftDialogueTimer = setTimeout(() => {
        if (held && generation === dragGeneration && !flapping && !awaitingReleaseT) {
          showTouchDialogue({ now: performance.now(), event: "lift" });
        }
      }, 600);
    }
  } catch (e) {
    logDebug("begin_drag_err " + e);
    if (held && generation === dragGeneration) {
      finishRelease(winX * SF, winY * SF, 0);
    }
  }
}

// 松手结算（pet-released 事件与看门狗共用）：摔回基线 or 就地安置
function settleRelease(xPhys, yPhys, dyPhys) {
  if (typeof xPhys === "number") winX = xPhys / SF;
  if (typeof yPhys === "number") winY = yPhys / SF;
  const dyCss = (dyPhys || 0) / SF;
  const maxX = screen.availWidth - winW;
  if (winX < 0) winX = 0;
  if (winX > maxX) winX = maxX;
  logDebug(`settle win=${winX | 0},${winY | 0} dy=${dyCss | 0} grabY=${grabWinY | 0} floorY=${floorY | 0}`);
  if (dyCss < -40 && -dyCss <= modelH) {
    // 拎起一个身高内松手 → 摔回脚下基线
    falling = { vy: 0, targetY: grabWinY };
  } else {
    // 搬家：就地安置，脚下定新基线；靠近地面（60px 内）或压出屏幕则吸附地面
    if (winY > floorY || winY < 0 || floorY - winY < 60) winY = floorY;
    baseY = winY;
    commitPos(performance.now(), true);
    showTouchDialogue({ now: performance.now(), event: "place" });
  }
}

// 首次轻触分区；连续触摸用受戳/抗议回应，不再举牌显示无关符号。
function onPoke(now,{region=pressRegion,sleeping=pressWasSleeping,reading=pressWasReading}={}) {
  const busy = !["idle", "sleeping"].includes(petState.state);
  const recentFall = fallCount > 0 && now - lastFallAt < 15000;
  director.pauseForTouch(now, 1600);
  logDebug(`touch region=${region} sleeping=${sleeping} busy=${busy} recentFall=${recentFall}`);
  if (sleeping) {
    startOverride("poked", now, 700);
    pokeCount = 0;
    showTouchDialogue({ now, region: region, sleeping: true, busy, recentFall, reading: reading, count: 1 });
    return;
  }
  if (now - pokeWindowStart > 3000) pokeCount = 0;
  if (pokeCount === 0) pokeWindowStart = now;
  pokeCount++;
  showTouchDialogue({ now, region: region, sleeping: false, busy, recentFall, reading: reading, count: pokeCount });
  if (pokeCount === 1) {
    // 刚摔倒时先保留落地表情，让安慰台词和画面衔接。
    if (recentFall && now < overrideUntil && ["land", "trip1", "trip2"].includes(overrideAction)) return;
    const action = region === "head" ? "await" : region === "face" ? "poked" : "wave";
    startOverride(action, now, region === "face" ? 700 : 1300);
  } else if (pokeCount <= 3) {
    startOverride("poked", now, 700);
  } else {
    startOverride("angry", now, 1600);
  }
}

let doubleClick={ms:500,x:4,y:4},pendingTouch=null,touchTimer=0;
function cancelPendingTouch(){clearTimeout(touchTimer);pendingTouch=null;}
function finishPendingTouch(){const touch=pendingTouch;cancelPendingTouch();if(touch)onPoke(performance.now(),touch.context);}
function shortPress(event){
  const now=performance.now();
  if(pendingTouch&&now-pendingTouch.at<=doubleClick.ms&&Math.abs(event.screenX-pendingTouch.x)<=doubleClick.x&&Math.abs(event.screenY-pendingTouch.y)<=doubleClick.y){
    cancelPendingTouch();clearOverride();idleSince=now;logDebug("input-ui action=pet-double-click stage=click");
    TAU.core.invoke("input_toggle").catch(()=>logDebug("input-ui action=pet-double-click stage=failed"));return;
  }
  finishPendingTouch();
  pendingTouch={at:now,x:event.screenX,y:event.screenY,context:{region:pressRegion,sleeping:pressWasSleeping,reading:pressWasReading}};
  touchTimer=setTimeout(finishPendingTouch,doubleClick.ms+20);
}

document.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || pressT || held || falling || scaleBusy) return;
  const region = touchRegion(e);
  if (!region) return;
  pressRegion = region;
  try { canvas.setPointerCapture(e.pointerId); } catch (_) { /* 原生释放仍可兜底 */ }
  pressT = performance.now();
  pressX = e.clientX; pressY = e.clientY;
  pressWasSleeping = player.actionName === "sleep" || player.actionName === "sleep_light";
  pressWasReading = director.isReading(pressT);
  idleSince = performance.now();
  liftTimer = setTimeout(startLift, 220);
});

document.addEventListener("pointermove", (e) => {
  if (held || !pressT) return;
  if (Math.hypot(e.clientX - pressX, e.clientY - pressY) > 8) startLift();
});

document.addEventListener("pointerup", (e) => {
  if (e.button !== 0 || !pressT) return;
  clearTimeout(liftTimer);
  const dt = performance.now() - pressT;
  const wasHeld = held;
  pressT = 0;
  if (wasHeld) {
    awaitingReleaseT = performance.now(); // 等 Rust 的 pet-released，超时看门狗兜底
  } else if (dt > 0 && dt < 300) {
    shortPress(e);
  }
});

document.addEventListener("pointercancel", () => {
  cancelPendingTouch();
  clearTimeout(liftTimer);
  pressT = 0;
  // pointercancel不代表真实左键已松开，拎起期间由原生线程确认释放。
});
canvas.addEventListener("lostpointercapture", () => {
  clearTimeout(liftTimer);
  if (!held) pressT = 0;
});

// 右键 → 原生菜单（设置/退出）
document.addEventListener("contextmenu", (e) => {
  if (!touchRegion(e)) return;
  cancelPendingTouch();
  e.preventDefault();
  dismissTouchDialogue();
  TAU.core.invoke("show_pet_menu").catch((e) => logDebug(`menu_err ${e}`));
});

// ---------- 设置应用 ----------

let pendingSettings = null, scaleBusy = false;
function applySettings(s) {
  if (!s) return;
  if (held || falling || scaleBusy) {
    pendingSettings = s;
    return;
  }
  const newScale = Number(s.scale) || 1;
  wanderOn = s.wander !== false;
  gazeEnabled = s.gaze !== false;
  dailySpeechOn = s.daily_speech !== false;
  cursorPlayOn = s.cursor_play !== false;
  cursorCarryOn = s.cursor_carry === true;
  if (!cursorCarryOn && director.attention.controlsActive) director.interrupt(performance.now(), "carry-disabled");
  if (!cursorPlayOn && !cursorCarryOn && director.active?.id === "attention") director.interrupt(performance.now(), "disabled");
  if (!dailySpeechOn && bubbleKind === "daily") dismissTouchDialogue();
  lastFrameKey = ""; // 视线开关立即重绘
  timeRhythm.configure(s);
  if (Math.abs(newScale - S) > 0.001) {
    scaleBusy = true;
    appWin.setSize(new TAU.window.LogicalSize(320 * newScale, 360 * newScale))
      .then(() => {
        const oldFeet = feetOff, oldWidth = winW;
        S = newScale;
        winW = 320 * S; winH = 360 * S;
        feetOff = 336 * S; modelH = 280 * S;
        floorY = screen.availHeight - feetOff;
        winX = Math.max(0, Math.min(screen.availWidth - winW, winX + (oldWidth - winW) / 2));
        winY = Math.max(0, Math.min(floorY, winY + oldFeet - feetOff));
        baseY = winY;
        commitPos(performance.now(), true);
        logDebug(`scale=${S}`);
      })
      .catch((e) => logDebug(`scale_err ${e}`))
      .finally(() => { scaleBusy = false; });
  }
}

// ---------- 状态事件订阅 ----------

let evtCount = 0;

async function subscribe() {
  await TAU.event.listen("cursor-carry-ended", (event) => {
    if (event.payload && Number.isFinite(event.payload.id)) {
      const now = performance.now();
      director.attention.onEnded(event.payload, now);
    }
  });
  await TAU.event.listen("pet-cursor", (event) => {
    const p = event.payload;
    if (p && [p.x, p.y, p.still_ms].every(Number.isFinite)) {
      acceptExtensions({ revision: p.extension_revision, cursor_blocked: p.extension_blocked });
      director.observeCursor(p, performance.now());
    }
  });
  await TAU.event.listen("pet-state", (ev) => {
    evtCount++;
    const p = ev.payload || {};
    const now = performance.now();
    acceptCliBubble(p.bubble);
    // CLI 离线只表示无工作信号；角色仍按本地待机阈值入睡。
    const idleLike = (state) => state === "idle" || state === "sleeping";
    if (!idleLike(p.state)) idleSince = now;
    // 事件态（庆祝/打招呼/失败）即时响应；持续态需稳定 1s 防抖
    const isEventState = ["celebrating", "greeting", "failed", "angry"].includes(p.state);
    if (p.state !== petState.state && !isEventState) {
      if (p.state === pendingState) {
        if (now - pendingSince < STATE_DEBOUNCE_MS) {
          return;
        }
      } else {
        pendingState = p.state;
        pendingSince = now;
        return;
      }
    }
    pendingState = null;
    if (p.state !== petState.state) {
      logDebug(`state=${petState.state}->${p.state} evtCount=${evtCount} source=${JSON.stringify(p.diagnostics || {})}`);
    }
    if (idleLike(p.state) && !idleLike(petState.state)) {
      idleSince = now;
      director.interrupt(now);
    }
    if (!idleLike(p.state) && idleLike(petState.state)) {
      director.interrupt(now);
      if (["daily", "life"].includes(bubbleKind)) dismissTouchDialogue();
    }
    petState = p;
  });

  // 拖拽：动过就开始扑腾，直到松手
  await TAU.event.listen("pet-drag-moved", () => {
    if (held) {
      flapping = true;
      clearTimeout(liftDialogueTimer);
      dismissTouchDialogue();
    }
  });

  // 视线：Rust 算出八向，待机静止期替换注视帧；空串=鼠标静止复位，还回眨眼
  await TAU.event.listen("pet-gaze", (ev) => {
    if (typeof ev.payload !== "string") return;
    if (ev.payload === "") gazeActive = false;
    else { gazeDir = ev.payload; gazeActive = true; }
  });

  await TAU.event.listen("pet-interaction", () => {
    idleSince = performance.now();
    dismissTouchDialogue();
  });
  await TAU.event.listen("pet-menu-visible", (ev) => {
    menuOpen = ev.payload === true;
    director.interrupt(performance.now());
    idleSince = performance.now();
    if (menuOpen) dismissTouchDialogue();else renderPendingSpeech();
  });
  await TAU.event.listen("bubble-dragging",event=>{
    const p=event.payload;if(p.owner!==bubbleOwner)return;
    clearTimeout(bubbleTimer);
    if(p.dragging){bubbleLockUntil=Infinity;return;}
    bubbleLockUntil=0;
    if(deferredUsage?.owner===bubbleOwner){const pending=deferredUsage;deferredUsage=null;pending.apply();return;}
    if(bubbleKind==="chat"){speechDeadline=performance.now()+(pendingSpeech?.streaming?125000:15000);renderPendingSpeech();}
    else{const owner=bubbleOwner;bubbleLockUntil=performance.now()+15000; bubbleTimer=setTimeout(()=>{if(bubbleOwner===owner){clearCurrentBubble();renderPendingSpeech();}},15000);}
  });
  await TAU.event.listen("bubble-hover",event=>{
    const p=event.payload;if(p.owner!==bubbleOwner)return;
    if(bubbleKind!=="chat"){speechHovered=p.hovered;return;}
    if(p.hovered&&!speechHovered){speechRemaining=Math.max(1000,bubbleLockUntil-performance.now());clearTimeout(bubbleTimer);speechHovered=true;bubbleLockUntil=Number.MAX_SAFE_INTEGER;speechDeadline=bubbleLockUntil;}
    else if(!p.hovered&&speechHovered){speechHovered=false;armSpeechTimer(speechRemaining);}
  });
  await TAU.event.listen("bubble-dismiss",event=>{
    if(bubbleKind!=="chat"||event.payload.owner!==bubbleOwner)return;
    speechHiddenRequest=pendingSpeech?.request_id;hideSpeech();
  });

  // 偏好面板保存/预览/取消 → 立即应用
  await TAU.event.listen("settings-changed", (ev) => applySettings(ev.payload));

  // 右键菜单"额度" → 查询并气泡展示
  await TAU.event.listen("show-usage", () => showUsage());

  // 松手结算：只认领本轮 seq 的释放事件，陈旧的丢弃
  await TAU.event.listen("pet-released", (ev) => {
    const p = ev.payload || {};
    logDebug(`released seq=${p.seq} my=${myDragSeq} held=${held} dy=${p.dy}`);
    if (!held) return;
    if (typeof p.seq !== "number") return;
    if (dragStarting) {
      // Rust 线程可在 invoke 返回序号前释放；等序号确认后再认领。
      earlyReleases.set(p.seq, p);
      if (earlyReleases.size > 8) earlyReleases.delete(earlyReleases.keys().next().value);
      return;
    }
    if (p.seq !== myDragSeq) return;
    finishRelease(p.x, p.y, p.dy);
  });
}

(async () => {
  await loadConfig();
  SF = await appWin.scaleFactor();
  const settings = await TAU.core.invoke("load_settings").catch(() => null);
  if (settings) {
    S = Number(settings.scale) || 1;
    winW = 320 * S; winH = 360 * S;
    feetOff = 336 * S; modelH = 280 * S;
    if (Math.abs(S - 1) > 0.001) {
      appWin.setSize(new TAU.window.LogicalSize(winW, winH)).catch(() => {});
    }
    wanderOn = settings.wander !== false;
    gazeEnabled = settings.gaze !== false;
    dailySpeechOn = settings.daily_speech !== false;
    cursorPlayOn = settings.cursor_play !== false;
    cursorCarryOn = settings.cursor_carry === true;
    timeRhythm.configure(settings);
  }
  floorY = screen.availHeight - feetOff;
  const pos = await appWin.outerPosition();
  const navigation = performance.getEntriesByType("navigation")[0];
  logDebug(`startup sf=${SF} pos=${pos.x},${pos.y} avail=${screen.availWidth}x${screen.availHeight} floorY=${floorY} scale=${S} timeOrigin=${performance.timeOrigin} navigation=${navigation?.type || "unknown"}`);
  winX = pos.x / SF;
  winY = pos.y / SF;
  // 启动校正：贴到工作区地面
  baseY = floorY;
  winY = floorY;
  const maxX = screen.availWidth - winW;
  if (winX < 0) winX = 0;
  if (winX > maxX) winX = maxX;
  commitPos(performance.now(), true);
  try{doubleClick=await TAU.core.invoke("input_gesture");}catch(_){logDebug("input-gesture default-thresholds");}
  await subscribe();
  const acceptChat = message => {
    const now = performance.now();
    if(message.generation<speechGeneration||(!message.active&&message.revision<speechRevision))return;
    if (!director.chat.update(message, now)) return;
    if (message.active && message.source === "user") idleSince = now;
    if(message.generation>speechGeneration){speechGeneration=message.generation;speechRevision=message.revision;speechHiddenRequest=null;hideSpeech();}
    if(message.completed)showChatSpeech({...message,streaming:false});
    else if(!message.active)hideSpeech();
  };
  await TAU.event.listen("chat-speech",event=>showChatSpeech(event.payload));
  await TAU.event.listen("chat-activity", event => acceptChat(event.payload));
  try { acceptChat(await TAU.core.invoke("chat_activity_state")); } catch (error) { logDebug(`chat-state-error ${error}`); }
  await TAU.event.listen("life-changed", event => director.life.update(event.payload));
  await TAU.event.listen("life-feedback", event => {
    if (!director.life.feedback(event.payload)) return;
    const now = performance.now();
    idleSince = now;
    dismissTouchDialogue();
    director.interrupt(now, "life-feedback");
  });
  await TAU.event.listen("input-presence",event=>{
    const p=event.payload;if(p.revision<inputPresenceRevision)return;inputPresenceRevision=p.revision;inputEditing=p.editing===true;
  });
  await TAU.event.listen("extensions-changed", event => acceptExtensions(event.payload));
  const acceptMusic = snapshot => {
    const now = performance.now();
    if (director.music.update(snapshot, now)) idleSince = now;
  };
  await TAU.event.listen("music-session", event => acceptMusic(event.payload));
  try { acceptMusic((await TAU.core.invoke("music_get")).state); }
  catch (error) { logDebug(`music-load-error ${error}`); }
  try { acceptExtensions(await TAU.core.invoke("extension_state")); }
  catch (error) { logDebug(`extension-state-error ${error}`); }
  try { director.life.update(await TAU.core.invoke("life_get")); }
  catch (error) { logDebug(`life-load-error ${error}`); }
  player.play("idle", performance.now());
  requestAnimationFrame(tick);
})();
