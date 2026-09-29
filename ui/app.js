import table from "./i18n.json" with { type: "json" };
import * as fmt from "./format.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const $ = (id) => document.getElementById(id);
const now = () => Math.floor(Date.now() / 1000);
const NATIVE_NAMES = { en: "English", es: "Español", it: "Italiano", fr: "Français", de: "Deutsch", pt: "Português" };

const state = {
  settings: null,
  expanded: false,
  edge: "right",
  usage: {},
  speech: { state: "idle" },
  transcript: "",
  notice: null,
  t: fmt.translator(table, "en"),
};

// ---------- Language ----------

function applyLanguage() {
  const language = fmt.resolveLanguage(state.settings.uiLanguage, navigator.language);
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

// ---------- Rendering ----------

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
    const p = document.createElement("p");
    p.className = "error";
    p.textContent = error ? t(error) : t("loading");
    body.append(p);
    return;
  }
  const age = document.createElement("div");
  age.className = "age";
  age.classList.toggle("stale", fmt.isStale(snapshot, now()));
  age.textContent = fmt.ageLabel(snapshot, now(), t);
  body.append(age);
  for (const window of snapshot.windows.slice(0, 2)) {
    const limit = document.createElement("div");
    limit.className = "limit";
    const row = document.createElement("div");
    row.className = "row";
    const label = document.createElement("span");
    label.textContent = fmt.windowLabel(window, t);
    const value = document.createElement("span");
    value.textContent = fmt.percent(window.usedPercent);
    row.append(label, value);
    const bar = document.createElement("div");
    bar.className = "bar";
    const fill = document.createElement("div");
    fill.style.width = `${Math.min(100, Math.max(0, window.usedPercent))}%`;
    bar.append(fill);
    const reset = document.createElement("div");
    reset.className = "reset";
    reset.textContent = fmt.resetDescription(window.resetsAt, now(), t);
    limit.append(row, bar, reset);
    body.append(limit);
  }
  if (error) {
    const p = document.createElement("p");
    p.className = "error warn";
    p.textContent = t(error);
    body.append(p);
  }
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
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = label;
      button.setAttribute("aria-pressed", String(value === current));
      button.addEventListener("click", () => onPick(value));
      return button;
    }),
  );
}

function render() {
  const { usage, settings, t } = state;
  const root = $("root");
  root.className = `edge-${state.edge} ${state.expanded ? "expanded" : "collapsed"}`;
  $("panel").hidden = !state.expanded;

  renderChip("codex", usage.codex);
  renderChip("claude", usage.claude);
  for (const mic of [$("mic"), $("mic-large")]) mic.dataset.state = state.speech.state;
  $("mic").title = speechLabel();

  if (!state.expanded) return;
  $("speech-label").textContent = speechLabel();
  renderColumn("codex", usage.codex, usage.codexError);
  renderColumn("claude", usage.claude, usage.claudeError);
  $("connect").hidden = Boolean(usage.claudeConnected);
  $("transcript-box").hidden = !state.transcript;
  $("transcript").textContent = state.transcript;
  $("notice").textContent = state.notice ? t(state.notice) : "";

  const languages = fmt.LANGUAGES.map((code) => [code, NATIVE_NAMES[code]]);
  renderSegmented($("ui-language"), [["system", t("settings.system")], ...languages], settings.uiLanguage,
    (value) => updateSettings({ uiLanguage: value }));
  renderSegmented($("dictation-language"), [["auto", t("settings.auto")], ...languages], settings.dictationLanguage,
    (value) => updateSettings({ dictationLanguage: value }));
  renderSegmented($("model"), [["base", t("model.base")], ["small", t("model.small")]], settings.model,
    (value) => updateSettings({ model: value }));
}

// ---------- Interaction ----------

async function setExpanded(expanded) {
  state.expanded = expanded;
  await invoke("set_expanded", { expanded });
  render();
}

/// Click toggles the panel; dragging more than a few pixels moves the widget,
/// and the backend snaps it to the nearest screen edge when released.
function makeDraggable(element, onClick) {
  element.addEventListener("mousedown", (down) => {
    if (down.button !== 0 || down.target.closest("button")) return;
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
  });
  // Tell the backend which language "system" resolved to, for the tray menu.
  await updateSettings({ uiLanguage: state.settings.uiLanguage });

  makeDraggable($("tab"), () => setExpanded(true));
  makeDraggable($("panel-header"));
  $("close").addEventListener("click", () => setExpanded(false));
  $("settings-toggle").addEventListener("click", () => { $("settings").hidden = !$("settings").hidden; });
  $("refresh").addEventListener("click", () => invoke("refresh_usage"));
  $("copy").addEventListener("click", async () => {
    if (await invoke("copy_transcript")) { state.notice = "notice.copied"; render(); }
  });
  $("connect").addEventListener("click", async () => {
    try { await invoke("connect_claude"); } catch (key) { state.notice = String(key); render(); }
  });
  window.addEventListener("keydown", (event) => { if (event.key === "Escape") setExpanded(false); });

  await listen("usage", ({ payload }) => { state.usage = payload; render(); });
  await listen("dock", ({ payload }) => { state.edge = payload.edge; state.expanded = payload.expanded; render(); });
  await listen("notice", ({ payload }) => { state.notice = payload.key; render(); });
  await listen("speech", ({ payload }) => {
    if (payload.kind === "state") state.speech = payload;
    if (payload.kind === "transcript") state.transcript = payload.text;
    if (payload.kind === "notice") state.notice = payload.key;
    render();
  });
  setInterval(render, 30_000); // Keep ages and reset countdowns current.
  render();
}

main();
