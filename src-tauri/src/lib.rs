mod agents;
mod dictation;
mod dock;
mod i18n;
#[cfg(target_os = "linux")]
mod linux;
mod paths;
mod settings;
mod usage;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use agents::events::AgentEvent;
use dictation::inject::PasteTarget;
use dictation::{Command, Event as SpeechEvent, SpeechState};
use dock::Rect;
use settings::Settings;
use usage::alerts::Tracker;
use usage::parser::{SessionInfo, UsageSnapshot};

pub use agents::events::{run as run_agent_event, EVENT_FLAG};
pub use usage::claude::{run_bridge, BRIDGE_FLAG};

pub const TOGGLE_FLAG: &str = "--toggle-dictation";

#[cfg(target_os = "linux")]
pub use linux::prepare_environment;

const MAIN: &str = "main";
const TRAY: &str = "tray";

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageState {
    codex: Option<UsageSnapshot>,
    codex_error: Option<String>,
    claude: Option<UsageSnapshot>,
    claude_error: Option<String>,
    claude_connected: bool,
    /// Latest Claude Code session (model, effort, cost, context).
    session: Option<SessionInfo>,
    /// Window id → Unix seconds when 100% is reached at the current pace.
    projections: std::collections::HashMap<String, i64>,
}

const HISTORY: usize = 8;

struct AppState {
    settings: Mutex<Settings>,
    /// Resolved interface language, used for the tray menu.
    language: Mutex<String>,
    expanded: AtomicBool,
    usage: Mutex<UsageState>,
    speech: Mutex<SpeechState>,
    transcript: Mutex<String>,
    /// Most recent dictations, newest first.
    history: Mutex<Vec<String>>,
    dictation: Mutex<Sender<Command>>,
    tracker: Mutex<Tracker>,
    /// Events newer than this (Unix ms) are shown; older ones were seen.
    events_seen: std::sync::atomic::AtomicI64,
    move_generation: AtomicU64,
    holding: AtomicBool,
    registered_hotkey: Mutex<Option<String>>,
}

impl AppState {
    fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
    fn send(&self, command: Command) {
        let _ = self.dictation.lock().unwrap().send(command);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    settings: Settings,
    expanded: bool,
    usage: UsageState,
    speech: SpeechState,
    transcript: String,
    history: Vec<String>,
    events: Vec<AgentEvent>,
}

// ---------- Docking ----------

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN)
}

fn work_area(monitor: &tauri::Monitor) -> Rect {
    let area = monitor.work_area();
    Rect {
        x: area.position.x,
        y: area.position.y,
        w: area.size.width as i32,
        h: area.size.height as i32,
    }
}

/// Monitor containing the window's centre, falling back to the primary one.
fn current_monitor(window: &WebviewWindow) -> Option<tauri::Monitor> {
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return window.primary_monitor().ok().flatten();
    };
    let cx = pos.x + size.width as i32 / 2;
    let cy = pos.y + size.height as i32 / 2;
    window
        .available_monitors()
        .ok()
        .and_then(|monitors| monitors.into_iter().find(|m| work_area(m).contains(cx, cy)))
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())
}

fn apply_dock(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        return;
    };
    let Some(monitor) = current_monitor(&window) else {
        return;
    };
    let state = app.state::<AppState>();
    let settings = state.settings();
    let expanded = state.expanded.load(Ordering::SeqCst);
    let area = work_area(&monitor);
    let (w, h) = dock::physical_size(settings.edge, expanded, monitor.scale_factor());
    let (x, y) = dock::place(area, settings.edge, settings.offset, w, h);
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    if window.outer_position().ok() != Some(PhysicalPosition::new(x, y)) {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = app.emit(
        "dock",
        serde_json::json!({ "edge": settings.edge, "expanded": expanded }),
    );
}

