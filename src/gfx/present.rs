//! VideoProcessor로 NV12/P010 프레임을 백버퍼(BGRA)에 바로 변환합니다.
//! 색 변환, 크기 조절, 회전을 한 번의 `VideoProcessorBlt`로 처리합니다.

use std::collections::HashMap;
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{RECT, TRUE};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::Interface;

use super::device::Gpu;
use crate::media::VideoFrame;
use crate::media::color::{ColorKind, Primaries, Transfer};
use crate::ui::layout::Rect;

const OUTPUT_SPACE: DXGI_COLOR_SPACE_TYPE = DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709;

pub struct Presenter {
    enumerator: Option<ID3D11VideoProcessorEnumerator>,
    processor: Option<ID3D11VideoProcessor>,
    key: (u32, u32, u32, u32),
    output_view: Option<ID3D11VideoProcessorOutputView>,
    input_views: HashMap<(usize, u32), ID3D11VideoProcessorInputView>,
    /// 디코더 텍스처를 입력으로 쓸 수 없는 드라이버에서는 GPU 안에서 복사해 씁니다.
    copy_input: bool,
    copy_texture: Option<(ID3D11Texture2D, u32, u32, DXGI_FORMAT)>,
    last_space: Option<DXGI_COLOR_SPACE_TYPE>,
    /// HDR 색공간으로 변환하다 실패한 적이 있으면 SDR로 해석합니다.
    hdr_fallback: bool,
}

