import { mountExtension } from "./extension-window.js";
import { refreshMemory } from "./memory-window.js";
import { refreshConnections, closeConnections, setConnectionsAvailable } from "./connections-window.js";
const TAU=window.__TAURI__, $=id=>document.getElementById(id);
let snapshot=null,revision=-1,busy=false,opening=0;
let historyEpoch=null,olderHistory=[],recentHistory=[],historyHasOlder=false,historyLoading=false,historyRequest=0;
const statusNames={streaming:"正在回复……",cancelled:"已停止",interrupted:"这次回复未完成",failed:"这次回复未完成",filtered:"已使用回退台词，本轮不作为后续上下文或记忆来源"};
function notice(text){$("notice").textContent=String(text||"");}
$("backup-chat").addEventListener("click",async()=>{
  const button=$("backup-chat");button.disabled=true;notice("正在备份交流与记忆……");
  try{const name=await TAU.core.invoke("backup_chat");notice(`已保存至“交流备份”文件夹：${name}`);if($("restore-chat").open)await refreshRestore();}
  catch(error){notice(String(error));}finally{button.disabled=false;}
});
let restoreBusy=false,restoreState=null,restoreSelection=null;
function restoreControls(){
  const pending=!!(restoreState?.pending||restoreState?.orphan),choice=$("restore-choice");
  choice.disabled=restoreBusy||!restoreState?.backups.length||pending||!$("restore-confirm").hidden;
  $("restore-select").disabled=restoreBusy||!restoreState||pending||!choice.value;
  $("restore-cancel").hidden=!pending;$("restore-cancel").disabled=restoreBusy;
  $("restore-confirm-yes").disabled=restoreBusy;$("restore-confirm-no").disabled=restoreBusy;
}
async function refreshRestore(){
  try{
    const previous=$("restore-choice").value;restoreState=await TAU.core.invoke("chat_restore_list");
    const choice=$("restore-choice");choice.replaceChildren();
    const placeholder=document.createElement("option");placeholder.value="";placeholder.textContent=restoreState.backups.length?"选择一份备份":"暂无备份";choice.append(placeholder);
    for(const backup of restoreState.backups){const option=document.createElement("option");option.value=backup.name;const date=backup.modified?new Date(backup.modified*1000).toLocaleString("zh-CN"):"时间未知";option.textContent=`${date} · ${(backup.bytes/1048576).toFixed(1)} MB · ${backup.name}`;choice.append(option);}
    if(restoreState.backups.some(backup=>backup.name===previous))choice.value=previous;
    $("restore-status").textContent=restoreState.pending?"已安排恢复，下次启动时执行。":restoreState.orphan?"有未完成的准备文件，请先取消。":restoreState.restored?`本次启动已恢复；恢复前文件保存在“交流备份”文件夹：${restoreState.restored}`:restoreState.backups.length?"请选择要恢复的备份。":"还没有交流备份。";
  }catch(error){restoreState=null;$("restore-status").textContent=String(error);}
  restoreControls();
}
$("restore-chat").addEventListener("toggle",()=>{if($("restore-chat").open)void refreshRestore();});
$("restore-choice").addEventListener("change",restoreControls);
$("restore-select").addEventListener("click",()=>{
  if(!restoreState||restoreState.pending||restoreState.orphan||!$("restore-choice").value)return;
  restoreSelection=$("restore-choice").value;
  $("restore-confirm-text").textContent=`下次启动时用“${restoreSelection}”替换当前交流和记忆？程序会先保留恢复前的数据库；备份后的新交流将不在恢复后的记录里。`;
  $("restore-confirm").hidden=false;restoreControls();
});
$("restore-confirm-no").addEventListener("click",()=>{$("restore-confirm").hidden=true;restoreSelection=null;restoreControls();});
$("restore-confirm-yes").addEventListener("click",async()=>{
  if(restoreBusy||!restoreSelection)return;restoreBusy=true;restoreControls();
  try{await TAU.core.invoke("chat_restore_schedule",{name:restoreSelection});$("restore-confirm").hidden=true;restoreSelection=null;notice("已安排恢复；退出并重新启动后执行。");}
  catch(error){notice(error);}finally{restoreBusy=false;await refreshRestore();}
});
$("restore-cancel").addEventListener("click",async()=>{
  if(restoreBusy)return;restoreBusy=true;restoreControls();
  try{await TAU.core.invoke("chat_restore_cancel");notice("已取消待恢复任务。");}
  catch(error){notice(error);}finally{restoreBusy=false;await refreshRestore();}
});
$("export-diagnostics").addEventListener("click",async()=>{
  const button=$("export-diagnostics");button.disabled=true;
  try{const name=await TAU.core.invoke("export_diagnostics");notice(`诊断摘要已保存到程序目录：${name}`);}
  catch(error){notice(String(error));}finally{button.disabled=false;}
});
function controls(){
  const unavailable=!snapshot||!!snapshot.error;
  for(const id of ["clear-history","confirm-clear"])$(id).disabled=busy||unavailable;
  $("stop-reply").disabled=busy||!snapshot?.active;$("retry-reply").disabled=busy||!snapshot?.retryable||!!snapshot?.active;
  $("stop-reply").hidden=!snapshot?.active;$("retry-reply").hidden=!snapshot?.retryable||!!snapshot?.active;
  $("reply-controls").hidden=!snapshot?.active&&!snapshot?.retryable;
  setConnectionsAvailable(!unavailable);
}
function historyItems(){
  const byId=new Map();for(const item of [...olderHistory,...recentHistory])byId.set(item.id,item);
  return [...byId.values()].sort((a,b)=>a.id-b.id);
}
async function loadOlder(){
  if(historyLoading||!historyHasOlder||!snapshot)return;
  const before=historyItems()[0]?.id,epoch=historyEpoch,request=++historyRequest;
  if(!before)return;
  historyLoading=true;const button=document.getElementById("history-older");if(button)button.disabled=true;
  let prepended=false;
  try{
    const page=await TAU.core.invoke("chat_history_page",{before,historyEpoch:epoch});
    if(historyEpoch!==epoch||historyRequest!==request)return;
    olderHistory=[...page.messages,...olderHistory];historyHasOlder=page.has_older;prepended=true;
  }catch(error){if(historyEpoch===epoch&&historyRequest===request)notice(error);}
  finally{if(historyRequest===request){historyLoading=false;if(historyEpoch===epoch&&snapshot?.view==="history")renderHistory(prepended);}}
}
function renderHistory(prepended=false){
  const root=$("messages"),oldTop=root.scrollTop,oldHeight=root.scrollHeight,bottom=oldHeight-oldTop-root.clientHeight<70;
  const expanded=new Set([...root.querySelectorAll(".message details[open]")].map(el=>el.closest(".message")?.dataset.id));
  const items=historyItems();root.replaceChildren();
  if(historyHasOlder){const button=document.createElement("button");button.id="history-older";button.type="button";button.textContent=historyLoading?"正在读取……":"加载更早的交流";button.disabled=historyLoading;button.addEventListener("click",loadOlder);root.append(button);}
  for(const item of items){
    const article=document.createElement("article");article.className="message";article.dataset.role=item.role;article.dataset.id=String(item.id);
    const header=document.createElement("header");const who=item.role==="user"?"你":item.role==="event"?"生活中的小事":item.source==="life"?"她的回应":"暗月";
    header.textContent=`${who} · ${new Date(item.created_at*1000).toLocaleString("zh-CN")}`;
    const text=document.createElement("p");text.textContent=snapshot.active?.message_id===item.id?snapshot.active.text:item.text;
    const status=document.createElement("small");status.textContent=item.error||statusNames[item.status]||"";
    article.append(header,text,status);
    if(item.attempts?.length){const details=document.createElement("details"),summary=document.createElement("summary");summary.textContent="使用的连接与尝试";details.className="attempts";details.open=expanded.has(String(item.id));details.append(summary);
      for(const attempt of item.attempts){const line=document.createElement("p");line.textContent=`${attempt.connection_name} · ${attempt.model||"未设置模型"} · ${attempt.status}${attempt.error?` · ${attempt.error}`:""}${attempt.usage?` · 用量 ${JSON.stringify(attempt.usage)}`:""}`;details.append(line);}article.append(details);}
    root.append(article);
  }
  if(!items.length){const p=document.createElement("p");p.className="empty";p.textContent="还没有交流记录。想说的话，写在她身边的输入气泡里。";root.append(p);}
  if(prepended)root.scrollTop=oldTop+root.scrollHeight-oldHeight;
  else if(bottom)root.scrollTop=root.scrollHeight;else root.scrollTop=oldTop;
  if(prepended)(document.getElementById("history-older")||root).focus({preventScroll:true});
}
function accept(next){
  if(!next||next.revision<revision)return;
  if(historyEpoch!==next.history_epoch||snapshot?.view==="history"&&next.view!=="history"){
    historyEpoch=next.history_epoch;olderHistory=[];recentHistory=[];historyHasOlder=false;historyLoading=false;historyRequest++;
  }
  if(next.view==="history"){
    const first=next.messages[0]?.id;
    if(first&&recentHistory.length&&recentHistory.at(-1).id<first){olderHistory=[];recentHistory=[];historyHasOlder=next.history_has_older;}
    if(first)olderHistory=[...olderHistory,...recentHistory.filter(item=>item.id<first)];
    if(!olderHistory.length)historyHasOlder=next.history_has_older;
    recentHistory=next.messages;
  }
  revision=next.revision;snapshot=next;
  $("settings").hidden=next.view!=="settings";$("messages").hidden=next.view!=="history";$("memory").hidden=next.view!=="memory";
  $("view-title").textContent=next.view==="memory"?"她的记忆":next.view==="settings"?"交流设置":"原始交流记录";
  document.querySelectorAll("[data-view]").forEach(button=>button.setAttribute("aria-selected",String(button.dataset.view===next.view)));
  const last=next.messages.at(-1),replyError=next.retryable?last?.error:"";
  $("route-status").textContent=replyError||next.routing_status||"";$("database-path").textContent=`本地记录：${next.database_path}`;
  if(next.error)notice(next.error);if(next.view==="history")renderHistory();controls();
}
async function operation(fn){if(busy)return;busy=true;controls();try{return await fn();}catch(error){notice(error);}finally{busy=false;controls();}}
document.querySelectorAll("[data-view]").forEach(button=>button.addEventListener("click",()=>TAU.core.invoke("chat_view",{view:button.dataset.view}).catch(notice)));
$("continue").addEventListener("click",()=>TAU.core.invoke("chat_view",{view:"input"}).catch(notice));
$("stop-reply").addEventListener("click",()=>operation(async()=>{await TAU.core.invoke("chat_stop");notice("已停止回复。");}));
$("retry-reply").addEventListener("click",()=>operation(async()=>{await TAU.core.invoke("chat_retry");notice("正在重新回复。");}));
$("clear-history").addEventListener("click",()=>{$("clear-confirm").hidden=false;});$("cancel-clear").addEventListener("click",()=>{$("clear-confirm").hidden=true;});
$("confirm-clear").addEventListener("click",()=>operation(async()=>{accept(await TAU.core.invoke("chat_conversation",{operation:"clear"}));await refreshMemory();$("clear-confirm").hidden=true;notice("交流、记忆和整理任务已清空。");}));
await TAU.event.listen("chat-changed",event=>accept(event.payload));await TAU.event.listen("chat-storage-error",event=>notice(event.payload));
controls();await mountExtension({onOpen:async()=>{const current=++opening;try{const next=await TAU.core.invoke("chat_state");if(current===opening){accept(next);await Promise.all([refreshMemory(),refreshConnections()]);}}catch(error){if(current===opening)notice(error);}},onClose:()=>{opening++;closeConnections();}});
