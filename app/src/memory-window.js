// 长期记忆只在交流资料页展示；输入组件不接收来源正文。
const TAU=window.__TAURI__, $=id=>document.getElementById(id);
let state=null,busy=false,editing=null;
const statuses={queued:"等空闲时整理",running:"正在整理",failed:"整理失败，可在设置中重试",complete:"已整理",skipped:"来源已停用或旧版已跳过"};
function node(tag,text,className){const el=document.createElement(tag);if(text!==undefined)el.textContent=text;if(className)el.className=className;return el;}
function notice(text){$("notice").textContent=String(text||"");}
function controls(){
  for(const id of ["memory-enabled","memory-flush","memory-retry","memory-model"])$(id).disabled=busy||!state;
  if(state){$("memory-flush").disabled=busy||!state.enabled;$("memory-model").disabled=busy||!state.enabled;}
  document.querySelectorAll("#memory button").forEach(button=>button.disabled=busy);
}
function accept(next){
  if(!next||state&&next.revision<state.revision)return;state=next;
  if(editing&&!next.entries.some(e=>e.id===editing.id))editing=null;
  $("memory-enabled").checked=next.enabled;
  const facts=next.entries.filter(e=>e.kind==="fact"&&!e.quarantined&&!e.archived),summaries=next.entries.filter(e=>e.kind==="summary"&&!e.quarantined&&!e.archived),quarantine=next.entries.filter(e=>e.quarantined),held=next.entries.filter(e=>!e.quarantined&&(e.archived||["candidate","superseded"].includes(e.kind)));
  $("memory-held").hidden=!held.length;$("memory-held-count").textContent=String(held.length);
  $("memory-quarantine").hidden=!quarantine.length;$("memory-quarantine-count").textContent=String(quarantine.length);
  const text=`${next.enabled?"记忆已开启":"记忆已关闭"} · ${facts.length}项关于你的事实 · ${summaries.length}段往事 · ${next.waiting}条待整理来源。${next.model_status}${next.error?`。${next.error}`:""}`;
  $("memory-status").textContent=text;$("memory-settings-status").textContent=text;
  const failed=next.enabled&&next.jobs.find(job=>job.status==="failed");
  const error=next.error||(next.enabled&&next.model_status.startsWith("加载失败")?next.model_status:"")||(failed?`记忆整理失败：${failed.error||"可展开更多设置重试"}`:"");
  $("memory-settings-alert").textContent=error||"";$("memory-settings-alert").hidden=!error;
  if(!editing){renderEntries($("memory-facts"),facts,"还没有关于你的稳定事实，不会把一次闲聊直接当成你的习惯。");renderEntries($("memory-summaries"),summaries,"有值得记住的交流后，她会在空闲时整理。");renderEntries($("memory-quarantine-list"),quarantine,"");renderEntries($("memory-held-list"),held,"");}
  const jobs=$("memory-job-list");jobs.replaceChildren();
  for(const job of next.jobs){const line=node("p",`${statuses[job.status]||job.status}${job.provider?` · ${next.connection_names?.[job.provider]||job.provider}`:""} · ${new Date(job.created_at*1000).toLocaleString("zh-CN")}${job.error?` · ${job.error}`:""}${job.result?` · ${job.result}`:""}`,"hint");
    if(job.usage)line.append(node("span",` · 服务商用量：${JSON.stringify(job.usage)}`));jobs.append(line);}
  if(!next.jobs.length)jobs.append(node("p","还没有整理任务。","hint"));controls();
}
export async function refreshMemory(){try{accept(await TAU.core.invoke("memory_state"));}catch(error){notice(error);}}
async function update(operation,id=null,text=null,revision=state?.revision){
  if(busy||!state)return;busy=true;controls();
  try{const next=await TAU.core.invoke("memory_update",{operation,id,text,revision});editing=null;accept(next);
    notice(operation==="flush"?"已安排，等待聊天空闲后整理。":operation==="edit"?(next.entries.find(e=>e.id===id)?.quarantined?"修正已保存，内容仍待确认，暂不用于回忆。":"已修正，可用于回忆；旧引文不再作为修正证据，同源旧摘要已移除；其他待办保留。"):operation==="delete_original"?"所选来源原话、连接尝试正文和直接同源提炼项已删除；已有备份与服务商记录不受影响。":operation==="delete_entry"?"仅删除这条提炼项；来源和其他记忆保留，仍可能再次提炼。":operation==="delete"?"已停止使用所选来源并删除直接同源提炼项；原始交流仍保留。":operation==="disable"?"记忆已关闭，已撤销正在进行的整理和回复。":"已更新记忆设置。");
  }catch(error){notice(error);await refreshMemory();if(editing){editing.revision=state.revision;notice(`${error}；修正文字已保留，请再次保存以确认。`);}}finally{busy=false;controls();}
}
function button(text,action){const b=node("button",text);b.type="button";b.addEventListener("click",action);return b;}
function renderEntries(root,entries,empty){
  root.replaceChildren();if(!entries.length){root.append(node("p",empty,"empty"));return;}
  for(const item of entries){
    const card=node("article",undefined,"memory-entry");card.append(node("header",`${item.kind==="summary"?"往事":item.topic} · ${new Date(item.created_at*1000).toLocaleDateString("zh-CN")}${item.quarantined?" · 暂不用于回忆":item.edited?" · 你已修正":" · 自动提炼，可能有误"}`));
    if(item.archive_reason)card.append(node("p",item.archive_reason,"hint"));card.append(node("p",item.text));const actions=node("div",undefined,"actions");
    const sourceBox=node("div",undefined,"memory-source");sourceBox.hidden=true;
    actions.append(button(`来源 ${item.source_ids.length}`,async()=>{
      if(!sourceBox.hidden){sourceBox.hidden=true;return;}sourceBox.hidden=false;sourceBox.replaceChildren(node("p","正在读取来源……","hint"));
      try{const sources=await TAU.core.invoke("memory_sources",{id:item.id});sourceBox.replaceChildren();
        if(item.edited)sourceBox.append(node("p","当前正文由你修正；旧引文已解除，下面仅保留最初来源供查看。","hint"));
        for(const source of sources){const details=node("details");details.append(node("summary",`${source.kind==="life"?"程序确认的生活事件":"完整交流"} · ${new Date(source.created_at*1000).toLocaleString("zh-CN")}`));details.append(node("p",source.user_text));if(source.reply)details.append(node("p",`暗月：${source.reply}`));sourceBox.append(details);}
      }catch(error){sourceBox.replaceChildren(node("p",String(error),"hint"));}
    }));
    actions.append(button("修正",()=>{
      if(editing){notice("请先保存或取消当前修正。");return;}editing={id:item.id,revision:state.revision};
      const box=node("div",undefined,"memory-editor"),input=node("textarea");input.value=item.text;input.maxLength=item.kind==="summary"?500:50;input.rows=item.kind==="summary"?5:2;input.setAttribute("aria-label","修正这条记忆");
      if(!item.quarantined)box.append(node("p","保存后以此为准；此来源不再重新整理，同源旧摘要移除，其他待办保留。","hint"));
      box.append(input,button("保存修正",()=>update("edit",item.id,input.value,editing.revision)),button("取消",()=>{editing=null;accept(state);}));card.append(box);input.focus();
    }));
    actions.append(button("删除范围…",()=>{
      const confirm=node("div",undefined,"memory-confirm");
      const related=state.entries.filter(e=>e.id===item.id||e.source_ids.some(id=>item.source_ids.includes(id))).length;
      confirm.append(node("p",`这条记忆关联${item.source_ids.length}条完整来源，直接同源共${related}条提炼项。请选择范围：`));
      confirm.append(node("p","仅删此项：其他提炼项和来源保留，内容仍可能再次提炼。"),button("仅删这条提炼项",()=>update("delete_entry",item.id)));
      confirm.append(node("p",item.quarantined?"删除待确认项：仅删除此隔离项，不影响正常同源记忆。":"删除同源记忆，停止整理整条来源；原始交流保留，近期仍可能提及。新交流可重新记住。"),button(item.quarantined?"删除待确认项":"停止使用来源并移除同源记忆",()=>update("delete",item.id)));
      const original=node("details");original.append(node("summary","同时删除来源原话"));
      original.append(node("p","删除关联的完整问答、尝试正文和同源记忆。已有备份、服务商记录及未关联的相似内容不受影响；生活存档不变。"));
      original.append(button("确认删除整条来源原话与同源提炼项",()=>update("delete_original",item.id)));
      confirm.append(original,button("取消",()=>confirm.remove()));card.append(confirm);
    }));card.append(actions,sourceBox);root.append(card);
  }
}
$("memory-enabled").addEventListener("change",()=>update($("memory-enabled").checked?"enable":"disable"));
$("memory-flush").addEventListener("click",()=>update("flush"));$("memory-retry").addEventListener("click",()=>update("retry"));$("memory-model").addEventListener("click",()=>update("model"));
await TAU.event.listen("memory-changed",event=>accept(event.payload));
await TAU.event.listen("memory-model-changed",()=>refreshMemory());
await TAU.event.listen("memory-error",event=>notice(event.payload));
controls();
