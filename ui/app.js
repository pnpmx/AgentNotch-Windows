import table from "./i18n.json" with { type: "json" };
import * as fmt from "./format.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const $ = (id) => document.getElementById(id);
const now = () => Math.floor(Date.now() / 1000);
const NATIVE_NAMES = { en: "English", es: "Español", it: "Italiano", fr: "Français", de: "Deutsch", pt: "Português" };
const LIMIT_ALERT_MS = 12_000;

const state = {
  settings: null,
  expanded: false,
  edge: "right",
  usage: {},
  speech: { state: "idle" },
  transcript: "",
  history: [],
  events: [],
  limitAlerts: [],
  agents: null,
  notice: null,
  language: "en",
  t: fmt.translator(table, "en"),
};

// ---------- Language ----------

function applyLanguage() {
  const language = fmt.resolveLanguage(state.settings.uiLanguage, navigator.language);
  state.language = language;
  state.t = fmt.translator(table, language);
  document.documentElement.lang = language;
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = state.t(el.dataset.i18n);
  for (const el of document.querySelectorAll("[data-i18n-title]")) el.title = state.t(el.dataset.i18nTitle);
  return language;
}

async function updateSettings(patch) {
  const next = { ...patch };
  if ("uiLanguage" in patch) {
    next.resolvedLanguage = fmt.resolveLanguage(patch.uiLanguage, navigator.language);
  }
  state.settings = await invoke("update_settings", { patch: next });
  applyLanguage();
  render();
}

// ---------- Rendering helpers ----------

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function findWindow(id) {
  for (const snapshot of [state.usage.codex, state.usage.claude]) {
    const window = snapshot?.windows?.find((w) => w.id === id);
    if (window) return window;
  }
  return undefined;
}

function renderChip(source, snapshot) {
  const chip = document.querySelector(`.tab [data-source="${source}"]`);
  const primary = snapshot?.windows?.[0];
  const stale = fmt.isStale(snapshot, now());
  chip.classList.toggle("stale", Boolean(snapshot) && stale);
  const progress = !snapshot || stale ? 0 : Math.min(1, Math.max(0, (primary?.usedPercent ?? 0) / 100));
  chip.querySelector(".value").style.strokeDashoffset = String(88 * (1 - progress));
  chip.querySelector(".chip-value").textContent = snapshot && stale ? "…" : primary ? fmt.percent(primary.usedPercent) : "--";
  chip.title = `${source === "codex" ? "Codex" : "Claude"} ${chip.querySelector(".chip-value").textContent}`;
}

function renderColumn(source, snapshot, error) {
  const body = document.querySelector(`.column[data-source="${source}"] .body`);
  const t = state.t;
  body.replaceChildren();
  if (!snapshot) {
    body.append(el("p", "error", error ? t(error) : t("loading")));
    return;
  }
  const age = el("div", "age", fmt.ageLabel(snapshot, now(), t));
  age.classList.toggle("stale", fmt.isStale(snapshot, now()));
  body.append(age);
  for (const window of snapshot.windows.slice(0, 2)) {
    const limit = el("div", "limit");
    const row = el("div", "row");
    row.append(el("span", null, fmt.windowLabel(window, t)), el("span", null, fmt.percent(window.usedPercent)));
    const bar = el("div", "bar");
    const fill = el("div");
    fill.style.width = `${Math.min(100, Math.max(0, window.usedPercent))}%`;
    bar.append(fill);
    limit.append(row, bar, el("div", "reset", fmt.resetDescription(window.resetsAt, now(), t)));
    const pace = fmt.paceLabel(state.usage.projections?.[window.id], now(), state.language, t);
    if (pace) limit.append(el("div", "pace", pace));
    body.append(limit);
  }
  if (error) body.append(el("p", "error warn", t(error)));
}

function speechLabel() {
  const { speech, settings, t } = state;
  switch (speech.state) {
    case "listening": return t("speech.listening");
    case "transcribing": return t("speech.transcribing");
    case "downloading": return t("speech.downloading", { percent: speech.percent ?? 0 });
    case "failed": return `${t("speech.failed")}: ${t(speech.key)}`;
    default: return t("speech.idle", { hotkey: fmt.hotkeyLabel(settings.hotkey) });
  }
}

function renderSegmented(container, options, current, onPick) {
  container.replaceChildren(
    ...options.map(([value, label]) => {
      const button = el("button", null, label);
      button.type = "button";
      button.setAttribute("aria-pressed", String(value === current));
      button.addEventListener("click", () => onPick(value));
      return button;
    }),
  );
}

