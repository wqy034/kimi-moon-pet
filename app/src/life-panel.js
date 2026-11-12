// 生活面板只呈现快照、提交领域命令；不自行修改钱包或计算用量。
const TAU = window.__TAURI__;
const root = document.getElementById("life");
const body = document.getElementById("life-content");
const notice = document.getElementById("life-notice");
const labels = { food: "饱食", water: "水分", energy: "精力", affection: "好感" };
let snapshot = null, tab = "home", pending = false;
const opened = new Map(), scrolls = new Map();
let pendingFocus = null;
function el(tag, text, className = "") {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function button(text, action, item, disabled = false) {
  const node = el("button", text, "life-action");
  node.dataset.lifeAction = action;
  if (item) node.dataset.item = item;
  node.disabled = pending || !!snapshot?.error || disabled;
  return node;
}
function card(title, description) {
  const node = el("section", undefined, "life-card");
  node.append(el("h3", title));
  if (description) node.append(el("p", description, "hint"));
  return node;
}
function fold(key, title, content) {
  const node = el("details"); node.dataset.section = key; node.open = opened.get(key) || false;
  const summary = el("summary", title); summary.dataset.focusKey = key;
  node.append(summary, content); return node;
}
function stats(s) {
  const box = card("暗月的状态");
  for (const [key, label] of Object.entries(labels)) {
    const value = Math.max(0, Math.min(100, s.stats[key]));
    const row = el("div", undefined, "life-stat");
    row.append(el("span", label), el("span", `${Math.round(value)} / 100`, "life-number"));
    const track = el("div", undefined, "life-track");
    track.setAttribute("role", "meter"); track.setAttribute("aria-label", label);
    track.setAttribute("aria-valuemin", "0"); track.setAttribute("aria-valuemax", "100"); track.setAttribute("aria-valuenow", String(Math.round(value)));
    const fill = el("i"); fill.style.width = `${value}%`; fill.className = key;
    track.append(fill); box.append(row, track);
  }
  box.append(el("p", "慢慢消耗，睡觉恢复精力。离线最多结算八小时，不会生病或离开。", "hint"));
  return box;
}
function jump(text, target) {
  const node = el("button", text, "life-action"); node.dataset.jump = target;
  node.dataset.focusKey = `jump-${target}`; return node;
}
function remember() {
  for (const node of body.querySelectorAll("details[data-section]")) opened.set(node.dataset.section, node.open);
  scrolls.set(tab, body.scrollTop);
}
function render() {
  if (!snapshot || root.hidden) return;
  const active = document.activeElement;
  const ownedFocus = root.contains(active) || active === document.body;
  const focus = active?.dataset;
  const focusKey = focus?.lifeAction ? { action: focus.lifeAction, item: focus.item || "" } : null;
  const namedFocus = focus?.focusKey;
  if (body.childElementCount) remember();
  const s = snapshot, rules = s.rules;
  document.getElementById("life-wallet").textContent = `${s.coins.toLocaleString()} 枚金币`;
  document.querySelectorAll("[data-life-tab]").forEach(node => {
    const selected = node.dataset.lifeTab === tab;
    node.setAttribute("aria-selected", String(selected)); node.classList.toggle("selected", selected);
  });
  body.replaceChildren();
  if (s.error) body.append(el("p", s.error, "life-error"));
  if (s.warning) body.append(el("p", s.warning, "hint"));
  if (tab === "home" || tab === "shop") {
    if (tab === "home") {
      const summary = s.stats.food <= 35 ? "肚子有点空，想吃点东西。" : s.stats.water <= 35 ? "想喝点水，润润嗓子。" : s.stats.energy <= 30 ? "有点困，想歇一歇。" : "今天状态不错，有你在就更好啦。";
      body.append(stats(s), el("p", summary, "life-mood"));
      if (s.job) {
        const job = card(s.job.ready ? "书架整理好啦" : "她还有一份小工作", s.job.ready ? "赚到的金币等你来领取。" : `${s.activity === "job" ? "正在整理" : "等空闲时继续"} · 还需 ${Math.max(0, Math.ceil(s.job.rule.duration_seconds - s.job.progress))} 秒`);
        job.append(jump(s.job.ready ? "去领取" : "看看进度", "daily")); body.append(job);
      }
    } else body.append(el("p", "带回背包，想送给她时再拿出来。", "hint"));
    const items = rules.items.filter(item => tab === "shop" || (s.inventory[item.id] || 0) > 0);
    for (const item of items) {
      const count = s.inventory[item.id] || 0;
      const box = card(`${item.icon} ${item.name}`);
      box.classList.add("life-food");
      const effects = Object.entries(labels).filter(([key]) => item.effect[key] > 0).map(([key, label]) => `${label} +${item.effect[key]}`).join(" · ");
      box.title = effects;
      const row = el("div", undefined, "life-item-row");
      row.append(el("span", tab === "home" ? `还有 ${count} 份` : `${item.price} 金币 · 已有 ${count} 份`, "hint"));
      const unavailable = count >= 99 ? "背包已满，每种最多99份" : s.coins < item.price ? "金币还不够" : "";
      const action = button(tab === "home" ? "送给她" : "买一份", tab === "home" ? "use" : "buy", item.id, tab === "shop" && !!unavailable);
      action.setAttribute("aria-label", `${tab === "home" ? "送给她" : "买一份"}：${item.name}，${effects}`);
      if (tab === "shop" && unavailable) { action.title = unavailable; box.append(el("p", unavailable, "hint")); }
      row.append(action); box.append(row); body.append(box);
    }
    if (!items.length) body.append(card("食盒空空的", "去小铺带些吃的回来吧。"));
    if (tab === "home") { body.append(jump("去小铺看看", "shop")); }
  } else {
    const job = s.job, rule = job?.rule || rules.job;
    const work = card(`📚 ${rule.name}`, `约 ${Math.ceil(rule.duration_seconds)} 秒 · 消耗 ${rule.energy_cost} 精力 · 奖励 ${rule.reward} 金币`);
    if (job) {
      const remaining = Math.max(0, Math.ceil(rule.duration_seconds - job.progress));
      work.append(el("p", job.ready ? "整理完成啦，来领取她赚到的金币吧。" : `还需 ${remaining} 秒 · ${s.activity === "job" ? "正在整理" : "等待空闲时继续"}`, "life-job-state"));
      const progress = el("progress"); progress.max = rule.duration_seconds; progress.value = job.progress;
      progress.setAttribute("aria-label", "整理书架进度"); work.append(progress);
      work.append(job.ready ? button("领取奖励", "claim_job") : button("取消这次工作", "cancel_job"));
    } else {
      const reason = s.stats.energy < rule.minimum_energy ? `精力达到 ${rule.minimum_energy} 后再来吧。` : s.work_coins_today + rule.reward > rules.daily_work_coins_cap ? "今天的打工奖励已到上限，明天再来吧。" : "";
      work.append(button("请她整理书架", "start_job", null, !!reason));
      if (reason) work.append(el("p", reason, "hint"));
    }
    const instructions = el("div");
    instructions.append(el("p", `接取需要 ${rule.minimum_energy} 精力；今日打工 ${s.work_coins_today} / ${rules.daily_work_coins_cap} 金币。`, "hint"));
    instructions.append(el("p", "生活窗口开着也能继续工作；CLI忙碌、触摸、拎起、右键菜单和睡觉时暂停。离线不打工，取消不返还已用精力。", "hint"));
    work.append(fold("work", "工作说明", instructions)); body.append(work);
    const tokens = el("div");
    tokens.append(el("p", `每 ${rules.tokens_per_coin.toLocaleString()} 个实际 token = 1 枚金币（含缓存）。`, "hint"));
    tokens.append(el("p", `距下枚金币：${s.token_remainder.toLocaleString()} / ${rules.tokens_per_coin.toLocaleString()}`, "life-job-state"));
    tokens.append(el("p", `已识别 ${s.usage_records.toLocaleString()} 笔用量 · ${s.token_total.toLocaleString()} token；旧历史不补发，余数随存档保留。`, "hint"));
    body.append(fold("tokens", "一起工作的积蓄", tokens));
    const list = el("ul", undefined, "life-ledger");
    [...s.recent].reverse().slice(0, 10).forEach(tx => list.append(el("li", `${tx.reason}${tx.coins ? ` · ${tx.coins > 0 ? "+" : ""}${tx.coins} 金币` : ""}`)));
    if (!s.recent.length) list.append(el("li", "还没有记录。"));
    body.append(fold("ledger", "最近的金币与物品记录", list));
  }
  body.scrollTop = scrolls.get(tab) || 0;
  if (ownedFocus) {
    let target;
    const key = focusKey || (pendingFocus?.tab === tab ? pendingFocus : null);
    if (namedFocus) target = [...body.querySelectorAll("[data-focus-key]")].find(node => node.dataset.focusKey === namedFocus);
    else if (key) target = [...body.querySelectorAll("[data-life-action]")].find(node => node.dataset.lifeAction === key.action && (node.dataset.item || "") === key.item);
    if (!target && key && !pending) target = root.querySelector(`[data-life-tab="${tab}"]`);
    target?.focus({ preventScroll: true });
  }
}
function accept(value) {
  if (!value || snapshot && value.revision < snapshot.revision) return;
  snapshot = value; render();
}
root.addEventListener("click", async event => {
  const tabButton = event.target.closest("[data-life-tab],[data-jump]");
  if (tabButton) {
    remember(); tab = tabButton.dataset.lifeTab || tabButton.dataset.jump;
    // 不把上一页滚动位置覆盖到新页。
    body.replaceChildren(); pendingFocus = null; render(); return;
  }
  const target = event.target.closest("[data-life-action]");
  if (!target || pending || target.disabled) return;
  pendingFocus = { action: target.dataset.lifeAction, item: target.dataset.item || "", tab };
  pending = true; render(); notice.textContent = "正在保存……";
  try {
    accept(await TAU.core.invoke("life_action", { action: target.dataset.lifeAction, item: target.dataset.item || null }));
    notice.textContent = target.dataset.lifeAction === "buy" ? "已放进背包。" : "已收到，看看暗月的反应吧。";
  } catch (error) { notice.textContent = String(error); }
  finally { pending = false; render(); pendingFocus = null; }
});
export async function openLife() {
  notice.textContent = "";
  try { accept(await TAU.core.invoke("life_get")); }
  catch (error) { notice.textContent = String(error); }
}
TAU.event.listen("life-changed", event => accept(event.payload)).catch(console.error);
