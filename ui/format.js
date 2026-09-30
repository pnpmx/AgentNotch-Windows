// Pure formatting helpers shared by the widget and its tests.

export const LANGUAGES = ["en", "es", "it", "fr", "de", "pt"];

export function resolveLanguage(choice, navigatorLanguage) {
  if (LANGUAGES.includes(choice)) return choice;
  const code = String(navigatorLanguage || "").slice(0, 2).toLowerCase();
  return LANGUAGES.includes(code) ? code : "en";
}

export function translator(table, language) {
  return (key, values = {}) => {
    const text = table[language]?.[key] ?? table.en?.[key] ?? key;
    return text.replace(/\{(\w+)\}/g, (match, name) => (name in values ? String(values[name]) : match));
  };
}

export function percent(value) {
  if (!Number.isFinite(value)) return "--";
  return `${Math.round(Math.min(100, Math.max(0, value)))}%`;
}

export function resetDescription(resetsAt, now, t) {
  if (resetsAt == null) return t("reset.unknown");
  if (resetsAt <= now) return t("reset.overdue");
  const seconds = resetsAt - now;
  if (seconds < 60) return t("reset.lt1");
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return t("reset.m", { m: minutes });
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours < 24) return rest === 0 ? t("reset.h", { h: hours }) : t("reset.hm", { h: hours, m: rest });
  return t("reset.dh", { d: Math.floor(hours / 24), h: hours % 24 });
}

export function windowLabel(window, t) {
  const key = window.id.split("-").pop();
  if (key === "five_hour" || window.durationMinutes === 300) return t("window.5h");
  if (key === "seven_day" || window.durationMinutes === 10080) return t("window.7d");
  if (key === "spend_limit") return t("window.spend");
  if (window.id.startsWith("codex-") && window.id.endsWith("-secondary")) {
    return t("window.weekly", { name: window.name });
  }
  return window.name;
}

export function isStale(snapshot, now) {
  if (!snapshot) return true;
  return now - snapshot.fetchedAt > 300 || snapshot.windows.some((w) => w.resetsAt != null && w.resetsAt < now);
}

export function ageLabel(snapshot, now, t) {
  const minutes = Math.max(0, Math.floor((now - snapshot.fetchedAt) / 60));
  const parts = [snapshot.origin, t("age", { m: minutes })];
  if (isStale(snapshot, now)) parts.push(t("outdated"));
  return parts.join(" · ");
}

export function hotkeyLabel(hotkey) {
  return String(hotkey || "").split("+").join(" + ");
}

const AGENT_NAMES = { claude: "Claude", codex: "Codex" };

export function agentName(source) {
  return AGENT_NAMES[source] ?? source;
}

export function eventTitle(event, t) {
  const key = { done: "agents.done", permission: "agents.permission", needsInput: "agents.needsInput" }[event.kind];
  return t(key ?? "agents.done", { agent: agentName(event.source) });
}

export function clockTime(unixSeconds, language) {
  return new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit" }).format(new Date(unixSeconds * 1000));
}

/// "at this pace: 100% at 16:40", only when the projection is in the future.
export function paceLabel(eta, now, language, t) {
  if (!Number.isFinite(eta) || eta <= now) return "";
  return t("limit.pace", { time: clockTime(eta, language) });
}

export function sessionLine(session, t) {
  if (!session) return "";
  const parts = [session.model];
  if (session.effort) parts.push(t(`effort.${session.effort}`));
  if (Number.isFinite(session.costUsd)) parts.push(`$${session.costUsd.toFixed(2)}`);
  if (Number.isFinite(session.contextPercent)) parts.push(t("session.context", { p: Math.round(session.contextPercent) }));
  return parts.join(" · ");
}

/// `window` is the matching usage window when known, for an exact label.
export function limitAlertText(alert, t, window) {
  window ??= { id: alert.windowId, name: alert.windowId.split("-").slice(1, -1).join("-") || alert.windowId };
  const agent = agentName(alert.windowId.split("-")[0]);
  const label = windowLabel(window, t);
  if (alert.kind === "reset") return t("limit.reset", { agent, window: label });
  if (alert.kind === "available") return t("limit.available", { agent, window: label });
  return t("limit.threshold", { agent, window: label, percent: alert.percent });
}

// ---------- Sessions ----------

export function activityText(activity, t) {
  if (!activity) return "";
  return t(`activity.${activity.kind}`, { detail: activity.detail });
}

export function duration(seconds, t) {
  if (!Number.isFinite(seconds) || seconds < 60) return t("task.seconds", { s: Math.max(0, Math.round(seconds || 0)) });
  return t("task.minutes", { m: Math.round(seconds / 60) });
}

/// "$0.42 · 4 min · +156/−23", leaving out what is unknown.
export function taskSummary(task, t) {
  if (!task) return "";
  const parts = [];
  if (Number.isFinite(task.costUsd) && task.costUsd > 0) parts.push(`$${task.costUsd.toFixed(2)}`);
  if (task.durationSecs > 0) parts.push(duration(task.durationSecs, t));
  if (Number.isFinite(task.linesAdded) || Number.isFinite(task.linesRemoved)) {
    parts.push(`+${task.linesAdded ?? 0}/−${task.linesRemoved ?? 0}`);
  }
  return parts.join(" · ");
}

/// Prompt that lets the other agent pick up where this session stopped.
export function handoffPrompt(session, t) {
  return t("handoff.template", {
    project: session.project || "?",
    cwd: session.cwd || "",
    prompt: session.lastPrompt || "—",
    message: session.lastMessage || "—",
  });
}

/// "47 min" or "2 h 5 min" until a Unix-seconds time.
export function countdown(target, now) {
  const minutes = Math.max(1, Math.ceil((target - now) / 60));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

/// Overall state for the tab: waiting beats working beats a just-finished flash.
export function overallState(sessions, nowMs, flashMs = 8000) {
  if (sessions.some((s) => s.state === "waiting")) return "waiting";
  if (sessions.some((s) => s.state === "working" && nowMs - s.updatedAt < 30 * 60 * 1000)) return "working";
  if (sessions.some((s) => s.state === "done" && nowMs - s.updatedAt < flashMs)) return "done";
  return "idle";
}
