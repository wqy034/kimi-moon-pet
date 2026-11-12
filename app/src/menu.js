import { startPanelShape } from "./panel-shape.js";
import "./surface.js";
startPanelShape(".menu", { padding: 5 });
const TAU = window.__TAURI__;
document.querySelector("header").addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  TAU.core.invoke("begin_panel_drag").catch(console.error);
});
import { loadPreferences, cancelPreferences } from "./preferences.js";
const items = [...document.querySelectorAll("#main-menu button")];
let preferences = false, viewGeneration = 0;
async function view(value) {
  const current = ++viewGeneration;
  cancelPreferences();
  preferences = value === true;
  document.getElementById("main-menu").hidden = preferences;
  document.getElementById("preferences").hidden = !preferences;
  await TAU.core.invoke("menu_view", { preferences });
  if (current !== viewGeneration) return;
  if (preferences) {
    await loadPreferences();
    if (current === viewGeneration) document.getElementById("back").focus();
  } else { items[0].focus(); }
}
let busy = false;
async function act(action) {
  if (action === "settings") {
    return view(true).catch(console.error);
  }
  if (busy) return;
  busy = true;
  try {
    await TAU.core.invoke("menu_action", { action });
  } catch (error) {
    const el = document.getElementById("error");
    el.hidden = false;
    el.textContent = String(error);
  } finally {
    busy = false;
  }
}
items.forEach((item) => item.addEventListener("click", () => act(item.dataset.action)));
document.getElementById("back").addEventListener("click", () => view(false).catch(console.error));
document.addEventListener("contextmenu", (event) => event.preventDefault());
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") { event.preventDefault(); cancelPreferences(); act("dismiss"); }
  if (preferences) return; // 输入控件保留方向键的原生行为
  if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
    event.preventDefault();
    const index = items.indexOf(document.activeElement);
    const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
      : index < 0 ? (event.key === "ArrowDown" ? 0 : items.length - 1)
      : (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    items[next].focus();
  }
});
await TAU.event.listen("menu-open", (event) => {
  document.getElementById("error").hidden = true;
  view(!!event.payload).catch(console.error);
});
await TAU.event.listen("menu-closed", () => {
  viewGeneration++;
  cancelPreferences();
});
await view(await TAU.core.invoke("menu_preferences"));
await TAU.core.invoke("menu_ready");
