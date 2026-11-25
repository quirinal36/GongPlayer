//! 스레드 사이에 함께 쓰는 상태와 신호.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::clock::{AudioObservation, Clock};
use crate::gfx::device::Gpu;
use crate::media::{MediaInfo, SessionShared};
use crate::ui::layout::Element;

/// 프로세스 안에서 쓰는 단조 시계 (초). 소리 콜백에서도 부를 수 있습니다.
pub fn now() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// 프로세스가 만들어진 뒤 지난 시간 (초). 실행 파일과 DLL을 읽는 시간까지 포함합니다.
/// `GONG_TIMING` 환경 변수가 있을 때 첫 프레임까지의 시간을 재는 데 씁니다.
pub fn since_process_start() -> Option<f64> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).ok()? };
    // FILETIME은 1601년부터 100ns 단위입니다.
    let created = ((c.dwHighDateTime as u64) << 32 | c.dwLowDateTime as u64) as f64 * 1e-7;
    let unix_offset = 11_644_473_600.0;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some(now + unix_offset - created)
}

/// `GONG_TIMING` 환경 변수가 있으면 프로세스 생성부터 지난 시간을 찍습니다.
pub fn timing(label: &str) {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("GONG_TIMING").is_some())
        && let Some(s) = since_process_start()
    {
        eprintln!("[시간] {label}: {:.0}ms", s * 1000.0);
    }
}

/// 잠든 스레드를 깨우는 신호. 깨우기 요청은 한 번 쌓였다가 `wait`에서 소비됩니다.
#[derive(Default)]
pub struct Wake {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Wake {
    pub fn notify(&self) {
        *self.flag.lock().unwrap() = true;
        self.cv.notify_all();
    }

    /// 깨우기 요청이 있거나 `timeout`이 지날 때까지 기다립니다.
    pub fn wait(&self, timeout: Option<Duration>) {
        let mut f = self.flag.lock().unwrap();
        if !*f {
            f = match timeout {
                Some(t) => self.cv.wait_timeout(f, t).unwrap().0,
                None => self.cv.wait(f).unwrap(),
            };
        }
        *f = false;
    }
}

/// 소리 콜백이 남기는 관측값. 콜백이 잠그지 않도록 시퀀스 잠금으로 씁니다.
#[derive(Default)]
pub struct ObservationCell {
    seq: AtomicU32,
    media: AtomicU64,
    at: AtomicU64,
    speed: AtomicU64,
    valid: AtomicBool,
}

impl ObservationCell {
    /// 쓰는 쪽은 소리 콜백 하나뿐입니다.
    pub fn store(&self, o: AudioObservation) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Release);
        self.media.store(o.media.to_bits(), Ordering::Relaxed);
        self.at.store(o.at.to_bits(), Ordering::Relaxed);
        self.speed.store(o.speed.to_bits(), Ordering::Relaxed);
        self.valid.store(true, Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    pub fn clear(&self) {
        self.valid.store(false, Ordering::Release);
    }

    pub fn load(&self) -> Option<AudioObservation> {
        for _ in 0..4 {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let o = AudioObservation {
                media: f64::from_bits(self.media.load(Ordering::Relaxed)),
                at: f64::from_bits(self.at.load(Ordering::Relaxed)),
                speed: f64::from_bits(self.speed.load(Ordering::Relaxed)),
            };
            let valid = self.valid.load(Ordering::Relaxed);
            std::sync::atomic::fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return valid.then_some(o);
            }
        }
        None
    }
}

