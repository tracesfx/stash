//! Adjustable frosted backdrop for the gallery and settings windows.
//!
//! Windows' Acrylic has a fixed blur, so the window glass settings draw their own: a Windows.UI.Composition
//! visual placed behind the (transparent) web view, filled with the host backdrop (what is behind the window,
//! live) run through our own saturation (and extra blur) effect.
//!
//! DWM delivers the host backdrop already blurred (about `HOST_BLUR` px, the Acrylic look) and there is no
//! API for the sharp content short of screen capture, so lower blurs fade that layer out over the sharp desktop
//! (the window itself is transparent): blur 0 = clear, `HOST_BLUR` = fully frosted, above it our own Gaussian
//! blur is stacked on top. Composition objects must live on a thread with a DispatcherQueue, so everything
//! here runs on the main (event loop) thread. If any step fails the caller falls back to Acrylic.
use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{implement, Interface, Result, GUID, HSTRING, PCWSTR};
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{IGraphicsEffect, IGraphicsEffect_Impl, IGraphicsEffectSource, IGraphicsEffectSource_Impl};
use windows::System::DispatcherQueueController;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionEffectSourceParameter, Compositor, SpriteVisual};
use windows::Win32::Foundation::{E_BOUNDS, E_NOTIMPL, HWND};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
};
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};

const CLSID_GAUSSIAN_BLUR: GUID = GUID::from_u128(0x1feb6d69_2fe6_4ac9_8c58_1d7f93e7a6a5);
const CLSID_COLOR_MATRIX: GUID = GUID::from_u128(0x921f03d6_641c_47df_852d_b4bb6153ae11);

/// Approximate blur (Gaussian std dev, px) DWM has already applied to the host backdrop.
pub const HOST_BLUR: f32 = 30.0;

/// Opacity of the frosted layer for a blur setting (below HOST_BLUR it fades out over the sharp desktop).
fn layer_opacity(blur: f32) -> f32 {
    (blur / HOST_BLUR).clamp(0.0, 1.0)
}

/// Extra Gaussian blur to stack on the host backdrop so the total reaches `blur` (std devs add in quadrature).
fn extra_blur(blur: f32) -> f32 {
    (blur * blur - HOST_BLUR * HOST_BLUR).max(0.0).sqrt().min(250.0)
}

/// The look of one window's backdrop.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    /// Gaussian standard deviation in px (0 = sharp).
    pub blur: f32, // 0 = sharp
    /// 1.0 = unchanged.
    pub saturation: f32,
}

struct Comp {
    _queue: DispatcherQueueController,
    compositor: Compositor,
}

struct Attached {
    _target: DesktopWindowTarget,
    sprite: SpriteVisual,
    params: Params,
}

thread_local! {
    static COMP: RefCell<Option<Comp>> = const { RefCell::new(None) };
    static WINDOWS: RefCell<HashMap<isize, Attached>> = RefCell::new(HashMap::new());
}

// ---------- a minimal IGraphicsEffect (what Win2D's effect classes provide) ----------

enum Prop {
    F32(f32),
    U32(u32),
    Bool(bool),
    F32s(Vec<f32>),
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct Effect {
    clsid: GUID,
    props: Vec<Prop>,
    sources: Vec<IGraphicsEffectSource>,
}

impl IGraphicsEffect_Impl for Effect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(HSTRING::new())
    }
    fn SetName(&self, _name: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for Effect_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for Effect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(self.clsid)
    }
    fn GetNamedPropertyMapping(&self, _name: &PCWSTR, _index: *mut u32, _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING) -> Result<()> {
        Err(E_NOTIMPL.into()) // no animatable properties: a settings change rebuilds the brush
    }
    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(self.props.len() as u32)
    }
    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        let v = match self.props.get(index as usize).ok_or(windows::core::Error::from(E_BOUNDS))? {
            Prop::F32(v) => PropertyValue::CreateSingle(*v)?,
            Prop::U32(v) => PropertyValue::CreateUInt32(*v)?,
            Prop::Bool(v) => PropertyValue::CreateBoolean(*v)?,
            Prop::F32s(v) => PropertyValue::CreateSingleArray(v)?,
        };
        v.cast()
    }
    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        self.sources.get(index as usize).cloned().ok_or(E_BOUNDS.into())
    }
    fn GetSourceCount(&self) -> Result<u32> {
        Ok(self.sources.len() as u32)
    }
}

fn effect(clsid: GUID, props: Vec<Prop>, source: IGraphicsEffectSource) -> Result<IGraphicsEffectSource> {
    let e: IGraphicsEffect = Effect { clsid, props, sources: vec![source] }.into();
    e.cast()
}

/// Saturation as a 5x4 colour matrix (row-major, rows = input R,G,B,A,offset; Rec.709 luma), which unlike
/// D2D's Saturation effect also goes above 1.
fn saturation_matrix(s: f32) -> Vec<f32> {
    let w = [0.2126f32, 0.7152, 0.0722];
    let mut m = vec![0.0f32; 20];
    for i in 0..3 {
        for j in 0..3 {
            m[i * 4 + j] = (1.0 - s) * w[i] + if i == j { s } else { 0.0 };
        }
    }
    m[15] = 1.0; // alpha passes through
    m
}

