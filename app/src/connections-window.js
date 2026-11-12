// 所有连接配置只在设置页管理，完整Key不从Rust回传。
const TAU=window.__TAURI__, $=id=>document.getElementById(id);
let state=null,busy=false,available=true,editId=null,dirty=false,lifecycle=0,renderedKinds=false;
const fields=["connection-kind","connection-name","connection-url","connection-model","connection-primary","connection-fallback","api-key"];
function node(tag,text,className){const el=document.createElement(tag);if(text!==undefined)el.textContent=text;if(className)el.className=className;return el;}
function notice(text){$("notice").textContent=String(text||"");}
function controls(){
  for(const id of [...fields,"new-connection","save-connection","query-models","cancel-connection","auto-fallback","interactions"])$(id).disabled=busy||!state||!available;
  document.querySelectorAll("#connection-list button,#connection-list input,#model-list button").forEach(el=>el.disabled=busy||!available);
}
export function setConnectionsAvailable(value){available=value;controls();}
export function closeConnections(){lifecycle++;$("api-key").value="";dirty=false;editId=null;$("connection-editor").hidden=true;}
async function operation(fn){if(busy)return;busy=true;controls();try{return await fn(lifecycle);}catch(error){notice(error);await refreshConnections();}finally{busy=false;controls();}}
function button(text,fn){const b=node("button",text);b.type="button";b.addEventListener("click",fn);return b;}
function accept(next){
  if(!next||state&&next.revision<state.revision)return;state=next;
  if(!renderedKinds){for(const p of next.presets){const option=node("option",p.name);option.value=p.kind;$("connection-kind").append(option);}renderedKinds=true;}
  $("auto-fallback").checked=next.auto_fallback;$("interactions").checked=next.interactions;
  const primary=next.profiles.find(p=>p.id===next.selected);$("connection-badge").textContent=primary?`主连接：${primary.name}`:"尚无主连接";
  renderList();if(editId&&!dirty){const p=next.profiles.find(p=>p.id===editId);if(p)fill(p);else closeConnections();}controls();
}
export async function refreshConnections(){try{accept(await TAU.core.invoke("chat_connections"));}catch(error){notice(error);}}
function draft(){return {id:editId,name:$("connection-name").value,kind:$("connection-kind").value,base_url:$("connection-url").value,model:$("connection-model").value,fallback:$("connection-fallback").checked};}
function addressLayout(){
  const target=$("connection-kind").value==="custom"?$("connection-address-slot"):$("connection-advanced-address");
  if($("connection-url-field").parentElement!==target)target.append($("connection-url-field"));
}
function fill(profile){
  $("connection-kind").value=profile.kind;$("connection-name").value=profile.name;$("connection-url").value=profile.base_url;$("connection-model").value=profile.model;
  $("connection-fallback").checked=profile.fallback;$("connection-primary").checked=profile.id===state.selected;$("api-key").value="";
  $("connection-editor-title").textContent=profile.id?`编辑：${profile.name}`:"添加连接";addressLayout();renderModels(profile);
}
function edit(profile){editId=profile.id;dirty=false;fill(profile);$("connection-advanced").open=false;$("model-picker").open=false;$("connection-editor").hidden=false;$("connection-kind").focus();}
async function save(epoch){
  if(!$("connection-name").validity.valid||(!$("connection-url").validity.valid&&$("connection-kind").value!=="custom"))$("connection-advanced").open=true;
  if(!$("connection-editor").reportValidity())return null;
  const value=draft(),key=$("api-key").value;$("api-key").value="";
  const before=new Set(state.profiles.map(p=>p.id));
  const next=await TAU.core.invoke("chat_connection_save",{draft:value,key:key||null,primary:$("connection-primary").checked,revision:state.revision});
  const saved=next.profiles.find(p=>p.id===editId)||next.profiles.find(p=>!before.has(p.id));
  if(epoch!==lifecycle){accept(next);return null;}
  editId=saved?.id||null;dirty=false;accept(next);return saved;
}
function renderList(){
  const root=$("connection-list"),open=new Set([...root.querySelectorAll("details[open]")].map(el=>el.dataset.id));
  const focused=root.contains(document.activeElement)?document.activeElement.closest(".connection-card")?.dataset.id:null;root.replaceChildren();
  if(!state.profiles.length){root.append(node("p","添加一组API连接，就可以继续和她说话。","hint"));return;}
  state.profiles.forEach((p,index)=>{
    const card=node("details",undefined,"connection-card");card.dataset.id=p.id;card.open=open.has(p.id);
    const summary=node("summary",undefined,"connection-summary"),name=node("span",p.name,"connection-name"),model=node("span",p.model||"待选模型","connection-model");name.title=p.name;model.title=p.model;
    const status=!p.configured||!p.model?"待配置":p.id===state.selected?"主用":p.fallback?"备用":"";
    summary.append(name,model,node("span",status,"connection-role"),node("span","›","connection-chevron"));
    const info=node("div",undefined,"connection-info");info.append(node("p",`模型：${p.model||"尚未选择"}`,"connection-model-full"),node("p",p.base_url,"hint path"),node("p",p.configured?`Key：${p.key_hint||"已保存"}`:"尚未保存Key","key-hint"));card.append(summary,info);
    const actions=node("div",undefined,"actions");
    if(p.id!==state.selected)actions.append(button("设为主用",()=>action(p.id,"primary")));
    actions.append(button("编辑",()=>edit(p)));
    const label=node("label",undefined,"check fallback-check"),check=node("input");check.type="checkbox";check.checked=p.fallback;check.addEventListener("change",()=>action(p.id,"fallback"));label.append(check,document.createTextNode("可作备选"));actions.append(label);
    if(index>0)actions.append(button("↑",()=>action(p.id,"up")));if(index+1<state.profiles.length)actions.append(button("↓",()=>action(p.id,"down")));
    actions.append(button("移除",()=>{
      if(card.querySelector(".connection-confirm"))return;const box=node("div",undefined,"connection-confirm");box.append(node("p",`移除“${p.name}”及其加密Key？聊天与记忆仍保留。`),button("确认移除",()=>action(p.id,"delete")),button("保留",()=>box.remove()));card.append(box);
    }));info.append(actions);root.append(card);if(focused===p.id)summary.focus({preventScroll:true});
  });
}
function action(id,operationName){operation(async epoch=>{const next=await TAU.core.invoke("chat_connection_action",{id,operation:operationName,revision:state.revision});accept(next);if(epoch===lifecycle)notice("连接配置已保存。");});}
function renderModels(profile=state?.profiles.find(p=>p.id===editId)){
  const models=profile?.models||[],root=$("model-list");root.replaceChildren();$("model-picker").hidden=!models.length;if(!models.length)return;
  const query=$("model-filter").value.toLowerCase(),filtered=models.filter(model=>model.toLowerCase().includes(query));
  $("model-count").textContent=`${filtered.length} 个模型${filtered.length>200?"，请搜索缩小范围":" · 点击选用"}`;
  for(const model of filtered.slice(0,200)){const b=button(model,()=>operation(async epoch=>{$("connection-model").value=model;dirty=true;const saved=await save(epoch);if(saved)notice(`已选用并保存：${model}`);}));b.className="model-choice";b.title=model;b.setAttribute("aria-pressed",String(model===profile.model));root.append(b);}controls();
}
$("new-connection").addEventListener("click",()=>{editId=null;dirty=true;const p=state.presets[0];fill({...p,id:null,fallback:false});$("connection-primary").checked=true;$("connection-advanced").open=false;$("model-picker").open=false;$("connection-editor").hidden=false;$("connection-kind").focus();});
$("cancel-connection").addEventListener("click",()=>closeConnections());
$("connection-editor").addEventListener("submit",event=>{event.preventDefault();operation(async epoch=>{if(await save(epoch))notice("连接已保存，可查询模型或继续交流。");});});
$("query-models").addEventListener("click",()=>operation(async epoch=>{
  let profile=state.profiles.find(p=>p.id===editId);if(dirty||!profile)profile=await save(epoch);if(!profile||epoch!==lifecycle)return;
  notice("正在查询模型……");const next=await TAU.core.invoke("chat_models",{id:profile.id,revision:state.revision});accept(next);if(epoch===lifecycle){$("model-picker").open=true;notice("点击模型即可选用并保存。");}
}));
$("connection-kind").addEventListener("change",()=>{const p=state.presets.find(p=>p.kind===$("connection-kind").value);$("connection-name").value=p.name;$("connection-url").value=p.base_url;$("connection-model").value=p.model;$("api-key").value="";$("model-picker").hidden=true;addressLayout();dirty=true;});
for(const id of fields){$(id).addEventListener("input",()=>{dirty=true;if(["connection-url","api-key"].includes(id))$("model-picker").hidden=true;});}
$("model-filter").addEventListener("input",()=>renderModels());
for(const id of ["auto-fallback","interactions"]){$(id).addEventListener("change",()=>operation(async epoch=>{
  await TAU.core.invoke("chat_configure",{selected:state.selected,autoFallback:$("auto-fallback").checked,interactions:$("interactions").checked,revision:state.revision});await refreshConnections();if(epoch===lifecycle)notice("交流偏好已保存。");
}));}
await TAU.event.listen("chat-connections",event=>accept(event.payload));controls();
