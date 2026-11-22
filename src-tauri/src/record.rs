//! Video clip recording: screen region -> H.264/AAC MP4 through Windows' built-in Media Foundation (no ffmpeg, no
//! extra crates). Nothing runs while idle: the whole pipeline lives on one thread that exists only while a clip
//! is being recorded. Frames come from GDI `BitBlt`; optional audio is WASAPI loopback (what you hear) and/or the
//! default microphone, mixed to 48 kHz stereo.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM, LRESULT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::winutil::{MonitorDesc, SourceInfo};
use crate::{library, tray, AppState};

const RATE: u32 = 48_000;
static STOP: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);


/// Stops the running recording. Returns false when nothing was recording.
pub fn stop() -> bool {
    match STOP.lock().unwrap().as_ref() {
        Some(s) => {
            s.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

pub struct Recorded {
    pub w: u32,
    pub h: u32,
    pub secs: f64,
    /// First frame, top-down RGB (feedback thumbnail).
    pub first: Vec<u8>,
}

/// Starts recording `rect` (virtual-desktop pixels) on a new thread; saves the clip into the library when stopped.
pub fn start(app: &AppHandle, rect: RECT, source: SourceInfo, monitor: MonitorDesc) {
    let stop_flag = Arc::new(AtomicBool::new(false));
    {
        let mut g = STOP.lock().unwrap();
        if g.is_some() {
            return;
        }
        *g = Some(stop_flag.clone());
    }
    let app = app.clone();
    let cfg = app.state::<AppState>().cfg();
    let hotkey = cfg.hotkey.clone();
    tray::set_status(&app, &format!("● Recording — press {hotkey} to stop"));
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(160)); // let the selection overlay disappear
        let tmp = crate::config::app_dir().join("recording.tmp.mp4");
        let t0 = Instant::now();
        let res = record_to(&tmp, rect, cfg.rec_fps, cfg.rec_max_secs, cfg.rec_system_audio, cfg.rec_mic, &stop_flag);
        *STOP.lock().unwrap() = None;
        tray::set_status(&app, &format!("Capture: {hotkey}"));
        match res {
            Ok(rec) => {
                tracing::info!("recorded {}x{} {:.1}s in {} ms wall", rec.w, rec.h, rec.secs, t0.elapsed().as_millis());
                match library::save_video(&app.state::<AppState>().cfg().library_path, &tmp, &rec, &source, &monitor) {
                    Ok(saved) => crate::radial::show(
                        &app,
                        crate::radial::Request { sidecars: vec![saved.sidecar], preview: saved.preview, meta: saved.meta, source_hwnd: source.hwnd, frame: None },
                    ),
                    Err(e) => {
                        tracing::error!("save clip: {e:#}");
                        crate::toast::notice(&app, "Saving clip failed — see log");
                    }
                }
            }
            Err(e) => {
                tracing::error!("record: {e:#}");
                crate::toast::notice(&app, "Recording failed — see log");
            }
        }
        let _ = std::fs::remove_file(&tmp);
    });
}

// ---------- red frame around the recorded region (excluded from capture, never inside the region) ----------

struct Border([HWND; 4]);

impl Border {
    fn new(r: RECT) -> Border {
        unsafe {
            let hinst = GetModuleHandleW(None).ok();
            let class = w!("StashRecBorder");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(border_proc),
                hInstance: hinst.map(Into::into).unwrap_or_default(),
                lpszClassName: class,
                hbrBackground: CreateSolidBrush(COLORREF(0x003c3cff)), // BGR: red
                ..Default::default()
            };
            RegisterClassW(&wc);
            let t = 3;
            let strips = [
                (r.left - t, r.top - t, r.right - r.left + 2 * t, t),
                (r.left - t, r.bottom, r.right - r.left + 2 * t, t),
                (r.left - t, r.top, t, r.bottom - r.top),
                (r.right, r.top, t, r.bottom - r.top),
            ];
            let mut hs = [HWND::default(); 4];
            for (i, (x, y, w, h)) in strips.into_iter().enumerate() {
                if let Ok(h_) = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                    class,
                    w!(""),
                    WS_POPUP,
                    x,
                    y,
                    w,
                    h,
                    None,
                    None,
                    hinst.map(Into::into),
                    None,
                ) {
                    let _ = SetWindowDisplayAffinity(h_, WINDOW_DISPLAY_AFFINITY(0x11)); // WDA_EXCLUDEFROMCAPTURE
                    let _ = ShowWindow(h_, SW_SHOWNOACTIVATE);
                    hs[i] = h_;
                }
            }
            Border(hs)
        }
    }
    fn pump(&self) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for Border {
    fn drop(&mut self) {
        for h in self.0 {
            if !h.0.is_null() {
                unsafe {
                    let _ = DestroyWindow(h);
                }
            }
        }
    }
}