function renderAlerts() {
  const { t } = state;
  const list = $("alerts");
  const cards = [];
  for (const alert of state.limitAlerts) {
    const card = el("div", `alert limit ${alert.kind}`);
    card.append(el("strong", null, fmt.limitAlertText(alert, t, findWindow(alert.windowId))));
    cards.push(card);
  }
  for (const event of state.events.slice(-3).reverse()) {
    const card = el("div", `alert agent ${event.source} ${event.kind}`);
    const text = el("div", "alert-text");
    const title = el("strong", null, fmt.eventTitle(event, t));
    if (event.project) title.append(el("span", "project", ` · ${event.project}`));
    text.append(title);
    if (event.message) text.append(el("p", null, event.message));
    const dismiss = el("button", "icon", "✕");
    dismiss.type = "button";
    dismiss.title = t("agents.dismiss");
    dismiss.addEventListener("click", () => dismissEvents(event.at));
    card.append(text, dismiss);
    cards.push(card);
  }
  list.replaceChildren(...cards);
  list.hidden = cards.length === 0;
}

function renderAttention() {
  const attention = $("attention");
  const latest = state.events.at(-1);
  const kind = latest ? latest.source : state.limitAlerts.length ? "limit" : null;
  attention.hidden = !kind;
  attention.className = `attention ${kind ?? ""} ${latest?.kind === "permission" ? "urgent" : ""}`;
}

function renderAgentSettings() {
  const agents = state.agents;
  if (!agents) return;
  const { t } = state;
  for (const name of ["claude", "codex"]) {
    const config = agents[name];
    const models = config.models.map((m) => [m.id, m.id === "" ? t("settings.default") : m.name]);
    if (config.model && !models.some(([id]) => id === config.model)) models.push([config.model, config.model]);
    renderSegmented($(`${name}-model`), models, config.model, (model) => setAgentDefaults(name, { model }));
    const selected = config.models.find((m) => m.id === config.model) ?? config.models[0];
    const efforts = [["", t("settings.default")], ...(selected?.efforts ?? []).map((e) => [e, t(`effort.${e}`)])];
    renderSegmented($(`${name}-effort`), efforts, config.effort, (effort) => setAgentDefaults(name, { effort }));
  }
  $("connect-agents").hidden = agents.claude.hooksInstalled && agents.codex.hooksInstalled;
}

function render() {
  const { usage, settings, t } = state;
  $("root").className = `edge-${state.edge} ${state.expanded ? "expanded" : "collapsed"}`;
  $("panel").hidden = !state.expanded;

  renderChip("codex", usage.codex);
  renderChip("claude", usage.claude);
  for (const mic of [$("mic"), $("mic-large")]) mic.dataset.state = state.speech.state;
  $("mic").title = speechLabel();
  renderAttention();

  if (!state.expanded) return;
  $("speech-label").textContent = speechLabel();
  renderAlerts();
  renderColumn("codex", usage.codex, usage.codexError);
  renderColumn("claude", usage.claude, usage.claudeError);
  $("session").textContent = fmt.sessionLine(usage.session, t);
  $("connect").hidden = Boolean(usage.claudeConnected);
  $("transcript-box").hidden = !state.transcript;
  $("transcript").textContent = state.transcript;

  const history = state.history.slice(1); // The newest is shown above.
  $("history-box").hidden = history.length === 0;
  $("history").replaceChildren(
    ...history.map((text, i) => {
      const item = el("li");
      const copy = el("button", "linklike", text);
      copy.type = "button";
      copy.title = t("copy");
      copy.addEventListener("click", async () => {
        if (await invoke("copy_transcript", { index: i + 1 })) { state.notice = "notice.copied"; render(); }
      });
      item.append(copy);
      return item;
    }),
  );
  $("notice").textContent = state.notice ? t(state.notice) : "";

  $("limit-alerts").checked = settings.limitAlerts;
  $("agent-alerts").checked = settings.agentAlerts;
  $("auto-enter").checked = settings.autoEnter;
  $("remove-fillers").checked = settings.removeFillers;
  if (document.activeElement !== $("vocabulary")) $("vocabulary").value = settings.vocabulary;

  const languages = fmt.LANGUAGES.map((code) => [code, NATIVE_NAMES[code]]);
  renderSegmented($("ui-language"), [["system", t("settings.system")], ...languages], settings.uiLanguage,
    (value) => updateSettings({ uiLanguage: value }));
  renderSegmented($("dictation-language"), [["auto", t("settings.auto")], ...languages], settings.dictationLanguage,
    (value) => updateSettings({ dictationLanguage: value }));
  renderSegmented($("model"), [["base", t("model.base")], ["small", t("model.small")]], settings.model,
    (value) => updateSettings({ model: value }));
  renderAgentSettings();
}

