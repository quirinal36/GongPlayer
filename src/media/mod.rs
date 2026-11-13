//! 파일 하나를 재생하는 동안의 스레드와 큐 (세션).
//!
//! 디먹스 → 패킷 큐 → 영상/소리 디코드 → 프레임 큐/링 버퍼 순으로 흐릅니다.
//! UI 스레드는 세션의 메서드로 명령만 내리고, FFmpeg는 직접 부르지 않습니다.

pub mod audio;
pub mod color;
pub mod demux;
pub mod ffi;
pub mod pack;
pub mod video;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use crate::gfx::device::Gpu;
use crate::queue::GenQueue;
use crate::shared::{EventSink, Shared, Wake};
use color::ColorKind;

/// 패킷 큐 크기 (바이트). 파일을 연 뒤 비트레이트로 약 2초 분량에 맞춥니다.
/// 실측: 16MB로 고정하면 늘 가득 차 메모리만 차지했습니다 (docs/ARCHITECTURE.md).
pub const VIDEO_PACKET_BYTES: usize = 12 << 20;
pub const VIDEO_PACKET_MIN: usize = 2 << 20;
pub const AUDIO_PACKET_BYTES: usize = 1 << 20;
pub const READ_AHEAD_SECONDS: f64 = 2.0;
/// 프레임 큐 크기 (장). 4K 한 장이 약 12MB이므로 작게 유지합니다.
pub const FRAME_QUEUE: usize = 4;

#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    /// 화소 비율 (가로/세로).
    pub sar: f64,
    /// 시계 방향 회전 (0, 90, 180, 270).
    pub rotation: u32,
    pub duration: f64,
    pub has_audio: bool,
    pub codec: String,
}

impl MediaInfo {
    pub fn rotated(&self) -> bool {
        self.rotation % 180 == 90
    }
}

pub enum PacketItem {
    Packet(ffi::Packet),
    Eof,
}

/// 프레임이 쥐고 있어야 하는 것. 디코더 텍스처 풀의 칸이거나 직접 올린 텍스처입니다.
/// 읽지는 않고, 프레임이 살아 있는 동안 텍스처가 재사용되지 않게 붙잡아 둡니다.
#[allow(dead_code)]
pub enum FrameHold {
    Hw(ffi::Frame),
    Sw(Arc<video::SwTexture>),
}

/// 화면에 낼 프레임 한 장. 텍스처는 GPU에 있습니다.
pub struct VideoFrame {
    pub pts: f64,
    pub duration: f64,
    pub texture: ID3D11Texture2D,
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub ten_bit: bool,
    pub color: ColorKind,
    pub hardware: bool,
    _hold: FrameHold,
}

unsafe impl Send for VideoFrame {}

pub enum FrameItem {
    Frame(VideoFrame),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeekState {
    pub epoch: u64,
    pub target: f64,
    pub precise: bool,
    /// 디먹스 스레드가 아직 파일 위치를 옮기지 않았는지.
    pub pending: bool,
    /// 요청한 시각 (반응 시간 측정용).
    pub at: f64,
}

/// 소리 디코드 스레드에 내리는 설정. UI 스레드가 쓰고 소리 스레드가 읽습니다.
pub struct AudioCtl {
    pub paused: AtomicBool,
    speed: AtomicU64,
    gain: AtomicU32,
    pub wake: Wake,
}

impl AudioCtl {
    fn new() -> Self {
        Self {
            paused: AtomicBool::new(true),
            speed: AtomicU64::new(1f64.to_bits()),
            gain: AtomicU32::new(1f32.to_bits()),
            wake: Wake::default(),
        }
    }
    pub fn speed(&self) -> f64 {
        f64::from_bits(self.speed.load(Ordering::Relaxed))
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }
}

/// 세션의 스레드들이 함께 쓰는 것.
pub struct SessionShared {
    pub id: u64,
    pub path: PathBuf,
    /// 파일이 열리면 채워집니다.
    pub info: OnceLock<MediaInfo>,
    pub gpu: Arc<Gpu>,
    pub app: Arc<Shared>,
    pub events: EventSink,
    pub epoch: AtomicU64,
    seek: Mutex<SeekState>,
    seek_cv: Condvar,
    pub video_packets: GenQueue<PacketItem>,
    pub audio_packets: GenQueue<PacketItem>,
    pub frames: GenQueue<FrameItem>,
    pub audio: AudioCtl,
    /// 소리를 쓸 수 없어 소리 패킷을 버립니다.
    pub audio_disabled: AtomicBool,
    pub quit: AtomicBool,
    /// 렌더 스레드가 기다리는 탐색의 세대. 첫 프레임이 나오면 지웁니다.
    pub awaiting: Mutex<Option<SeekState>>,
}

impl SessionShared {
    pub fn seek_state(&self) -> SeekState {
        *self.seek.lock().unwrap()
    }

    /// 디먹스 스레드가 처리할 탐색 요청을 가져갑니다.
    pub fn take_pending_seek(&self) -> Option<SeekState> {
        let mut s = self.seek.lock().unwrap();
        if s.pending {
            s.pending = false;
            Some(*s)
        } else {
            None
        }
    }

