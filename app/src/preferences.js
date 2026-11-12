const TAU = window.__TAURI__;
const $ = (id) => document.getElementById(id);
let baseline = null, timer = 0, generation = 0;
let chain = Promise.resolve();
const inputs = [...document.querySelectorAll("#preferences input, #preferences select")];
function disabled(value) {
  inputs.forEach((input) => { input.disabled = value; });
  $("save").disabled = value;
}
export function cancelPreferences() {
  generation++;
  clearTimeout(timer);
  baseline = null;
}
export async function loadPreferences() {
  cancelPreferences();
  const current = generation;
  disabled(true);
  $("saved").textContent = "";
  try {
    const s = await TAU.core.invoke("load_settings");
    if (current !== generation) return;
    baseline = s;
    $("scale").value = s.scale;
    $("panelStyle").value = s.panel_style || "light";
    $("scaleVal").textContent = Number(s.scale).toFixed(1) + "x";
    $("wander").checked = s.wander;
    $("gaze").checked = s.gaze;
    $("dailySpeech").checked = s.daily_speech !== false;
    $("cursorPlay").checked = s.cursor_play !== false;
    $("cursorCarry").checked = s.cursor_carry === true;
    $("lightSleep").value = Math.round(s.light_sleep_s / 60);
    $("deepSleep").value = Math.round(s.deep_sleep_s / 60);
    $("nightLightSleep").value = Math.round(s.night_light_sleep_s / 60);
    $("nightDeepSleep").value = Math.round(s.night_deep_sleep_s / 60);
    disabled(false);
  } catch (e) {
    if (current === generation) $("saved").textContent = `读取失败：${e}`;
  }
}
function preview() {
  if (!baseline) return;
  clearTimeout(timer);
  const current = generation;
  timer = setTimeout(() => {
    if (current !== generation || !baseline) return;
    const settings = { ...baseline, scale: Number($("scale").value), panel_style: $("panelStyle").value,
      wander: $("wander").checked, gaze: $("gaze").checked, daily_speech: $("dailySpeech").checked,
      cursor_play: $("cursorPlay").checked, cursor_carry: $("cursorCarry").checked };
    chain = chain.then(async () => {
      if (current === generation) await TAU.core.invoke("preview_settings", { settings });
    }).catch((e) => {
      if (current === generation) $("saved").textContent = `预览失败：${e}`;
    });
  }, 40);
}
$("scale").addEventListener("input", () => {
  $("scaleVal").textContent = Number($("scale").value).toFixed(1) + "x";
  preview();
});
$("wander").addEventListener("change", preview);
$("panelStyle").addEventListener("change", preview);
$("gaze").addEventListener("change", preview);
$("dailySpeech").addEventListener("change", preview);
$("cursorPlay").addEventListener("change", preview);
$("cursorCarry").addEventListener("change", preview);
$("save").addEventListener("click", async () => {
  clearTimeout(timer);
  const current = generation;
  disabled(true);
  const light = Math.min(30, Math.max(1, Math.round(Number($("lightSleep").value) || 15)));
  const deep = Math.min(60, Math.max(light + 1, Math.round(Number($("deepSleep").value) || 20)));
  const nightLight = Math.min(30, Math.max(1, Math.round(Number($("nightLightSleep").value) || 5)));
  const nightDeep = Math.min(60, Math.max(nightLight + 1, Math.round(Number($("nightDeepSleep").value) || 10)));
  $("nightLightSleep").value = nightLight;
  $("nightDeepSleep").value = nightDeep;
  $("lightSleep").value = light;
  $("deepSleep").value = deep;
  const settings = { scale: Number($("scale").value), panel_style: $("panelStyle").value, wander: $("wander").checked,
    gaze: $("gaze").checked, daily_speech: $("dailySpeech").checked, cursor_play: $("cursorPlay").checked,
    cursor_carry: $("cursorCarry").checked,
    light_sleep_s: light * 60, deep_sleep_s: deep * 60,
    night_light_sleep_s: nightLight * 60, night_deep_sleep_s: nightDeep * 60 };
  try {
    await chain;
    if (current !== generation) return;
    await TAU.core.invoke("save_settings", { settings });
    if (current !== generation) return;
    baseline = settings;
    $("saved").textContent = "已保存";
  } catch (e) {
    if (current === generation) $("saved").textContent = `保存未完成：${e}`;
  } finally {
    if (current === generation) disabled(false);
  }
});