unsafe extern "system" fn border_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

// ---------- audio ----------

struct AudioIn {
    client: IAudioClient,
    cap: IAudioCaptureClient,
    rate: f64,
    ch: usize,
    bits: u16,
    float: bool,
    inbuf: Vec<[f32; 2]>,
    pos: f64,
    q: VecDeque<f32>, // interleaved stereo @ 48 kHz
}

impl AudioIn {
    fn open(loopback: bool) -> Result<AudioIn> {
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let dev = en.GetDefaultAudioEndpoint(if loopback { eRender } else { eCapture }, eConsole)?;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
            let fmt = client.GetMixFormat()?;
            let f = *fmt;
            let (rate, ch, bits, tag) = (f.nSamplesPerSec, f.nChannels as usize, f.wBitsPerSample, f.wFormatTag);
            let float = tag == 3 || (tag == 0xFFFE && bits == 32);
            let flags = if loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
            client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 10_000_000, 0, fmt, None)?;
            CoTaskMemFree(Some(fmt as *const _));
            let cap: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            Ok(AudioIn { client, cap, rate: rate as f64, ch: ch.max(1), bits, float, inbuf: Vec::new(), pos: 0.0, q: VecDeque::new() })
        }
    }

    /// Drains pending packets into the 48 kHz stereo queue.
    fn pump(&mut self) {
        unsafe {
            while let Ok(n) = self.cap.GetNextPacketSize() {
                if n == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                if self.cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                    break;
                }
                let silent = flags & 0x2 != 0; // AUDCLNT_BUFFERFLAGS_SILENT
                let bytes_per = (self.bits as usize / 8).max(1);
                for i in 0..frames as usize {
                    let mut s = [0f32; 2];
                    if !silent && !data.is_null() {
                        for c in 0..2.min(self.ch) {
                            let p = data.add((i * self.ch + c) * bytes_per);
                            s[c] = if self.float && bytes_per == 4 {
                                (p as *const f32).read_unaligned()
                            } else if bytes_per == 2 {
                                (p as *const i16).read_unaligned() as f32 / 32768.0
                            } else if bytes_per == 4 {
                                (p as *const i32).read_unaligned() as f32 / 2_147_483_648.0
                            } else {
                                0.0
                            };
                        }
                        if self.ch == 1 {
                            s[1] = s[0];
                        }
                    }
                    self.inbuf.push(s);
                }
                let _ = self.cap.ReleaseBuffer(frames);
            }
        }
        let step = self.rate / RATE as f64;
        while self.pos + 1.0 < self.inbuf.len() as f64 {
            let i = self.pos as usize;
            let f = (self.pos - i as f64) as f32;
            let (a, b) = (self.inbuf[i], self.inbuf[i + 1]);
            self.q.push_back(a[0] + (b[0] - a[0]) * f);
            self.q.push_back(a[1] + (b[1] - a[1]) * f);
            self.pos += step;
        }
        let used = self.pos as usize;
        self.inbuf.drain(..used.min(self.inbuf.len()));
        self.pos -= used as f64;
        let max = RATE as usize * 2; // never lag more than 1 s behind the wall clock
        if self.q.len() > max {
            let extra = self.q.len() - max;
            self.q.drain(..extra);
        }
    }
}