/// After the user drags the tab, snap it to the nearest edge of whichever
/// monitor it was dropped on.
fn snap_after_drag(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        return;
    };
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return;
    };
    let Some(monitor) = current_monitor(&window) else {
        return;
    };
    let area = work_area(&monitor);
    let cx = pos.x + size.width as i32 / 2;
    let cy = pos.y + size.height as i32 / 2;
    let edge = dock::nearest_edge(area, cx, cy);
    let offset = dock::offset_along(area, edge, cx, cy);
    let state = app.state::<AppState>();
    {
        let mut settings = state.settings.lock().unwrap();
        if settings.edge == edge && (settings.offset - offset).abs() < 0.002 {
            drop(settings);
            apply_dock(app);
            return;
        }
        settings.edge = edge;
        settings.offset = offset;
        settings.save();
    }
    apply_dock(app);
}

#[cfg(windows)]
fn primary_button_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    (unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } as u16 & 0x8000) != 0
}

#[cfg(not(windows))]
fn primary_button_down() -> bool {
    false
}

fn schedule_snap(app: &AppHandle) {
    let state = app.state::<AppState>();
    let generation = state.move_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(220));
        let state = app.state::<AppState>();
        if state.move_generation.load(Ordering::SeqCst) != generation {
            return; // A newer move superseded this one.
        }
        if primary_button_down() {
            continue; // Still dragging.
        }
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || snap_after_drag(&handle));
        return;
    });
}

// ---------- Usage ----------

fn emit_usage(app: &AppHandle) {
    let usage = app.state::<AppState>().usage.lock().unwrap().clone();
    let _ = app.emit("usage", usage);
}

/// Feeds a fresh snapshot to the limit tracker and forwards alerts.
fn track_limits(app: &AppHandle, snapshot: &UsageSnapshot) {
    let state = app.state::<AppState>();
    let (alerts, projections) = {
        let mut tracker = state.tracker.lock().unwrap();
        (tracker.update(snapshot), tracker.projections())
    };
    state.usage.lock().unwrap().projections = projections;
    if state.settings().limit_alerts {
        for alert in alerts {
            let _ = app.emit("limit-alert", alert);
        }
    }
}

fn refresh_claude(app: &AppHandle) {
    let connected = usage::claude::is_bridge_configured();
    let snapshot = usage::claude::load();
    let session = usage::claude::load_session();
    if let Some(snapshot) = &snapshot {
        track_limits(app, snapshot);
    }
    {
        let state = app.state::<AppState>();
        let mut usage = state.usage.lock().unwrap();
        usage.claude_connected = connected;
        usage.claude_error = snapshot.is_none().then(|| {
            if connected {
                "claude.waiting"
            } else {
                "claude.notConnected"
            }
            .to_owned()
        });
        usage.claude = snapshot;
        usage.session = session;
    }
    emit_usage(app);
}

fn refresh_codex(app: &AppHandle) {
    let result = usage::codex::fetch();
    if let Ok(snapshot) = &result {
        track_limits(app, snapshot);
    }
    {
        let state = app.state::<AppState>();
        let mut usage = state.usage.lock().unwrap();
        match result {
            Ok(snapshot) => {
                usage.codex = Some(snapshot);
                usage.codex_error = None;
            }
            // Keep the last good snapshot; the UI marks it stale by age.
            Err(error) => usage.codex_error = Some(error.to_string()),
        }
    }
    emit_usage(app);
}

fn start_usage_loop(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut tick: u64 = 0;
        loop {
            refresh_claude(&app);
            if tick.is_multiple_of(6) {
                refresh_codex(&app);
            }
            tick += 1;
            std::thread::sleep(Duration::from_secs(20));
        }
    });
}

// ---------- Agent activity ----------

fn unseen_events(state: &AppState) -> Vec<AgentEvent> {
    let seen = state.events_seen.load(Ordering::SeqCst);
    agents::events::load()
        .into_iter()
        .filter(|e| e.at > seen)
        .collect()
}

/// Watches the events file written by the agents' hooks.
fn start_event_watcher(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let path = paths::agent_events();
        let mut last_modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        loop {
            std::thread::sleep(Duration::from_millis(700));
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            if modified == last_modified {
                continue;
            }
            last_modified = modified;
            let state = app.state::<AppState>();
            if !state.settings().agent_alerts {
                continue;
            }
            let _ = app.emit("agent-events", unseen_events(&state));
        }
    });
}

// ---------- Dictation ----------

