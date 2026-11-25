//! 영상 디코드 스레드.
//!
//! 가능하면 D3D11VA로 디코딩해 디코더 텍스처를 그대로 넘깁니다. 하드웨어가 그 코덱이나
//! 프로파일을 지원하지 않거나 디코딩 중 오류가 나면 소프트웨어로 넘어가, CPU로 디코딩한
//! 프레임을 NV12/P010 텍스처에 올립니다. 렌더러는 프레임이 어디서 왔는지 모릅니다.

use std::cell::Cell;
use std::ffi::{CStr, c_void};
use std::sync::Arc;

use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::Interface;

use super::ffi::{self, CodecCtx, Frame, Packet, StreamParams, sys};
use super::{FrameHold, FrameItem, PacketItem, SessionShared, VideoFrame};
use super::{color, pack};
use crate::gfx::device::Gpu;
use crate::queue::QueueError;

/// 디코더 텍스처 풀에 더 잡아 둘 장 수. 프레임 큐와 화면에 걸린 프레임만큼 필요합니다.
const EXTRA_HW_FRAMES: i32 = super::FRAME_QUEUE as i32 + 4;
/// 하드웨어 디코딩이 연달아 이만큼 실패하면 소프트웨어로 넘어갑니다.
const HW_ERROR_LIMIT: u32 = 3;
/// 시계보다 이만큼(초) 더 늦은 프레임은 버립니다.
const LATE_MARGIN: f64 = 0.05;

struct HwChoice {
    allow: bool,
    /// 소프트웨어 형식을 고를 수 없는 디코더 (FFmpeg 자체 AV1 디코더).
    hw_only: bool,
    chosen: Cell<bool>,
}

unsafe extern "C" fn get_format(
    ctx: *mut sys::AVCodecContext,
    fmts: *const sys::AVPixelFormat,
) -> sys::AVPixelFormat {
    unsafe {
        let choice = &*((*ctx).opaque as *const HwChoice);
        if choice.allow {
            let mut p = fmts;
            while *p != sys::AVPixelFormat::NONE {
                if *p == sys::AVPixelFormat::D3D11 {
                    choice.chosen.set(true);
                    return *p;
                }
                p = p.add(1);
            }
        }
        choice.chosen.set(false);
        if choice.hw_only {
            return sys::AVPixelFormat::NONE;
        }
        sys::avcodec_default_get_format(ctx, fmts)
    }
}

pub struct SwTexture {
    pub texture: ID3D11Texture2D,
    width: u32,
    height: u32,
    ten_bit: bool,
}
unsafe impl Send for SwTexture {}
unsafe impl Sync for SwTexture {}

pub struct VideoDecoder {
    ctx: CodecCtx,
    choice: Box<HwChoice>,
    params: StreamParams,
    gpu: Arc<Gpu>,
    frame: Frame,
    pool: Vec<Arc<SwTexture>>,
    upload: Vec<u8>,
    default_duration: f64,
    last_pts: Option<f64>,
    hw_errors: u32,
    /// 이 시각보다 먼저 끝나는 프레임은 화면에 올리지 않고 버립니다.
    pub skip_before: f64,
}

pub enum Recv {
    Frame(VideoFrame),
    /// 이미 늦어 버린 프레임.
    Late,
    Again,
    Eof,
    Error(String),
}

fn has_d3d11va(codec: *const sys::AVCodec) -> bool {
    let device_ctx: i32 = sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX.0 as _;
    let mut i = 0;
    loop {
        let cfg = unsafe { sys::avcodec_get_hw_config(codec, i) };
        if cfg.is_null() {
            return false;
        }
        unsafe {
            if (*cfg).device_type == sys::AVHWDeviceType::D3D11VA
                && ((*cfg).methods & device_ctx) != 0
            {
                return true;
            }
        }
        i += 1;
    }
}

/// 디버그 빌드에서 소프트웨어 경로를 시험하기 위한 환경 변수. 설정이 아닙니다.
fn force_software() -> bool {
    cfg!(debug_assertions) && std::env::var_os("GONG_FORCE_SW").is_some()
}