impl Drop for AudioIn {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

// ---------- Media Foundation helpers ----------

unsafe fn video_types(w: u32, h: u32, fps: u32) -> Result<(IMFMediaType, IMFMediaType)> {
    unsafe {
        let bitrate = ((w as u64 * h as u64 * fps as u64) / 8).clamp(1_000_000, 24_000_000) as u32;
        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        set_pair(&out, &MF_MT_FRAME_SIZE, w, h)?;
        set_pair(&out, &MF_MT_FRAME_RATE, fps, 1)?;
        set_pair(&out, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
        let inp = MFCreateMediaType()?;
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        set_pair(&inp, &MF_MT_FRAME_SIZE, w, h)?;
        set_pair(&inp, &MF_MT_FRAME_RATE, fps, 1)?;
        set_pair(&inp, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
        Ok((out, inp))
    }
}

unsafe fn audio_types() -> Result<(IMFMediaType, IMFMediaType)> {
    unsafe {
        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
        out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 20_000)?; // 160 kbit/s
        out.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 1)?;
        let inp = MFCreateMediaType()?;
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        inp.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        inp.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
        inp.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        inp.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
        inp.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, RATE * 4)?;
        Ok((out, inp))
    }
}

unsafe fn write_sample(w: &IMFSinkWriter, stream: u32, data: &[u8], time_100ns: i64, dur_100ns: i64) -> Result<()> {
    unsafe {
        let buf = MFCreateMemoryBuffer(data.len() as u32)?;
        let mut p: *mut u8 = std::ptr::null_mut();
        buf.Lock(&mut p, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        buf.Unlock()?;
        buf.SetCurrentLength(data.len() as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buf)?;
        sample.SetSampleTime(time_100ns)?;
        sample.SetSampleDuration(dur_100ns)?;
        w.WriteSample(stream, &sample)?;
        Ok(())
    }
}

/// Records until `stop` is set (or `max_secs` elapses) into `path`. Blocking; call from a dedicated thread.
pub fn record_to(path: &Path, rect: RECT, fps: u32, max_secs: u32, sys_audio: bool, mic: bool, stop: &AtomicBool) -> Result<Recorded> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup")?;
        let r = record_inner(path, rect, fps.clamp(5, 60), max_secs, sys_audio, mic, stop);
        let _ = MFShutdown();
        r
    }
}

