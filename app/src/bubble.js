import { startPanelShape } from "./panel-shape.js";
import "./surface.js";
startPanelShape(".card", { padding: 6 });
const TAU = window.__TAURI__;
const message = document.getElementById("message");
const usage = document.getElementById("usage");
let revision = 0;
let requestId = null, expanded = false, currentContent = null, hoverOwner=null;
document.querySelector("header").addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  TAU.core.invoke("drag_bubble").catch(console.error);
});

async function render(content) {
  if (!content || content.revision < revision) return;
  revision = content.revision;
  currentContent=content;
  const chat=content.kind==="chat";
  if(chat&&requestId!==content.request_id){requestId=content.request_id;expanded=false;message.scrollTop=0;}
  const bottom=message.scrollHeight-message.scrollTop-message.clientHeight<30;
  document.body.dataset.expanded=String(expanded);
  document.getElementById("chat-controls").hidden=!chat;
  document.getElementById("speech-stop").hidden=!content.streaming;
  document.getElementById("speech-status").textContent=content.streaming?"正在说……":"";
  document.getElementById("speech-expand").textContent=expanded?"折叠":"展开";
  document.body.dataset.kind = content.kind || "status";
  document.getElementById("title").textContent = content.usage ? "星光额度" : "暗月";
  message.textContent = content.text || "";
  if(bottom)message.scrollTop=message.scrollHeight;
  message.hidden = !!content.usage;
  usage.hidden = !content.usage;
  usage.replaceChildren();
  if (content.usage) {
    for (const item of content.usage) {
      const row = document.createElement("div");
      const top = document.createElement("div");
      top.className = "row-top";
      const label = document.createElement("span");
      label.className = "label";
      label.textContent = item.label;
      const amount = document.createElement("span");
      amount.className = "amount";
      const valid = Number.isFinite(item.percent);
      const percent = valid ? Math.max(0, Math.min(100, item.percent)) : 0;
      if (valid) {
        const number = document.createElement("span");
        number.textContent = percent;
        const unit = document.createElement("span");
        unit.className = "unit";
        unit.textContent = "%";
        amount.append(number, unit);
      } else {
        amount.textContent = "未知";
      }
      top.append(label, amount);
      const track = document.createElement("div");
      track.className = "track";
      const fill = document.createElement("div");
      fill.className = "fill";
      fill.style.width = `${percent}%`;
      track.append(fill);
      const reset = document.createElement("div");
      reset.className = "reset";
      reset.textContent = `已用 · ${item.reset}重置`;
      row.append(top, track, reset);
      usage.append(row);
    }
  }
  const current = revision;
  await document.fonts.ready;
  if (current !== revision) return;
  await TAU.core.invoke("bubble_ready", { revision: current,
    height: Math.ceil(document.getElementById("card").getBoundingClientRect().height + 20) });
  if(current===revision&&hoverOwner!==content.owner){hoverOwner=content.owner;TAU.core.invoke("bubble_hover",{owner:content.owner,hovered:document.getElementById("card").matches(":hover")}).catch(()=>{});}
}
document.getElementById("speech-expand").addEventListener("click",()=>{expanded=!expanded;if(currentContent)render(currentContent).catch(console.error);});
document.getElementById("speech-stop").addEventListener("click",()=>TAU.core.invoke("chat_stop",{requestId:currentContent?.request_id||null}).catch(console.error));
document.getElementById("speech-hide").addEventListener("click",()=>TAU.core.invoke("bubble_dismiss",{owner:currentContent?.owner||""}).catch(console.error));
document.getElementById("card").addEventListener("pointerenter",()=>TAU.core.invoke("bubble_hover",{hovered:true,owner:currentContent?.owner||""}).catch(()=>{}));
document.getElementById("card").addEventListener("pointerleave",()=>TAU.core.invoke("bubble_hover",{hovered:false,owner:currentContent?.owner||""}).catch(()=>{}));

await TAU.event.listen("bubble-content", (event) => {
  if (!event.payload) { revision++; return; }
  render(event.payload).catch(console.error);
});
await TAU.event.listen("bubble-placement", (event) => {
  document.body.dataset.placement = event.payload;
});
render(await TAU.core.invoke("bubble_content")).catch(console.error);