impl VideoDecoder {
    pub fn open(params: StreamParams, gpu: Arc<Gpu>) -> Result<Self, String> {
        let hw = !force_software();
        match Self::open_mode(params, gpu, hw) {
            Ok(d) => Ok(d),
            Err((params, gpu, e)) if hw => {
                eprintln!("하드웨어 디코더를 열지 못해 소프트웨어로 엽니다: {e}");
                Self::open_mode(params, gpu, false).map_err(|(_, _, e)| e)
            }
            Err((_, _, e)) => Err(e),
        }
    }

    #[allow(clippy::type_complexity)]
    fn open_mode(
        params: StreamParams,
        gpu: Arc<Gpu>,
        hw: bool,
    ) -> Result<Self, (StreamParams, Arc<Gpu>, String)> {
        unsafe {
            let id = (*params.par).codec_id;
            // FFmpeg 자체 AV1 디코더는 하드웨어 가속 전용입니다. 소프트웨어는 libdav1d를 씁니다.
            let native_av1 = id == sys::AVCodecID::AV1;
            // AV1은 디코딩 중에 소프트웨어로 넘어갈 수 없으므로 GPU 지원을 먼저 확인합니다.
            let av1_unsupported =
                native_av1 && !gpu.supports_profile(&D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0);
            let hw = hw && !av1_unsupported;
            let mut codec = if hw && native_av1 {
                sys::avcodec_find_decoder_by_name(c"av1".as_ptr())
            } else {
                sys::avcodec_find_decoder(id)
            };
            if !hw && native_av1 {
                let dav1d = sys::avcodec_find_decoder_by_name(c"libdav1d".as_ptr());
                if !dav1d.is_null() {
                    codec = dav1d;
                }
            }
            if codec.is_null() {
                let name = CStr::from_ptr(sys::avcodec_get_name(id)).to_string_lossy();
                return Err((
                    params,
                    gpu,
                    format!("지원하지 않는 영상 코덱입니다: {name}"),
                ));
            }
            let name = CStr::from_ptr((*codec).name).to_string_lossy().into_owned();
            let use_hw = hw && has_d3d11va(codec);
            let hw_only = name == "av1";
            if hw_only && !use_hw {
                return Err((params, gpu, "AV1 소프트웨어 디코더가 없습니다".into()));
            }
            let ctx = CodecCtx(sys::avcodec_alloc_context3(codec));
            if ctx.0.is_null() {
                return Err((params, gpu, "디코더를 만들 수 없습니다".into()));
            }
            if let Err(e) = ffi::check(
                sys::avcodec_parameters_to_context(ctx.0, params.par),
                "avcodec_parameters_to_context",
            ) {
                return Err((params, gpu, e));
            }
            (*ctx.0).pkt_timebase = params.time_base;
            let choice = Box::new(HwChoice {
                allow: use_hw,
                hw_only,
                chosen: Cell::new(false),
            });
            (*ctx.0).opaque = &*choice as *const HwChoice as *mut c_void;
            (*ctx.0).get_format = Some(get_format);
            if use_hw {
                (*ctx.0).hw_device_ctx = gpu.hw.new_ref();
                (*ctx.0).extra_hw_frames = EXTRA_HW_FRAMES;
                (*ctx.0).thread_count = 1;
            } else {
                (*ctx.0).thread_count = 0;
            }
            let r = sys::avcodec_open2(ctx.0, codec, ffi::null());
            if r < 0 {
                return Err((
                    params,
                    gpu,
                    format!("영상 디코더를 열 수 없습니다: {}", ffi::err_str(r)),
                ));
            }
            let fps = {
                let st_fps = ffi::q2d((*params.par).framerate);
                if st_fps > 1.0 { st_fps } else { 30.0 }
            };
            eprintln!(
                "영상 디코더: {name} ({})",
                if use_hw {
                    "D3D11VA 시도"
                } else {
                    "소프트웨어"
                }
            );
            Ok(Self {
                ctx,
                choice,
                params,
                gpu,
                frame: Frame::new(),
                pool: Vec::new(),
                upload: Vec::new(),
                default_duration: 1.0 / fps,
                last_pts: None,
                hw_errors: 0,
                skip_before: f64::NEG_INFINITY,
            })
        }
    }

