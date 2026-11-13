//! 렌더 스레드: 화면 갱신마다 시계를 읽어 맞는 프레임을 고르고, 영상과 컨트롤을 그려
//! 내보냅니다. 재생 중에는 수직 동기에 맞춰 돌고, 일시정지 중에는 화면이 바뀔 때만
//! 깨어납니다.

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{DXGI_STATUS_OCCLUDED, HWND};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::WaitForSingleObjectEx;

use crate::gfx::device::{Gpu, Swap};
use crate::gfx::overlay::Overlay;
use crate::gfx::present::Presenter;
use crate::media::{FrameItem, SessionShared, VideoFrame};
use crate::shared::{Event, EventSink, Shared, now};
use crate::ui::layout::{Layout, fit_video};

pub fn spawn(hwnd: isize, shared: Arc<Shared>, events: EventSink) -> JoinHandle<()> {
    thread::Builder::new()
        .name("render".into())
        .spawn(move || run(hwnd, shared, events))
        .expect("렌더 스레드")
}

struct Current {
    sid: u64,
    frame: VideoFrame,
    shown: bool,
}

/// 디버그 빌드(또는 `GONG_STATS` 환경 변수)에서 2초마다 찍는 재생 통계. 동기 오차와 프레임 누락을 잽니다.
#[derive(Default)]
struct Stats {
    since: f64,
    media_since: f64,
    presents: u64,
    frames: u64,
    dropped: u64,
    max_audio_diff: f64,
}

impl Stats {
    fn report(&mut self, t: f64, media: f64, running: bool, extra: impl FnOnce() -> String) {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ON.get_or_init(|| cfg!(debug_assertions) || std::env::var_os("GONG_STATS").is_some()) {
            return;
        }
        if self.since == 0.0 {
            self.since = t;
            self.media_since = media;
            return;
        }
        if t - self.since >= 2.0 {
            if running {
                eprintln!(
                    "[통계] 벽시계 {:.3}s 동안 미디어 {:.3}s 진행, 표시 {} 새 프레임 {} 누락 {}, 소리 시계 최대 오차 {:.1}ms",
                    t - self.since,
                    media - self.media_since,
                    self.presents,
                    self.frames,
                    self.dropped,
                    self.max_audio_diff * 1000.0
                );
                eprintln!("        {}", extra());
            }
            *self = Stats {
                since: t,
                media_since: media,
                ..Default::default()
            };
        }
    }
}

enum Exit {
    Quit,
    DeviceLost,
}

