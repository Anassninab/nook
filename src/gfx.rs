//! Direct2D / DirectComposition / DirectWrite plumbing for the notch overlay.
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

#[derive(Clone, Copy, Debug)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);
impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
    }
    pub fn a(self, a: f32) -> Self {
        Color(self.0, self.1, self.2, self.3 * a)
    }
    pub fn mix(self, o: Color, t: f32) -> Self {
        Color(
            self.0 + (o.0 - self.0) * t,
            self.1 + (o.1 - self.1) * t,
            self.2 + (o.2 - self.2) * t,
            self.3 + (o.3 - self.3) * t,
        )
    }
    fn d2d(self) -> D2D1_COLOR_F {
        // premultiplication is handled by D2D when drawing with straight alpha brushes
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Gfx {
    pub dc: ID2D1DeviceContext,
    pub d2d: ID2D1Factory1,
    pub dw: IDWriteFactory,
    pub wic: IWICImagingFactory,
    swap: Option<IDXGISwapChain1>,
    brush: ID2D1SolidColorBrush,
    formats: RefCell<HashMap<(u32, u32, bool), IDWriteTextFormat>>,
    widths: RefCell<HashMap<(String, u32, u32), f32>>,
    fits: RefCell<HashMap<(String, u32, u32, u32), String>>,
    round_join: Option<ID2D1StrokeStyle>,
    round_cap: Option<ID2D1StrokeStyle>,
    dashed: Option<ID2D1StrokeStyle>,
    stop_cache: RefCell<HashMap<[u32; 8], ID2D1GradientStopCollection1>>,
    _dcomp: Option<IDCompositionDevice>,
    _target: Option<IDCompositionTarget>,
    _visual: Option<IDCompositionVisual>,
    pub width: u32,
    pub height: u32,
}

impl Gfx {
    /// Composition-swapchain renderer for a layered, transparent overlay window.
    pub fn new_overlay(hwnd: HWND, w: u32, h: u32, dpi: f32) -> Result<Gfx> {
        unsafe {
            let mut dev: Option<ID3D11Device> = None;
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                None,
            )?;
            let dev = dev.ok_or_else(|| Error::from(E_FAIL))?;
            let dxgi: IDXGIDevice = dev.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;

            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swap = factory.CreateSwapChainForComposition(&dev, &desc, None)?;

            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_dev = d2d.CreateDevice(&dxgi)?;
            let dc = d2d_dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            dc.SetDpi(dpi, dpi);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let brush = dc.CreateSolidColorBrush(&Color(1.0, 1.0, 1.0, 1.0).d2d(), None)?;

            let (round_join, round_cap, dashed) = stroke_styles(&d2d);
            Ok(Gfx {
                dc,
                d2d,
                dw,
                wic,
                swap: Some(swap),
                brush,
                formats: RefCell::new(HashMap::new()),
                widths: RefCell::new(HashMap::new()),
                fits: RefCell::new(HashMap::new()),
                round_join,
                round_cap,
                dashed,
                stop_cache: RefCell::new(HashMap::new()),
                _dcomp: Some(dcomp),
                _target: Some(target),
                _visual: Some(visual),
                width: w,
                height: h,
            })
        }
    }

    /// Opaque swap-chain renderer for ordinary top-level windows (settings, dialogs).
    pub fn new_panel(hwnd: HWND, w: u32, h: u32, dpi: f32) -> Result<Gfx> {
        unsafe {
            let mut dev: Option<ID3D11Device> = None;
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                None,
            )?;
            let dev = dev.ok_or_else(|| Error::from(E_FAIL))?;
            let dxgi: IDXGIDevice = dev.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w.max(1),
                Height: h.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_NONE,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: 0,
            };
            let swap = factory.CreateSwapChainForHwnd(&dev, hwnd, &desc, None, None)?;
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_dev = d2d.CreateDevice(&dxgi)?;
            let dc = d2d_dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            dc.SetDpi(dpi, dpi);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let brush = dc.CreateSolidColorBrush(&Color(1.0, 1.0, 1.0, 1.0).d2d(), None)?;
            let (round_join, round_cap, dashed) = stroke_styles(&d2d);
            Ok(Gfx {
                dc,
                d2d,
                dw,
                wic,
                swap: Some(swap),
                brush,
                formats: RefCell::new(HashMap::new()),
                widths: RefCell::new(HashMap::new()),
                fits: RefCell::new(HashMap::new()),
                round_join,
                round_cap,
                dashed,
                stop_cache: RefCell::new(HashMap::new()),
                _dcomp: None,
                _target: None,
                _visual: None,
                width: w,
                height: h,
            })
        }
    }

    /// Show the slice of the canvas that starts at (dx, dy) pixels (the window is smaller than the canvas).
    pub fn set_view_offset(&self, dx: i32, dy: i32) {
        if let (Some(v), Some(d)) = (&self._visual, &self._dcomp) {
            unsafe {
                let _ = v.SetOffsetX2(-(dx as f32));
                let _ = v.SetOffsetY2(-(dy as f32));
                let _ = d.Commit();
            }
        }
    }

    /// Two-stop gradient collections are reused (they are rebuilt many times per frame otherwise).
    fn stop_cache(&self, stops: &[D2D1_GRADIENT_STOP; 2]) -> Result<ID2D1GradientStopCollection1> {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        let key = [q(stops[0].color.r), q(stops[0].color.g), q(stops[0].color.b), q(stops[0].color.a), q(stops[1].color.r), q(stops[1].color.g), q(stops[1].color.b), q(stops[1].color.a)];
        if let Some(c) = self.stop_cache.borrow().get(&key) {
            return Ok(c.clone());
        }
        let coll = unsafe { self.dc.CreateGradientStopCollection(stops, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED)? };
        let mut c = self.stop_cache.borrow_mut();
        if c.len() > 160 {
            c.clear();
        }
        c.insert(key, coll.clone());
        Ok(coll)
    }

    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        unsafe {
            if let Some(s) = &self.swap {
                self.dc.SetTarget(None);
                s.ResizeBuffers(0, w.max(1), h.max(1), DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
            }
            self.width = w;
            self.height = h;
            Ok(())
        }
    }

    /// Begin a frame on an opaque panel (clears to `bg`).
    pub fn begin_panel(&self, bg: Color) -> Result<()> {
        unsafe {
            let swap = self.swap.as_ref().unwrap();
            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bmp = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&bmp);
            self.dc.BeginDraw();
            self.dc.Clear(Some(&bg.d2d()));
            Ok(())
        }
    }

    /// Filled polygon with optionally rounded corners (round stroke join).
    pub fn poly(&self, pts: &[(f32, f32)], c: Color, round: f32) {
        unsafe {
            let Ok(geo) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(D2D_POINT_2F { x: pts[0].0, y: pts[0].1 }, D2D1_FIGURE_BEGIN_FILLED);
            for p in &pts[1..] {
                sink.AddLine(D2D_POINT_2F { x: p.0, y: p.1 });
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.brush.SetColor(&c.d2d());
            self.dc.FillGeometry(&geo, &self.brush, None);
            if round > 0.0 {
                self.dc.DrawGeometry(&geo, &self.brush, round, self.round_join.as_ref());
            }
        }
    }

    /// Outlined polygon with round stroke join.
    pub fn poly_stroke(&self, pts: &[(f32, f32)], sw: f32, c: Color) {
        unsafe {
            let Ok(geo) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(D2D_POINT_2F { x: pts[0].0, y: pts[0].1 }, D2D1_FIGURE_BEGIN_HOLLOW);
            for p in &pts[1..] {
                sink.AddLine(D2D_POINT_2F { x: p.0, y: p.1 });
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.brush.SetColor(&c.d2d());
            self.dc.DrawGeometry(&geo, &self.brush, sw, self.round_join.as_ref());
        }
    }

    /// Paper airplane send glyph (matching the AI assistant prompt action).
    pub fn paper_plane(&self, cx: f32, cy: f32, size: f32, c: Color) {
        let p0 = (cx + size * 0.88, cy - size * 0.88);
        let p1 = (cx - size * 0.88, cy - size * 0.16);
        let p2 = (cx - size * 0.22, cy + size * 0.22);
        let p3 = (cx + size * 0.16, cy + size * 0.88);
        self.poly_stroke(&[p0, p1, p2, p3], 1.25, c);
        self.line(p2.0, p2.1, p0.0, p0.1, 1.25, c);
    }

    pub fn begin_overlay(&self) -> Result<()> {
        unsafe {
            let swap = self.swap.as_ref().unwrap();
            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bmp = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&bmp);
            self.dc.BeginDraw();
            self.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            Ok(())
        }
    }

    pub fn end_overlay(&self) -> Result<()> {
        unsafe {
            self.dc.EndDraw(None, None)?;
            self.dc.SetTarget(None);
            if let Some(s) = &self.swap {
                let hr = s.Present(1, DXGI_PRESENT(0));
                if hr.is_err() {
                    return Err(Error::from(hr));
                }
            }
            Ok(())
        }
    }

    // ---------------------------------------------------------------- drawing
    pub fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            self.dc.FillRectangle(&D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, &self.brush);
        }
    }
    pub fn fill_round(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h },
                radiusX: r.min(w / 2.0),
                radiusY: r.min(h / 2.0),
            };
            self.dc.FillRoundedRectangle(&rr, &self.brush);
        }
    }
    pub fn stroke_round(&self, x: f32, y: f32, w: f32, h: f32, r: f32, sw: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h },
                radiusX: r,
                radiusY: r,
            };
            self.dc.DrawRoundedRectangle(&rr, &self.brush, sw, None);
        }
    }
    /// Outline of a path geometry in one colour.
    pub fn stroke_geo(&self, geo: &ID2D1PathGeometry, width: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            self.dc.DrawGeometry(geo, &self.brush, width, self.round_join.as_ref());
        }
    }
    /// Fill a rectangle with `c` except for a rounded hole (the tutorial spotlight).
    pub fn dim_with_hole(&self, x: f32, y: f32, w: f32, h: f32, hole: (f32, f32, f32, f32), r: f32, c: Color) {
        unsafe {
            let Ok(geo) = self.d2d.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
            let p = |x: f32, y: f32| D2D_POINT_2F { x, y };
            sink.BeginFigure(p(x, y), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(p(x + w, y));
            sink.AddLine(p(x + w, y + h));
            sink.AddLine(p(x, y + h));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let (hx, hy, hw, hh) = hole;
            let r = r.min(hw / 2.0).min(hh / 2.0).max(0.0);
            let arc = |pt: D2D_POINT_2F| D2D1_ARC_SEGMENT { point: pt, size: D2D_SIZE_F { width: r, height: r }, rotationAngle: 0.0, sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE, arcSize: D2D1_ARC_SIZE_SMALL };
            sink.BeginFigure(p(hx + r, hy), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(p(hx + hw - r, hy));
            sink.AddArc(&arc(p(hx + hw, hy + r)));
            sink.AddLine(p(hx + hw, hy + hh - r));
            sink.AddArc(&arc(p(hx + hw - r, hy + hh)));
            sink.AddLine(p(hx + r, hy + hh));
            sink.AddArc(&arc(p(hx, hy + hh - r)));
            sink.AddLine(p(hx, hy + r));
            sink.AddArc(&arc(p(hx + r, hy)));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            if sink.Close().is_ok() {
                self.brush.SetColor(&c.d2d());
                self.dc.FillGeometry(&geo, &self.brush, None);
            }
        }
    }
    /// Dotted rounded outline (drop zones).
    pub fn stroke_round_dashed(&self, x: f32, y: f32, w: f32, h: f32, r: f32, sw: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let rr = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, radiusX: r, radiusY: r };
            self.dc.DrawRoundedRectangle(&rr, &self.brush, sw, self.dashed.as_ref());
        }
    }
    /// Rounded rectangle filled with a left-to-right gradient.
    pub fn fill_round_hgrad(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c0: Color, c1: Color) {
        unsafe {
            let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: c0.d2d() }, D2D1_GRADIENT_STOP { position: 1.0, color: c1.d2d() }];
            let Ok(coll) = self.stop_cache(&stops) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: D2D_POINT_2F { x, y }, endPoint: D2D_POINT_2F { x: x + w, y } };
            if let Ok(b) = self.dc.CreateLinearGradientBrush(&props, None, &coll) {
                let rr = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, radiusX: r.min(h / 2.0), radiusY: r.min(h / 2.0) };
                self.dc.FillRoundedRectangle(&rr, &b);
            }
        }
    }
    /// Scale everything drawn next about (cx, cy); undo with `reset_transform`.
    pub fn scale_about(&self, cx: f32, cy: f32, s: f32) {
        unsafe {
            self.dc.SetTransform(&windows::Foundation::Numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: cx - cx * s, M32: cy - cy * s });
        }
    }
    pub fn reset_transform(&self) {
        unsafe { self.dc.SetTransform(&windows_numerics_identity()) }
    }
    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: r, radiusY: r };
            self.dc.FillEllipse(&e, &self.brush);
        }
    }
    pub fn stroke_circle(&self, cx: f32, cy: f32, r: f32, sw: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: r, radiusY: r };
            self.dc.DrawEllipse(&e, &self.brush, sw, None);
        }
    }
    pub fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, sw: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            self.dc.DrawLine(D2D_POINT_2F { x: x1, y: y1 }, D2D_POINT_2F { x: x2, y: y2 }, &self.brush, sw, None);
        }
    }
    /// Arc ring (progress ring): `frac` 0..1 starting at 12 o'clock, clockwise.
    pub fn arc(&self, cx: f32, cy: f32, r: f32, frac: f32, sw: f32, c: Color) {
        if frac <= 0.001 {
            return;
        }
        if frac >= 0.999 {
            self.stroke_circle(cx, cy, r, sw, c);
            return;
        }
        unsafe {
            if let Ok(geo) = self.d2d.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    let a = frac * std::f32::consts::TAU;
                    let (sx, sy) = (cx, cy - r);
                    let (ex, ey) = (cx + r * a.sin(), cy - r * a.cos());
                    sink.BeginFigure(D2D_POINT_2F { x: sx, y: sy }, D2D1_FIGURE_BEGIN_HOLLOW);
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: D2D_POINT_2F { x: ex, y: ey },
                        size: D2D_SIZE_F { width: r, height: r },
                        rotationAngle: 0.0,
                        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                        arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    });
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    let _ = sink.Close();
                    self.brush.SetColor(&c.d2d());
                    self.dc.DrawGeometry(&geo, &self.brush, sw, self.round_cap.as_ref());
                }
            }
        }
    }

    /// The notch silhouette: concave flares at the top, rounded bottom corners.
    /// `l..r` is the body; flares extend `rt` beyond each side at y = 0.
    pub fn notch_geometry(&self, l: f32, r: f32, h: f32, rt: f32, rb: f32) -> Result<ID2D1PathGeometry> {
        unsafe {
            let geo = self.d2d.CreatePathGeometry()?;
            let sink = geo.Open()?;
            let p = |x: f32, y: f32| D2D_POINT_2F { x, y };
            let rb = rb.min((r - l) / 2.0).min(h - rt).max(0.0);
            let arc = |pt: D2D_POINT_2F, rad: f32, cw: bool| D2D1_ARC_SEGMENT {
                point: pt,
                size: D2D_SIZE_F { width: rad, height: rad },
                rotationAngle: 0.0,
                sweepDirection: if cw { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                arcSize: D2D1_ARC_SIZE_SMALL,
            };
            sink.BeginFigure(p(l - rt, 0.0), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddArc(&arc(p(l, rt), rt, true));
            sink.AddLine(p(l, h - rb));
            sink.AddArc(&arc(p(l + rb, h), rb, false));
            sink.AddLine(p(r - rb, h));
            sink.AddArc(&arc(p(r, h - rb), rb, false));
            sink.AddLine(p(r, rt));
            sink.AddArc(&arc(p(r + rt, 0.0), rt, true));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            geo.cast()
        }
    }
    /// Fully rounded rectangle as a path (used for the floating style so glass can trace it).
    pub fn pill_geometry(&self, x: f32, y: f32, w: f32, h: f32, r: f32) -> Result<ID2D1PathGeometry> {
        unsafe {
            let geo = self.d2d.CreatePathGeometry()?;
            let sink = geo.Open()?;
            let p = |x: f32, y: f32| D2D_POINT_2F { x, y };
            let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
            let arc = |pt: D2D_POINT_2F| D2D1_ARC_SEGMENT {
                point: pt,
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_SMALL,
            };
            sink.BeginFigure(p(x + r, y), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(p(x + w - r, y));
            sink.AddArc(&arc(p(x + w, y + r)));
            sink.AddLine(p(x + w, y + h - r));
            sink.AddArc(&arc(p(x + w - r, y + h)));
            sink.AddLine(p(x + r, y + h));
            sink.AddArc(&arc(p(x, y + h - r)));
            sink.AddLine(p(x, y + r));
            sink.AddArc(&arc(p(x + r, y)));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            geo.cast()
        }
    }

    /// Outline of a geometry painted with a linear gradient (specular rim of the glass).
    pub fn stroke_geo_grad(&self, geo: &ID2D1PathGeometry, from: (f32, f32), to: (f32, f32), stops: &[(f32, Color)], width: f32) {
        unsafe {
            let st: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|(p, c)| D2D1_GRADIENT_STOP { position: *p, color: c.d2d() }).collect();
            let Ok(coll) = self.dc.CreateGradientStopCollection(&st, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x: from.0, y: from.1 },
                endPoint: D2D_POINT_2F { x: to.0, y: to.1 },
            };
            if let Ok(b) = self.dc.CreateLinearGradientBrush(&props, None, &coll) {
                self.dc.DrawGeometry(geo, &b, width, None);
            }
        }
    }

    pub fn fill_geometry(&self, g: &ID2D1PathGeometry, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            self.dc.FillGeometry(g, &self.brush, None);
        }
    }

    pub fn push_clip(&self, x: f32, y: f32, w: f32, h: f32) {
        unsafe {
            self.dc.PushAxisAlignedClip(
                &D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h },
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
        }
    }
    pub fn pop_clip(&self) {
        unsafe { self.dc.PopAxisAlignedClip() }
    }
    /// Clip to a rounded rect via a geometry layer (anti-aliased).
    pub fn push_round_clip(&self, x: f32, y: f32, w: f32, h: f32, r: f32) {
        unsafe {
            let rr = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h },
                radiusX: r,
                radiusY: r,
            };
            if let Ok(g) = self.d2d.CreateRoundedRectangleGeometry(&rr) {
                let params = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                    geometricMask: ManuallyDrop::new(Some(g.cast::<ID2D1Geometry>().unwrap())),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: windows_numerics_identity(),
                    opacity: 1.0,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                };
                self.dc.PushLayer(&params, None);
                // drop the mask reference held by ManuallyDrop
                let _ = ManuallyDrop::into_inner(params.geometricMask);
            }
        }
    }
    /// Clip following an arbitrary geometry (used to keep glows inside the notch silhouette).
    pub fn push_geo_clip(&self, geo: &ID2D1PathGeometry) {
        unsafe {
            let params = D2D1_LAYER_PARAMETERS1 {
                contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                geometricMask: ManuallyDrop::new(Some(geo.cast::<ID2D1Geometry>().unwrap())),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: windows_numerics_identity(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&params, None);
            let _ = ManuallyDrop::into_inner(params.geometricMask);
        }
    }

    /// Vertical linear gradient fill.
    pub fn vgradient(&self, x: f32, y: f32, w: f32, h: f32, c0: Color, c1: Color) {
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: c0.d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: c1.d2d() },
            ];
            let Ok(coll) = self.stop_cache(&stops) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: D2D_POINT_2F { x, y },
                endPoint: D2D_POINT_2F { x, y: y + h },
            };
            if let Ok(b) = self.dc.CreateLinearGradientBrush(&props, None, &coll) {
                self.dc.FillRectangle(&D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }, &b);
            }
        }
    }

    /// Soft radial glow (centre colour fading to transparent).
    pub fn glow(&self, cx: f32, cy: f32, r: f32, c: Color) {
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP { position: 0.0, color: c.d2d() },
                D2D1_GRADIENT_STOP { position: 1.0, color: c.a(0.0).d2d() },
            ];
            let Ok(coll) = self.stop_cache(&stops) else { return };
            let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                center: D2D_POINT_2F { x: cx, y: cy },
                gradientOriginOffset: D2D_POINT_2F { x: 0.0, y: 0.0 },
                radiusX: r,
                radiusY: r,
            };
            if let Ok(b) = self.dc.CreateRadialGradientBrush(&props, None, &coll) {
                let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: r, radiusY: r };
                self.dc.FillEllipse(&e, &b);
            }
        }
    }

    /// Polyline helper for curves.
    pub fn polyline(&self, pts: &[(f32, f32)], sw: f32, c: Color) {
        for w in pts.windows(2) {
            self.line(w[0].0, w[0].1, w[1].0, w[1].1, sw, c);
        }
    }

    pub fn pop_layer(&self) {
        unsafe { self.dc.PopLayer() }
    }

    // ------------------------------------------------------------------ text
    fn format(&self, size: f32, weight: u32, icon: bool, italic: bool) -> Option<IDWriteTextFormat> {
        let key = ((size * 10.0) as u32 | if italic { 0x4000_0000 } else { 0 }, weight, icon);
        if let Some(f) = self.formats.borrow().get(&key) {
            return Some(f.clone());
        }
        unsafe {
            let family = if icon { w!("Segoe Fluent Icons") } else { w!("Segoe UI Variable Text") };
            let style = if italic { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL };
            let f = self
                .dw
                .CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT(weight as i32),
                    style,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
                .ok()?;
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            self.formats.borrow_mut().insert(key, f.clone());
            Some(f)
        }
    }

    /// Draws `s` vertically centred in the box. Trailing "…" is applied when it does not fit.
    pub fn text(&self, s: &str, x: f32, y: f32, w: f32, h: f32, size: f32, weight: u32, c: Color, al: Align) {
        self.text_impl(s, x, y, w, h, size, weight, c, al, false, false);
    }
    /// Draws `s` with italic style.
    pub fn text_italic(&self, s: &str, x: f32, y: f32, w: f32, h: f32, size: f32, weight: u32, c: Color, al: Align) {
        self.text_impl(s, x, y, w, h, size, weight, c, al, false, true);
    }
    pub fn icon(&self, glyph: char, cx: f32, cy: f32, size: f32, c: Color) {
        let s = glyph.to_string();
        self.text_impl(&s, cx - size, cy - size, size * 2.0, size * 2.0, size, 400, c, Align::Center, true, false);
    }
    fn text_impl(&self, s: &str, x: f32, y: f32, w: f32, h: f32, size: f32, weight: u32, c: Color, al: Align, icon: bool, italic: bool) {
        if s.is_empty() || c.3 <= 0.003 {
            return;
        }
        let Some(f) = self.format(size, weight, icon, italic) else { return };
        unsafe {
            let _ = f.SetTextAlignment(match al {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let wide: Vec<u16> = s.encode_utf16().collect();
            self.brush.SetColor(&c.d2d());
            let rect = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
            let opts = D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT;
            self.dc.DrawText(&wide, &f, &rect, &self.brush, opts, DWRITE_MEASURING_MODE_NATURAL);
        }
    }
    fn wrap_format(&self, size: f32, weight: u32) -> Option<IDWriteTextFormat> {
        let key = ((size * 10.0) as u32, weight | 0x8000_0000, false);
        if let Some(f) = self.formats.borrow().get(&key) {
            return Some(f.clone());
        }
        unsafe {
            let f = self
                .dw
                .CreateTextFormat(
                    w!("Segoe UI Variable Text"),
                    None,
                    DWRITE_FONT_WEIGHT(weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
                .ok()?;
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
            let _ = f.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, size * 1.28, size * 1.0);
            self.formats.borrow_mut().insert(key, f.clone());
            Some(f)
        }
    }

    /// Height of `s` wrapped to `w` DIPs.
    pub fn measure_wrap(&self, s: &str, size: f32, weight: u32, w: f32) -> f32 {
        let Some(f) = self.wrap_format(size, weight) else { return size * 1.3 };
        unsafe {
            let wide: Vec<u16> = s.encode_utf16().collect();
            if let Ok(layout) = self.dw.CreateTextLayout(&wide, &f, w, 1000.0) {
                let mut m = DWRITE_TEXT_METRICS::default();
                if layout.GetMetrics(&mut m).is_ok() {
                    return m.height;
                }
            }
        }
        size * 1.3
    }

    /// Wrapped text, top-aligned in the box.
    pub fn text_wrap(&self, s: &str, x: f32, y: f32, w: f32, h: f32, size: f32, weight: u32, c: Color, al: Align) {
        if s.is_empty() || c.3 <= 0.003 {
            return;
        }
        let Some(f) = self.wrap_format(size, weight) else { return };
        unsafe {
            let _ = f.SetTextAlignment(match al {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let wide: Vec<u16> = s.encode_utf16().collect();
            self.brush.SetColor(&c.d2d());
            let rect = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
            self.dc.DrawText(&wide, &f, &rect, &self.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    /// Saturation-weighted average colour of an image (for artwork-derived theming).
    pub fn dominant(&self, bytes: &[u8]) -> Option<Color> {
        unsafe {
            let stream = self.wic.CreateStream().ok()?;
            stream.InitializeFromMemory(bytes).ok()?;
            let dec = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
            let frame = dec.GetFrame(0).ok()?;
            let scaler = self.wic.CreateBitmapScaler().ok()?;
            scaler.Initialize(&frame, 8, 8, WICBitmapInterpolationModeFant).ok()?;
            let conv = self.wic.CreateFormatConverter().ok()?;
            conv.Initialize(&scaler, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)
                .ok()?;
            let mut buf = [0u8; 8 * 8 * 4];
            conv.CopyPixels(std::ptr::null(), 8 * 4, &mut buf).ok()?;
            let (mut r, mut g, mut b, mut wsum) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for px in buf.chunks(4) {
                let (pb, pg, pr) = (px[0] as f32, px[1] as f32, px[2] as f32);
                let mx = pr.max(pg).max(pb);
                let mn = pr.min(pg).min(pb);
                let wgt = (mx - mn) + 12.0;
                r += pr * wgt;
                g += pg * wgt;
                b += pb * wgt;
                wsum += wgt;
            }
            if wsum <= 0.0 {
                return None;
            }
            let (mut r, mut g, mut b) = (r / wsum, g / wsum, b / wsum);
            let mx = r.max(g).max(b).max(1.0);
            let k = (170.0 / mx).min(1.6); // lift dull colours so the glow reads
            r = (r * k).min(255.0);
            g = (g * k).min(255.0);
            b = (b * k).min(255.0);
            Some(Color::rgb(r as u8, g as u8, b as u8))
        }
    }

    /// Width of `s` at the given size/weight (DIPs).
    pub fn measure(&self, s: &str, size: f32, weight: u32) -> f32 {
        // text widths are asked for every frame; layouts are expensive, so remember the answers
        let key = (s.to_string(), (size * 10.0) as u32, weight);
        if let Some(w) = self.widths.borrow().get(&key) {
            return *w;
        }
        let Some(f) = self.format(size, weight, false, false) else { return 0.0 };
        let mut out = 0.0;
        unsafe {
            let wide: Vec<u16> = s.encode_utf16().collect();
            if let Ok(layout) = self.dw.CreateTextLayout(&wide, &f, 4000.0, 100.0) {
                let mut m = DWRITE_TEXT_METRICS::default();
                if layout.GetMetrics(&mut m).is_ok() {
                    out = m.widthIncludingTrailingWhitespace;
                }
            }
        }
        let mut c = self.widths.borrow_mut();
        if c.len() > 600 {
            c.clear();
        }
        c.insert(key, out);
        out
    }
    /// Truncate to fit `maxw` with an ellipsis.
    pub fn fit(&self, s: &str, size: f32, weight: u32, maxw: f32) -> String {
        let key = (s.to_string(), (size * 10.0) as u32, weight, maxw as u32);
        if let Some(r) = self.fits.borrow().get(&key) {
            return r.clone();
        }
        let r = self.fit_uncached(s, size, weight, maxw);
        let mut c = self.fits.borrow_mut();
        if c.len() > 300 {
            c.clear();
        }
        c.insert(key, r.clone());
        r
    }
    fn fit_uncached(&self, s: &str, size: f32, weight: u32, maxw: f32) -> String {
        if self.measure(s, size, weight) <= maxw {
            return s.to_string();
        }
        let chars: Vec<char> = s.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let t: String = chars[..mid].iter().collect::<String>() + "…";
            if self.measure(&t, size, weight) <= maxw {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        chars[..lo].iter().collect::<String>() + "…"
    }

    // ---------------------------------------------------------------- bitmaps
    pub fn bitmap_from_bytes(&self, bytes: &[u8]) -> Option<ID2D1Bitmap1> {
        unsafe {
            let stream = self.wic.CreateStream().ok()?;
            stream.InitializeFromMemory(bytes).ok()?;
            let dec = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
            let frame = dec.GetFrame(0).ok()?;
            let conv = self.wic.CreateFormatConverter().ok()?;
            conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)
                .ok()?;
            self.dc.CreateBitmapFromWicBitmap(&conv, None).ok()
        }
    }
    pub fn draw_bitmap(&self, b: &ID2D1Bitmap1, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
        unsafe {
            let dest = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
            self.dc.DrawBitmap(b, Some(&dest), opacity, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
        }
    }
    /// Draw a bitmap cropped to a square ("cover") into a rounded rect.
    pub fn draw_cover(&self, b: &ID2D1Bitmap1, x: f32, y: f32, s: f32, r: f32, opacity: f32) {
        unsafe {
            let sz = b.GetSize();
            let side = sz.width.min(sz.height);
            let src = D2D_RECT_F {
                left: (sz.width - side) / 2.0,
                top: (sz.height - side) / 2.0,
                right: (sz.width + side) / 2.0,
                bottom: (sz.height + side) / 2.0,
            };
            self.push_round_clip(x, y, s, s, r);
            let dest = D2D_RECT_F { left: x, top: y, right: x + s, bottom: y + s };
            self.dc.DrawBitmap(b, Some(&dest), opacity, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, Some(&src), None);
            self.pop_layer();
        }
    }
}

fn stroke_styles(d2d: &ID2D1Factory1) -> (Option<ID2D1StrokeStyle>, Option<ID2D1StrokeStyle>, Option<ID2D1StrokeStyle>) {
    unsafe {
        let join = d2d
            .CreateStrokeStyle(&D2D1_STROKE_STYLE_PROPERTIES1 { lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() }, None)
            .ok()
            .and_then(|s| s.cast::<ID2D1StrokeStyle>().ok());
        let cap = d2d
            .CreateStrokeStyle(&D2D1_STROKE_STYLE_PROPERTIES1 { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, ..Default::default() }, None)
            .ok()
            .and_then(|s| s.cast::<ID2D1StrokeStyle>().ok());
        let dashed = d2d
            .CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, dashCap: D2D1_CAP_STYLE_ROUND, dashStyle: D2D1_DASH_STYLE_CUSTOM, ..Default::default() },
                Some(&[0.2, 2.4]),
            )
            .ok()
            .and_then(|s| s.cast::<ID2D1StrokeStyle>().ok());
        (join, cap, dashed)
    }
}

fn windows_numerics_identity() -> windows::Foundation::Numerics::Matrix3x2 {
    windows::Foundation::Numerics::Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 }
}
