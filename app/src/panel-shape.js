// 面板外留白让给下层，缩放手柄和少量阴影边距保留在窗口内。
export function startPanelShape(selector, { handles = false, padding = 4 } = {}) {
  const target = document.querySelector(selector);
  if (!target) return;
  const invoke = window.__TAURI__.core.invoke;
  let busy=false, pending=false, last="", logged=false;
  async function update() {
    pending=true;
    if (busy) return;
    busy=true;
    try {
      while (pending) {
        pending=false;
        const rects=[];
        for (const el of [target,...(handles?document.querySelectorAll('[data-resize]'):[])]) {
          const r=el.getBoundingClientRect();
          if (!r.width || !r.height) continue;
          const p=el===target?padding:0;
          rects.push([r.left-p,r.top-p,r.right+p,r.bottom+p]);
        }
        if (!rects.length || !innerWidth || !innerHeight) continue;
        const viewport=[innerWidth,innerHeight], key=JSON.stringify([viewport,rects]);
        if (key===last) continue;
        try { await invoke('panel_shape',{viewport,rects});last=key;logged=false; }
        catch (_) { if(!logged)invoke('log_debug',{line:'panel-shape stage=failed'}).catch(()=>{});logged=true; }
      }
    } finally { busy=false; }
  }
  let scheduled=false;
  function schedule() { if(!scheduled){scheduled=true;requestAnimationFrame(()=>{scheduled=false;void update();});} }
  new ResizeObserver(schedule).observe(target);
  window.addEventListener('resize',schedule);
  schedule();
}
