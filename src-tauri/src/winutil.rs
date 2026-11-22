//! Small Win32 helpers: foreground-window source info, monitor info, local capture of the screen.
use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;

use anyhow::{bail, Result};
use serde::Serialize;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What the user was looking at when they pressed the hotkey.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SourceInfo {
    pub title: String,
    pub process: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Foreground window at capture time (not saved): focus goes back to it when a Stash input closes.
    #[serde(skip)]
    pub hwnd: isize,
}

/// Foreground window at hotkey time: metadata plus what is needed for window capture / URL lookup.
pub struct Foreground {
    pub source: SourceInfo,
    pub hwnd: isize,
    #[allow(dead_code)]
    pub rect: Option<RECT>,
}

pub fn foreground_info() -> Foreground {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Foreground { source: SourceInfo::default(), hwnd: 0, rect: None };
        }
        let mut rect = RECT::default();
        let rect = if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut c_void, size_of::<RECT>() as u32).is_ok()
            || GetWindowRect(hwnd, &mut rect).is_ok()
        {
            Some(rect)
        } else {
            None
        };
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut exe_path = None;
        if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut pbuf = [0u16; 1024];
            let mut len = pbuf.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(pbuf.as_mut_ptr()), &mut len).is_ok() {
                exe_path = Some(String::from_utf16_lossy(&pbuf[..len as usize]));
            }
            let _ = CloseHandle(h);
        }
        let process = exe_path
            .as_deref()
            .and_then(|p| Path::new(p).file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Foreground { source: SourceInfo { title, process, exe_path, url: None, hwnd: hwnd.0 as isize }, hwnd: hwnd.0 as isize, rect }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MonitorDesc {
    pub name: String,
    pub scale: f32,
}

/// Monitor containing the given virtual-screen point.
pub fn monitor_at(x: i32, y: i32) -> MonitorDesc {
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let mut name = String::new();
        if GetMonitorInfoW(mon, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
            let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            name = String::from_utf16_lossy(&info.szDevice[..end]);
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        MonitorDesc { name, scale: dx as f32 / 96.0 }
    }
}

/// A frozen copy of the whole virtual desktop (all monitors), top-down BGRA.
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub bgra: Vec<u8>,
}

pub fn grab_virtual_screen() -> Result<Frame> {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        grab_rect(RECT { left: x, top: y, right: x + w, bottom: y + h }, false)
    }
}

/// Copies a virtual-desktop rect of the screen. CAPTUREBLT is off by default: with DWM composition layered
/// windows are already in the screen image, and the flag made ~1 in 10 grabs take 1-4 s (measured, SPEC §8).
pub fn grab_rect(r: RECT, captureblt: bool) -> Result<Frame> {
    unsafe {
        let (x, y, w, h) = (r.left, r.top, r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 {
            bail!("empty grab rect");
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = match CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(e) => {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return Err(e.into());
            }
        };
        let old = SelectObject(mem, bmp.into());
        let rop = if captureblt { SRCCOPY | CAPTUREBLT } else { SRCCOPY };
        let res = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, rop);
        let mut bgra = Vec::new();
        if res.is_ok() && !bits.is_null() {
            bgra = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        res?;
        Ok(Frame { x, y, w, h, bgra })
    }
}

impl Frame {
    /// Crop to a virtual-desktop rect and return tightly packed RGBA.
    pub fn crop_rgba(&self, r: RECT) -> (Vec<u8>, u32, u32) {
        let l = (r.left - self.x).clamp(0, self.w);
        let t = (r.top - self.y).clamp(0, self.h);
        let rr = (r.right - self.x).clamp(0, self.w);
        let b = (r.bottom - self.y).clamp(0, self.h);
        let (cw, ch) = ((rr - l).max(0) as usize, (b - t).max(0) as usize);
        let mut out = Vec::with_capacity(cw * ch * 4);
        for row in t..t + ch as i32 {
            let start = ((row * self.w + l) * 4) as usize;
            for px in self.bgra[start..start + cw * 4].chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        (out, cw as u32, ch as u32)
    }
}

/// Full bounds of the monitor containing the point.
pub fn monitor_rect_at(x: i32, y: i32) -> RECT {
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        info.rcMonitor
    }
}

/// Work area (screen minus taskbar) of the primary monitor, in physical pixels, and its DPI scale.
pub fn primary_work_area() -> (RECT, f32) {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (info.rcWork, dx as f32 / 96.0)
    }
}

/// Brings a window to the front and gives it focus, working around the foreground lock that makes a plain
/// SetForegroundWindow from a background app only flash the taskbar button. Falls back to raising it visually.
pub fn bring_to_front(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    if hwnd == 0 {
        return;
    }
    unsafe {
        let h = HWND(hwnd as *mut c_void);
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }
        let fg = GetForegroundWindow();
        let fg_thread = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let me = GetCurrentThreadId();
        let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(h);
        let ok = SetForegroundWindow(h).as_bool();
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        if !ok {
            // at least put it above other windows without making it permanently topmost
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
            let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
            let _ = SetWindowPos(h, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
        }
    }
}

/// Gives focus back to the window the user was in (after a Stash input window closes).
pub fn restore_focus(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    if hwnd == 0 {
        return;
    }
    unsafe {
        let h = HWND(hwnd as *mut c_void);
        if IsWindow(Some(h)).as_bool() {
            let _ = SetForegroundWindow(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(mut v: Vec<u128>) -> String {
        v.sort();
        format!("median {} ms, p90 {} ms, max {} ms", v[v.len() / 2], v[v.len() * 9 / 10], v[v.len() - 1])
    }

    /// Full-desktop grab timing with and without CAPTUREBLT. Run: cargo test grab_bench -- --ignored --nocapture
    #[test]
    #[ignore]
    fn grab_bench() {
        let r = unsafe {
            let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            RECT { left: x, top: y, right: x + GetSystemMetrics(SM_CXVIRTUALSCREEN), bottom: y + GetSystemMetrics(SM_CYVIRTUALSCREEN) }
        };
        for flag in [true, false] {
            let mut t = Vec::new();
            for _ in 0..40 {
                let t0 = std::time::Instant::now();
                grab_rect(r, flag).unwrap();
                t.push(t0.elapsed().as_millis());
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
            println!("CAPTUREBLT={flag}: {}", stats(t));
        }
    }
}