// ---------- Actions ----------

async function setExpanded(expanded) {
  state.expanded = expanded;
  if (!expanded) setSettingsOpen(false);
  await invoke("set_expanded", { expanded });
  if (expanded) refreshAgents();
  render();
}

/// The widget never takes keyboard focus, except while settings are open so
/// the vocabulary can be typed.
function setSettingsOpen(open) {
  $("settings").hidden = !open;
  invoke("set_interactive", { interactive: open });
}

async function refreshAgents() {
  state.agents = await invoke("get_agents");
  render();
}

async function setAgentDefaults(agent, change) {
  try {
    state.agents = await invoke("set_agent_defaults", { agent, model: change.model ?? null, effort: change.effort ?? null });
  } catch (key) {
    state.notice = String(key);
  }
  render();
}

async function dismissEvents(until) {
  await invoke("dismiss_events", { until });
  state.events = state.events.filter((e) => e.at > until);
  render();
}

function showLimitAlert(alert) {
  state.limitAlerts = [...state.limitAlerts.filter((a) => a.windowId !== alert.windowId), alert];
  render();
  setTimeout(() => {
    state.limitAlerts = state.limitAlerts.filter((a) => a !== alert);
    render();
  }, LIMIT_ALERT_MS);
}

/// Click toggles the panel; dragging more than a few pixels moves the widget,
/// and the backend snaps it to the nearest screen edge when released.
function makeDraggable(element, onClick) {
  element.addEventListener("mousedown", (down) => {
    if (down.button !== 0 || down.target.closest("button, input, label, summary")) return;
    let dragging = false;
    const move = (event) => {
      if (!dragging && Math.hypot(event.screenX - down.screenX, event.screenY - down.screenY) > 4) {
        dragging = true;
        cleanup();
        getCurrentWindow().startDragging();
      }
    };
    const up = () => {
      cleanup();
      if (!dragging && onClick) onClick();
    };
    const cleanup = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
}

async function main() {
  const snapshot = await invoke("get_snapshot");
  Object.assign(state, {
    settings: snapshot.settings,
    expanded: snapshot.expanded,
    edge: snapshot.settings.edge,
    usage: snapshot.usage,
    speech: snapshot.speech,
    transcript: snapshot.transcript,
    history: snapshot.history,
    events: snapshot.events,
  });
  // Tell the backend which language "system" resolved to, for the tray menu.
  await updateSettings({ uiLanguage: state.settings.uiLanguage });

  makeDraggable($("tab"), () => setExpanded(true));
  makeDraggable($("panel-header"));
  $("close").addEventListener("click", () => setExpanded(false));
  $("settings-toggle").addEventListener("click", () => setSettingsOpen($("settings").hidden));
  $("refresh").addEventListener("click", () => { invoke("refresh_usage"); refreshAgents(); });
  $("copy").addEventListener("click", async () => {
    if (await invoke("copy_transcript", { index: null })) { state.notice = "notice.copied"; render(); }
  });
  $("connect").addEventListener("click", async () => {
    try { await invoke("connect_claude"); } catch (key) { state.notice = String(key); render(); }
  });
  $("connect-agents").addEventListener("click", async () => {
    const errors = await invoke("connect_agent_alerts");
    state.notice = errors.length ? errors[0][1] : "agents.connected";
    await refreshAgents();
  });
  for (const [id, key] of [["limit-alerts", "limitAlerts"], ["agent-alerts", "agentAlerts"],
    ["auto-enter", "autoEnter"], ["remove-fillers", "removeFillers"]]) {
    $(id).addEventListener("change", (event) => updateSettings({ [key]: event.target.checked }));
  }
  $("vocabulary").addEventListener("change", (event) => updateSettings({ vocabulary: event.target.value }));
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") setExpanded(false);
  });

  await listen("usage", ({ payload }) => { state.usage = payload; render(); });
  await listen("dock", ({ payload }) => { state.edge = payload.edge; state.expanded = payload.expanded; render(); });
  await listen("notice", ({ payload }) => { state.notice = payload.key; render(); });
  await listen("agent-events", ({ payload }) => { state.events = payload; render(); });
  await listen("limit-alert", ({ payload }) => showLimitAlert(payload));
  await listen("speech", ({ payload }) => {
    if (payload.kind === "state") state.speech = payload;
    if (payload.kind === "transcript") {
      state.transcript = payload.text;
      state.history = [payload.text, ...state.history].slice(0, 8);
    }
    if (payload.kind === "notice") state.notice = payload.key;
    render();
  });
  setInterval(render, 30_000); // Keep ages, countdowns and paces current.
  render();
}

main();
