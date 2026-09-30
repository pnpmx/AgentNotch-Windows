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
  sessions: [],
  presence: { claude: 0, codex: 0 },
  openSession: null,
  tab: "limits",
  showAllSessions: false,
  /// Agent events already seen in the panel (by timestamp).
  readEvents: new Set(),
  dragging: false,
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
    if (window.usedPercent >= 90 && window.resetsAt > now()) {
      limit.append(el("div", "back-in", t("limit.backIn", { time: fmt.countdown(window.resetsAt, now()) })));
    }
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
  const cards = state.limitAlerts.map((alert) => {
    const card = el("div", `alert limit ${alert.kind}`);
    card.append(el("strong", null, fmt.limitAlertText(alert, t, findWindow(alert.windowId))));
    return card;
  });
  $("alerts").replaceChildren(...cards);
  $("alerts").hidden = cards.length === 0;
}

function unreadEvents() {
  return state.events.filter((e) => e.sessionId && !state.readEvents.has(e.at));
}

function unreadSessionIds() {
  return new Set(unreadEvents().map((e) => e.sessionId).filter(Boolean));
}

/// Opening a session marks its alerts as read; once nothing is unread the
/// backend forgets them too.
function markRead(session) {
  for (const e of state.events) if (e.sessionId === session.id) state.readEvents.add(e.at);
  if (unreadEvents().length === 0 && state.events.length) {
    invoke("dismiss_events", { until: Math.max(...state.events.map((e) => e.at)) });
  }
}

function sessionLine(session) {
  const { t } = state;
  let line = t(`state.${session.state}`);
  if (session.state === "working" && session.activity) line = fmt.activityText(session.activity, t);
  if (session.state === "working" && session.startedAt) line += ` · ${fmt.duration((Date.now() - session.startedAt) / 1000, t)}`;
  if (session.state === "done") {
    const summary = fmt.taskSummary(session.lastTask, t);
    if (summary) line += ` · ${summary}`;
  }
  return line;
}

function sessionRow(session, unread) {
  const { t } = state;
  const open = state.openSession === session.id;
  const row = el("div", `session-row ${session.source} ${session.state}${open ? " open" : ""}${unread ? " unread" : ""}`);
  const head = el("button", "session-head");
  head.type = "button";
  head.append(
    el("span", "dot"),
    el("span", "session-title", session.project || fmt.agentName(session.source)),
    el("span", "session-line", sessionLine(session)),
  );
  if (unread) head.append(el("span", "unread-dot"));
  head.append(el("span", "chevron", open ? "▴" : "▾"));
  head.addEventListener("click", () => {
    state.openSession = open ? null : session.id;
    markRead(session);
    render();
  });
  row.append(head);

  if (open) {
    const details = el("div", "session-details");
    details.append(el("p", "session-meta", [fmt.agentName(session.source), session.model].filter(Boolean).join(" · ")));
    if (session.lastMessage) details.append(el("p", "response", fmt.plainText(session.lastMessage)));
    const buttons = el("div", "session-buttons");
    if (session.lastMessage) {
      const copy = el("button", null, t("response.copy"));
      copy.type = "button";
      copy.addEventListener("click", async () => {
        await navigator.clipboard.writeText(session.lastMessage);
        state.notice = "notice.copied";
        render();
      });
      buttons.append(copy);
    }
    const other = session.source === "codex" ? "claude" : "codex";
    const handoff = el("button", null, t(other === "codex" ? "handoff.toCodex" : "handoff.toClaude"));
    handoff.type = "button";
    handoff.addEventListener("click", async () => {
      await navigator.clipboard.writeText(fmt.handoffPrompt(session, t));
      state.notice = { text: t("handoff.copied", { agent: fmt.agentName(other) }) };
      render();
    });
    buttons.append(handoff);
    details.append(buttons);
    row.append(details);
  }
  return row;
}

