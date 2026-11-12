// 只切换面板背景，不改变文字透明度或窗口鼠标命中。
const TAU=window.__TAURI__;
let eventSeen=false;
function apply(settings){document.documentElement.dataset.surface=settings?.panel_style==="clear"?"clear":"light";}
(async()=>{
  try{
    await TAU.event.listen("settings-changed",event=>{eventSeen=true;apply(event.payload);});
    const settings=await TAU.core.invoke("load_settings");
    if(!eventSeen)apply(settings);
  }catch(_){/* 保留CSS默认材质 */}
})();
