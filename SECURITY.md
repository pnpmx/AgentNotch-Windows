# Security and privacy

AgentNotch for Windows is experimental. A passing build or unit test does not
establish that microphone capture, the global shortcut or pasting behave safely
on every machine.

## Data flow

- **Speech** is transcribed locally with whisper.cpp. No audio or text is sent
  to a hosted service. Audio is kept in memory only while the shortcut is held
  and is discarded after transcription; it is capped at about five minutes.
- **Speech models** are downloaded once over HTTPS from the whisper.cpp model
  repository on Hugging Face into `%LOCALAPPDATA%\AgentNotch\models`. The file is
  checked for the ggml header and complete length before use. There is no
  pinned cryptographic hash yet; see the open item below.
- **Pasting** temporarily places the transcript on the clipboard and sends
  Ctrl+V to the window that was focused when the shortcut was pressed. If the
  focused window changed, nothing is pasted and the text stays on the clipboard.
  Previous clipboard text or image content is restored after about a second
  unless something else was copied meanwhile. Other clipboard formats (files,
  rich text) are not preserved.
- **Global shortcut**: only the configured shortcut is registered with Windows
  (`RegisterHotKey`, via Tauri's global-shortcut plugin), and only that key's
  state is polled to detect release. The app installs no keyboard hook and
  never observes other keystrokes.
- **Codex usage** starts the locally installed `codex app-server`, which uses
  the CLI's existing sign-in and may contact its service.
- **Claude usage**: connecting modifies `%USERPROFILE%\.claude\settings.json`
  after writing a backup. An existing, unrelated status line is never
  overwritten. The bridge stores only rate-limit percentages and reset times in
  `%APPDATA%\AgentNotch\claude-usage.json`.
- Builds are not code-signed. Verify downloads against `SHA256SUMS.txt`.

## Open items

- Pin SHA-256 hashes for the supported speech models.
- Code signing for release installers.

## Reporting

Use [private vulnerability reporting](https://github.com/pnpmx/AgentNotch-Windows/security/advisories/new).
Do not put credentials, transcripts, clipboard contents or personal screenshots
in public issues. There is no guaranteed response time.
