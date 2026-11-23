//! Video posters: the first non-black frame of a video, read with Media Foundation's source reader (the same
//! decoders Windows uses; no extra dependency). The gallery shows the poster as a plain picture, so opening it
//! never spins up a video pipeline per card.

use std::path::Path;

use anyhow::{bail, Context, Result};
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// Extensions Media Foundation can usually open.
pub fn is_video_ext(ext: &str) -> bool {
    matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "m4v" | "mov" | "wmv" | "avi" | "mkv")
}

/// Decodes a frame near the start of `path` (skipping black lead-in frames) as RGB.
pub fn video_frame(path: &Path) -> Result<image::RgbImage> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED); // already initialised (any model) is fine
        MFStartup(MF_VERSION, MFSTARTUP_LITE).context("MFStartup")?;
        let r = read_frame(path);
        let _ = MFShutdown();
        r
    }
}

unsafe fn read_frame(path: &Path) -> Result<image::RgbImage> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("attributes")?;
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?; // lets the reader convert to RGB32
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attrs).context("open video")?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
        reader.SetStreamSelection(stream, true)?;
        let mt = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(stream, None, &mt).context("RGB32 output")?;
        let cur = reader.GetCurrentMediaType(stream)?;
        let size = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (w, h) = ((size >> 32) as u32, (size & 0xFFFF_FFFF) as u32);
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            bail!("bad frame size {w}x{h}");
        }
        let stride = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or((w * 4) as i32);

        let mut best: Option<image::RgbImage> = None;
        for _ in 0..12 {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else { continue };
            let buf = sample.ConvertToContiguousBuffer()?;
            let mut p = std::ptr::null_mut();
            let mut len = 0u32;
            buf.Lock(&mut p, None, Some(&mut len))?;
            let row = (w * 4) as usize;
            let need = row * h as usize;
            let img = if (len as usize) >= need {
                let data = std::slice::from_raw_parts(p, len as usize);
                let mut out = image::RgbImage::new(w, h);
                for y in 0..h as usize {
                    // a negative stride means bottom-up rows
                    let src_y = if stride < 0 { h as usize - 1 - y } else { y };
                    let src = &data[src_y * row..src_y * row + row];
                    let dst = &mut out.as_mut()[y * w as usize * 3..(y + 1) * w as usize * 3];
                    for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
                        d.copy_from_slice(&[s[2], s[1], s[0]]); // BGRX -> RGB
                    }
                }
                Some(out)
            } else {
                None
            };
            let _ = buf.Unlock();
            let Some(img) = img else { continue };
            let lit = brightness(&img) > 14.0;
            best = Some(img);
            if lit {
                break; // a fade-in from black would make a black poster
            }
        }
        best.context("no video frame")
    }
}

/// Mean brightness (0-255) of a coarse sample of pixels.
fn brightness(img: &image::RgbImage) -> f32 {
    let (mut sum, mut n) = (0u64, 0u64);
    for p in img.pixels().step_by(97) {
        sum += (p[0] as u64 * 3 + p[1] as u64 * 6 + p[2] as u64) / 10;
        n += 1;
    }
    sum as f32 / n.max(1) as f32
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore] // STASH_VIDEO=<file.mp4> cargo test poster_smoke -- --ignored --nocapture
    fn poster_smoke() {
        let p = std::env::var("STASH_VIDEO").expect("STASH_VIDEO");
        let t = std::time::Instant::now();
        let img = super::video_frame(std::path::Path::new(&p)).expect("frame");
        println!("{}x{} in {} ms", img.width(), img.height(), t.elapsed().as_millis());
        img.save(std::env::temp_dir().join("stash_poster_test.png")).unwrap();
    }
}