    /// 탐색 요청이나 종료가 올 때까지 디먹스 스레드를 재웁니다.
    pub fn wait_for_seek(&self) {
        let s = self.seek.lock().unwrap();
        drop(
            self.seek_cv
                .wait_while(s, |s| !s.pending && !self.quit.load(Ordering::Relaxed)),
        );
    }

    /// 탐색. 세대를 올리고 큐를 낡은 것으로 만든 뒤 디먹스 스레드를 깨웁니다.
    /// 처리 중인 탐색이 있으면 그것을 덮어씁니다 (연속 탐색 병합).
    /// 영상 스레드가 디코더를 바꾼 뒤 키프레임부터 다시 읽을 때도 씁니다.
    pub fn seek(&self, target: f64, precise: bool) -> u64 {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let state = SeekState {
            epoch,
            target,
            precise,
            pending: true,
            at: crate::shared::now(),
        };
        *self.seek.lock().unwrap() = state;
        *self.awaiting.lock().unwrap() = Some(state);
        self.video_packets.advance(epoch);
        self.audio_packets.advance(epoch);
        self.frames.advance(epoch);
        {
            let mut c = self.app.clock.lock().unwrap();
            c.pause(crate::shared::now());
            c.set(target, crate::shared::now());
        }
        self.app.audio_obs.clear();
        self.seek_cv.notify_all();
        self.audio.wake.notify();
        self.app.render_wake.notify();
        epoch
    }

    pub fn quitting(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }
}

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

/// 열린 파일 하나. 버리면 스레드가 모두 끝납니다.
pub struct Session {
    pub shared: Arc<SessionShared>,
    threads: Vec<JoinHandle<()>>,
}

impl Session {
    /// 파일 열기를 시작합니다. 결과는 `Event::Opened`나 `Event::OpenFailed`로 옵니다.
    pub fn open(path: PathBuf, gpu: Arc<Gpu>, app: Arc<Shared>, events: EventSink) -> Self {
        let shared = Arc::new(SessionShared {
            id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            path,
            info: OnceLock::new(),
            gpu,
            app,
            events,
            epoch: AtomicU64::new(0),
            seek: Mutex::new(SeekState {
                epoch: 0,
                target: 0.0,
                precise: true,
                pending: false,
                at: 0.0,
            }),
            seek_cv: Condvar::new(),
            video_packets: GenQueue::new(VIDEO_PACKET_BYTES),
            audio_packets: GenQueue::new(AUDIO_PACKET_BYTES),
            frames: GenQueue::new(FRAME_QUEUE),
            audio: AudioCtl::new(),
            audio_disabled: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            // 첫 프레임이 나올 때까지 시계를 세워 둡니다. 여는 것도 0초로의 탐색처럼 다룹니다.
            awaiting: Mutex::new(Some(SeekState {
                epoch: 0,
                target: 0.0,
                precise: false,
                pending: false,
                at: crate::shared::now(),
            })),
        });
        let s = shared.clone();
        let demux = thread::Builder::new()
            .name("demux".into())
            .spawn(move || demux::run(s))
            .expect("디먹스 스레드");
        Self {
            shared,
            threads: vec![demux],
        }
    }

    pub fn id(&self) -> u64 {
        self.shared.id
    }

    /// 탐색. 세대를 올리고 큐를 낡은 것으로 만든 뒤 디먹스 스레드를 깨웁니다.
    /// 처리 중인 탐색이 있으면 그것을 덮어씁니다 (연속 탐색 병합).
    pub fn seek(&self, target: f64, precise: bool) -> u64 {
        self.shared.seek(target, precise)
    }

    pub fn set_paused(&self, paused: bool) {
        let s = &self.shared;
        s.audio.paused.store(paused, Ordering::Relaxed);
        s.audio.wake.notify();
        let t = crate::shared::now();
        {
            let mut c = s.app.clock.lock().unwrap();
            let waiting = s.awaiting.lock().unwrap().is_some();
            if paused {
                c.pause(t);
            } else if !waiting {
                c.resume(t);
            }
        }
        s.app.audio_obs.clear();
        s.app.render_wake.notify();
    }

    pub fn set_speed(&self, speed: f64) {
        let s = &self.shared;
        s.audio.speed.store(speed.to_bits(), Ordering::Relaxed);
        s.audio.wake.notify();
        // 시계의 배속은 렌더 스레드가 바꿉니다. 소리가 있으면 새 배속의 소리가 나올 때,
        // 없으면 바로 바꿉니다.
        s.app.render_wake.notify();
    }

    pub fn set_gain(&self, gain: f32) {
        self.shared
            .audio
            .gain
            .store(gain.to_bits(), Ordering::Relaxed);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let s = &self.shared;
        s.quit.store(true, Ordering::SeqCst);
        s.video_packets.close();
        s.audio_packets.close();
        s.frames.close();
        s.seek_cv.notify_all();
        s.audio.wake.notify();
        // 스레드는 곧 끝나지만, UI 스레드가 기다리지 않도록 다른 스레드에서 거둡니다.
        let threads = std::mem::take(&mut self.threads);
        let _ = thread::Builder::new()
            .name("reaper".into())
            .spawn(move || threads.into_iter().for_each(|t| drop(t.join())));
    }
}