    fn is_hw(&self) -> bool {
        self.choice.allow
    }

    /// 하드웨어 디코딩을 포기하고 소프트웨어 디코더로 다시 엽니다.
    fn fall_back_to_software(&mut self) -> Result<(), String> {
        let params = StreamParams::copy_from_params(&self.params);
        let new = match Self::open_mode(params, self.gpu.clone(), false) {
            Ok(d) => d,
            Err((_, _, e)) => return Err(e),
        };
        eprintln!("하드웨어 디코딩 실패. 소프트웨어 디코딩으로 넘어갑니다");
        let pool = std::mem::take(&mut self.pool);
        *self = new;
        self.pool = pool;
        Ok(())
    }

    pub fn flush(&mut self) {
        unsafe { sys::avcodec_flush_buffers(self.ctx.0) };
        self.last_pts = None;
    }

    /// 패킷을 넣습니다. `None`이면 끝을 알립니다.
    pub fn send(&mut self, pkt: Option<&Packet>) -> Result<(), String> {
        let p = pkt.map(|p| p.0 as *const _).unwrap_or(std::ptr::null());
        let r = unsafe { sys::avcodec_send_packet(self.ctx.0, p) };
        if r < 0 && r != ffi::EAGAIN && r != sys::AVERROR_EOF {
            return Err(ffi::err_str(r));
        }
        Ok(())
    }

    pub fn receive(&mut self) -> Recv {
        let r = unsafe { sys::avcodec_receive_frame(self.ctx.0, self.frame.0) };
        if r == ffi::EAGAIN {
            return Recv::Again;
        }
        if r == sys::AVERROR_EOF {
            return Recv::Eof;
        }
        if r < 0 {
            return Recv::Error(ffi::err_str(r));
        }
        match self.convert() {
            Ok(Some(f)) => Recv::Frame(f),
            Ok(None) => Recv::Late,
            Err(e) => Recv::Error(e),
        }
    }

    fn convert(&mut self) -> Result<Option<VideoFrame>, String> {
        let f = self.frame.as_ref();
        let tb = self.params.time_base;
        let pts = ffi::ts_to_secs(f.best_effort_timestamp, tb)
            .or_else(|| ffi::ts_to_secs(f.pts, tb))
            .map(|t| t - self.params.start)
            .or_else(|| self.last_pts.map(|t| t + self.default_duration))
            .unwrap_or(0.0);
        let duration = if f.duration > 0 {
            f.duration as f64 * ffi::q2d(tb)
        } else {
            self.default_duration
        };
        self.last_pts = Some(pts);
        if pts + duration < self.skip_before {
            unsafe { sys::av_frame_unref(self.frame.0) };
            return Ok(None);
        }
        let (width, height) = (f.width.max(1) as u32, f.height.max(1) as u32);
        let color = color::classify(
            f.colorspace.0 as _,
            f.color_primaries.0 as _,
            f.color_trc.0 as _,
            f.color_range.0 as _,
            height,
        );

        if f.format == sys::AVPixelFormat::D3D11.0 {
            let raw = f.data[0] as *mut c_void;
            let index = f.data[1] as usize as u32;
            let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }
                .ok_or("디코더 텍스처 없음")?
                .clone();
            let ten_bit = unsafe {
                let frames = (*f.hw_frames_ctx).data as *const sys::AVHWFramesContext;
                (*frames).sw_format == sys::AVPixelFormat::P010LE
            };
            let hold = Frame::new();
            unsafe { sys::av_frame_move_ref(hold.0, self.frame.0) };
            self.hw_errors = 0;
            return Ok(Some(VideoFrame {
                pts,
                duration,
                texture,
                index,
                width,
                height,
                ten_bit,
                color,
                hardware: true,
                _hold: FrameHold::Hw(hold),
            }));
        }

