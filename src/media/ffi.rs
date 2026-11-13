//! 안전 래퍼에 없는 FFmpeg 원시 API와 작은 소유 래퍼.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

pub use ffmpeg_sys_the_third as sys;
use sys::*;

pub const EAGAIN: c_int = AVERROR(11);

/// FFmpeg 오류 코드를 글로 바꿉니다.
pub fn err_str(code: c_int) -> String {
    let mut buf = [0 as c_char; 128];
    unsafe {
        av_strerror(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

pub fn check(code: c_int, what: &str) -> Result<c_int, String> {
    if code < 0 {
        Err(format!("{what}: {}", err_str(code)))
    } else {
        Ok(code)
    }
}

pub fn q2d(q: AVRational) -> f64 {
    if q.den == 0 {
        0.0
    } else {
        q.num as f64 / q.den as f64
    }
}

/// 시간 기준 `tb`의 타임스탬프를 초로. 값이 없으면 `None`.
pub fn ts_to_secs(ts: i64, tb: AVRational) -> Option<f64> {
    (ts != AV_NOPTS_VALUE).then(|| ts as f64 * q2d(tb))
}

/// libavutil/hwcontext_d3d11va.h의 `AVD3D11VADeviceContext`.
/// 바인딩 생성 대상 헤더에 들어 있지 않아 직접 선언합니다.
#[repr(C)]
pub struct AVD3D11VADeviceContext {
    pub device: *mut c_void,
    pub device_context: *mut c_void,
    pub video_device: *mut c_void,
    pub video_context: *mut c_void,
    pub lock: Option<unsafe extern "C" fn(*mut c_void)>,
    pub unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    pub lock_ctx: *mut c_void,
}

/// 소유하는 `AVPacket`.
pub struct Packet(pub *mut AVPacket);
unsafe impl Send for Packet {}

impl Packet {
    pub fn new() -> Self {
        Self(unsafe { av_packet_alloc() })
    }
    pub fn size(&self) -> usize {
        unsafe { (*self.0).size.max(0) as usize }
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { av_packet_free(&mut self.0) }
    }
}

/// 소유하는 `AVFrame`.
pub struct Frame(pub *mut AVFrame);
unsafe impl Send for Frame {}

impl Frame {
    pub fn new() -> Self {
        Self(unsafe { av_frame_alloc() })
    }
    pub fn as_ref(&self) -> &AVFrame {
        unsafe { &*self.0 }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { av_frame_free(&mut self.0) }
    }
}

/// 소유하는 `AVCodecContext`.
pub struct CodecCtx(pub *mut AVCodecContext);
unsafe impl Send for CodecCtx {}

impl Drop for CodecCtx {
    fn drop(&mut self) {
        unsafe { avcodec_free_context(&mut self.0) }
    }
}

/// 소유하는 `AVBufferRef`.
pub struct BufferRef(pub *mut AVBufferRef);
unsafe impl Send for BufferRef {}
unsafe impl Sync for BufferRef {}

impl BufferRef {
    pub fn new_ref(&self) -> *mut AVBufferRef {
        unsafe { av_buffer_ref(self.0) }
    }
}

impl Drop for BufferRef {
    fn drop(&mut self) {
        unsafe { av_buffer_unref(&mut self.0) }
    }
}

/// 디코더 열기에 필요한 스트림 정보 사본. 디먹스 스레드 밖으로 넘길 수 있습니다.
pub struct StreamParams {
    pub par: *mut AVCodecParameters,
    pub time_base: AVRational,
    /// 파일의 시작 시각 (초). 모든 시각에서 빼서 0부터 시작하게 합니다.
    pub start: f64,
}
unsafe impl Send for StreamParams {}

impl StreamParams {
    pub fn copy_from(stream: *const AVStream, start: f64) -> Self {
        unsafe {
            let par = avcodec_parameters_alloc();
            avcodec_parameters_copy(par, (*stream).codecpar);
            Self {
                par,
                time_base: (*stream).time_base,
                start,
            }
        }
    }
}

impl Drop for StreamParams {
    fn drop(&mut self) {
        unsafe { avcodec_parameters_free(&mut self.par) }
    }
}

/// 스트림의 회전 메타데이터(시계 방향, 0/90/180/270).
pub fn stream_rotation(par: *const AVCodecParameters) -> u32 {
    unsafe {
        let sd = av_packet_side_data_get(
            (*par).coded_side_data,
            (*par).nb_coded_side_data,
            AVPacketSideDataType::DISPLAYMATRIX,
        );
        if sd.is_null() || (*sd).size < 36 {
            return 0;
        }
        // av_display_rotation_get은 반시계 방향 각도를 돌려줍니다.
        let ccw = av_display_rotation_get((*sd).data as *const i32);
        if !ccw.is_finite() {
            return 0;
        }
        let cw = (-ccw).rem_euclid(360.0);
        (((cw + 45.0) / 90.0).floor() as u32 % 4) * 90
    }
}

/// 필터 그래프. 해제를 맡습니다.
pub struct FilterGraph {
    pub graph: *mut AVFilterGraph,
    pub src: *mut AVFilterContext,
    pub sink: *mut AVFilterContext,
}
unsafe impl Send for FilterGraph {}

impl Drop for FilterGraph {
    fn drop(&mut self) {
        unsafe { avfilter_graph_free(&mut self.graph) }
    }
}

pub fn cstring(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

pub fn null<T>() -> *mut T {
    ptr::null_mut()
}
