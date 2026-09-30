import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as f from "./format.js";

const table = JSON.parse(readFileSync(new URL("./i18n.json", import.meta.url)));
const en = f.translator(table, "en");
const de = f.translator(table, "de");

test("language resolution", () => {
  assert.equal(f.resolveLanguage("system", "es-ES"), "es");
  assert.equal(f.resolveLanguage("system", "ja-JP"), "en");
  assert.equal(f.resolveLanguage("fr", "de-DE"), "fr");
});

test("reset descriptions", () => {
  assert.equal(f.resetDescription(4900, 1000, en), "reset 1 h 5 min");
  assert.equal(f.resetDescription(4900, 1000, de), "Reset 1 Std. 5 Min.");
  assert.equal(f.resetDescription(null, 0, en), "reset unknown");
  assert.equal(f.resetDescription(10, 20, en), "reset overdue · refresh");
  assert.equal(f.resetDescription(1000 + 90000, 1000, en), "reset 1 d 1 h");
});

test("window labels follow language and ids", () => {
  assert.equal(f.windowLabel({ id: "claude-five_hour", name: "five_hour" }, de), "5 Stunden");
  assert.equal(f.windowLabel({ id: "codex-GPT-secondary", name: "GPT", durationMinutes: 43200 }, en), "GPT weekly");
  assert.equal(f.windowLabel({ id: "codex-x-primary", name: "x", durationMinutes: 300 }, en), "5 hours");
});

test("percent clamps and handles NaN", () => {
  assert.equal(f.percent(120), "100%");
  assert.equal(f.percent(NaN), "--");
  assert.equal(f.percent(41.2), "41%");
});

test("staleness", () => {
  const snap = { fetchedAt: 1000, origin: "Claude Code", windows: [{ resetsAt: 5000 }] };
  assert.equal(f.isStale(snap, 1100), false);
  assert.equal(f.isStale(snap, 1400), true);
  assert.match(f.ageLabel(snap, 1400, en), /outdated/);
});

test("missing placeholder stays visible", () => {
  assert.equal(en("window.weekly"), "{name} weekly");
});

test("agent event titles", () => {
  assert.equal(f.eventTitle({ kind: "permission", source: "claude" }, en), "Claude needs your approval");
  assert.equal(f.eventTitle({ kind: "done", source: "codex" }, f.translator(table, "es")), "Codex ha terminado");
});

test("session line", () => {
  const session = { model: "Opus", effort: "high", costUsd: 1.4, contextPercent: 61.6 };
  assert.equal(f.sessionLine(session, en), "Opus · High · $1.40 · context 62%");
  assert.equal(f.sessionLine({ model: "Sonnet", effort: null, costUsd: null, contextPercent: null }, en), "Sonnet");
  assert.equal(f.sessionLine(null, en), "");
});

test("pace label only for future projections", () => {
  assert.equal(f.paceLabel(100, 200, "en", en), "");
  assert.match(f.paceLabel(1_800_000_000, 1_700_000_000, "en", en), /^at this pace: 100% at \d\d:\d\d/);
});

test("limit alert text", () => {
  assert.equal(
    f.limitAlertText({ kind: "threshold", windowId: "claude-five_hour", percent: 80 }, en),
    "Claude 5 hours limit at 80%",
  );
  assert.equal(f.limitAlertText({ kind: "reset", windowId: "claude-seven_day" }, en), "Claude 7 days limit has reset");
});

test("session helpers", () => {
  assert.equal(f.activityText({ kind: "edit", detail: "auth.ts" }, en), "editing auth.ts");
  assert.equal(f.activityText({ kind: "run", detail: "npm test" }, f.translator(table, "es")), "ejecutando npm test");
  assert.equal(f.taskSummary({ costUsd: 0.42, durationSecs: 240, linesAdded: 156, linesRemoved: 23 }, en), "$0.42 · 4 min · +156/−23");
  assert.equal(f.taskSummary({ costUsd: null, durationSecs: 0, linesAdded: null, linesRemoved: null }, en), "");
  assert.equal(f.countdown(1000 + 47 * 60, 1000), "47 min");
  assert.equal(f.countdown(1000 + 125 * 60, 1000), "2 h 5 min");
  const prompt = f.handoffPrompt({ project: "api", cwd: "/w/api", lastPrompt: "fix login", lastMessage: "Half done." }, en);
  assert.match(prompt, /“api” \(\/w\/api\)/);
  assert.match(prompt, /fix login[\s\S]*Half done\./);
});

test("overall state priority", () => {
  const now = 100_000;
  assert.equal(f.overallState([{ state: "working", updatedAt: now }, { state: "waiting", updatedAt: now }], now), "waiting");
  assert.equal(f.overallState([{ state: "working", updatedAt: now }], now), "working");
  assert.equal(f.overallState([{ state: "done", updatedAt: now - 1000 }], now), "done");
  assert.equal(f.overallState([{ state: "done", updatedAt: now - 60_000 }], now), "idle");
  assert.equal(f.overallState([], now), "idle");
});

test("plain text from agent output", () => {
  assert.equal(f.plainText('{"summary":"Fixed the flow.","files":[]}'), "Fixed the flow.");
  const md = "## Result\n\n| Platform | Version |\n|---|---|\n| **Mac** | `v0.5.0` |\n\n\n\n- done";
  assert.equal(f.plainText(md), "Result\n\nPlatform · Version\nMac · v0.5.0\n\n• done");
  assert.equal(f.taskSummary({ costUsd: 0.47, durationSecs: 55, linesAdded: 0, linesRemoved: 0 }, en), "$0.47 · 55 s");
});

test("session ordering", () => {
  const now = 10_000_000;
  const sessions = [
    { id: "old", state: "done", updatedAt: now - 7_200_000 },
    { id: "a", state: "done", updatedAt: now - 1000 },
    { id: "w", state: "working", updatedAt: now - 5000 },
    { id: "u", state: "done", updatedAt: now - 9000 },
    { id: "wait", state: "waiting", updatedAt: now - 20000 },
  ];
  assert.deepEqual(f.orderSessions(sessions, new Set(["u"]), now).map((s) => s.id), ["wait", "u", "w", "a"]);
});
