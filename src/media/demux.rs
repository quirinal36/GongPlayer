//! 디먹스 스레드: 파일을 열고, 패킷을 읽어 종류별 큐에 넣고, 탐색을 처리합니다.

use std::ffi::c_int;
use std::sync::Arc;
use std::thread;

use super::ffi::{self, Packet, StreamParams, sys};
use super::{MediaInfo, PacketItem, SessionShared, audio, video};
use crate::queue::QueueError;
use crate::shared::Event;

struct Input(*mut sys::AVFormatContext);

impl Drop for Input {
    fn drop(&mut self) {
        unsafe { sys::avformat_close_input(&mut self.0) }
    }
}

/// 손상된 파일에서 연속으로 읽기에 실패하면 끝으로 봅니다.
const MAX_READ_ERRORS: u32 = 64;

pub fn run(s: Arc<SessionShared>) {
    let opened = match open(&s) {
        Ok(o) => o,
        Err(message) => {
            s.events.send(Event::OpenFailed { sid: s.id, message });
            return;
        }
    };
    let Opened {
        input,
        video_index,
        audio_index,
        info,
        video_dec,
        audio_params,
    } = opened;

    let mut children = Vec::new();
    {
        let s2 = s.clone();
        if let Ok(t) = thread::Builder::new()
            .name("video".into())
            .spawn(move || video::run(s2, video_dec))
        {
            children.push(t);
        }
    }
    if let Some(params) = audio_params {
        let s2 = s.clone();
        if let Ok(t) = thread::Builder::new()
            .name("audio".into())
            .spawn(move || audio::run(s2, params))
        {
            children.push(t);
        }
    }

    // 미리 읽는 양을 비트레이트로 약 2초 분량에 맞춥니다.
    let bit_rate = unsafe { (*input.0).bit_rate };
    s.video_packets.set_capacity(crate::queue::read_ahead_bytes(
        bit_rate,
        super::READ_AHEAD_SECONDS,
        super::VIDEO_PACKET_MIN,
        super::VIDEO_PACKET_BYTES,
    ));
    let _ = s.info.set(info.clone());
    s.events.send(Event::Opened { sid: s.id, info });
    read_loop(&s, &input, video_index, audio_index);

    s.video_packets.close();
    s.audio_packets.close();
    s.frames.close();
    for t in children {
        let _ = t.join();
    }
    drop(input);
}

struct Opened {
    input: Input,
    video_index: c_int,
    audio_index: Option<c_int>,
    info: MediaInfo,
    video_dec: video::VideoDecoder,
    audio_params: Option<StreamParams>,
}

fn open(s: &SessionShared) -> Result<Opened, String> {
    crate::shared::timing("파일 열기 시작");
    let path = s.path.to_string_lossy();
    let cpath = ffi::cstring(&path);
    let mut fmt: *mut sys::AVFormatContext = ffi::null();
    unsafe {
        let r = sys::avformat_open_input(&mut fmt, cpath.as_ptr(), std::ptr::null(), ffi::null());
        if r < 0 {
            return Err(format!("파일을 열 수 없습니다: {}", ffi::err_str(r)));
        }
        let input = Input(fmt);
        crate::shared::timing("avformat_open_input");
        let r = sys::avformat_find_stream_info(fmt, ffi::null());
        crate::shared::timing("find_stream_info");
        if r < 0 {
            return Err(format!("파일 형식을 읽을 수 없습니다: {}", ffi::err_str(r)));
        }
        let vi = sys::av_find_best_stream(fmt, sys::AVMediaType::VIDEO, -1, -1, ffi::null(), 0);
        if vi < 0 {
            return Err("영상 스트림이 없습니다".into());
        }
        let ai = sys::av_find_best_stream(fmt, sys::AVMediaType::AUDIO, -1, vi, ffi::null(), 0);
        let audio_index = (ai >= 0).then_some(ai);

        // 쓰지 않는 스트림은 읽지 않게 합니다.
        for i in 0..(*fmt).nb_streams as c_int {
            if i != vi && Some(i) != audio_index {
                let st = *(*fmt).streams.add(i as usize);
                (*st).discard = sys::AVDiscard::ALL;
            }
        }

        let vst = *(*fmt).streams.add(vi as usize);
        let par = (*vst).codecpar;
        let sar = {
            let q = sys::av_guess_sample_aspect_ratio(fmt, vst, ffi::null());
            let v = ffi::q2d(q);
            if v > 0.0 { v } else { 1.0 }
        };
        let duration = if (*fmt).duration > 0 {
            (*fmt).duration as f64 / sys::AV_TIME_BASE as f64
        } else {
            ffi::ts_to_secs((*vst).duration, (*vst).time_base).unwrap_or(0.0)
        };
        let codec_name = {
            let n = sys::avcodec_get_name((*par).codec_id);
            std::ffi::CStr::from_ptr(n).to_string_lossy().into_owned()
        };
        let start = if (*fmt).start_time != sys::AV_NOPTS_VALUE {
            (*fmt).start_time as f64 / sys::AV_TIME_BASE as f64
        } else {
            0.0
        };
        let params = StreamParams::copy_from(vst, start);
        let video_dec = video::VideoDecoder::open(params, s.gpu.clone())?;
        crate::shared::timing("영상 디코더 열림");

        let audio_params = audio_index.map(|ai| {
            let ast = *(*fmt).streams.add(ai as usize);
            StreamParams::copy_from(ast, start)
        });

        let info = MediaInfo {
            width: (*par).width.max(0) as u32,
            height: (*par).height.max(0) as u32,
            sar,
            rotation: ffi::stream_rotation(par),
            duration,
            has_audio: audio_index.is_some(),
            codec: codec_name,
        };
        Ok(Opened {
            input,
            video_index: vi,
            audio_index,
            info,
            video_dec,
            audio_params,
        })
    }
}

