//! Direct2D로 컨트롤 바, OSD, 가운데 안내 글을 백버퍼 위에 그립니다.
//! 아이콘은 도형으로 그려 이미지 파일을 싣지 않습니다.

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Interface, w};
use windows_numerics::Vector2;

use super::device::Gpu;
use crate::player::{format_speed, format_time};
use crate::shared::OverlayState;
use crate::ui::layout::{Element, Layout, Rect};

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn rf(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

struct Formats {
    scale: f32,
    small: IDWriteTextFormat,
    osd: IDWriteTextFormat,
    message: IDWriteTextFormat,
}

pub struct Overlay {
    factory: ID2D1Factory1,
    dc: ID2D1DeviceContext,
    dwrite: IDWriteFactory,
    target: Option<ID2D1Bitmap1>,
    formats: Option<Formats>,
    white: ID2D1SolidColorBrush,
    dim: ID2D1SolidColorBrush,
    faint: ID2D1SolidColorBrush,
    shadow: ID2D1SolidColorBrush,
    panel: Option<(Rect, ID2D1LinearGradientBrush)>,
}

impl Overlay {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        unsafe {
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                .map_err(|e| format!("Direct2D: {e}"))?;
            let dxgi: IDXGIDevice = gpu.device.cast().map_err(|e| e.to_string())?;
            let device = factory.CreateDevice(&dxgi).map_err(|e| e.to_string())?;
            let dc = device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .map_err(|e| e.to_string())?;
            // 배치를 물리 화소로 계산하므로 DPI는 96으로 고정합니다.
            dc.SetDpi(96.0, 96.0);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dwrite: IDWriteFactory =
                DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|e| e.to_string())?;
            let brush = |c| {
                dc.CreateSolidColorBrush(&c, None)
                    .map_err(|e| e.to_string())
            };
            Ok(Self {
                white: brush(color(1.0, 1.0, 1.0, 1.0))?,
                dim: brush(color(1.0, 1.0, 1.0, 0.35))?,
                faint: brush(color(1.0, 1.0, 1.0, 0.15))?,
                shadow: brush(color(0.0, 0.0, 0.0, 0.6))?,
                factory,
                dc,
                dwrite,
                target: None,
                formats: None,
                panel: None,
            })
        }
    }

    /// 스왑체인 백버퍼를 그릴 대상으로 잡습니다.
    pub fn bind(&mut self, chain: &IDXGISwapChain2) -> Result<(), String> {
        unsafe {
            let surface: IDXGISurface = chain.GetBuffer(0).map_err(|e| e.to_string())?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let bmp = self
                .dc
                .CreateBitmapFromDxgiSurface(&surface, Some(&props))
                .map_err(|e| e.to_string())?;
            self.dc.SetTarget(&bmp);
            self.target = Some(bmp);
        }
        Ok(())
    }

    /// 백버퍼 크기를 바꾸기 전에 놓습니다.
    pub fn unbind(&mut self) {
        unsafe { self.dc.SetTarget(None) };
        self.target = None;
        self.panel = None;
    }

    fn text_format(
        &self,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
    ) -> Result<IDWriteTextFormat, String> {
        unsafe {
            let f = self
                .dwrite
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("ko-kr"),
                )
                .map_err(|e| e.to_string())?;
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            Ok(f)
        }
    }

    fn ensure_formats(&mut self, scale: f32) -> Result<(), String> {
        if self.formats.as_ref().is_some_and(|f| f.scale == scale) {
            return Ok(());
        }
        self.formats = Some(Formats {
            scale,
            small: self.text_format(13.0 * scale, DWRITE_FONT_WEIGHT_NORMAL)?,
            osd: self.text_format(22.0 * scale, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
            message: self.text_format(16.0 * scale, DWRITE_FONT_WEIGHT_NORMAL)?,
        });
        Ok(())
    }

    fn panel_brush(&mut self, r: Rect) -> Result<ID2D1LinearGradientBrush, String> {
        if let Some((pr, b)) = &self.panel
            && *pr == r
        {
            return Ok(b.clone());
        }
        unsafe {
            let stops = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: color(0.0, 0.0, 0.0, 0.0),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: color(0.0, 0.0, 0.0, 0.72),
                },
            ];
            let rt: &ID2D1RenderTarget = &self.dc;
            let coll = rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .map_err(|e| e.to_string())?;
            let b = self
                .dc
                .CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: pt(0.0, r.top),
                        endPoint: pt(0.0, r.bottom),
                    },
                    None,
                    &coll,
                )
                .map_err(|e| e.to_string())?;
            self.panel = Some((r, b.clone()));
            Ok(b)
        }
    }

    fn polygon(&self, points: &[(f32, f32)], brush: &ID2D1SolidColorBrush) {
        unsafe {
            let Ok(geo) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(pt(points[0].0, points[0].1), D2D1_FIGURE_BEGIN_FILLED);
            let rest: Vec<_> = points[1..].iter().map(|&(x, y)| pt(x, y)).collect();
            sink.AddLines(&rest);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.dc.FillGeometry(&geo, brush, None);
        }
    }

    fn fill_rect(&self, r: Rect, brush: &ID2D1Brush) {
        unsafe { self.dc.FillRectangle(&rf(r), brush) }
    }

    fn rounded(&self, r: Rect, radius: f32, brush: &ID2D1SolidColorBrush) {
        unsafe {
            self.dc.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rf(r),
                    radiusX: radius,
                    radiusY: radius,
                },
                brush,
            )
        }
    }

    fn circle(&self, x: f32, y: f32, radius: f32, brush: &ID2D1SolidColorBrush) {
        unsafe {
            self.dc.FillEllipse(
                &D2D1_ELLIPSE {
                    point: pt(x, y),
                    radiusX: radius,
                    radiusY: radius,
                },
                brush,
            )
        }
    }

    fn line(&self, a: (f32, f32), b: (f32, f32), width: f32, brush: &ID2D1SolidColorBrush) {
        unsafe {
            self.dc
                .DrawLine(pt(a.0, a.1), pt(b.0, b.1), brush, width, None)
        }
    }

    fn text(
        &self,
        s: &str,
        r: Rect,
        format: &IDWriteTextFormat,
        align: DWRITE_TEXT_ALIGNMENT,
        shadow: bool,
    ) {
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let _ = format.SetTextAlignment(align);
            if shadow {
                let o = 1.5;
                let sr = Rect::new(r.left + o, r.top + o, r.right + o, r.bottom + o);
                self.dc.DrawText(
                    &wide,
                    format,
                    &rf(sr),
                    &self.shadow,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            self.dc.DrawText(
                &wide,
                format,
                &rf(r),
                &self.white,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn hover_bg(&self, st: &OverlayState, e: Element, r: Rect) {
        if st.hover == Some(e) {
            let rad = r.width().min(r.height()) * 0.5;
            self.circle(r.center_x(), r.center_y(), rad, &self.faint);
        }
    }

    /// 겹쳐 그립니다. `clear`가 참이면 먼저 검정으로 지웁니다 (영상이 없을 때).
    /// GPU 잠금을 잡은 상태에서 부릅니다.
    pub fn draw(
        &mut self,
        st: &OverlayState,
        l: &Layout,
        position: f64,
        clear: bool,
    ) -> Result<(), String> {
        if self.target.is_none() {
            return Ok(());
        }
        self.ensure_formats(l.scale)?;
        let panel_brush = if st.controls {
            Some(self.panel_brush(l.panel)?)
        } else {
            None
        };
        unsafe {
            self.dc.BeginDraw();
            if clear {
                self.dc.Clear(Some(&color(0.0, 0.0, 0.0, 1.0)));
            }
        }
        if let Some(b) = panel_brush {
            self.draw_controls(st, l, position, &b.cast::<ID2D1Brush>().unwrap());
        }
        let f = self.formats.as_ref().unwrap();
        if let Some(osd) = &st.osd {
            self.text(osd, l.osd, &f.osd, DWRITE_TEXT_ALIGNMENT_LEADING, true);
        }
        if let Some(msg) = &st.message {
            let (w, h) = (l.panel.width(), l.panel.bottom);
            let r = Rect::new(0.0, h * 0.5 - 20.0 * l.scale, w, h * 0.5 + 20.0 * l.scale);
            self.text(msg, r, &f.message, DWRITE_TEXT_ALIGNMENT_CENTER, true);
        }
        unsafe {
            self.dc
                .EndDraw(None, None)
                .map_err(|e| format!("Direct2D: {e}"))
        }
    }

    fn draw_controls(&self, st: &OverlayState, l: &Layout, position: f64, panel: &ID2D1Brush) {
        let s = l.scale;
        self.fill_rect(l.panel, panel);

        // 탐색 바
        let t = l.seek_track;
        let frac = if st.duration > 0.0 {
            (position / st.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let played_x = t.left + t.width() * frac;
        let radius = t.height() * 0.5;
        self.rounded(t, radius, &self.dim);
        if played_x > t.left {
            self.rounded(
                Rect::new(t.left, t.top, played_x, t.bottom),
                radius,
                &self.white,
            );
        }
        let knob = if st.hover == Some(Element::SeekBar) || st.seek_preview.is_some() {
            6.5
        } else {
            4.5
        };
        self.circle(played_x, t.center_y(), knob * s, &self.white);

        // 뒤로, 재생/일시정지, 앞으로
        self.hover_bg(st, Element::Back, l.back);
        self.hover_bg(st, Element::Play, l.play);
        self.hover_bg(st, Element::Forward, l.forward);
        let icon = 8.0 * s;
        for (r, dir) in [(l.back, -1.0f32), (l.forward, 1.0)] {
            let (cx, cy) = (r.center_x(), r.center_y());
            let w = icon * 0.85;
            for start in [-w, 0.0] {
                let base = cx + start * dir;
                let tip = base + w * dir;
                self.polygon(
                    &[(base, cy - icon * 0.7), (tip, cy), (base, cy + icon * 0.7)],
                    &self.white,
                );
            }
        }
        let (cx, cy) = (l.play.center_x(), l.play.center_y());
        if st.playing {
            let (bw, bh) = (3.5 * s, 14.0 * s);
            for dx in [-4.5 * s, 4.5 * s] {
                self.rounded(
                    Rect::new(
                        cx + dx - bw * 0.5,
                        cy - bh * 0.5,
                        cx + dx + bw * 0.5,
                        cy + bh * 0.5,
                    ),
                    1.0 * s,
                    &self.white,
                );
            }
        } else {
            let h = 8.0 * s;
            self.polygon(
                &[(cx - h * 0.6, cy - h), (cx + h, cy), (cx - h * 0.6, cy + h)],
                &self.white,
            );
        }

        let f = self.formats.as_ref().unwrap();
        if l.show_time() {
            let text = if st.duration > 0.0 {
                let hours = st.duration >= 3600.0;
                format!(
                    "{} / {}",
                    format_time(position.min(st.duration), hours),
                    format_time(st.duration, hours)
                )
            } else {
                format_time(position, false)
            };
            self.text(
                &text,
                l.time,
                &f.small,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            );
        }

        if !l.show_right() {
            return;
        }
        // 배속
        self.hover_bg(st, Element::Speed, l.speed);
        self.text(
            &format_speed(st.speed),
            l.speed,
            &f.small,
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        );

        // 음량 아이콘
        self.hover_bg(st, Element::Mute, l.mute);
        let (cx, cy) = (l.mute.center_x() - 3.0 * s, l.mute.center_y());
        self.polygon(
            &[
                (cx - 7.0 * s, cy - 3.0 * s),
                (cx - 3.5 * s, cy - 3.0 * s),
                (cx + 1.5 * s, cy - 7.5 * s),
                (cx + 1.5 * s, cy + 7.5 * s),
                (cx - 3.5 * s, cy + 3.0 * s),
                (cx - 7.0 * s, cy + 3.0 * s),
            ],
            &self.white,
        );
        if st.muted || st.volume == 0 {
            let (x0, x1) = (cx + 5.0 * s, cx + 11.0 * s);
            self.line((x0, cy - 3.0 * s), (x1, cy + 3.0 * s), 1.6 * s, &self.white);
            self.line((x0, cy + 3.0 * s), (x1, cy - 3.0 * s), 1.6 * s, &self.white);
        } else {
            let bars = if st.volume > 50 { 2 } else { 1 };
            for i in 0..bars {
                let x = cx + (5.0 + 4.0 * i as f32) * s;
                let h = (3.0 + 3.0 * i as f32) * s;
                self.line((x, cy - h), (x, cy + h), 1.6 * s, &self.white);
            }
        }

        // 음량 막대
        let v = l.volume_track;
        let frac = if st.muted {
            0.0
        } else {
            st.volume as f32 / 100.0
        };
        let vx = v.left + v.width() * frac;
        self.rounded(v, v.height() * 0.5, &self.dim);
        if vx > v.left {
            self.rounded(
                Rect::new(v.left, v.top, vx, v.bottom),
                v.height() * 0.5,
                &self.white,
            );
        }
        if st.hover == Some(Element::Volume) {
            self.circle(vx, v.center_y(), 5.5 * s, &self.white);
        }

        // 전체화면
        self.hover_bg(st, Element::Fullscreen, l.fullscreen);
        let (cx, cy) = (l.fullscreen.center_x(), l.fullscreen.center_y());
        let (o, k) = (7.0 * s, 4.0 * s);
        let w = 1.8 * s;
        for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let corner = (cx + sx * o, cy + sy * o);
            // 전체화면이면 모서리가 안쪽을 향하게 그립니다.
            let d = if st.fullscreen { 1.0 } else { -1.0 };
            let c = if st.fullscreen {
                (corner.0 - sx * k, corner.1 - sy * k)
            } else {
                corner
            };
            self.line(c, (c.0 + sx * d * k, c.1), w, &self.white);
            self.line(c, (c.0, c.1 + sy * d * k), w, &self.white);
        }
    }
}