/// True when the widget itself has keyboard focus: dictation must then never
/// paste into it.
fn widget_has_focus(app: &AppHandle, target: PasteTarget) -> bool {
    let Some(window) = main_window(app) else {
        return false;
    };
    #[cfg(windows)]
    if let PasteTarget::Window(target) = target {
        return window
            .hwnd()
            .map(|h| h.0 as isize == target.0)
            .unwrap_or(false);
    }
    let _ = target;
    window.is_focused().unwrap_or(false)
}

fn on_hotkey(app: &AppHandle, pressed: bool) {
    let state = app.state::<AppState>();
    let settings = state.settings();
    if pressed {
        if state.holding.swap(true, Ordering::SeqCst) {
            return; // Key repeat, or the same press seen by two shortcut backends.
        }
        let mut target = dictation::inject::capture_target();
        if widget_has_focus(app, target) {
            target = PasteTarget::CopyOnly;
        }
        state.send(Command::Start {
            target,
            model: settings.model,
        });
    } else if state.holding.swap(false, Ordering::SeqCst) {
        state.send(Command::Stop(dictation::StopOptions {
            language: settings.dictation_language,
            model: settings.model,
            vocabulary: settings.vocabulary,
            auto_enter: settings.auto_enter,
            remove_fillers: settings.remove_fillers,
        }));
    }
}

/// `agentnotch --toggle-dictation`: start on the first call, stop on the next.
/// For desktops where a hold-to-talk shortcut is unavailable, bind this command
/// to any key in the system keyboard settings.
fn toggle_dictation(app: &AppHandle) {
    let holding = app.state::<AppState>().holding.load(Ordering::SeqCst);
    on_hotkey(app, !holding);
}

fn register_hotkey(app: &AppHandle) {
    let state = app.state::<AppState>();
    let hotkey = state.settings().hotkey;
    let shortcuts = app.global_shortcut();
    if let Some(previous) = state.registered_hotkey.lock().unwrap().take() {
        let _ = shortcuts.unregister(previous.as_str());
    }
    match shortcuts.on_shortcut(hotkey.as_str(), |app, _shortcut, event| {
        on_hotkey(app, event.state == ShortcutState::Pressed)
    }) {
        Ok(()) => *state.registered_hotkey.lock().unwrap() = Some(hotkey),
        Err(_) => {
            let _ = app.emit(
                "notice",
                serde_json::json!({ "key": "notice.hotkeyFailed" }),
            );
        }
    }
}

// ---------- Tray ----------

fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let language = app.state::<AppState>().language.lock().unwrap().clone();
    let t = |key: &str| i18n::t(&language, key);
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "toggle", t("tray.toggle"), true, None::<&str>)?,
            &MenuItem::with_id(app, "refresh", t("tray.refresh"), true, None::<&str>)?,
            &MenuItem::with_id(app, "connect", t("tray.connect"), true, None::<&str>)?,
            &CheckMenuItem::with_id(
                app,
                "autostart",
                t("tray.autostart"),
                true,
                autostart,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", t("tray.quit"), true, None::<&str>)?,
        ],
    )
}