fn read_loop(s: &SessionShared, input: &Input, vi: c_int, ai: Option<c_int>) {
    let fmt = input.0;
    let mut epoch = 0u64;
    let mut eof = false;
    let mut errors = 0u32;
    // 큐가 낡아져 넣지 못한 패킷. 탐색이 아니면 다시 넣지 않습니다.
    loop {
        if s.quitting() {
            return;
        }
        if let Some(req) = s.take_pending_seek() {
            epoch = req.epoch;
            seek(fmt, req.target);
            eof = false;
            errors = 0;
        }
        if eof {
            s.wait_for_seek();
            continue;
        }
        let pkt = Packet::new();
        let r = unsafe { sys::av_read_frame(fmt, pkt.0) };
        if r < 0 {
            if r == sys::AVERROR_EOF || unsafe { sys::avio_feof((*fmt).pb) } != 0 {
                eof = true;
            } else {
                errors += 1;
                if errors >= MAX_READ_ERRORS {
                    eof = true;
                }
            }
            if eof {
                let _ = s.video_packets.push(epoch, 0, PacketItem::Eof);
                if ai.is_some() && !s.audio_disabled.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = s.audio_packets.push(epoch, 0, PacketItem::Eof);
                }
            }
            continue;
        }
        errors = 0;
        let index = unsafe { (*pkt.0).stream_index };
        let size = pkt.size();
        let res = if index == vi {
            s.video_packets.push(epoch, size, PacketItem::Packet(pkt))
        } else if Some(index) == ai && !s.audio_disabled.load(std::sync::atomic::Ordering::Relaxed)
        {
            s.audio_packets.push(epoch, size, PacketItem::Packet(pkt))
        } else {
            Ok(())
        };
        match res {
            Ok(()) | Err(QueueError::Stale) => {}
            Err(QueueError::Closed) => return,
        }
    }
}

/// 목표 앞의 가장 가까운 키프레임으로 옮깁니다.
fn seek(fmt: *mut sys::AVFormatContext, target: f64) {
    let ts = (target.max(0.0) * sys::AV_TIME_BASE as f64) as i64;
    unsafe {
        let start = (*fmt).start_time;
        let ts = if start != sys::AV_NOPTS_VALUE {
            ts + start
        } else {
            ts
        };
        let r = sys::avformat_seek_file(fmt, -1, i64::MIN, ts, ts, 0);
        if r < 0 {
            // 일부 형식은 범위 탐색을 못 합니다. 뒤쪽 키프레임으로 다시 시도합니다.
            sys::av_seek_frame(fmt, -1, ts, sys::AVSEEK_FLAG_BACKWARD as c_int);
        }
    }
}