fn run(hwnd: isize, shared: Arc<Shared>, events: EventSink) {
    loop {
        let gpu = match Gpu::new() {
            Ok(g) => Arc::new(g),
            Err(e) => {
                events.send(Event::Gpu(Err(e)));
                return;
            }
        };
        eprintln!("GPU: {}", gpu.adapter_name);
        crate::shared::timing("GPU 준비");
        events.send(Event::Gpu(Ok(gpu.clone())));
        match frames(hwnd, &shared, &events, &gpu) {
            Ok(Exit::Quit) => return,
            Ok(Exit::DeviceLost) | Err(_) => {
                if shared.quit.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                eprintln!("디바이스를 잃었습니다. 다시 만듭니다");
                events.send(Event::GpuLost);
                thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn frames(
    hwnd: isize,
    shared: &Shared,
    events: &EventSink,
    gpu: &Arc<Gpu>,
) -> Result<Exit, String> {
    let (w, h) = *shared.size.lock().unwrap();
    let mut swap = Swap::new(gpu, HWND(hwnd as *mut _), w, h)?;
    let mut presenter = Presenter::new();
    let mut overlay = Overlay::new(gpu)?;
    overlay.bind(&swap.chain)?;
    crate::shared::timing("스왑체인 준비");
    let mut current: Option<Current> = None;
    let mut ended_sent: Option<(u64, u64)> = None;
    let mut stats = Stats::default();
    let mut first_frame_reported = false;
    let mut last_vsync = 0.0f64;
    let mut refresh = 1.0f64 / 60.0;
    let mut active_sid = 0u64;

    loop {
        if shared.quit.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(Exit::Quit);
        }
        // 스왑체인이 새 프레임을 받을 수 있을 때까지 기다립니다. 매 프레임 그리기 전에 꼭
        // 기다려야 Present가 막히지 않습니다. Present가 막히면 디바이스 잠금 때문에
        // 디코더까지 화면 갱신 속도에 묶입니다.
        unsafe {
            WaitForSingleObjectEx(swap.waitable, 100, true);
        }
        // 화면 갱신 주기를 잽니다. 지금 고른 프레임은 다음 갱신에 화면에 나옵니다.
        let tv = now();
        let dt = tv - last_vsync;
        last_vsync = tv;
        if (0.004..0.05).contains(&dt) {
            refresh = refresh * 0.95 + dt * 0.05;
        }

        let (w, h) = *shared.size.lock().unwrap();
        if (w.max(1), h.max(1)) != (swap.width, swap.height) {
            overlay.unbind();
            presenter.release_output();
            swap.resize(gpu, w, h)?;
            overlay.bind(&swap.chain)?;
        }

        let session = shared.session.lock().unwrap().clone();
        let sid = session.as_ref().map(|s| s.id).unwrap_or(0);
        if sid != active_sid {
            active_sid = sid;
            current = None;
            presenter.forget_inputs();
        }
        // 다른 디바이스로 만든 세션의 프레임은 쓸 수 없습니다.
        let session = session.filter(|s| Arc::ptr_eq(&s.gpu, gpu));

        let t = now();
        let media = {
            let target_speed = session.as_ref().map(|s| s.audio.speed());
            let mut c = shared.clock.lock().unwrap();
            if c.is_running() {
                let obs = shared.audio_obs.load();
                match obs.and_then(|o| o.extrapolate(t).map(|m| (m, o.speed))) {
                    // 소리가 나오는 동안은 소리를 따릅니다. 배속도 새 배속의 소리가 실제로
                    // 스피커에 닿을 때 바꿉니다. 버퍼에 남은 이전 배속의 소리 때문입니다.
                    Some((m, speed)) => {
                        if speed != c.speed() {
                            c.set_speed(speed, t);
                        }
                        let d = c.sync(m, t);
                        stats.max_audio_diff = stats.max_audio_diff.max(d.abs());
                    }
                    None => {
                        if let Some(sp) = target_speed
                            && sp != c.speed()
                        {
                            c.set_speed(sp, t);
                        }
                    }
                }
            }
            c.now(t)
        };

        if let Some(s) = &session {
            // 프레임은 다음 화면 갱신 때 보이므로 그 시각에 맞는 것을 고릅니다.
            let ahead = if running_now(shared) {
                refresh * speed_now(shared)
            } else {
                0.0
            };
            pick_frame(
                s,
                shared,
                events,
                &mut current,
                &mut ended_sent,
                &mut stats,
                media + ahead,
                t,
            );
        }

        let st = shared.overlay.lock().unwrap().clone();
        let duration = st.duration;
        let running = shared.clock.lock().unwrap().is_running();
        let shown = current.as_ref().map(|c| c.frame.pts);
        if let Some(p) = shown {
            shared.set_position(p);
        }
        let position = st.seek_preview.unwrap_or_else(|| {
            let p = if running {
                media
            } else {
                shown.unwrap_or(media)
            };
            if duration > 0.0 {
                p.clamp(0.0, duration)
            } else {
                p.max(0.0)
            }
        });

        let layout = Layout::new(swap.width as f32, swap.height as f32, st.scale);
        {
            let back = swap.back_buffer()?;
            let _l = gpu.lock();
            let mut drew_video = false;
            if let (Some(c), Some(s)) = (&current, &session)
                && let Some(info) = s.info.get()
            {
                let dst = fit_video(
                    c.frame.width,
                    c.frame.height,
                    info.sar,
                    info.rotated(),
                    swap.width,
                    swap.height,
                );
                match presenter.blit(
                    gpu,
                    &c.frame,
                    &back,
                    swap.width,
                    swap.height,
                    dst,
                    info.rotation,
                ) {
                    Ok(()) => drew_video = true,
                    Err(e) => eprintln!("{e}"),
                }
            }
            overlay.draw(&st, &layout, position, !drew_video)?;
        }

        let hr = unsafe { swap.chain.Present(1, DXGI_PRESENT(0)) };
        if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET {
            return Ok(Exit::DeviceLost);
        }
        if gpu.removed_reason().is_some() {
            return Ok(Exit::DeviceLost);
        }
        let occluded = hr == DXGI_STATUS_OCCLUDED;
        if let Some(c) = &mut current {
            if !c.shown && !first_frame_reported && std::env::var_os("GONG_TIMING").is_some() {
                first_frame_reported = true;
                crate::shared::timing("첫 프레임 표시");
            }
            c.shown = true;
        }
        stats.presents += 1;
        stats.report(t, media, running, || {
            let shown = current.as_ref().map(|c| c.frame.pts).unwrap_or(-1.0);
            match &session {
                Some(s) => format!(
                    "시계 {media:.2} 화면 {shown:.2} 프레임큐 {} 영상패킷 {}KB 소리패킷 {}KB",
                    s.frames.len(),
                    s.video_packets.used() / 1024,
                    s.audio_packets.used() / 1024,
                ),
                None => String::new(),
            }
        });

        // 다음 갱신까지 기다립니다.
        let awaiting = session
            .as_ref()
            .is_some_and(|s| s.awaiting.lock().unwrap().is_some());
        let running = shared.clock.lock().unwrap().is_running();
        if running {
            if occluded {
                thread::sleep(Duration::from_millis(16));
            }
        } else if awaiting {
            shared.render_wake.wait(Some(Duration::from_millis(100)));
        } else {
            // 일시정지, 끝, 빈 화면: 화면이 바뀌어야 할 때까지 잠듭니다.
            shared.render_wake.wait(None);
        }
    }
}

/// 시계에 맞는 프레임을 큐에서 고릅니다. 늦은 프레임은 버립니다.
#[allow(clippy::too_many_arguments)]
fn pick_frame(
    s: &Arc<SessionShared>,
    shared: &Shared,
    events: &EventSink,
    current: &mut Option<Current>,
    ended_sent: &mut Option<(u64, u64)>,
    stats: &mut Stats,
    media: f64,
    t: f64,
) {
    loop {
        let awaiting = *s.awaiting.lock().unwrap();
        let front = s.frames.peek_with(|it| match it {
            FrameItem::Frame(f) => Some(f.pts),
            FrameItem::Eof => None,
        });
        match front {
            None => {
                // 비어 있어도 낡은 항목은 이 스레드에서 해제합니다.
                drop(s.frames.purge_stale());
                return;
            }
            Some(None) => {
                // 파일 끝. 마지막 프레임의 표시 시간이 지나면 끝을 알립니다.
                let epoch = s.frames.epoch();
                if awaiting.is_none()
                    && *ended_sent != Some((s.id, epoch))
                    && current
                        .as_ref()
                        .is_none_or(|c| media >= c.frame.pts + c.frame.duration)
                {
                    *ended_sent = Some((s.id, epoch));
                    shared.clock.lock().unwrap().pause(t);
                    events.send(Event::Ended { sid: s.id, epoch });
                }
                return;
            }
            Some(Some(pts)) => {
                let first = awaiting.is_some() || current.as_ref().is_none_or(|c| c.sid != s.id);
                if !first && pts > media {
                    return;
                }
                let Some((epoch, item)) = s.frames.try_pop() else {
                    return;
                };
                let FrameItem::Frame(f) = item else { return };
                let pts = f.pts;
                if current.as_ref().is_some_and(|c| !c.shown && c.sid == s.id) && awaiting.is_none()
                {
                    stats.dropped += 1;
                }
                stats.frames += 1;
                *current = Some(Current {
                    sid: s.id,
                    frame: f,
                    shown: false,
                });
                if let Some(a) = awaiting {
                    // 탐색 뒤 첫 프레임: 시계를 이 프레임에 맞추고, 재생 중이면 다시 흐르게 합니다.
                    let mut aw = s.awaiting.lock().unwrap();
                    if aw.map(|x| x.epoch) == Some(a.epoch) && epoch >= a.epoch {
                        *aw = None;
                        drop(aw);
                        let mut c = shared.clock.lock().unwrap();
                        c.set(pts, t);
                        if !s.audio.paused.load(std::sync::atomic::Ordering::Relaxed) {
                            c.resume(t);
                        }
                        drop(c);
                        *ended_sent = None;
                        if cfg!(debug_assertions) {
                            eprintln!(
                                "[탐색] 목표 {:.3}s → 프레임 {:.3}s, {:.0}ms ({})",
                                a.target,
                                pts,
                                (t - a.at) * 1000.0,
                                if a.precise { "정확" } else { "빠른" }
                            );
                        }
                        events.send(Event::SeekSettled {
                            sid: s.id,
                            epoch: a.epoch,
                        });
                    }
                    return;
                }
            }
        }
    }
}

fn running_now(shared: &Shared) -> bool {
    shared.clock.lock().unwrap().is_running()
}

fn speed_now(shared: &Shared) -> f64 {
    shared.clock.lock().unwrap().speed()
}
