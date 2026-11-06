//! Best-effort browser URL lookup via UI Automation (address-bar Edit control).
//! Only called for known browsers. Side effect: Chromium enables its accessibility tree once queried.
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::*;

const BROWSERS: &[&str] = &[
    "chrome.exe", "msedge.exe", "brave.exe", "vivaldi.exe", "opera.exe", "firefox.exe", "arc.exe",
];

pub fn is_browser(process: &str) -> bool {
    BROWSERS.iter().any(|b| process.eq_ignore_ascii_case(b))
}

/// Returns the page URL shown in the browser's address bar, if it can be read.
pub fn browser_url(hwnd: isize, process: &str) -> Option<String> {
    if hwnd == 0 || !is_browser(process) {
        return None;
    }
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let out = read(hwnd);
        if com {
            CoUninitialize();
        }
        out
    }
}

unsafe fn read(hwnd: isize) -> Option<String> {
    unsafe {
        let auto: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let root = auto.ElementFromHandle(HWND(hwnd as *mut _)).ok()?;
        let cond = auto.CreatePropertyCondition(UIA_ControlTypePropertyId, &UIA_EditControlTypeId.0.into()).ok()?;
        // The address bar is the first Edit in tree order (toolbar precedes page content).
        let edit = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
        let pat: IUIAutomationValuePattern = edit.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
        let v = pat.CurrentValue().ok()?.to_string();
        normalize(&v)
    }
}

fn normalize(v: &str) -> Option<String> {
    let v = v.trim();
    if v.is_empty() || v.contains(char::is_whitespace) || !(v.contains('.') || v.contains("://") || v.starts_with("localhost")) {
        return None; // not a URL (e.g. a search box or focused text)
    }
    if v.contains("://") {
        Some(v.to_string())
    } else {
        Some(format!("https://{v}"))
    }
}
