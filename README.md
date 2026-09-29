# AgentNotch for Windows and Linux

[![CI](https://github.com/pnpmx/AgentNotch-Windows/actions/workflows/ci.yml/badge.svg)](https://github.com/pnpmx/AgentNotch-Windows/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A small always-on-top tab that docks to any edge of any screen and shows your
**Codex** and **Claude Code** usage limits, with **local push-to-talk dictation**
into whatever window you are typing in.

Windows PCs have no notch, so instead of sitting in the camera cutout like
[AgentNotch for macOS](https://github.com/pnpmx/AgentNotch), the widget clings to
a screen edge. Drag it anywhere, on any monitor, and it snaps to the nearest
edge when you let go. Click it to open the full panel.

**Experimental software.** Version 0.1 is built and unit-tested in CI on
Windows, but the physical checks (dragging across monitors, microphone,
pasting into other apps, different display scales) still need to be done on
real machines. Please report what you find.

## Features

- Edge-docked tab: left, right, top or bottom, with snapping and multi-monitor
  support. The position is remembered as a fraction along the edge, so it
  survives resolution changes.
- Codex usage through the Codex CLI's `app-server`, and Claude Code usage through
  its status-line hook.
- Hold **Ctrl + Shift + Space** to dictate. Speech is transcribed on-device with
  [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and pasted into the
  window that was focused when you pressed the shortcut.
- Interface in English, Spanish, Italian, French, German and Portuguese; it
  follows the Windows language by default. Dictation language is separate and
  defaults to automatic detection.
- The widget is a non-activating window, so clicking it should not take focus
  from the app you are working in (to be confirmed on real machines).

## Requirements

- Windows 10 (1809+) or Windows 11, x64. The WebView2 runtime is included
  with Windows 11 and installed by the setup program if missing.
- For Codex usage: the [Codex CLI](https://github.com/openai/codex) installed
  and signed in (`codex.exe` or npm's `codex.cmd` on `PATH` or in `%APPDATA%\npm`).
- For Claude usage: [Claude Code](https://docs.anthropic.com/en/docs/claude-code).
- A microphone. The first dictation downloads the speech model once
  (about 150 MB for *Base*, about 470 MB for *Small*).

## Install

Download `AgentNotch_<version>_x64-setup.exe` from the
[latest release](https://github.com/pnpmx/AgentNotch-Windows/releases/latest)
(an `.msi` is also provided for managed installs).
Builds are **not code-signed**, so Windows SmartScreen will warn: choose
*More info → Run anyway* only if you trust the build. Compare the file against
`SHA256SUMS.txt` from the release with:

```powershell
Get-FileHash .\AgentNotch_0.1.0_x64-setup.exe -Algorithm SHA256
```

The NSIS installer installs for the current user only and needs no admin rights.

## Use

1. The tab appears on the right edge. Drag it to wherever you like.
2. Click it to open the panel. Use **Connect Claude** once; then send any message
   in Claude Code and its limits appear.
3. Put the cursor in any text field, hold **Ctrl + Shift + Space**, speak, release.
   The first time, the speech model downloads; try again when it finishes.
4. ⚙ in the panel changes the interface language, dictation language and model.
   The tray icon has *Start with Windows*, refresh and quit.

If pasting fails (for example into an app running as administrator), the text is
kept on the clipboard and in the panel's **Copy** button.

## Linux

Download the `.AppImage` (any distribution) or the `.deb` (Debian/Ubuntu) from
the [latest release](https://github.com/pnpmx/AgentNotch-Windows/releases/latest).

```sh
chmod +x AgentNotch_*.AppImage && ./AgentNotch_*.AppImage
# or
sudo apt install ./AgentNotch_*_amd64.deb
```

**Wayland** (default on current GNOME, KDE and others) restricts what apps
may do, so AgentNotch adapts:

- **Docking:** the widget runs through XWayland so it can place itself on a
  screen edge. Set `AGENTNOTCH_NATIVE_WAYLAND=1` to opt out.
- **Hold-to-talk:** registered through the desktop's *GlobalShortcuts* portal
  (KDE Plasma 6, GNOME 48+, Hyprland). The first launch may show a system
  dialog to confirm the keys. If your desktop has no such portal, bind the
  command `agentnotch --toggle-dictation` to a key in your keyboard settings:
  press once to start, again to stop.
- **Pasting:** Wayland blocks synthetic typing, so install one helper:
  [`ydotool`](https://github.com/ReimuNotMoe/ydotool) (any desktop; its
  `ydotoold` service must be running) or `wtype` (Sway, Hyprland and other
  wlroots desktops). Without one, the text is copied and you press Ctrl+V.
- **Tray icon:** GNOME needs the *AppIndicator* extension to show it.

On X11 everything works as on Windows (`xdotool` is used for pasting).

## Build from source

Requires Rust 1.88+, Node.js 20+, CMake, and the platform libraries from the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (MSVC build tools
and WebView2 on Windows; WebKitGTK 4.1, libayatana-appindicator and ALSA headers
on Linux).

```powershell
git clone https://github.com/pnpmx/AgentNotch-Windows.git
cd AgentNotch-Windows
npm ci
npm test            # Rust unit tests + UI formatting tests
npx tauri dev       # run
npx tauri build     # installers in src-tauri/target/release/bundle
```

The app also compiles on macOS for development, without pasting.

## How it works

| Part | Implementation |
| --- | --- |
| Shell | [Tauri 2](https://v2.tauri.app/) (Rust + system WebView2), plain HTML/JS UI |
| Docking | `src-tauri/src/dock.rs`: pure geometry, unit-tested; snaps after the mouse button is released |
| Codex | `src-tauri/src/usage/codex.rs`: JSON-RPC to `codex app-server`, same protocol as the macOS app |
| Claude | `src-tauri/src/usage/claude.rs`: `AgentNotch.exe --claude-bridge` as Claude Code's status line |
| Dictation | `cpal` capture → 16 kHz mono → `whisper-rs` → clipboard + `SendInput` Ctrl+V |
| Text | `ui/i18n.json`, shared by the UI and the tray menu |

## Uninstall

Quit from the tray icon, then uninstall *AgentNotch* from *Settings → Apps*.
If you connected Claude, remove the `statusLine` entry that runs
`AgentNotch.exe --claude-bridge` from `%USERPROFILE%\.claude\settings.json`
(a `settings.agentnotch-backup-*.json` copy was made when connecting). Delete
`%APPDATA%\AgentNotch` (settings and usage snapshot) and
`%LOCALAPPDATA%\AgentNotch` (speech models).

## License

[MIT](LICENSE). Third-party components keep their own licenses; see
[THIRD_PARTY.md](THIRD_PARTY.md). Codex and Claude are services of their
respective owners; this project is not affiliated with OpenAI or Anthropic.