// ---------- composition ----------

fn with_compositor<T>(f: impl FnOnce(&Compositor) -> Result<T>) -> Result<T> {
    COMP.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            let opts = DispatcherQueueOptions {
                dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            };
            let queue = unsafe { CreateDispatcherQueueController(opts)? };
            *c = Some(Comp { _queue: queue, compositor: Compositor::new()? });
        }
        f(&c.as_ref().expect("compositor").compositor)
    })
}

fn set_brush(comp: &Compositor, sprite: &SpriteVisual, p: Params) -> Result<()> {
    if cfg!(debug_assertions) {
        match std::env::var("STASH_BACKDROP_DEBUG").as_deref() {
            Ok("color") => return sprite.SetBrush(&comp.CreateColorBrushWithColor(windows::UI::Color { A: 160, R: 255, G: 0, B: 0 })?),
            Ok("raw") => return sprite.SetBrush(&comp.CreateHostBackdropBrush()?), // what DWM provides
            _ => {}
        }
    }
    let name = HSTRING::from("backdrop");
    let src: IGraphicsEffectSource = CompositionEffectSourceParameter::Create(&name)?.cast()?;
    let extra = extra_blur(p.blur);
    let blurred = if extra > 0.5 {
        // optimization: balanced (1); border: hard (1), so the edges do not fade to transparent
        effect(CLSID_GAUSSIAN_BLUR, vec![Prop::F32(extra), Prop::U32(1), Prop::U32(1)], src)?
    } else {
        src
    };
    // alpha mode: premultiplied (1); clamp output
    let graph = effect(
        CLSID_COLOR_MATRIX,
        vec![Prop::F32s(saturation_matrix(p.saturation)), Prop::U32(1), Prop::Bool(true)],
        blurred,
    )?;
    let factory = comp.CreateEffectFactory(&graph.cast::<IGraphicsEffect>()?)?;
    let brush = factory.CreateBrush()?;
    brush.SetSourceParameter(&name, &comp.CreateHostBackdropBrush()?)?;
    sprite.SetBrush(&brush)?;
    sprite.SetOpacity(layer_opacity(p.blur))
}

/// Gives `hwnd` our backdrop, or updates it. Main thread only.
pub fn apply(hwnd: isize, p: Params) -> Result<()> {
    let on: windows::core::BOOL = true.into();
    unsafe { DwmSetWindowAttribute(HWND(hwnd as *mut _), DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as *const _, 4)? };
    if let Some(r) = WINDOWS.with(|w| {
        w.borrow().get(&hwnd).map(|a| {
            if a.params == p {
                Ok(())
            } else {
                with_compositor(|c| set_brush(c, &a.sprite, p))
            }
        })
    }) {
        r?;
        WINDOWS.with(|w| {
            if let Some(a) = w.borrow_mut().get_mut(&hwnd) {
                a.params = p;
            }
        });
        return Ok(());
    }
    let att = with_compositor(|c| {
        let h = HWND(hwnd as *mut _);
        let interop: ICompositorDesktopInterop = c.cast()?;
        // not topmost: the visual sits behind the window's children (the transparent web view)
        let target = unsafe { interop.CreateDesktopWindowTarget(h, false)? };
        let sprite = c.CreateSpriteVisual()?;
        sprite.SetRelativeSizeAdjustment(windows_numerics::Vector2 { X: 1.0, Y: 1.0 })?;
        set_brush(c, &sprite, p)?;
        target.SetRoot(&sprite)?;
        Ok(Attached { _target: target, sprite, params: p })
    })?;
    WINDOWS.with(|w| w.borrow_mut().insert(hwnd, att));
    Ok(())
}

pub fn attached(hwnd: isize) -> bool {
    WINDOWS.with(|w| w.borrow().contains_key(&hwnd))
}

/// Drops the backdrop of a window (destroyed, or switching to the Acrylic fallback). Main thread only.
pub fn detach(hwnd: isize) {
    WINDOWS.with(|w| w.borrow_mut().remove(&hwnd));
}

/// Whether Windows "Transparency effects" is on; with it off the host backdrop is not provided.
pub fn transparency_enabled() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    unsafe {
        let mut v = 1u32;
        let mut n = 4u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut n),
        );
        r.is_err() || v != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_mapping() {
        assert_eq!(layer_opacity(0.0), 0.0);
        assert_eq!(layer_opacity(15.0), 0.5);
        assert_eq!(layer_opacity(60.0), 1.0);
        assert_eq!(extra_blur(HOST_BLUR), 0.0);
        assert!((extra_blur(50.0) - 40.0).abs() < 1e-4);
    }

    #[test]
    fn saturation_one_is_identity() {
        let m = saturation_matrix(1.0);
        for i in 0..4 {
            for j in 0..4 {
                assert!((m[i * 4 + j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-6);
            }
        }
        assert!(m[16..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn saturation_zero_is_grey() {
        let m = saturation_matrix(0.0);
        // pure red maps to its luma on every channel
        for j in 0..3 {
            assert!((m[j] - 0.2126).abs() < 1e-6);
        }
    }
}
