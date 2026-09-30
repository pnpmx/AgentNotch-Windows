# Changelog

## 0.5.0

- Compact panel: one tab at a time (**Sessions · Limits · Voice**), opening on
  Sessions when there is activity. Agent alerts merge into their session row as
  an unread mark with a badge on the tab; rows are one line with details on
  demand; at most four sessions show, and ones finished over an hour ago hide.
- Replies are shown as plain text (Markdown tables and emphasis, and Codex JSON
  replies, are cleaned). No "+0/−0" for tasks that changed no lines.
- Setup buttons only appear when something still needs connecting.
- Past dictations live in the Voice tab.

## 0.4.0

- Sessions panel: every Claude Code and Codex session at once, with project,
  model, state and live activity ("editing auth.ts", "running npm test").
  Uses `UserPromptSubmit` and `PreToolUse` hooks; only a short description of
  each action is kept, never file contents. Parallel hooks write under a lock.
- The tab shows the overall state: shimmer while working, breathing while an
  agent waits, a flash when one finishes.
- Drop files on the widget to paste their paths where you are typing.
- Last response of each session with copy, and "Continue in Codex/Claude",
  which copies a handoff prompt with the project, the request and the progress.
- Cost per task ($, time, lines added/removed) on finished alerts.
- Countdown when a limit is nearly used up, and a notice when it is available.
- One reminder when a session has been waiting for you for three minutes.
- Weekly "Wrapped" image with tasks, lines, cost, hours, favourite model,
  top project and busiest day, saved to the Desktop to share.

## 0.3.0

- Agent alerts: Claude Code hooks and Codex's notify program report when an
  agent finishes, needs approval or is waiting. The tab pulses and the panel
  lists what happened, per project. One click installs both integrations;
  existing hooks and notify programs are preserved.
- Default model and effort for new Claude Code and Codex sessions, chosen from
  the panel. Codex options come from the models Codex itself lists.
- Live Claude Code session line: model, effort, cost and context used.
- Limit alerts at 80% and 95%, a notice when a limit resets, and a pace
  projection ("at this pace: 100% at 16:40").
- Dictation: custom vocabulary for names and jargon, optional Enter after
  pasting, filler-word removal, and a history of recent dictations.

## 0.2.0

- Linux support: AppImage and .deb packages. On Wayland the widget docks via
  XWayland, hold-to-talk uses the GlobalShortcuts portal, and pasting uses
  ydotool, wtype or xdotool when installed (otherwise the text is copied).
- New `--toggle-dictation` command to bind in desktop keyboard settings.
- Dictation never pastes into the widget itself when it has focus.

## 0.1.0

- First Windows version of AgentNotch: an edge-docked tab that can be dragged
  to any edge of any monitor and snaps into place.
- Codex usage via `codex app-server`; Claude Code usage via the status-line
  bridge (`AgentNotch.exe --claude-bridge`).
- Hold-to-talk dictation (Ctrl+Shift+Space) with on-device whisper.cpp, pasted
  into the previously focused window with clipboard restore.
- Interface in six languages, following the Windows language by default.
- Tray menu: show/hide, refresh, connect Claude Code, start with Windows, quit.
