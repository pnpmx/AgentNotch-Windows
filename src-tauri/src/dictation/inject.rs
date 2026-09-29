//! Pastes dictated text into the window that was focused when the hotkey was
//! pressed, via the clipboard and a synthetic Ctrl+V. The previous clipboard
//! text or image is restored afterwards unless something else copied in between.

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

enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
    Nothing,
}

pub fn copy_to_clipboard(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_owned()))
        .is_ok()
}

pub fn paste(text: &str, target: Option<Target>) -> PasteResult {
    let Some(target) = target else {
        return PasteResult::Unavailable;
    };
    if text.is_empty() {
        return PasteResult::Unavailable;
    }
    if foreground() != Some(target) {
        return PasteResult::DestinationChanged;
    }
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return PasteResult::Unavailable;
    };
    let saved = match clipboard.get_text() {
        Ok(t) => Saved::Text(t),
        Err(_) => clipboard
            .get_image()
            .map(|i| Saved::Image(i.to_owned_img()))
            .unwrap_or(Saved::Nothing),
    };
    if clipboard.set_text(text.to_owned()).is_err() {
        return PasteResult::Unavailable;
    }
    if !send_ctrl_v() {
        return PasteResult::Unavailable;
    }
    let ours = text.to_owned();
    std::thread::spawn(move || {
        // Give the target time to read the clipboard before restoring it.
        std::thread::sleep(Duration::from_millis(900));
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return;
        };
        if clipboard.get_text().ok().as_deref() != Some(ours.as_str()) {
            return; // The user copied something else meanwhile; keep it.
        }
        let _ = match saved {
            Saved::Text(t) => clipboard.set_text(t),
            Saved::Image(i) => clipboard.set_image(i),
            Saved::Nothing => clipboard.clear(),
        };
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

#[cfg(not(windows))]
fn send_ctrl_v() -> bool {
    false
}
