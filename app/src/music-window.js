import { mountExtension } from "./extension-window.js";
import { PianoAudio } from "./music-audio.js";
import { MusicEngine } from "./music-engine.js";
import { MusicFlow } from "./music-flow.js";
import { DEFAULT_KEYS, JUDGMENTS, DIFFICULTIES, pitchName, recordKey } from "./music-layout.js";

const TAU = window.__TAURI__, invoke = (name, args) => TAU.core.invoke(name, args);
const $ = id => document.getElementById(id);
const audio = new PianoAudio(), canvas = $("flow"), ctx = canvas.getContext("2d");
const flow = new MusicFlow();
const NAMES = ["do", "re", "mi", "fa", "sol", "la", "si"], OCTAVES = ["低", "中", "高"];
const CODES = ["ZXCVBNM", "ASDFGHJ", "QWERTYU"];
const keyNodes = new Map(), held = new Map(), flashes = new Map();
let catalog = null, mode = "free", enabled = false, loading = false, busy = false;
let engine = null, id = 0, nativeSession = null, revision = -1, operation = 0;
let paused = false, finishing = false, freeAt = -Infinity, freeStarting = null;
let freeKeys = null;
let view = "free", practiceMode = "sight", lastKind = "sight", transitioning = false, viewEpoch = 0;
function renderView() {
  document.body.dataset.view = view;
  document.body.dataset.playMode = engine?.mode === "demo" ? "demo" : mode;
  const setup = view === "setup", running = view === "playing";
  $("song-setup").hidden = !setup;
  $("choose-song").hidden = view !== "free";
  $("back-free").hidden = !setup;
  $("round-controls").hidden = !running;
  $("round-result").hidden = view !== "result";
  $("music-stage").hidden = !running;
  $("song-progress").hidden = !running;
  $("music-keyboard").hidden = !["free", "playing"].includes(view);
  $("playing-title").textContent = engine?.song.title || selected()?.title || "";
  $("enable-audio").hidden = enabled;
  $("free-key").textContent = selected()?.key_label || "C大调";
  $("free-key").hidden = view !== "free";
  $("settings-toggle").title = enabled ? (audio.muted ? "小设置 · 已静音" : "小设置 · 琴声已启用") : "小设置";
}
async function navigate(next) {
  if (transitioning || busy || finishing) return;
  transitioning = true;
  const before = view, epoch = viewEpoch;
  try {
    await stop();
    if (epoch !== viewEpoch || !visible) return;
    await invoke("extension_music_layout", { presentation: next === "free" ? "compact" : "expanded" });
    if (epoch !== viewEpoch || !visible) return;
    view = next; mode = next === "free" ? "free" : practiceMode;
    settings(false);
    controls();
    status(next === "free" ? "随手弹一段吧。" : "选一首喜欢的旋律。");
    (next === "free" ? $("choose-song") : $("song")).focus({ preventScroll: true });
  } catch (error) { if (epoch === viewEpoch) { view = before === "playing" ? "setup" : before; mode = view === "free" ? "free" : practiceMode; controls(); status(String(error)); } }
  finally { if (epoch === viewEpoch) transitioning = false; }
}
let visible = false, beatQueue = Promise.resolve(), beatPending = false, lastStatus = "";
function status(text) { if (text !== lastStatus) { lastStatus = text; $("music-status").textContent = text; } }
function selected() { return catalog?.songs.find(song => song.id === $("song").value); }
function keys() { return engine?.song.keys || freeKeys || selected()?.keys || DEFAULT_KEYS; }
function judgment() { return $("judgment").value === "recommended" ? selected()?.recommended || "coarse" : $("judgment").value; }
function controls() {
  $("enable-audio").disabled = loading || enabled;
  $("enable-audio").textContent = loading ? "准备琴声……" : "启用琴声";
  $("assist").disabled = !!id || busy;
  $("play").disabled = !enabled || busy || !!id || mode === "free" || !selected() || selected().player_count < 4;
  $("demo").disabled = !enabled || busy || !!id || !selected();
  $("pause").disabled = !id || !engine || busy || finishing;
  $("pause").textContent = paused ? "继续" : "暂停";
  $("stop").disabled = !id && !busy;
  $("song").disabled = !!id || busy; $("random-song").disabled = !!id || busy;
  $("timing-offset").disabled = !!id || busy; $("judgment").disabled = !!id || busy;
  document.querySelectorAll("[data-mode]").forEach(node => node.setAttribute("aria-selected", String(node.dataset.mode === practiceMode)));
  const song = engine?.song || selected(), record = song && catalog?.records[recordKey(song)];
  const levels = ["", "粗判", "严判", "超严判"];
  $("record-label").textContent = [record?.sight && `光流·${levels[record.sight]}通过`, record?.mole && `地鼠·${levels[record.mole]}通过`].filter(Boolean).join(" / ");
  $("song-meta").textContent = song ? `${song.key_label} · ${DIFFICULTIES[song.difficulty]} · 玩家${song.player_count}音 / 她补${song.assist_count}音 · 推荐${JUDGMENTS[song.recommended].label}` : "自由琴盘 · C大调";
  $("song-meta").dataset.difficulty = song?.difficulty || "green";
  $("judgment-info").textContent = `±${JUDGMENTS[engine?.judgment || judgment()].window}ms`;
  for (const [index, node] of keyNodes) {
    const mapped = keys()[index], octave = Math.floor(index / 7), key = index % 7;
    node.querySelector(".note-name").textContent = `${mapped.label}`;
    node.querySelector(".note-info").textContent = `${CODES[octave][key]}`;
    node.title = `${OCTAVES[octave]} · ${pitchName(mapped.pitch)}`;
    node.setAttribute("aria-label", `${OCTAVES[octave]}音区${mapped.label}，${pitchName(mapped.pitch)}，键盘${CODES[octave][key]}`);
  }
  $("round-label").textContent = engine?.mode === "demo" ? "听她弹 · 示范不计通关" : ({ free: "自由弹奏 · 无判定", sight: "光流 · 提前看，轻轻敲", mole: "地鼠 · 跟着探头的小家伙敲" }[mode]);
  $("flow-instruction").textContent = engine?.mode === "demo" ? "示范中 · 仅展示你的声部" : "音符下沿碰亮线时按键";
  renderView();
}
function clearHeld() { for (const v of held.values()) audio.release(v.voice); held.clear(); }
function resetLocal() {
  operation++; engine?.pause(); engine = null; id = 0; nativeSession = null;
  paused = false; finishing = false; freeAt = -Infinity; freeKeys = null; clearHeld(); audio.stopAll(); controls();
}
async function stop() {
  const old = id; resetLocal();
  if (old) await invoke("music_stop", { id: old });
}
function accept(snapshot) {
  if (!snapshot || snapshot.revision < revision) return;
  revision = snapshot.revision; nativeSession = snapshot.session;
  if (!id) return;
  if (!nativeSession || nativeSession.id !== id) {
    // finish的成功事件可能先于RPC结果到达，由finish处理最终提示。
    if (!finishing) { resetLocal(); if (view === "playing") { view = "setup"; mode = practiceMode; controls(); } status("演奏已停止，可以再来一首。"); }
    return;
  }
  if (nativeSession.phase === "paused" && !paused) {
    engine?.pause(); paused = true; clearHeld(); controls();
  }
  reconcile();
}
function reconcile() {
  if (!engine || paused || !visible || finishing) return;
  if (engine.mode === "demo") {
    if (nativeSession?.granted && nativeSession.phase === "playing") {
      if (!engine.playing) engine.start();
    } else if (engine.playing) { engine.pause(); clearHeld(); }
  }
}
function heartbeat(phase, final = false) {
  const targetId = id, token = operation;
  if (!targetId || !visible || beatPending && !final) return beatQueue;
  beatPending = true;
  beatQueue = beatQueue.catch(() => {}).then(async () => {
    if (targetId !== id || token !== operation) return;
    const position = engine ? engine.time() : 0;
    const state = await invoke("music_beat", { id: targetId, phase, position });
    if (targetId === id && token === operation) accept(state);
  }).catch(error => {
    if (targetId === id && token === operation) { resetLocal(); if (view === "playing") view = "setup"; controls(); status(`演奏已暂停：${error}`); }
  }).finally(() => { beatPending = false; });
  return beatQueue;
}
async function start(kind) {
  if (!enabled || busy || id || !visible || transitioning) return;
  const song = selected(); if (kind !== "free" && !song) return;
  busy = true; controls(); const token = ++operation;
  try {
    if (kind === "free") freeKeys = [...keys()];
    await audio.context.resume();
    const reply = await invoke("music_start", { mode: kind, songId: song?.id || null, judgment: judgment() });
    if (token !== operation || !visible) {
      if (reply.state.session) await invoke("music_stop", { id: reply.state.session.id });
      return;
    }
    id = reply.state.session.id; paused = false;
    if (kind !== "free") { view = "playing"; lastKind = kind; }
    engine = kind === "free" ? null : new MusicEngine(audio, reply.song, kind, Number($("timing-offset").value), reply.judgment);
    accept(reply.state);
    if (kind !== "demo" && engine) engine.start();
    status(kind === "free" ? "随便弹几下吧，她在旁边听着。" : kind === "demo" ? "等她准备好，就开始弹给你听。" : "跟着预备拍，下一段旋律交给你。 ");
  } catch (error) {
    if (token === operation) {
      const previous = id; resetLocal();
      if (previous) await invoke("music_stop", { id: previous }).catch(() => {});
      if (view === "playing") view = "setup";
      controls(); status(`暂时无法开始：${error}`);
    }
  }
  finally { busy = false; controls(); }
}
async function finish() {
  if (!engine || !id || finishing) return;
  finishing = true; const current = engine, targetId = id, token = operation;
  current.advance(); current.position = current.duration; current.playing = false; audio.stopAll(); clearHeld(); controls();
  try {
    if (current.mode === "demo") {
      await invoke("music_stop", { id: targetId });
      if (token === operation) { resetLocal(); view = "result"; controls(); status("暗月弹完啦，换你试试看？"); }
    } else {
      await heartbeat("playing", true);
      if (targetId !== id || token !== operation) return;
      const result = await invoke("music_finish", { id: targetId, hits: current.hits, wrong: current.wrong });
      if (token !== operation) return;
      resetLocal(); view = "result"; await refresh();
      status(result.passed ? `这一段弹下来啦！已记住21键·${JUDGMENTS[result.judgment].label}通过。` : "没关系，旋律还在这里，慢慢来，再弹一次吧。 ");
    }
  } catch (error) {
    if (token === operation) {
      resetLocal(); await invoke("music_stop", { id: targetId }).catch(() => {});
      view = "result"; controls(); status(`本轮结束，结果暂未保存：${error}`);
    }
  }
}
function eventTime(event) { return Math.abs(event.timeStamp - performance.now()) < 60000 ? event.timeStamp : performance.now(); }
function press(index, source, event) {
  if (!enabled || !visible || held.has(source) || busy || finishing || transitioning || !$("music-settings").hidden || !["free", "playing"].includes(view)) return;
  if ([...held.values()].some(value => value.index === index)) return;
  if (mode !== "free" && (!engine || !engine.playing || engine.mode === "demo")) return;
  if (engine?.mode === "demo") return;
  let result;
  if (engine) result = engine.press(index, eventTime(event));
  else {
    result = { voice: audio.playPitch(keys()[index].pitch), hit: false };
    freeAt = performance.now(); paused = false;
    if (!id && !freeStarting) {
      freeStarting = start("free").finally(() => { freeStarting = null; });
    } else if (nativeSession?.phase === "paused") heartbeat("playing");
  }
  held.set(source, { index, voice: result.voice });
  if (result.hit) flashes.set(index, performance.now() + 240);
}
function release(source) { const v = held.get(source); if (v) audio.release(v.voice); held.delete(source); }
for (const octave of [2, 1, 0]) for (let key = 0; key < 7; key++) {
  const index = octave * 7 + key, node = document.createElement("button");
  node.className = `note-key oct-${octave}`; node.type = "button";
  node.setAttribute("aria-label", `${OCTAVES[octave]}音${NAMES[key]}，键盘${CODES[octave][key]}`);
  const name = document.createElement("span"), info = document.createElement("span"), glow = document.createElement("i"), mole = document.createElement("i");
  name.className = "note-name"; name.textContent = NAMES[key]; info.className = "note-info"; info.textContent = `${OCTAVES[octave]} · ${CODES[octave][key]}`;
  glow.className = "key-glow"; mole.className = "mole"; mole.setAttribute("aria-hidden", "true");
  node.append(name, info, glow, mole); keyNodes.set(index, node); $("music-keyboard").append(node);
  node.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    event.preventDefault(); node.setPointerCapture(event.pointerId); press(index, `pointer:${event.pointerId}`, event);
  });
  for (const type of ["pointerup", "pointercancel", "lostpointercapture"]) node.addEventListener(type, event => release(`pointer:${event.pointerId}`));
}
document.addEventListener("keydown", event => {
  if (event.repeat || event.ctrlKey || event.altKey || event.metaKey || event.target.closest("select,input,[contenteditable=true]")) return;
  const letter = event.code.startsWith("Key") ? event.code.slice(3) : "";
  const octave = CODES.findIndex(row => row.includes(letter) && letter.length === 1);
  if (octave < 0) return;
  event.preventDefault(); press(octave * 7 + CODES[octave].indexOf(letter), event.code, event);
});
document.addEventListener("keyup", event => release(event.code));
window.addEventListener("blur", () => { clearHeld(); audio.stopAll(); if (engine?.mode !== "demo" && id) pauseRound(); });
async function pauseRound() {
  if (!id || finishing) return;
  engine?.pause(); paused = true; clearHeld(); audio.stopAll(); controls();
  await heartbeat("paused");
}
$("enable-audio").addEventListener("click", async () => {
  if (loading || enabled) return;
  loading = true; controls();
  try { await audio.enable(); enabled = true; status("琴声准备好啦。可以自由敲键，也可以听她弹一首。 "); }
  catch (error) { status(`琴声还没准备好：${error}`); }
  finally { loading = false; controls(); }
});
$("volume").addEventListener("input", () => audio.setVolume(Number($("volume").value) / 100));
$("mute").addEventListener("click", () => { audio.setVolume(audio.volume, !audio.muted); $("mute").textContent = audio.muted ? "取消静音" : "静音"; $("mute").setAttribute("aria-pressed", String(audio.muted)); });
$("timing-offset").addEventListener("input", () => { $("offset-value").textContent = `${$("timing-offset").value} ms`; });
document.querySelectorAll("[data-mode]").forEach(node => node.addEventListener("click", async () => {
  try { await stop(); practiceMode = node.dataset.mode; mode = practiceMode; controls(); status("选好曲子，点开始，看两拍预备再敲。"); }
  catch (error) { status(String(error)); }
}));
$("play").addEventListener("click", () => start(mode)); $("demo").addEventListener("click", () => start("demo"));
$("stop").addEventListener("click", () => navigate("setup"));
$("choose-song").addEventListener("click", () => navigate("setup"));
$("back-free").addEventListener("click", () => navigate("free"));
$("change-song").addEventListener("click", () => navigate("setup"));
$("result-free").addEventListener("click", () => navigate("free"));
$("again").addEventListener("click", () => start(lastKind));
function settings(open) {
  clearHeld(); $("music-settings").hidden = !open;
  for (const name of ["song-setup", "round-controls", "music-keyboard", "round-result", "choose-song", "back-free"]) $(name).inert = open;
  if (open && id && engine && engine.mode !== "demo" && !paused) pauseRound().catch(error => status(String(error)));
  $("settings-toggle").setAttribute("aria-expanded", String(open));
  (open ? $("settings-close") : $("settings-toggle")).focus({ preventScroll: true });
}
$("settings-toggle").addEventListener("click", () => settings($("music-settings").hidden));
$("settings-close").addEventListener("click", () => settings(false));
$("pause").addEventListener("click", async () => {
  if (!engine || !id) return;
  if (!paused) { await pauseRound(); return; }
  try {
    await audio.context.resume(); paused = false; await heartbeat("playing", true);
    if (engine && id && !paused && engine.mode !== "demo") engine.start();
    reconcile(); controls();
  } catch (error) { status(String(error)); }
});
$("song").addEventListener("change", controls);
$("judgment").addEventListener("change", controls);
$("assist").addEventListener("change", () => audio.setAssist($("assist").checked));
$("random-song").addEventListener("click", () => {
  if (!catalog?.songs.length || id) return;
  const choices = catalog.songs.filter(song => song.id !== $("song").value);
  const list = choices.length ? choices : catalog.songs;
  let ticket = Math.random() * list.reduce((sum, song) => sum + (catalog.weights[song.id] || 1), 0);
  const song = list.find(song => (ticket -= catalog.weights[song.id] || 1) <= 0) || list[0];
  $("song").value = song.id; controls();
});
async function refresh() {
  const selectedId = $("song").value;
  const data = await invoke("music_get"); catalog = data; accept(data.state);
  $("song").replaceChildren(...data.songs.map(song => { const option = document.createElement("option"); option.value = song.id; option.textContent = `${song.title} · ${song.bpm}拍/分`; return option; }));
  if (data.songs.some(song => song.id === selectedId)) $("song").value = selectedId;
  if (data.warnings.length) status(data.warnings.join(" "));
  controls();
}
function draw() {
  engine?.advance(); reconcile();
  // 练习的画面与输入判定使用相同的微调量；示范沿实际自动演奏时刻显示。
  const t = engine ? engine.rawTime() + (engine.mode === "demo" ? 0 : engine.offset) : 0, now = performance.now();
  const hints = new Map();
  if (engine?.playing) for (const note of engine.displayNotes) {
    const ahead = note.time - t;
    if (note.player && note.state !== "hit" && ahead >= -engine.window && ahead <= (mode === "mole" && engine.mode !== "demo" ? .5 : .8) && !hints.has(note.index)) hints.set(note.index, ahead);
  }
  for (const [index, node] of keyNodes) {
    const hint = hints.get(index), target = hint !== undefined;
    node.classList.toggle("pressed", [...held.values()].some(v => v.index === index));
    node.classList.toggle("hit", (flashes.get(index) || 0) > now);
    node.classList.toggle("target", target); node.classList.toggle("mole-target", mode === "mole" && engine?.mode !== "demo");
    if (target) {
      const anticipation = mode === "mole" ? .5 : .8;
      node.style.setProperty("--hint-opacity", String(Math.max(.15, Math.min(1, 1 - hint / anticipation))));
      node.style.setProperty("--approach", String(1 + Math.max(0, hint) / anticipation * .45));
    } else node.style.setProperty("--hint-opacity", "0");
  }
  const rect = canvas.getBoundingClientRect(), dpr = window.devicePixelRatio || 1;
  if (canvas.width !== Math.round(rect.width * dpr) || canvas.height !== Math.round(rect.height * dpr)) { canvas.width = Math.round(rect.width * dpr); canvas.height = Math.round(rect.height * dpr); }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, rect.width, rect.height);
  const showFlow = view === "playing" && engine && (mode !== "mole" || engine.mode === "demo");
  if (showFlow) flow.draw(ctx, rect.width, rect.height, engine, t, flashes, now);
  $("flow-guide").hidden = !showFlow;
  $("stage-caption").hidden = !!showFlow;
  $("stage-caption").textContent = engine && mode === "mole" ? "小家伙会在正确的琴键上探头。" : "小小的琴键，也能装下今晚的旋律。";
  $("count-in").textContent = engine?.countIn ? String(engine.countIn) : "";
  $("song-progress").value = engine ? engine.time() / engine.duration : 0;
  if (engine && !finishing) {
    if (paused) status("暂停一下。点继续，会重新给你预备拍。 ");
    else if (engine.mode === "demo" && !nativeSession?.granted) status(`${nativeSession?.reason || "她在准备"}，示范稍后接着弹。`);
    else if (engine.mode !== "demo" && nativeSession && !nativeSession.granted) status("她正在忙，你可以继续练；忙完就回来陪你。 ");
    else status(engine.mode === "demo" ? `听她弹《${engine.song.title}》` : `《${engine.song.title}》 · ${mode === "mole" ? "跟着地鼠敲" : "跟着光流敲"}`);
  }
  if (engine?.ended && !finishing) finish();
  requestAnimationFrame(draw);
}
setInterval(() => { engine?.advance(); if (id && visible && !finishing) heartbeat(paused ? "paused" : engine || performance.now() - freeAt < 4000 ? "playing" : "paused"); }, 1000);
setInterval(() => engine?.advance(), 25);
async function initialize() {
  await TAU.event.listen("music-session", event => accept(event.payload));
  await mountExtension({
    // 打开仅刷新资料；显隐及视图代次统一在onState处理。
    onOpen: refresh,
    onClose: stop,
    onState: state => {
      const wasVisible = visible;
      visible = state.visible;
      if (visible && !wasVisible) { view = "free"; mode = "free"; controls(); }
      if (!visible) { if (wasVisible) viewEpoch++; transitioning = false; view = "free"; mode = "free"; settings(false); const previous = id; resetLocal(); if (previous) invoke("music_stop", { id: previous }).catch(() => {}); }
      else if ((!state.focused || state.dragging) && engine?.mode !== "demo" && id && !paused) pauseRound();
    },
  });
  requestAnimationFrame(draw);
}
initialize().catch(error => { status(String(error)); invoke("log_debug", { line: `music-window-start-error ${error}` }).catch(() => {}); });
