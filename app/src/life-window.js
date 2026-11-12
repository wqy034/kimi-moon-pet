import { openLife } from "./life-panel.js";
import { mountExtension } from "./extension-window.js";

mountExtension({ onOpen: openLife }).catch(reason => {
  const box = document.getElementById("extension-error");
  box.hidden = false; box.textContent = String(reason);
  window.__TAURI__.core.invoke("log_debug", { line: `life-window-start-error ${reason}` }).catch(() => {});
});