unsafe fn record_inner(path: &Path, rect: RECT, fps: u32, max_secs: u32, sys_audio: bool, mic: bool, stop: &AtomicBool) -> Result<Recorded> {
    unsafe {
        let (x, y) = (rect.left, rect.top);
        let w = ((rect.right - rect.left).max(16) as u32) & !1; // H.264 needs even sizes
        let h = ((rect.bottom - rect.top).max(16) as u32) & !1;

        let _ = std::fs::remove_file(path);
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 2)?;
        let attrs = attrs.ok_or_else(|| anyhow!("no attributes"))?;
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        let writer = MFCreateSinkWriterFromURL(PCWSTR(wide.as_ptr()), None, &attrs).context("create MP4 writer")?;

        let (vout, vin) = video_types(w, h, fps)?;
        let vstream = writer.AddStream(&vout)?;
        writer.SetInputMediaType(vstream, &vin, None).context("H.264 encoder unavailable")?;

        let mut sources: Vec<AudioIn> = Vec::new();
        if sys_audio {
            match AudioIn::open(true) {
                Ok(a) => sources.push(a),
                Err(e) => tracing::warn!("system audio unavailable: {e}"),
            }
        }
        if mic {
            match AudioIn::open(false) {
                Ok(a) => sources.push(a),
                Err(e) => tracing::warn!("microphone unavailable: {e}"),
            }
        }
        let astream = if sources.is_empty() {
            None
        } else {
            let (aout, ain) = audio_types()?;
            let s = writer.AddStream(&aout)?;
            writer.SetInputMediaType(s, &ain, None).context("AAC encoder unavailable")?;
            Some(s)
        };
        writer.BeginWriting()?;

        // GDI capture surface (bottom-up 32-bit DIB = Media Foundation's RGB32 layout)
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: h as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(mem, dib.into());
        let frame_len = (w * h * 4) as usize;

        let border = Border::new(RECT { left: x, top: y, right: x + w as i32, bottom: y + h as i32 });
        let start = Instant::now();
        let frame_dur = Duration::from_secs_f64(1.0 / fps as f64);
        let mut next = start;
        let mut last_t: i64 = -1;
        let mut audio_written: u64 = 0;
        let mut first: Vec<u8> = Vec::new();
        let mut frames = 0u64;

        loop {
            let elapsed = start.elapsed();
            if stop.load(Ordering::SeqCst) || (max_secs > 0 && elapsed.as_secs() >= max_secs as u64) {
                break;
            }
            let _ = BitBlt(mem, 0, 0, w as i32, h as i32, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
            // mouse cursor
            let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
            if GetCursorInfo(&mut ci).is_ok() && ci.flags.0 & 1 != 0 {
                let _ = DrawIconEx(mem, ci.ptScreenPos.x - x - 8, ci.ptScreenPos.y - y - 8, HICON(ci.hCursor.0), 0, 0, 0, None, DI_NORMAL);
            }
            let data = std::slice::from_raw_parts(bits as *const u8, frame_len);
            let t = (elapsed.as_nanos() / 100) as i64;
            let t = t.max(last_t + 1);
            last_t = t;
            if first.is_empty() {
                // bottom-up BGRA -> top-down RGB
                first = Vec::with_capacity((w * h * 3) as usize);
                for row in (0..h as usize).rev() {
                    for px in data[row * w as usize * 4..(row + 1) * w as usize * 4].chunks_exact(4) {
                        first.extend_from_slice(&[px[2], px[1], px[0]]);
                    }
                }
            }
            write_sample(&writer, vstream, data, t, frame_dur.as_nanos() as i64 / 100)?;
            frames += 1;

            if let Some(astream) = astream {
                for s in sources.iter_mut() {
                    s.pump();
                }
                let due = (start.elapsed().as_secs_f64() * RATE as f64) as u64;
                let n = due.saturating_sub(audio_written);
                if n >= 480 {
                    let mut pcm = Vec::with_capacity(n as usize * 4);
                    for _ in 0..n * 2 {
                        let mut v = 0f32;
                        for s in sources.iter_mut() {
                            v += s.q.pop_front().unwrap_or(0.0);
                        }
                        pcm.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
                    }
                    let t0 = (audio_written * 10_000_000 / RATE as u64) as i64;
                    let d = (n * 10_000_000 / RATE as u64) as i64;
                    write_sample(&writer, astream, &pcm, t0, d)?;
                    audio_written += n;
                }
            }

            border.pump();
            next += frame_dur;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else if now - next > frame_dur * 3 {
                next = now; // fell behind: don't burst to catch up
            }
        }
        let secs = start.elapsed().as_secs_f64();
        drop(border);
        drop(sources);
        let fin = writer.Finalize();
        SelectObject(mem, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        fin.context("finalize MP4")?;
        if frames == 0 {
            return Err(anyhow!("no frames captured"));
        }
        Ok(Recorded { w, h, secs, first })
    }
}

/// Where a finished clip's temp file lives (exposed for tests).
#[allow(dead_code)]
pub fn temp_path() -> PathBuf {
    crate::config::app_dir().join("recording.tmp.mp4")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records 2 s of the primary screen (silent) and checks a non-trivial MP4 appears. Run explicitly:
    /// `cargo test --manifest-path src-tauri/Cargo.toml record_selftest -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn record_selftest() {
        let path = std::env::temp_dir().join("stash-selftest.mp4");
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2000));
            s2.store(true, Ordering::SeqCst);
        });
        let r = record_to(&path, RECT { left: 0, top: 0, right: 640, bottom: 360 }, 30, 0, true, false, &stop).expect("record");
        let len = std::fs::metadata(&path).unwrap().len();
        println!("{}x{} {:.2}s {} bytes at {}", r.w, r.h, r.secs, len, path.display());
        assert!(len > 10_000);
    }
}

/// `MFSetAttributeSize` / `MFSetAttributeRatio` are C macros, not exported functions: pack two u32 into a u64.
unsafe fn set_pair(a: &IMFMediaType, key: &windows::core::GUID, hi: u32, lo: u32) -> windows::core::Result<()> {
    unsafe { a.SetUINT64(key, ((hi as u64) << 32) | lo as u64) }
}
