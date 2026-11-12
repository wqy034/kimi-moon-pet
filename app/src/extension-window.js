import { startPanelShape } from "./panel-shape.js";
import "./surface.js";
// 所有扩展共享窗口外壳；业务页只负责数据和意图，不接管桌宠动画。
const TAU = window.__TAURI__;
export async function mountExtension({ onOpen, onClose = () => {}, onState = () => {} }) {
  startPanelShape(".extension-shell", { handles: true });
  const kind = document.body.dataset.extension;
  const errorBox = document.getElementById("extension-error");
  const pin = document.getElementById("extension-pin");
  let revision = -1, pinned = true, pinPending = false, closePending = false;
  const error = reason => {
    errorBox.hidden = false; errorBox.textContent = String(reason);
    TAU.core.invoke("log_debug", { line: `extension-error id=${kind} ${reason}` }).catch(() => {});
  };
  function accept(state) {
    if (!state || state.revision < revision) return;
    revision = state.revision;
    const own = state.windows.find(item => item.id === kind);
    if (!own) return;
    onState(own);
    pinned = own.pinned;
    const label = pinned ? "取消置顶" : "保持置顶";
    pin.setAttribute("aria-pressed", String(pinned)); pin.setAttribute("aria-label", label); pin.title = label;
  }
  async function close() {
    if (closePending) return;
    closePending = true;
    try {
      try { await onClose(); } catch (reason) { error(reason); }
      await TAU.core.invoke("extension_close");
    }
    catch (reason) { error(reason); }
    finally { closePending = false; }
  }
  document.querySelector(".extension-header").addEventListener("pointerdown", event => {
    if (event.button !== 0 || event.target.closest("button")) return;
    event.preventDefault();
    TAU.core.invoke("begin_panel_drag").catch(error);
  });
  document.getElementById("extension-close").addEventListener("click", close);
  pin.addEventListener("click", async () => {
    if (pinPending) return;
    pinPending = true; pin.disabled = true;
    try { accept(await TAU.core.invoke("extension_pin", { pinned: !pinned })); }
    catch (reason) { error(reason); }
    finally { pinPending = false; pin.disabled = false; }
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") { event.preventDefault(); close(); }
  });
  document.querySelectorAll("[data-resize]").forEach(handle => handle.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    event.preventDefault();
    TAU.window.getCurrentWindow().startResizeDragging(handle.dataset.resize).catch(error);
  }));
  await TAU.event.listen("extensions-changed", event => accept(event.payload));
  await TAU.event.listen("extension-opened", event => {
    if (event.payload !== kind) return;
    errorBox.hidden = true;
    Promise.resolve(onOpen()).catch(error);
  });
  accept(await TAU.core.invoke("extension_state"));
  await onOpen();
  accept(await TAU.core.invoke("extension_ready"));
}
