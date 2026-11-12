import { startPanelShape } from "./panel-shape.js";
import "./surface.js";
startPanelShape("#shell", { padding: 1 });
const TAU=window.__TAURI__, editor=document.getElementById("text"), orb=document.getElementById("orb");
let state=null, chat=null, chatRevision=-1, seenFocus=0, composing=false, sending=false, editVersion=0, sequence=0, gesture=null, layoutTimer=0;
function log(line){TAU.core.invoke("log_debug",{line:`input-ui ${line}`}).catch(()=>{});}
function logLayout(){
  clearTimeout(layoutTimer);layoutTimer=setTimeout(()=>{
    const a=orb.getBoundingClientRect(),b=editor.getBoundingClientRect();
    log(`stage=layout mode=${state?.mode||"initial"} viewport=${innerWidth}x${innerHeight} orb=${Math.round(a.width)}x${Math.round(a.height)} editor=${Math.round(b.width)}x${Math.round(b.height)}`);
  },100);
}
window.addEventListener("resize",logLayout);
function notice(text=""){
  document.getElementById("notice").textContent=text;document.body.dataset.error=String(!!text);
  editor.title=text;orb.title=text||"点击收起或恢复，按住拖动";
}
async function invoke(action,args={}){
  const id=++sequence;log(`action=${action} id=${id} stage=click`);
  const timer=setTimeout(()=>log(`action=${action} id=${id} stage=waiting`),3000);
  try{const result=await TAU.core.invoke(action,args);log(`action=${action} id=${id} stage=done`);return result;}
  catch(error){log(`action=${action} id=${id} stage=failed`);throw error;}
  finally{clearTimeout(timer);}
}
function accept(next){
  if(!next||next.revision<(state?.revision??-1))return;
  const modeChanged=state?.mode!==next.mode;
  if(state&&next.session!==state.session){composing=false;editor.value="";editVersion++;notice();}
  state=next;if(next.mode!=="normal"){composing=false;editor.blur();}document.body.dataset.mode=next.mode;orb.setAttribute("aria-label",next.mode==="minimized"?"恢复输入":"收起输入");
  if(modeChanged)logLayout();
  if(next.focus_serial>seenFocus){seenFocus=next.focus_serial;requestAnimationFrame(()=>{if(state.mode==="normal"){editor.focus({preventScroll:true});TAU.core.invoke("input_editing",{editing:true}).catch(()=>{});}});}
}
function acceptChat(next){
  if(!next||next.revision<chatRevision)return;
  chatRevision=next.revision;chat=next;
  if(next.error||next.reply_error)notice(next.error||next.reply_error);else notice();
}
editor.addEventListener("input",()=>{editVersion++;if(!composing&&[...editor.value].length>4000)editor.value=[...editor.value].slice(0,4000).join("");notice();});
editor.addEventListener("compositionstart",()=>{composing=true;});
editor.addEventListener("compositionend",()=>{composing=false;if([...editor.value].length>4000)editor.value=[...editor.value].slice(0,4000).join("");editVersion++;});
editor.addEventListener("focus",()=>TAU.core.invoke("input_editing",{editing:true}).catch(()=>{}));
editor.addEventListener("blur",()=>TAU.core.invoke("input_editing",{editing:false}).catch(()=>{}));
document.addEventListener("keydown",async event=>{
  if(composing||event.isComposing)return;
  if(event.key==="Escape"){event.preventDefault();try{accept(await invoke("input_mode",{mode:"hidden"}));}catch(error){notice(String(error));}return;}
  if(event.target!==editor||event.key!=="Enter"||event.shiftKey)return;
  event.preventDefault();if(sending||!editor.value.trim())return;
  if(!chat?.configured){notice("尚未配置连接，请从桌宠右键菜单进入交流设置。");return;}
  if(chat.active?.source==="user"){notice("她正在回复，可以先写下一句。");return;}
  const text=editor.value,version=editVersion,session=state?.session;sending=true;
  try{
    acceptChat(await invoke("chat_send",{text,generation:chat.generation}));
    if(version===editVersion&&session===state?.session){editor.value="";editVersion++;}notice();
  }catch(error){notice(String(error));}finally{sending=false;}
});
orb.addEventListener("pointerdown",event=>{if(event.button!==0)return;gesture={id:event.pointerId,x:event.screenX,y:event.screenY,moved:false};orb.setPointerCapture(event.pointerId);});
orb.addEventListener("pointermove",event=>{
  if(!gesture||gesture.id!==event.pointerId||gesture.moved||!(event.buttons&1)||Math.hypot(event.screenX-gesture.x,event.screenY-gesture.y)<4)return;
  gesture.moved=true;invoke("begin_panel_drag").catch(error=>notice(String(error)));
});
orb.addEventListener("pointercancel",()=>{gesture={moved:true};});
orb.addEventListener("click",async event=>{
  const moved=event.detail!==0&&gesture?.moved;gesture=null;if(moved||!state)return;
  try{accept(await invoke("input_mode",{mode:"toggle"}));}catch(error){notice(String(error));}
});
window.addEventListener("error",()=>log("stage=script-error"));window.addEventListener("unhandledrejection",()=>log("stage=unhandled-rejection"));
await TAU.event.listen("input-reset",()=>{editor.value="";editVersion++;notice();});
await TAU.event.listen("input-state",event=>accept(event.payload));await TAU.event.listen("input-chat",event=>acceptChat(event.payload));
try{accept(await TAU.core.invoke("input_state"));acceptChat(await TAU.core.invoke("chat_input_state"));await invoke("extension_ready");}catch(error){notice(String(error));}