function renderSessions() {
  const { t } = state;
  const unread = unreadSessionIds();
  const ordered = fmt.orderSessions(state.sessions, unread, Date.now());
  const shown = state.showAllSessions ? ordered : ordered.slice(0, 4);
  $("sessions-empty").hidden = ordered.length > 0;
  $("sessions").replaceChildren(...shown.map((s) => sessionRow(s, unread.has(s.id))));
  $("sessions-more").hidden = ordered.length <= 4;
  $("sessions-more").textContent = state.showAllSessions ? t("sessions.showLess") : t("sessions.showAll", { n: ordered.length });
}

function renderTabs() {
  for (const button of document.querySelectorAll("[data-tab]")) {
    const selected = button.dataset.tab === state.tab;
    button.setAttribute("aria-selected", String(selected));
    $(`tab-${button.dataset.tab}`).hidden = !selected;
  }
  const count = unreadEvents().length;
  $("unread-badge").hidden = count === 0;
  $("unread-badge").textContent = String(count);
}

function renderVoice() {
  const { t } = state;
  $("voice-empty").hidden = state.history.length > 0;
  $("history").replaceChildren(
    ...state.history.map((text, i) => {
      const item = el("li", i === 0 ? "latest" : null);
      const copy = el("button", "linklike", text);
      copy.type = "button";
      copy.title = t("copy");
      copy.addEventListener("click", async () => {
        if (await invoke("copy_transcript", { index: i })) { state.notice = "notice.copied"; render(); }
      });
      item.append(copy);
      return item;
    }),
  );
}

/// One comet orbits the tab per live agent type: orange Claude, mint Codex.
/// Faster while that agent works, pulsing while one waits.
function renderOrbit() {
  const svg = $("orbit");
  const tab = $("tab");
  const w = tab.clientWidth;
  const h = tab.clientHeight;
  const recent = Date.now() - 30 * 60 * 1000;
  const comets = ["claude", "codex"]
    .map((source) => {
      const mine = state.sessions.filter((s) => s.source === source);
      const working = mine.some((s) => s.state === "working" && s.updatedAt > recent);
      const waiting = mine.some((s) => s.state === "waiting");
      return { source, working, waiting, alive: state.presence[source] > 0 || working };
    })
    .filter((c) => c.alive);
  const key = comets.map((c) => `${c.source}${c.working}${c.waiting}`).join("|") + `${w}x${h}`;
  if (svg.dataset.key === key) return; // Keep animations running smoothly.
  svg.dataset.key = key;
  svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
  svg.replaceChildren(
    ...comets.map((comet, i) => {
      const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      rect.setAttribute("x", "1.5");
      rect.setAttribute("y", "1.5");
      rect.setAttribute("width", String(Math.max(0, w - 3)));
      rect.setAttribute("height", String(Math.max(0, h - 3)));
      rect.setAttribute("rx", "16");
      rect.setAttribute("pathLength", "100");
      rect.setAttribute("class", `${comet.source}${comet.working ? " working" : ""}${comet.waiting ? " waiting" : ""}`);
      const period = comet.working ? 3.2 : 7;
      rect.style.animationDelay = `${(-period * i) / comets.length}s`;
      return rect;
    }),
  );
}

