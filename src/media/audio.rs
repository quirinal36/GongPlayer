//! 소리 디코드 스레드: 디코딩 → atempo(배속) → 장치 형식으로 변환 → 링 버퍼.
//!
//! 링 버퍼에 넣는 덩어리마다 원래의 미디어 시각을 표시해 둡니다. 배속 필터를 거치면
//! 소리의 길이가 바뀌기 때문입니다.

use std::collections::VecDeque;
use std::ffi::{CStr, c_char};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::ffi::{self, CodecCtx, FilterGraph, Frame, StreamParams, sys};
use super::{PacketItem, SessionShared};
use crate::audio_out::AudioOut;
use crate::shared::Event;

/// 링 버퍼를 이만큼(초)까지만 채웁니다. 배속 변경이 늦게 들리지 않게 작게 유지합니다.
const FILL_SECONDS: f64 = 0.12;
/// 장치를 다시 열어 볼 간격 (초).
const REOPEN_INTERVAL: f64 = 1.0;

struct Decoder {
    ctx: CodecCtx,
    frame: Frame,
    time_base: sys::AVRational,
    start: f64,
}

impl Decoder {
    fn open(p: &StreamParams) -> Result<Self, String> {
        unsafe {
            let codec = sys::avcodec_find_decoder((*p.par).codec_id);
            if codec.is_null() {
                let name =
                    CStr::from_ptr(sys::avcodec_get_name((*p.par).codec_id)).to_string_lossy();
                return Err(format!("지원하지 않는 소리 코덱입니다: {name}"));
            }
            let ctx = CodecCtx(sys::avcodec_alloc_context3(codec));
            ffi::check(
                sys::avcodec_parameters_to_context(ctx.0, p.par),
                "avcodec_parameters_to_context",
            )?;
            (*ctx.0).pkt_timebase = p.time_base;
            ffi::check(sys::avcodec_open2(ctx.0, codec, ffi::null()), "소리 디코더")?;
            Ok(Self {
                ctx,
                frame: Frame::new(),
                time_base: p.time_base,
                start: p.start,
            })
        }
    }
}

/// 디코딩한 소리를 배속과 장치 형식에 맞추는 필터 그래프.
struct Tempo {
    fg: FilterGraph,
    key: (i32, i32, String),
    out: Frame,
}

