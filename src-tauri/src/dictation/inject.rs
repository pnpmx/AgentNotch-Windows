//! Pastes dictated text into the window that was focused when the hotkey was
//! pressed, via the clipboard and a synthetic Ctrl+V. The previous clipboard
//! text or image is restored afterwards unless something else copied in between.

use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteResult {
    Attempted,
    DestinationChanged,
    Unavailable,
}

/// Opaque handle of the focused top-level window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target(pub isize);

/// Where dictated text may be pasted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteTarget {
    /// A known window; paste only if it is still focused (Windows).
    Window(Target),
    /// The focused window cannot be identified (Wayland), so paste into
    /// whatever has focus when transcription finishes.
    Unverified,
    /// Never paste, e.g. the widget itself had focus. Copy instead.
    CopyOnly,
}

#[cfg(windows)]
pub fn foreground() -> Option<Target> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_invalid()).then_some(Target(hwnd.0 as isize))
}

#[cfg(not(windows))]
pub fn foreground() -> Option<Target> {
    None
}

/// The paste target to record when the shortcut is pressed.
pub fn capture_target() -> PasteTarget {
    match foreground() {
        Some(target) => PasteTarget::Window(target),
        None if cfg!(target_os = "linux") => PasteTarget::Unverified,
        None => PasteTarget::CopyOnly,
    }
}

/// On Linux the clipboard is served by its owner process, so contents vanish
/// when the owning `Clipboard` is dropped. Keep one alive for the app's life.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with_clipboard<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    let mut guard = CLIPBOARD.lock().ok()?;
    if guard.is_none() {
        *guard = arboard::Clipboard::new().ok();
    }
    f(guard.as_mut()?)
}

enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
    Nothing,
}

pub fn copy_to_clipboard(text: &str) -> bool {
    with_clipboard(|c| c.set_text(text.to_owned()).ok()).is_some()
}

pub fn paste(text: &str, target: PasteTarget) -> PasteResult {
    if text.is_empty() {
        return PasteResult::Unavailable;
    }
    match target {
        PasteTarget::CopyOnly => return PasteResult::Unavailable,
        PasteTarget::Window(window) if foreground() != Some(window) => {
            return PasteResult::DestinationChanged
        }
        _ => {}
    }
    let Some(saved) = with_clipboard(|c| {
        let saved = match c.get_text() {
            Ok(t) => Saved::Text(t),
            Err(_) => c
                .get_image()
                .map(|i| Saved::Image(i.to_owned_img()))
                .unwrap_or(Saved::Nothing),
        };
        c.set_text(text.to_owned()).ok().map(|_| saved)
    }) else {
        return PasteResult::Unavailable;
    };
    if !send_ctrl_v() {
        return PasteResult::Unavailable;
    }
    let ours = text.to_owned();
    std::thread::spawn(move || {
        // Give the target time to read the clipboard before restoring it.
        std::thread::sleep(Duration::from_millis(900));
        with_clipboard(|c| {
            if c.get_text().ok().as_deref() != Some(ours.as_str()) {
                return None; // The user copied something else meanwhile; keep it.
            }
            match saved {
                Saved::Text(t) => c.set_text(t).ok(),
                Saved::Image(i) => c.set_image(i).ok(),
                Saved::Nothing => c.clear().ok(),
            }
        });
    });
    PasteResult::Attempted
}

#[cfg(windows)]
fn send_ctrl_v() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    // The hotkey's modifiers may still be physically held; release them
    // logically so the target sees plain Ctrl+V rather than Ctrl+Shift+V.
    let held: Vec<VIRTUAL_KEY> = [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .filter(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0)
        .collect();

    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let v = VIRTUAL_KEY(0x56);
    let mut inputs: Vec<INPUT> = held.iter().map(|vk| key(*vk, true)).collect();
    inputs.extend([
        key(VK_CONTROL, false),
        key(v, false),
        key(v, true),
        key(VK_CONTROL, true),
    ]);
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

/// Wayland does not let ordinary apps synthesize input, so use whichever
/// helper the user has installed: ydotool (any compositor, needs ydotoold),
/// wtype (wlroots compositors such as Sway or Hyprland) or xdotool (X11).
#[cfg(target_os = "linux")]
fn send_ctrl_v() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    // Linux input event codes: 29 = KEY_LEFTCTRL, 47 = KEY_V.
    let attempts: &[(&str, &[&str])] = &[
        ("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]),
        ("wtype", &["-M", "ctrl", "v", "-m", "ctrl"]),
        ("xdotool", &["key", "--clearmodifiers", "ctrl+v"]),
    ];
    attempts
        .iter()
        .filter(|(tool, _)| wayland || *tool == "xdotool")
        .any(|(tool, args)| {
            std::process::Command::new(tool)
                .args(*args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn send_ctrl_v() -> bool {
    false
}
