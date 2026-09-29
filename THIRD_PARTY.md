# Third-party components

AgentNotch for Windows is MIT-licensed. It depends on third-party packages that
keep their own licenses; exact versions are recorded in `src-tauri/Cargo.lock`
and `package-lock.json`. Notable components:

| Component | Use | License |
| --- | --- | --- |
| Tauri and its plugins | Application shell, tray, global shortcut, autostart, single instance | MIT or Apache-2.0 |
| whisper.cpp (via whisper-rs) | On-device speech recognition, compiled into the app | MIT |
| cpal | Microphone capture | Apache-2.0 |
| arboard | Clipboard access | MIT or Apache-2.0 |
| ureq | Model download | MIT or Apache-2.0 |
| windows-rs | Win32 APIs | MIT or Apache-2.0 |

**Whisper models** are not included in the repository or installer. They are
downloaded on first use from the whisper.cpp project's model repository; the
OpenAI Whisper weights are released under the MIT license.

**Microsoft Edge WebView2** is a Microsoft runtime installed with Windows 11 or
by the setup program; it is not redistributed by this repository.

The app icon is original to this project. Codex, Claude and Claude Code are
names of their respective owners and are used only to describe compatibility.
