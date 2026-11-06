//! Local text recognition with the OCR engine built into Windows (no network, no bundled model).
use std::path::Path;

use anyhow::{anyhow, Result};
use tauri::{AppHandle, Manager};
use windows::core::HSTRING;
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Media::Ocr::OcrEngine;
use windows::Storage::{FileAccessMode, StorageFile};

use crate::{library, AppState};

/// Recognises text in an image file. Runs blocking WinRT calls: call from a worker thread.
pub fn recognize(path: &Path) -> Result<String> {
    let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s); // WinRT rejects the verbatim prefix
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(s))?.join()?;
    let stream = file.OpenAsync(FileAccessMode::Read)?.join()?;
    let dec = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| anyhow!("no OCR language installed"))?;
    let max = OcrEngine::MaxImageDimension()?;
    if dec.PixelWidth()? > max || dec.PixelHeight()? > max {
        return Ok(String::new());
    }
    let bmp = dec.GetSoftwareBitmapAsync()?.join()?;
    let res = engine.RecognizeAsync(&bmp)?.join()?;
    Ok(res.Text()?.to_string())
}

/// After a capture is saved: OCR it in the background and store the text in the sidecar.
pub fn spawn(app: &AppHandle, sidecar: std::path::PathBuf) {
    if !app.state::<AppState>().cfg().ocr {
        return;
    }
    std::thread::spawn(move || {
        let Ok(v) = library::read_json(&sidecar) else { return };
        if v["type"] != "image" {
            return;
        }
        let (Some(name), Some(dir)) = (v["file"]["name"].as_str(), sidecar.parent()) else { return };
        let text = recognize(&dir.join(name)).unwrap_or_default();
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let _ = library::update_sidecar(&sidecar, |v| v["ocr"] = serde_json::Value::String(text));
    });
}