        let ten_bit = self.pack_software()?;
        let tex = self.acquire_texture(width, height, ten_bit)?;
        let (aw, ah) = (even(width), even(height));
        {
            let _l = self.gpu.lock();
            unsafe {
                self.gpu.context.UpdateSubresource(
                    &tex.texture,
                    0,
                    None,
                    self.upload.as_ptr() as *const c_void,
                    aw * if ten_bit { 2 } else { 1 },
                    0,
                );
            }
        }
        let _ = ah;
        unsafe { sys::av_frame_unref(self.frame.0) };
        Ok(Some(VideoFrame {
            pts,
            duration,
            texture: tex.texture.clone(),
            index: 0,
            width,
            height,
            ten_bit,
            color,
            hardware: false,
            _hold: FrameHold::Sw(tex),
        }))
    }

    fn acquire_texture(&mut self, w: u32, h: u32, ten_bit: bool) -> Result<Arc<SwTexture>, String> {
        self.pool
            .retain(|t| t.width == w && t.height == h && t.ten_bit == ten_bit);
        if let Some(t) = self.pool.iter().find(|t| Arc::strong_count(t) == 1) {
            return Ok(t.clone());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: even(w),
            Height: even(h),
            MipLevels: 1,
            ArraySize: 1,
            Format: if ten_bit {
                DXGI_FORMAT_P010
            } else {
                DXGI_FORMAT_NV12
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            // VideoProcessor 입력으로 쓰려면 디코더용 텍스처로 만들어야 합니다.
            BindFlags: D3D11_BIND_DECODER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex = None;
        unsafe {
            self.gpu
                .device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .map_err(|e| format!("텍스처를 만들 수 없습니다: {e}"))?;
        }
        let t = Arc::new(SwTexture {
            texture: tex.ok_or("텍스처 없음")?,
            width: w,
            height: h,
            ten_bit,
        });
        self.pool.push(t.clone());
        Ok(t)
    }

    /// 소프트웨어 프레임을 NV12(8bit)나 P010(10bit 이상)의 연속 메모리로 옮깁니다.
    fn pack_software(&mut self) -> Result<bool, String> {
        let f = self.frame.as_ref();
        let desc = unsafe { sys::av_pix_fmt_desc_get(sys::AVPixelFormat(f.format)) };
        if desc.is_null() {
            return Err("알 수 없는 화소 형식".into());
        }
        let d = unsafe { &*desc };
        let unsupported = (sys::AV_PIX_FMT_FLAG_RGB
            | sys::AV_PIX_FMT_FLAG_BE
            | sys::AV_PIX_FMT_FLAG_PAL
            | sys::AV_PIX_FMT_FLAG_HWACCEL
            | sys::AV_PIX_FMT_FLAG_BITSTREAM) as u64;
        if d.flags & unsupported != 0 || f.linesize.iter().take(4).any(|&l| l < 0) {
            let name = unsafe { CStr::from_ptr(d.name) }.to_string_lossy();
            return Err(format!("지원하지 않는 화소 형식입니다: {name}"));
        }
        let comp = |i: usize| pack::Component {
            plane: d.comp[i].plane as usize,
            step: d.comp[i].step as usize,
            offset: d.comp[i].offset as usize,
            shift: d.comp[i].shift as u32,
        };
        let fmt = pack::Format {
            comps: [comp(0), comp(1), comp(2)],
            components: d.nb_components as usize,
            depth: d.comp[0].depth as u32,
            log2_chroma_w: d.log2_chroma_w as u32,
            log2_chroma_h: d.log2_chroma_h as u32,
        };
        let (w, h) = (f.width as usize, f.height as usize);
        let mut data: [&[u8]; 4] = [&[]; 4];
        let mut stride = [0usize; 4];
        for p in 0..4 {
            if f.data[p].is_null() {
                continue;
            }
            stride[p] = f.linesize[p] as usize;
            let rows = pack::plane_rows(&fmt, p, h);
            data[p] = unsafe { std::slice::from_raw_parts(f.data[p], stride[p] * rows) };
        }
        let src = pack::Planes { data, stride };
        Ok(pack::pack(&fmt, &src, w, h, &mut self.upload))
    }
}

impl StreamParams {
    pub fn copy_from_params(other: &StreamParams) -> Self {
        unsafe {
            let par = sys::avcodec_parameters_alloc();
            sys::avcodec_parameters_copy(par, other.par);
            Self {
                par,
                time_base: other.time_base,
                start: other.start,
            }
        }
    }
}

fn even(v: u32) -> u32 {
    (v + 1) & !1
}

pub fn run(s: Arc<SessionShared>, mut dec: VideoDecoder) {
    let mut epoch = 0u64;
    let mut seek = s.seek_state();
    // 정확한 탐색에서 목표보다 앞이라 버린 마지막 프레임. 목표가 파일 끝을 넘으면 이것을 보여 줍니다.
    let mut held: Option<VideoFrame> = None;
    let mut pushed = false;
    let mut announced_hw = false;

    'packets: loop {
        let (pe, item) = match s.video_packets.pop() {
            Ok(x) => x,
            Err(_) => break,
        };
        if pe != epoch {
            epoch = pe;
            seek = s.seek_state();
            dec.flush();
            drop(s.frames.purge_stale());
            held = None;
            pushed = false;
        }
        let res = match &item {
            PacketItem::Packet(p) => dec.send(Some(p)),
            PacketItem::Eof => dec.send(None),
        };
        if let Err(e) = res {
            if dec.is_hw() {
                dec.hw_errors += 1;
                if dec.hw_errors >= HW_ERROR_LIMIT {
                    fall_back(&s, &mut dec);
                }
            } else {
                eprintln!("영상 디코딩 오류: {e}");
            }
            continue;
        }
        // 디코딩이 재생을 못 따라가면 늦은 프레임을 올리지 않고 버려 따라잡습니다.
        dec.skip_before = if s.awaiting.lock().unwrap().is_some() {
            f64::NEG_INFINITY
        } else {
            let c = s.app.clock.lock().unwrap();
            if c.is_running() {
                c.now(crate::shared::now()) - LATE_MARGIN
            } else {
                f64::NEG_INFINITY
            }
        };
        loop {
            match dec.receive() {
                Recv::Frame(f) => {
                    if !announced_hw {
                        announced_hw = true;
                        crate::shared::timing("첫 프레임 디코딩");
                        eprintln!(
                            "첫 프레임: {}x{} {} {}{}",
                            f.width,
                            f.height,
                            if f.hardware {
                                "하드웨어"
                            } else {
                                "소프트웨어"
                            },
                            if f.ten_bit { "10bit" } else { "8bit" },
                            if f.color.is_hdr() { " HDR" } else { "" }
                        );
                    }
                    if seek.precise && seek.epoch == epoch && f.pts + f.duration * 0.5 < seek.target
                    {
                        held = Some(f);
                        continue;
                    }
                    held = None;
                    match s.frames.push(epoch, 1, FrameItem::Frame(f)) {
                        Ok(()) => {
                            pushed = true;
                            s.app.render_wake.notify();
                        }
                        Err(QueueError::Stale) => continue 'packets,
                        Err(QueueError::Closed) => break 'packets,
                    }
                }
                Recv::Late => continue,
                Recv::Again => break,
                Recv::Eof => {
                    if let Some(f) = held.take() {
                        let _ = s.frames.push(epoch, 1, FrameItem::Frame(f));
                    }
                    let _ = pushed;
                    let _ = s.frames.push(epoch, 0, FrameItem::Eof);
                    s.app.render_wake.notify();
                    break;
                }
                Recv::Error(e) => {
                    if dec.is_hw() {
                        dec.hw_errors += 1;
                        if dec.hw_errors >= HW_ERROR_LIMIT {
                            fall_back(&s, &mut dec);
                        }
                    } else {
                        eprintln!("영상 디코딩 오류: {e}");
                    }
                    break;
                }
            }
        }
    }
}

/// 하드웨어 디코딩을 포기합니다. 새 디코더는 키프레임부터 읽어야 하므로 지금 위치로
/// 다시 탐색합니다.
fn fall_back(s: &SessionShared, dec: &mut VideoDecoder) {
    match dec.fall_back_to_software() {
        Ok(()) => {
            let pos = s.app.position();
            s.seek(pos, true);
        }
        Err(e) => eprintln!("소프트웨어 디코더로 넘어가지 못했습니다: {e}"),
    }
}