function renderAttention() {
  $("tab").dataset.state = fmt.overallState(state.sessions, Date.now());
  const attention = $("attention");
  const latest = unreadEvents().at(-1);
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
  $("setup").hidden = $("connect").hidden && $("connect-agents").hidden;
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
  renderOrbit();

  if (!state.expanded) return;
  $("speech-label").textContent = speechLabel();
  renderAlerts();
  $("main-view").hidden = !$("settings").hidden;
  renderTabs();
  renderSessions();
  renderColumn("codex", usage.codex, usage.codexError);
  renderColumn("claude", usage.claude, usage.claudeError);
  $("session").textContent = fmt.sessionLine(usage.session, t);
  renderVoice();
  $("connect").hidden = Boolean(usage.claudeConnected);
  $("setup").hidden = $("connect").hidden && $("connect-agents").hidden;
  // A notice is either an i18n key or { text } for already formatted text.
  const notice = state.notice;
  $("notice").textContent = !notice ? "" : typeof notice === "object" ? notice.text : t(notice);

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
  // Open where there is something to see.
  if (expanded) {
    state.tab = fmt.orderSessions(state.sessions, unreadSessionIds(), Date.now()).length ? "sessions" : "limits";
  }
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
  render();
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

function showLimitAlert(alert) {
  state.limitAlerts = [...state.limitAlerts.filter((a) => a.windowId !== alert.windowId), alert];
  render();
  setTimeout(() => {
    state.limitAlerts = state.limitAlerts.filter((a) => a !== alert);
    render();
  }, LIMIT_ALERT_MS);
}

// ---------- Wrapped ----------

const WEEKDAY = (day, language) =>
  new Intl.DateTimeFormat(language, { weekday: "long" }).format(new Date(Date.UTC(2024, 0, day))); // 2024-01-01 is a Monday

/// Draws the weekly summary as a 1080×1350 image for sharing.
function drawWrapped(week) {
  const { t, language } = state;
  const canvas = document.createElement("canvas");
  canvas.width = 1080;
  canvas.height = 1350;
  const c = canvas.getContext("2d");
  const gradient = c.createLinearGradient(0, 0, 1080, 1350);
  gradient.addColorStop(0, "#11151d");
  gradient.addColorStop(0.6, "#1b2432");
  gradient.addColorStop(1, "#3a2518");
  c.fillStyle = gradient;
  c.fillRect(0, 0, 1080, 1350);
  // Notch mark
  c.fillStyle = "#000";
  c.beginPath();
  c.roundRect(390, 0, 300, 70, [0, 0, 30, 30]);
  c.fill();
  c.lineWidth = 7;
  c.strokeStyle = "#3ee6b5";
  c.beginPath(); c.arc(450, 35, 14, 0, Math.PI * 2); c.stroke();
  c.strokeStyle = "#ff8a3d";
  c.beginPath(); c.arc(630, 35, 14, 0, Math.PI * 2); c.stroke();

  const font = (weight, size) => `${weight} ${size}px "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif`;
  c.fillStyle = "#f4f5f7";
  c.font = font(700, 64);
  c.fillText(t("wrapped.title"), 90, 200);
  c.fillStyle = "#9aa0ab";
  c.font = font(400, 32);
  c.fillText(`${week.from} → ${week.to}`, 90, 252);

  const tasks = week.claudeTasks + week.codexTasks;
  const stats = [
    [String(tasks), t("wrapped.tasks"), "#f4f5f7"],
    [`+${week.linesAdded.toLocaleString(language)}`, t("wrapped.lines"), "#3ee6b5"],
    [`$${week.costUsd.toFixed(2)}`, t("wrapped.spent"), "#ff8a3d"],
    [String(week.busyHours), t("wrapped.hours"), "#f4f5f7"],
  ];
  stats.forEach(([value, label, color], i) => {
    const x = 90 + (i % 2) * 470;
    const y = 400 + Math.floor(i / 2) * 230;
    c.fillStyle = color;
    c.font = font(700, 110);
    c.fillText(value, x, y);
    c.fillStyle = "#9aa0ab";
    c.font = font(400, 34);
    c.fillText(label, x, y + 55);
  });

  // Tasks per day
  const max = Math.max(1, ...week.dailyTasks);
  week.dailyTasks.forEach((n, i) => {
    const h = Math.round((n / max) * 190);
    c.fillStyle = n === max && n > 0 ? "#3ee6b5" : "#ffffff33";
    c.beginPath();
    c.roundRect(90 + i * 132, 1020 - h, 96, Math.max(h, 6), 14);
    c.fill();
  });

  const facts = [
    week.topModel && [t("wrapped.topModel"), week.topModel],
    week.topProject && [t("wrapped.topProject"), week.topProject],
    week.busiestWeekday && [t("wrapped.busiest"), WEEKDAY(week.busiestWeekday, language)],
  ].filter(Boolean);
  facts.forEach(([label, value], i) => {
    c.fillStyle = "#9aa0ab";
    c.font = font(400, 30);
    c.fillText(label, 90, 1110 + i * 52);
    c.fillStyle = "#f4f5f7";
    c.font = font(600, 30);
    c.fillText(value, 420, 1110 + i * 52);
  });
  c.fillStyle = "#6b7280";
  c.font = font(400, 26);
  c.fillText(`${t("wrapped.footer")} · agentnotch.vercel.app`, 90, 1300);
  return canvas.toDataURL("image/png");
}

async function openWrapped() {
  const week = await invoke("get_week_summary");
  const empty = week.claudeTasks + week.codexTasks === 0;
  $("wrapped").hidden = false;
  $("wrapped-show").hidden = true;
  $("wrapped-status").textContent = empty ? state.t("wrapped.empty") : "";
  $("wrapped-save").hidden = empty;
  $("wrapped-image").hidden = empty;
  if (!empty) $("wrapped-image").src = drawWrapped(week);
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
  state.sessions = await invoke("get_sessions");
  // Tell the backend which language "system" resolved to, for the tray menu.
  await updateSettings({ uiLanguage: state.settings.uiLanguage });

  makeDraggable($("tab"), () => setExpanded(true));
  makeDraggable($("panel-header"));
  $("close").addEventListener("click", () => setExpanded(false));
  $("settings-toggle").addEventListener("click", () => setSettingsOpen($("settings").hidden));
  $("refresh").addEventListener("click", () => { invoke("refresh_usage"); refreshAgents(); });
  for (const button of document.querySelectorAll("[data-tab]")) {
    button.addEventListener("click", () => { state.tab = button.dataset.tab; render(); });
  }
  $("sessions-more").addEventListener("click", () => { state.showAllSessions = !state.showAllSessions; render(); });
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
  $("wrapped-open").addEventListener("click", openWrapped);
  $("wrapped-close").addEventListener("click", () => { $("wrapped").hidden = true; });
  let savedPath = null;
  $("wrapped-save").addEventListener("click", async () => {
    try {
      savedPath = await invoke("save_wrapped", { pngBase64: $("wrapped-image").src });
      $("wrapped-status").textContent = state.t("wrapped.saved");
      $("wrapped-show").hidden = false;
    } catch (key) {
      $("wrapped-status").textContent = state.t(String(key));
    }
  });
  $("wrapped-show").addEventListener("click", () => savedPath && invoke("reveal_path", { path: savedPath }));

  await listen("usage", ({ payload }) => { state.usage = payload; render(); });
  await listen("dock", ({ payload }) => { state.edge = payload.edge; state.expanded = payload.expanded; render(); });
  await listen("notice", ({ payload }) => { state.notice = payload.key; render(); });
  await listen("agent-events", ({ payload }) => { state.events = payload; render(); });
  await listen("limit-alert", ({ payload }) => showLimitAlert(payload));
  await listen("sessions", ({ payload }) => { state.sessions = payload; render(); });
  await listen("presence", ({ payload }) => { state.presence = payload; render(); });
  await listen("drag", ({ payload }) => { $("drop-hint").hidden = !payload; });
  await listen("agent-reminder", ({ payload }) => {
    state.notice = { text: state.t("reminder", { agent: fmt.agentName(payload.source), project: payload.project || "?" }) };
    render();
  });
  await listen("speech", ({ payload }) => {
    if (payload.kind === "state") state.speech = payload;
    if (payload.kind === "transcript") {
      state.transcript = payload.text;
      state.history = [payload.text, ...state.history].slice(0, 8);
    }
    if (payload.kind === "notice") state.notice = payload.key;
    render();
  });
  setInterval(render, 5_000); // Keep elapsed times, countdowns and paces current.
  render();
}

main();