fn refresh_tray(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY), build_tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn on_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "toggle" => {
            if let Some(window) = main_window(app) {
                if window.is_visible().unwrap_or(true) {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                }
            }
        }
        "refresh" => {
            let app = app.clone();
            std::thread::spawn(move || {
                refresh_claude(&app);
                refresh_codex(&app);
            });
        }
        "connect" => {
            let _ = connect_claude_inner(app);
        }
        "autostart" => {
            let launcher = app.autolaunch();
            let _ = if launcher.is_enabled().unwrap_or(false) {
                launcher.disable()
            } else {
                launcher.enable()
            };
            refresh_tray(app);
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

// ---------- Commands ----------

#[tauri::command]
fn get_snapshot(state: tauri::State<AppState>) -> Snapshot {
    Snapshot {
        settings: state.settings(),
        expanded: state.expanded.load(Ordering::SeqCst),
        usage: state.usage.lock().unwrap().clone(),
        speech: state.speech.lock().unwrap().clone(),
        transcript: state.transcript.lock().unwrap().clone(),
        history: state.history.lock().unwrap().clone(),
        events: if state.settings().agent_alerts {
            unseen_events(&state)
        } else {
            Vec::new()
        },
    }
}

#[tauri::command]
fn set_expanded(app: AppHandle, expanded: bool) {
    app.state::<AppState>()
        .expanded
        .store(expanded, Ordering::SeqCst);
    apply_dock(&app);
}

#[tauri::command]
fn refresh_usage(app: AppHandle) {
    std::thread::spawn(move || {
        refresh_claude(&app);
        refresh_codex(&app);
    });
}

fn connect_claude_inner(app: &AppHandle) -> Result<(), String> {
    usage::claude::install_bridge().map_err(|e| e.to_string())?;
    let _ = app.emit(
        "notice",
        serde_json::json!({ "key": "notice.claudeConnected" }),
    );
    refresh_claude(app);
    Ok(())
}

#[tauri::command]
fn connect_claude(app: AppHandle) -> Result<(), String> {
    connect_claude_inner(&app)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPatch {
    ui_language: Option<String>,
    /// Language the UI resolved "system" to; used for the tray.
    resolved_language: Option<String>,
    dictation_language: Option<String>,
    model: Option<String>,
    limit_alerts: Option<bool>,
    agent_alerts: Option<bool>,
    vocabulary: Option<String>,
    auto_enter: Option<bool>,
    remove_fillers: Option<bool>,
}

#[tauri::command]
fn update_settings(app: AppHandle, patch: SettingsPatch) -> Settings {
    let state = app.state::<AppState>();
    let updated = {
        let mut settings = state.settings.lock().unwrap();
        if let Some(v) = patch
            .ui_language
            .filter(|v| v == "system" || settings::LANGUAGES.contains(&v.as_str()))
        {
            settings.ui_language = v;
        }
        if let Some(v) = patch
            .dictation_language
            .filter(|v| v == "auto" || settings::LANGUAGES.contains(&v.as_str()))
        {
            settings.dictation_language = v;
        }
        if let Some(v) = patch
            .model
            .filter(|v| settings::MODELS.contains(&v.as_str()))
        {
            settings.model = v;
        }
        if let Some(v) = patch.limit_alerts {
            settings.limit_alerts = v;
        }
        if let Some(v) = patch.agent_alerts {
            settings.agent_alerts = v;
        }
        if let Some(v) = patch.vocabulary {
            settings.vocabulary = settings::sanitize_vocabulary(&v);
        }
        if let Some(v) = patch.auto_enter {
            settings.auto_enter = v;
        }
        if let Some(v) = patch.remove_fillers {
            settings.remove_fillers = v;
        }
        settings.save();
        settings.clone()
    };
    if let Some(language) = patch
        .resolved_language
        .filter(|v| settings::LANGUAGES.contains(&v.as_str()))
    {
        *state.language.lock().unwrap() = language;
        refresh_tray(&app);
    }
    updated
}

/// Copies the latest transcript, or an entry of the history by index.
#[tauri::command]
fn copy_transcript(state: tauri::State<AppState>, index: Option<usize>) -> bool {
    let text = match index {
        Some(i) => state.history.lock().unwrap().get(i).cloned(),
        None => Some(state.transcript.lock().unwrap().clone()),
    };
    text.is_some_and(|t| dictation::inject::copy_to_clipboard(&t))
}

#[tauri::command]
fn dismiss_events(state: tauri::State<AppState>, until: i64) {
    state.events_seen.fetch_max(until, Ordering::SeqCst);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentsState {
    claude: agents::config::AgentConfig,
    codex: agents::config::AgentConfig,
}

#[tauri::command]
fn get_agents() -> AgentsState {
    AgentsState {
        claude: agents::config::claude_config(),
        codex: agents::config::codex_config(),
    }
}

/// Default model and effort for new sessions. Empty strings restore the
/// agent's own default.
#[tauri::command]
fn set_agent_defaults(
    agent: String,
    model: Option<String>,
    effort: Option<String>,
) -> Result<AgentsState, String> {
    let result = match agent.as_str() {
        "claude" => agents::config::set_claude_defaults(model.as_deref(), effort.as_deref()),
        "codex" => agents::config::set_codex_defaults(model.as_deref(), effort.as_deref()),
        _ => Err(agents::config::ConfigError::Invalid),
    };
    result.map_err(|e| e.to_string())?;
    Ok(get_agents())
}

/// Installs Claude Code's hooks and Codex's notify command so the widget
/// learns when an agent finishes or needs approval. Returns per-agent errors.
#[tauri::command]
fn connect_agent_alerts() -> Vec<(String, String)> {
    let mut errors = Vec::new();
    if let Err(e) = agents::config::install_claude_hooks() {
        errors.push(("claude".to_owned(), e.to_string()));
    }
    if let Err(e) = agents::config::install_codex_notify() {
        errors.push(("codex".to_owned(), e.to_string()));
    }
    errors
}

// ---------- Setup ----------

/// Clicking the widget must not steal focus from the app you dictate into.
/// `interactive` temporarily allows keyboard focus, e.g. to type settings.
#[cfg(windows)]
fn set_activating(window: &WebviewWindow, interactive: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) | WS_EX_TOOLWINDOW.0 as isize;
            let style = if interactive {
                style & !(WS_EX_NOACTIVATE.0 as isize)
            } else {
                style | WS_EX_NOACTIVATE.0 as isize
            };
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        }
    }
    if interactive {
        let _ = window.set_focus();
    }
}

