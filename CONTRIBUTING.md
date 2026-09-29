# Contributing

Requires Rust 1.88+ and Node.js 20+. Windows builds also need the MSVC build
tools and CMake (see the Tauri prerequisites).

```sh
npm ci
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm test
```

Keep logic that can be tested without a window system in pure modules
(`dock.rs`, `usage/parser.rs`, `ui/format.js`) and cover it with tests. For
changes to docking, the shortcut, the microphone or pasting, describe what you
checked on a real Windows machine: monitor layout, display scaling, and target
apps. Test pasting with synthetic text in an isolated editor, never in a live
terminal command or a real message box.

New interface text goes into every language in `ui/i18n.json`; a unit test
fails if a key or `{placeholder}` is missing.

Do not commit runtime data (`settings.json`, usage snapshots, models),
screenshots of private applications or signing material. Contributions are
under the project's MIT license; include attribution and license notices for
third-party code or assets.