fn layout_name(l: &sys::AVChannelLayout) -> String {
    let mut buf = [0 as c_char; 128];
    unsafe {
        let mut tmp: sys::AVChannelLayout = std::mem::zeroed();
        let src = if l.order == sys::AVChannelOrder::UNSPEC {
            sys::av_channel_layout_default(&mut tmp, l.nb_channels);
            &tmp as *const _
        } else {
            l as *const _
        };
        sys::av_channel_layout_describe(src, buf.as_mut_ptr(), buf.len());
        sys::av_channel_layout_uninit(&mut tmp);
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

fn tempo_stages(speed: f64) -> (f64, f64) {
    // atempo는 2배 이하에서 음질이 좋습니다. 그 위는 두 단계로 나눕니다.
    if speed <= 2.0 {
        (speed, 1.0)
    } else {
        (2.0, speed / 2.0)
    }
}

impl Tempo {
    fn new(
        f: &sys::AVFrame,
        speed: f64,
        out_rate: u32,
        out_channels: usize,
    ) -> Result<Self, String> {
        let fmt_name = unsafe {
            let n = sys::av_get_sample_fmt_name(sys::AVSampleFormat(f.format));
            if n.is_null() {
                return Err("알 수 없는 소리 형식".into());
            }
            CStr::from_ptr(n).to_string_lossy().into_owned()
        };
        let layout = layout_name(&f.ch_layout);
        let out_layout = unsafe {
            let mut l: sys::AVChannelLayout = std::mem::zeroed();
            sys::av_channel_layout_default(&mut l, out_channels as i32);
            let s = layout_name(&l);
            sys::av_channel_layout_uninit(&mut l);
            s
        };
        let (t0, t1) = tempo_stages(speed);
        let desc = format!(
            "abuffer@in=sample_rate={rate}:sample_fmt={fmt_name}:channel_layout={layout}:time_base=1/{rate},\
             atempo@t0=tempo={t0},atempo@t1=tempo={t1},\
             aresample={out_rate},aformat=sample_fmts=flt:channel_layouts={out_layout},\
             abuffersink@out",
            rate = f.sample_rate,
        );
        unsafe {
            let graph = sys::avfilter_graph_alloc();
            if graph.is_null() {
                return Err("필터 그래프를 만들 수 없습니다".into());
            }
            let mut fg = FilterGraph {
                graph,
                src: ffi::null(),
                sink: ffi::null(),
            };
            (*graph).nb_threads = 1;
            let cdesc = ffi::cstring(&desc);
            let mut ins: *mut sys::AVFilterInOut = ffi::null();
            let mut outs: *mut sys::AVFilterInOut = ffi::null();
            let r = sys::avfilter_graph_parse_ptr(
                graph,
                cdesc.as_ptr(),
                &mut ins,
                &mut outs,
                ffi::null(),
            );
            sys::avfilter_inout_free(&mut ins);
            sys::avfilter_inout_free(&mut outs);
            ffi::check(r, "소리 필터")?;
            ffi::check(
                sys::avfilter_graph_config(graph, ffi::null()),
                "소리 필터 구성",
            )?;
            fg.src = sys::avfilter_graph_get_filter(graph, c"abuffer@in".as_ptr());
            fg.sink = sys::avfilter_graph_get_filter(graph, c"abuffersink@out".as_ptr());
            if fg.src.is_null() || fg.sink.is_null() {
                return Err("소리 필터의 입출력을 찾을 수 없습니다".into());
            }
            Ok(Self {
                fg,
                key: (f.sample_rate, f.format, layout),
                out: Frame::new(),
            })
        }
    }

    fn matches(&self, f: &sys::AVFrame) -> bool {
        self.key.0 == f.sample_rate
            && self.key.1 == f.format
            && self.key.2 == layout_name(&f.ch_layout)
    }

    fn set_speed(&self, speed: f64) {
        let (t0, t1) = tempo_stages(speed);
        for (target, v) in [(c"atempo@t0", t0), (c"atempo@t1", t1)] {
            let arg = ffi::cstring(&format!("{v}"));
            unsafe {
                sys::avfilter_graph_send_command(
                    self.fg.graph,
                    target.as_ptr(),
                    c"tempo".as_ptr(),
                    arg.as_ptr(),
                    ffi::null(),
                    0,
                    0,
                );
            }
        }
    }

    /// 프레임을 넣고(없으면 끝을 알림) 나온 샘플을 `out`에 덧붙입니다.
    fn process(&mut self, input: Option<&Frame>, out: &mut Vec<f32>) {
        unsafe {
            let fp = input.map(|f| f.0).unwrap_or(ffi::null());
            if sys::av_buffersrc_add_frame_flags(self.fg.src, fp, 0) < 0 {
                return;
            }
            while sys::av_buffersink_get_frame(self.fg.sink, self.out.0) >= 0 {
                let f = &*self.out.0;
                let n = f.nb_samples as usize * f.ch_layout.nb_channels as usize;
                let data = std::slice::from_raw_parts(f.data[0] as *const f32, n);
                out.extend_from_slice(data);
                sys::av_frame_unref(self.out.0);
            }
        }
    }
}

/// 링 버퍼에 넣기를 기다리는 덩어리.
struct Chunk {
    samples: Vec<f32>,
    written: usize,
    media: f64,
    speed: f64,
}

struct State {
    epoch: u64,
    seek: super::SeekState,
    tempo: Option<Tempo>,
    speed: f64,
    /// 다음에 나올 출력 샘플의 미디어 시각.
    out_media: Option<f64>,
    chunks: VecDeque<Chunk>,
    scratch: Vec<f32>,
}

pub fn run(s: Arc<SessionShared>, params: StreamParams) {
    let mut dec = match Decoder::open(&params) {
        Ok(d) => d,
        Err(e) => {
            disable(&s, e);
            return;
        }
    };
    let mut out = match AudioOut::open(&s) {
        Ok(o) => Some(o),
        Err(e) => {
            s.events.send(Event::Warning(e));
            None
        }
    };
    let mut last_reopen = crate::shared::now();
    let mut st = State {
        epoch: 0,
        seek: s.seek_state(),
        tempo: None,
        speed: s.audio.speed(),
        out_media: None,
        chunks: VecDeque::new(),
        scratch: Vec::new(),
    };
    let mut applied_pause: Option<bool> = None;

    loop {
        if s.quitting() {
            break;
        }
        // 장치가 사라졌으면 기본 장치로 다시 엽니다. 실패하면 영상만 재생합니다.
        if out.as_ref().is_none_or(|o| o.has_failed()) {
            let now = crate::shared::now();
            if now - last_reopen >= REOPEN_INTERVAL {
                last_reopen = now;
                out = None;
                s.app.audio_obs.clear();
                if let Ok(o) = AudioOut::open(&s) {
                    eprintln!("소리 장치를 다시 열었습니다");
                    // 새 장치는 처음부터 채웁니다. 시계는 벽시계가 이어 갑니다.
                    st.chunks.clear();
                    st.tempo = None;
                    applied_pause = None;
                    out = Some(o);
                }
            }
        }

        let paused = s.audio.paused.load(Ordering::Relaxed);
        if let Some(o) = &out
            && applied_pause != Some(paused)
        {
            if paused {
                o.pause()
            } else {
                o.play()
            }
            applied_pause = Some(paused);
        }
        let speed = s.audio.speed();
        if speed != st.speed {
            st.speed = speed;
            if let Some(t) = &st.tempo {
                t.set_speed(speed);
            }
        }

        // 탐색이 들어왔으면 쌓아 둔 것을 버립니다.
        if s.audio_packets.epoch() != st.epoch {
            st.chunks.clear();
        }

        // 1. 쌓인 덩어리를 링 버퍼에 넣습니다.
        if let Some(o) = &out {
            let limit = (o.rate as f64 * FILL_SECONDS) as usize;
            let mut blocked = false;
            while let Some(c) = st.chunks.front_mut() {
                if o.ring.queued_frames() >= limit {
                    blocked = true;
                    break;
                }
                if c.written == 0 {
                    o.ring.mark(c.media, c.speed);
                }
                let room = (limit - o.ring.queued_frames()) * o.channels;
                let end = (c.written + room).min(c.samples.len());
                let n = o.ring.write(&c.samples[c.written..end]);
                c.written += n * o.channels;
                if c.written >= c.samples.len() {
                    st.chunks.pop_front();
                } else if n == 0 {
                    blocked = true;
                    break;
                }
            }
            if blocked {
                // 일시정지 중에는 콜백이 꺼내지 않으므로 명령이 올 때까지 잠듭니다.
                let timeout = if paused {
                    None
                } else {
                    Some(Duration::from_millis(15))
                };
                s.audio.wake.wait(timeout);
                continue;
            }
        } else if !st.chunks.is_empty() {
            // 장치가 없으면 소리는 버리고 영상만 재생합니다.
            st.chunks.clear();
        }

        // 2. 패킷을 하나 디코딩합니다.
        let (pe, item) = match s.audio_packets.pop_timeout(Duration::from_millis(50)) {
            Ok(Some(x)) => x,
            Ok(None) => continue,
            Err(_) => break,
        };
        if pe != st.epoch {
            st.epoch = pe;
            st.seek = s.seek_state();
            unsafe { sys::avcodec_flush_buffers(dec.ctx.0) };
            st.tempo = None;
            st.out_media = None;
            st.chunks.clear();
            if let Some(o) = &out {
                o.ring.discard();
            }
        }
        if out.is_none() {
            // 장치가 없으면 디코딩하지 않고 버립니다. 시계는 벽시계가 맡습니다.
            continue;
        }
        let rate = out.as_ref().map(|o| o.rate).unwrap_or(48000);
        let channels = out.as_ref().map(|o| o.channels).unwrap_or(2);
        match item {
            PacketItem::Packet(p) => {
                unsafe { sys::avcodec_send_packet(dec.ctx.0, p.0) };
                drain_decoder(&mut dec, &mut st, rate, channels);
            }
            PacketItem::Eof => {
                unsafe { sys::avcodec_send_packet(dec.ctx.0, std::ptr::null()) };
                drain_decoder(&mut dec, &mut st, rate, channels);
                if let Some(t) = &mut st.tempo {
                    st.scratch.clear();
                    t.process(None, &mut st.scratch);
                    push_chunk(&mut st, rate, channels);
                }
                // 끝난 디코더는 탐색 전까지 쓸 수 없으므로 비워 둡니다.
                unsafe { sys::avcodec_flush_buffers(dec.ctx.0) };
                st.tempo = None;
            }
        }
    }
}

fn drain_decoder(dec: &mut Decoder, st: &mut State, rate: u32, channels: usize) {
    loop {
        let r = unsafe { sys::avcodec_receive_frame(dec.ctx.0, dec.frame.0) };
        if r < 0 {
            return;
        }
        let f = unsafe { &*dec.frame.0 };
        let pts = ffi::ts_to_secs(f.best_effort_timestamp, dec.time_base).map(|t| t - dec.start);
        let dur = f.nb_samples as f64 / f.sample_rate.max(1) as f64;
        // 정확한 탐색이면 목표 앞의 소리를 버립니다.
        if st.seek.precise
            && st.seek.epoch == st.epoch
            && let Some(p) = pts
            && p + dur <= st.seek.target
        {
            unsafe { sys::av_frame_unref(dec.frame.0) };
            continue;
        }
        if st.out_media.is_none() {
            st.out_media = Some(pts.unwrap_or(st.seek.target));
        }
        if st.tempo.as_ref().is_none_or(|t| !t.matches(f)) {
            match Tempo::new(f, st.speed, rate, channels) {
                Ok(t) => st.tempo = Some(t),
                Err(e) => {
                    eprintln!("{e}");
                    unsafe { sys::av_frame_unref(dec.frame.0) };
                    continue;
                }
            }
        }
        st.scratch.clear();
        if let Some(t) = &mut st.tempo {
            t.process(Some(&dec.frame), &mut st.scratch);
        }
        unsafe { sys::av_frame_unref(dec.frame.0) };
        push_chunk(st, rate, channels);
    }
}

fn push_chunk(st: &mut State, rate: u32, channels: usize) {
    if st.scratch.is_empty() {
        return;
    }
    let media = st.out_media.unwrap_or(0.0);
    let frames = st.scratch.len() / channels;
    st.out_media = Some(media + frames as f64 / rate as f64 * st.speed);
    st.chunks.push_back(Chunk {
        samples: std::mem::take(&mut st.scratch),
        written: 0,
        media,
        speed: st.speed,
    });
}

/// 소리를 쓸 수 없을 때. 디먹스가 소리 패킷을 넣다 막히지 않게 끄고 알립니다.
fn disable(s: &SessionShared, message: String) {
    s.audio_disabled.store(true, Ordering::SeqCst);
    drop(s.audio_packets.flush(s.audio_packets.epoch()));
    s.events.send(Event::Warning(message));
}
