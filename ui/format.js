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