fn input_space(c: ColorKind) -> DXGI_COLOR_SPACE_TYPE {
    match (c.transfer, c.primaries, c.full_range) {
        (Transfer::Pq, _, _) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G2084_LEFT_P2020,
        (Transfer::Hlg, _, true) => DXGI_COLOR_SPACE_YCBCR_FULL_GHLG_TOPLEFT_P2020,
        (Transfer::Hlg, _, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_GHLG_TOPLEFT_P2020,
        (_, Primaries::Bt2020, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P2020,
        (_, Primaries::Bt2020, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P2020,
        (_, Primaries::Bt601, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601,
        (_, Primaries::Bt601, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601,
        (_, Primaries::Bt709, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709,
        (_, Primaries::Bt709, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
    }
}

fn rect(r: Rect) -> RECT {
    RECT {
        left: r.left as i32,
        top: r.top as i32,
        right: r.right as i32,
        bottom: r.bottom as i32,
    }
}

impl Presenter {
    pub fn new() -> Self {
        Self {
            enumerator: None,
            processor: None,
            key: (0, 0, 0, 0),
            output_view: None,
            input_views: HashMap::new(),
            copy_input: false,
            copy_texture: None,
            last_space: None,
            hdr_fallback: false,
        }
    }

    /// 백버퍼 크기를 바꾸기 전에 부릅니다.
    pub fn release_output(&mut self) {
        self.output_view = None;
    }

    /// 세션이 바뀌면 낡은 텍스처의 뷰를 버립니다.
    pub fn forget_inputs(&mut self) {
        self.input_views.clear();
        self.copy_texture = None;
    }

    fn ensure(&mut self, gpu: &Gpu, f: &VideoFrame, out_w: u32, out_h: u32) -> Result<(), String> {
        let key = (f.width, f.height, out_w, out_h);
        if self.processor.is_some() && self.key == key {
            return Ok(());
        }
        self.processor = None;
        self.enumerator = None;
        self.output_view = None;
        self.input_views.clear();
        self.last_space = None;
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputWidth: f.width,
            InputHeight: f.height,
            OutputWidth: out_w,
            OutputHeight: out_h,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            ..Default::default()
        };
        unsafe {
            let e = gpu
                .video_device
                .CreateVideoProcessorEnumerator(&desc)
                .map_err(|e| format!("VideoProcessorEnumerator: {e}"))?;
            let p = gpu
                .video_device
                .CreateVideoProcessor(&e, 0)
                .map_err(|e| format!("VideoProcessor: {e}"))?;
            self.enumerator = Some(e);
            self.processor = Some(p);
        }
        self.key = key;
        Ok(())
    }

    fn input_view(
        &mut self,
        gpu: &Gpu,
        f: &VideoFrame,
    ) -> Result<ID3D11VideoProcessorInputView, String> {
        let enumerator = self.enumerator.as_ref().unwrap();
        if !self.copy_input {
            let k = (f.texture.as_raw() as usize, f.index);
            if let Some(v) = self.input_views.get(&k) {
                return Ok(v.clone());
            }
            match create_input_view(gpu, enumerator, &f.texture, f.index) {
                Ok(v) => {
                    if self.input_views.len() > 64 {
                        self.input_views.clear();
                    }
                    self.input_views.insert(k, v.clone());
                    return Ok(v);
                }
                Err(e) => {
                    eprintln!("디코더 텍스처를 바로 쓸 수 없어 GPU 안에서 복사합니다: {e}");
                    self.copy_input = true;
                }
            }
        }
        // 복사 경로: 같은 형식의 텍스처에 해당 칸만 복사합니다. 시스템 메모리는 거치지 않습니다.
        let format = if f.ten_bit {
            DXGI_FORMAT_P010
        } else {
            DXGI_FORMAT_NV12
        };
        let (w, h) = ((f.width + 1) & !1, (f.height + 1) & !1);
        let fresh = !matches!(&self.copy_texture, Some((_, cw, ch, cf)) if (*cw, *ch, *cf) == (w, h, format));
        if fresh {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_DECODER.0 as u32,
                ..Default::default()
            };
            let mut t = None;
            unsafe {
                gpu.device
                    .CreateTexture2D(&desc, None, Some(&mut t))
                    .map_err(|e| e.to_string())?;
            }
            self.copy_texture = Some((t.unwrap(), w, h, format));
            self.input_views.clear();
        }
        let tex = self.copy_texture.as_ref().unwrap().0.clone();
        let src_box = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: w,
            bottom: h,
            back: 1,
        };
        unsafe {
            gpu.context.CopySubresourceRegion(
                &tex,
                0,
                0,
                0,
                0,
                &f.texture,
                f.index,
                Some(&src_box),
            );
        }
        let k = (tex.as_raw() as usize, 0);
        if let Some(v) = self.input_views.get(&k) {
            return Ok(v.clone());
        }
        let v = create_input_view(gpu, enumerator, &tex, 0)?;
        self.input_views.insert(k, v.clone());
        Ok(v)
    }

    /// 프레임을 백버퍼의 `dst` 사각형에 그립니다. 나머지는 검정으로 채웁니다.
    /// GPU 잠금을 잡은 상태에서 부릅니다.
    #[allow(clippy::too_many_arguments)]
    pub fn blit(
        &mut self,
        gpu: &Gpu,
        f: &VideoFrame,
        back: &ID3D11Texture2D,
        out_w: u32,
        out_h: u32,
        dst: Rect,
        rotation: u32,
    ) -> Result<(), String> {
        self.ensure(gpu, f, out_w, out_h)?;
        let vc = &gpu.video_context;
        let processor = self.processor.clone().unwrap();
        if self.output_view.is_none() {
            let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut v = None;
            unsafe {
                gpu.video_device
                    .CreateVideoProcessorOutputView(
                        back,
                        self.enumerator.as_ref().unwrap(),
                        &desc,
                        Some(&mut v),
                    )
                    .map_err(|e| format!("출력 뷰: {e}"))?;
            }
            self.output_view = v;
        }
        let input = self.input_view(gpu, f)?;

        let space = self.pick_space(f);
        unsafe {
            if self.last_space != Some(space) {
                if let Ok(vc1) = vc.cast::<ID3D11VideoContext1>() {
                    vc1.VideoProcessorSetStreamColorSpace1(&processor, 0, space);
                    vc1.VideoProcessorSetOutputColorSpace1(&processor, OUTPUT_SPACE);
                }
                self.last_space = Some(space);
            }
            let src = RECT {
                left: 0,
                top: 0,
                right: f.width as i32,
                bottom: f.height as i32,
            };
            let full = RECT {
                left: 0,
                top: 0,
                right: out_w as i32,
                bottom: out_h as i32,
            };
            vc.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            vc.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            vc.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src));
            vc.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&rect(dst)));
            vc.VideoProcessorSetOutputTargetRect(&processor, true, Some(&full));
            let black = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    RGBA: D3D11_VIDEO_COLOR_RGBA {
                        R: 0.0,
                        G: 0.0,
                        B: 0.0,
                        A: 1.0,
                    },
                },
            };
            vc.VideoProcessorSetOutputBackgroundColor(&processor, false, &black);
            let rot = match rotation {
                90 => D3D11_VIDEO_PROCESSOR_ROTATION_90,
                180 => D3D11_VIDEO_PROCESSOR_ROTATION_180,
                270 => D3D11_VIDEO_PROCESSOR_ROTATION_270,
                _ => D3D11_VIDEO_PROCESSOR_ROTATION_IDENTITY,
            };
            vc.VideoProcessorSetStreamRotation(&processor, 0, rotation != 0, rot);

            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: TRUE,
                pInputSurface: ManuallyDrop::new(Some(input)),
                ..Default::default()
            };
            let streams = [stream];
            let r =
                vc.VideoProcessorBlt(&processor, self.output_view.as_ref().unwrap(), 0, &streams);
            let [mut stream] = streams;
            ManuallyDrop::drop(&mut stream.pInputSurface);
            r.map_err(|e| format!("VideoProcessorBlt: {e}"))
        }
    }

    /// 입력 색공간. HDR(PQ, HLG)은 VideoProcessor가 SDR로 변환하게 합니다.
    /// `CheckVideoProcessorFormatConversion`은 실제로 변환하는 드라이버(NVIDIA)에서도
    /// "지원 안 함"을 돌려주므로 믿지 않고, 변환이 실패할 때만 SDR BT.2020으로 낮춥니다.
    fn pick_space(&mut self, f: &VideoFrame) -> DXGI_COLOR_SPACE_TYPE {
        let want = input_space(f.color);
        if self.hdr_fallback && f.color.is_hdr() {
            DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P2020
        } else {
            want
        }
    }
}

fn create_input_view(
    gpu: &Gpu,
    e: &ID3D11VideoProcessorEnumerator,
    tex: &ID3D11Texture2D,
    index: u32,
) -> Result<ID3D11VideoProcessorInputView, String> {
    let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
        FourCC: 0,
        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_VPIV {
                MipSlice: 0,
                ArraySlice: index,
            },
        },
    };
    let mut v = None;
    unsafe {
        gpu.video_device
            .CreateVideoProcessorInputView(tex, e, &desc, Some(&mut v))
            .map_err(|e| e.to_string())?;
    }
    v.ok_or_else(|| "입력 뷰 없음".into())
}
