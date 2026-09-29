# Changelog

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