/// 화면에 겹쳐 그릴 것. UI 스레드가 쓰고 렌더 스레드가 읽습니다.
#[derive(Clone, Debug)]
pub struct OverlayState {
    pub scale: f32,
    pub controls: bool,
    pub hover: Option<Element>,
    pub playing: bool,
    pub repeat: bool,
    pub volume: u32,
    pub muted: bool,
    pub speed: f64,
    pub duration: f64,
    /// 탐색 바를 끄는 중이거나 탐색이 처리 중일 때 보여 줄 위치.
    pub seek_preview: Option<f64>,
    pub osd: Option<String>,
    /// 가운데 한 줄 (빈 상태, 오류).
    pub message: Option<String>,
    pub fullscreen: bool,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            scale: 1.0,
            controls: false,
            hover: None,
            playing: false,
            repeat: false,
            volume: 100,
            muted: false,
            speed: 1.0,
            duration: 0.0,
            seek_preview: None,
            osd: None,
            message: None,
            fullscreen: false,
        }
    }
}

/// UI 스레드에 보내는 사건.
pub enum Event {
    Gpu(Result<Arc<Gpu>, String>),
    Opened {
        sid: u64,
        info: MediaInfo,
    },
    OpenFailed {
        sid: u64,
        message: String,
    },
    SeekSettled {
        sid: u64,
        epoch: u64,
    },
    Ended {
        sid: u64,
        epoch: u64,
    },
    /// 화면을 그릴 수 없게 됨 (디바이스 손실 등). 렌더 스레드가 새 디바이스를 만듭니다.
    GpuLost,
    /// 재생은 계속되지만 알려야 하는 문제.
    Warning(String),
}

/// 사건을 UI 스레드에 보내고 창 메시지로 깨웁니다.
#[derive(Clone)]
pub struct EventSink {
    tx: Sender<Event>,
    hwnd: isize,
    msg: u32,
}

impl EventSink {
    pub fn new(tx: Sender<Event>, hwnd: isize, msg: u32) -> Self {
        Self { tx, hwnd, msg }
    }

    pub fn send(&self, ev: Event) {
        if self.tx.send(ev).is_ok() {
            crate::ui::window::post_wake(self.hwnd, self.msg);
        }
    }
}

/// 앱 전체가 함께 쓰는 상태.
pub struct Shared {
    pub clock: Mutex<Clock>,
    pub overlay: Mutex<OverlayState>,
    pub render_wake: Wake,
    /// 창 클라이언트 크기 (물리 화소).
    pub size: Mutex<(u32, u32)>,
    /// 현재 재생 중인 파일. 렌더 스레드는 여기서 프레임을 꺼냅니다.
    pub session: Mutex<Option<Arc<SessionShared>>>,
    pub audio_obs: ObservationCell,
    /// 지금 화면에 보이는 프레임의 미디어 시각.
    position: AtomicU64,
    pub quit: AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            clock: Mutex::new(Clock::new()),
            overlay: Mutex::new(OverlayState::default()),
            render_wake: Wake::default(),
            size: Mutex::new((1, 1)),
            session: Mutex::new(None),
            audio_obs: ObservationCell::default(),
            position: AtomicU64::new(0),
            quit: AtomicBool::new(false),
        }
    }

    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }

    pub fn set_position(&self, t: f64) {
        self.position.store(t.to_bits(), Ordering::Relaxed);
    }

    /// 겹쳐 그릴 것을 바꾸고 렌더 스레드를 깨웁니다.
    pub fn update_overlay(&self, f: impl FnOnce(&mut OverlayState)) {
        f(&mut self.overlay.lock().unwrap());
        self.render_wake.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_roundtrip() {
        let c = ObservationCell::default();
        assert!(c.load().is_none());
        let o = AudioObservation {
            media: 1.5,
            at: 2.0,
            speed: 1.25,
        };
        c.store(o);
        assert_eq!(c.load(), Some(o));
        c.clear();
        assert!(c.load().is_none());
    }

    #[test]
    fn wake_consumes_notification() {
        let w = Wake::default();
        w.notify();
        let t = Instant::now();
        w.wait(Some(Duration::from_secs(1)));
        assert!(t.elapsed() < Duration::from_millis(500));
        let t = Instant::now();
        w.wait(Some(Duration::from_millis(20)));
        assert!(t.elapsed() >= Duration::from_millis(15));
    }
}
