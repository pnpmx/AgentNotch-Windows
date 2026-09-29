//! Linux specifics. Wayland does not let apps position their own windows or
//! grab global keys, so the widget runs through XWayland for docking and uses
//! the XDG GlobalShortcuts portal (KDE Plasma 6, GNOME 48+, Hyprland, ...)
//! for hold-to-talk.

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

/// Must run before GTK initialises, i.e. before `run()`.
/// Set `AGENTNOTCH_NATIVE_WAYLAND=1` to opt out of XWayland.
pub fn prepare_environment() {
    let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if wayland
        && std::env::var_os("AGENTNOTCH_NATIVE_WAYLAND").is_none()
        && std::env::var_os("GDK_BACKEND").is_none()
    {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    // WebKitGTK's DMA-BUF renderer shows a blank window on some GPU drivers.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

const SHORTCUT_ID: &str = "dictate";

/// Registers hold-to-talk with the desktop portal. The desktop may show a
/// dialog the first time asking the user to confirm or change the keys.
/// Failure is silent apart from a notice: the X11 shortcut and the
/// `--toggle-dictation` command remain available.
pub fn bind_portal_shortcut(app: &AppHandle, on_change: fn(&AppHandle, bool)) {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return; // Plain X11: the regular global shortcut works.
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_portal(&app, on_change).await {
            eprintln!("AgentNotch: GlobalShortcuts portal unavailable: {error}");
            let _ = app.emit(
                "notice",
                serde_json::json!({ "key": "notice.portalUnavailable" }),
            );
        }
    });
}

async fn run_portal(app: &AppHandle, on_change: fn(&AppHandle, bool)) -> ashpd::Result<()> {
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
    use ashpd::desktop::CreateSessionOptions;

    let portal = GlobalShortcuts::new().await?;
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await?;
    let shortcut = NewShortcut::new(SHORTCUT_ID, "Hold to dictate (AgentNotch)")
        .preferred_trigger("CTRL+SHIFT+space");
    portal
        .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
        .await?
        .response()?;

    let mut activated = portal.receive_activated().await?;
    let mut deactivated = portal.receive_deactivated().await?;
    loop {
        tokio::select! {
            Some(event) = activated.next() => {
                if event.shortcut_id() == SHORTCUT_ID {
                    on_change(app, true);
                }
            }
            Some(event) = deactivated.next() => {
                if event.shortcut_id() == SHORTCUT_ID {
                    on_change(app, false);
                }
            }
            else => break,
        }
    }
    // Keep the session alive for as long as the streams run.
    drop(session);
    Ok(())
}
