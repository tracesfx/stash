//! Clipboard access and "stash selection" (send Ctrl+C to the foreground app, read, restore).
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

pub enum Clip {
    Text(String),
    Image { rgba: Vec<u8>, w: u32, h: u32 },
    Files(Vec<PathBuf>),
}

/// Reads the clipboard: files first, then image, then text.
pub fn read() -> Option<Clip> {
    let mut cb = arboard::Clipboard::new().ok()?;
    if let Ok(files) = cb.get().file_list() {
        if !files.is_empty() {
            return Some(Clip::Files(files));
        }
    }
    if let Ok(img) = cb.get_image() {
        return Some(Clip::Image { rgba: img.bytes.into_owned(), w: img.width as u32, h: img.height as u32 });
    }
    match cb.get_text() {
        Ok(t) if !t.trim().is_empty() => Some(Clip::Text(t)),
        _ => None,
    }
}

fn restore(prev: Option<Clip>) {
    let Ok(mut cb) = arboard::Clipboard::new() else { return };
    match prev {
        Some(Clip::Text(t)) => {
            let _ = cb.set_text(t);
        }
        Some(Clip::Image { rgba, w, h }) => {
            let _ = cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: rgba.into() });
        }
        // File lists can't be restored via arboard; leave as is. Empty clipboard stays empty.
        Some(Clip::Files(_)) => {}
        None => {
            let _ = cb.clear();
        }
    }
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

fn send(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                ..Default::default()
            },
        },
    }
}

/// Copies the foreground app's current selection. Returns None if nothing was selected.
/// Restores the previous clipboard text/image afterwards (other formats are not preserved).
pub fn copy_selection() -> Result<Option<Clip>> {
    // The user is probably still releasing Ctrl/Alt/Shift from the hotkey; a held Alt would turn our Ctrl+C into Ctrl+Alt+C.
    let t0 = Instant::now();
    while [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN].iter().any(|k| key_down(*k)) {
        if t0.elapsed() > Duration::from_millis(1500) {
            return Err(anyhow!("modifier keys still held"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    tracing::info!("selection copy: modifiers released after {} ms", t0.elapsed().as_millis());
    let prev = read();
    let seq = unsafe { GetClipboardSequenceNumber() };
    let c = VK_CONTROL;
    let key_c = VIRTUAL_KEY(0x43);
    let inputs = [send(c, false), send(key_c, false), send(key_c, true), send(c, true)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };

    let t1 = Instant::now();
    while unsafe { GetClipboardSequenceNumber() } == seq {
        if t1.elapsed() > Duration::from_millis(600) {
            tracing::info!("selection copy: clipboard did not change within 600 ms");
            return Ok(None); // nothing selected / app did not respond
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(30)); // let the app finish writing all formats
    let got = read();
    restore(prev);
    Ok(got)
}