#[cfg(not(windows))]
fn set_activating(window: &WebviewWindow, interactive: bool) {
    let _ = window.set_focusable(interactive);
    if interactive {
        let _ = window.set_focus();
    }
}

fn make_non_activating(window: &WebviewWindow) {
    set_activating(window, false);
}

#[tauri::command]
fn set_interactive(app: AppHandle, interactive: bool) {
    if let Some(window) = main_window(&app) {
        set_activating(&window, interactive);
    }
}

pub fn run() {
    let settings = Settings::load();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == TOGGLE_FLAG) {
                toggle_dictation(app);
            } else if let Some(window) = main_window(app) {
                let _ = window.show();
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            let handle = app.handle().clone();
            let emitter = handle.clone();
            let sender = dictation::spawn(move |event| {
                let state = emitter.state::<AppState>();
                match &event {
                    SpeechEvent::State(s) => *state.speech.lock().unwrap() = s.clone(),
                    SpeechEvent::Transcript { text } => {
                        *state.transcript.lock().unwrap() = text.clone();
                        let mut history = state.history.lock().unwrap();
                        history.insert(0, text.clone());
                        history.truncate(HISTORY);
                    }
                    SpeechEvent::Notice { .. } => {}
                }
                let _ = emitter.emit("speech", event);
            });
            app.manage(AppState {
                settings: Mutex::new(settings),
                language: Mutex::new("en".into()),
                expanded: AtomicBool::new(false),
                usage: Mutex::new(UsageState::default()),
                speech: Mutex::new(SpeechState::Idle),
                transcript: Mutex::new(String::new()),
                history: Mutex::new(Vec::new()),
                dictation: Mutex::new(sender),
                tracker: Mutex::new(Tracker::default()),
                // Activity from before this launch is not news.
                events_seen: std::sync::atomic::AtomicI64::new(paths::now_millis()),
                move_generation: AtomicU64::new(0),
                holding: AtomicBool::new(false),
                registered_hotkey: Mutex::new(None),
            });

            let menu = build_tray_menu(&handle)?;
            let mut tray = TrayIconBuilder::with_id(TRAY)
                .tooltip("AgentNotch")
                .menu(&menu)
                .on_menu_event(|app, event| on_tray_menu(app, event.id.as_ref()));
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            if let Some(window) = main_window(&handle) {
                make_non_activating(&window);
                let app_for_events = handle.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Moved(_) = event {
                        schedule_snap(&app_for_events);
                    }
                });
                apply_dock(&handle);
                let _ = window.show();
            }
            register_hotkey(&handle);
            #[cfg(target_os = "linux")]
            linux::bind_portal_shortcut(&handle, on_hotkey);
            start_usage_loop(&handle);
            start_event_watcher(&handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            set_expanded,
            refresh_usage,
            connect_claude,
            update_settings,
            copy_transcript,
            dismiss_events,
            get_agents,
            set_agent_defaults,
            connect_agent_alerts,
            set_interactive
        ])
        .run(tauri::generate_context!())
        .expect("error while running AgentNotch");
}
