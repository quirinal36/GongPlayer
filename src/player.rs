//! 재생 상태 기계와 명령 처리.
//!
//! 사용자의 명령을 받아 상태를 바꾸고, 재생 파이프라인이 해야 할 일(효과)을 돌려줍니다.
//! 파이프라인을 직접 부르지 않으므로 Windows API 없이 테스트할 수 있습니다.

/// 배속 단계. 모든 단계에서 음정을 유지합니다.
pub const SPEEDS: [f64; 9] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];
/// 컨트롤 바의 배속 표시를 클릭할 때 도는 순서.
const SPEED_CYCLE: [f64; 4] = [1.0, 1.25, 1.5, 2.0];
const NORMAL_SPEED: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Empty,
    Opening,
    Playing,
    Paused,
    Ended,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    TogglePlay,
    ToggleRepeat,
    /// 현재 위치(또는 아직 처리되지 않은 탐색 목표)에서 상대 이동.
    SeekBy(f64),
    /// 절대 위치로 이동. `precise`가 거짓이면 가까운 키프레임을 보여 줍니다.
    SeekTo {
        pos: f64,
        precise: bool,
    },
    Home,
    SpeedUp,
    SpeedDown,
    SpeedReset,
    SpeedCycle,
    VolumeBy(i32),
    SetVolume(u32),
    ToggleMute,
}

/// 파이프라인과 화면이 해야 할 일.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Seek {
        target: f64,
        precise: bool,
    },
    Pause(bool),
    Speed(f64),
    /// 소리에 곱할 값 (0.0~1.0).
    Gain(f32),
    Osd(String),
}

#[derive(Debug, Clone)]
pub struct Player {
    state: State,
    repeat: bool,
    speed_index: usize,
    volume: u32,
    muted: bool,
    duration: f64,
    pending_seek: Option<f64>,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Self {
            state: State::Empty,
            repeat: false,
            speed_index: NORMAL_SPEED,
            volume: 100,
            muted: false,
            duration: 0.0,
            pending_seek: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }
    pub fn speed(&self) -> f64 {
        SPEEDS[self.speed_index]
    }
    pub fn volume(&self) -> u32 {
        self.volume
    }
    pub fn muted(&self) -> bool {
        self.muted
    }
    pub fn duration(&self) -> f64 {
        self.duration
    }
    pub fn is_playing(&self) -> bool {
        self.state == State::Playing
    }
    pub fn repeat(&self) -> bool {
        self.repeat
    }
    /// 미디어가 열려 조작할 수 있는 상태인가.
    pub fn has_media(&self) -> bool {
        matches!(self.state, State::Playing | State::Paused | State::Ended)
    }

    pub fn gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            // 귀에 고르게 들리도록 제곱 곡선을 씁니다.
            let v = self.volume as f32 / 100.0;
            v * v
        }
    }

    /// 파일을 열기 시작했습니다. 배속과 음량은 이전 파일의 것을 이어 씁니다.
    pub fn opening(&mut self) {
        self.state = State::Opening;
        self.repeat = false;
        self.pending_seek = None;
        self.duration = 0.0;
    }

    /// 파일이 열렸습니다. 바로 재생합니다.
    pub fn opened(&mut self, duration: f64) -> Vec<Effect> {
        self.state = State::Playing;
        self.duration = duration.max(0.0);
        vec![
            Effect::Speed(self.speed()),
            Effect::Gain(self.gain()),
            Effect::Pause(false),
        ]
    }

    pub fn failed(&mut self) {
        self.state = State::Error;
        self.pending_seek = None;
    }

    pub fn ended(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        if !self.is_playing() {
            return fx;
        }
        if self.repeat {
            self.seek(0.0, true, &mut fx);
        } else {
            self.state = State::Ended;
            fx.push(Effect::Pause(true));
        }
        fx
    }

    /// 탐색이 끝나 새 프레임이 나왔습니다.
    pub fn seek_settled(&mut self) {
        self.pending_seek = None;
    }

    /// 처리 중인 탐색의 목표. 탐색 바는 이 값을 우선 보여 줍니다.
    pub fn pending_seek(&self) -> Option<f64> {
        self.pending_seek
    }

    /// 명령을 처리합니다. `position`은 지금 화면에 보이는 미디어 시각입니다.
    pub fn handle(&mut self, cmd: Command, position: f64) -> Vec<Effect> {
        let mut fx = Vec::new();
        match cmd {
            Command::VolumeBy(d) => {
                self.volume = (self.volume as i32 + d).clamp(0, 100) as u32;
                self.muted = false;
                fx.push(Effect::Gain(self.gain()));
                fx.push(Effect::Osd(format!("음량 {}%", self.volume)));
                return fx;
            }
            Command::SetVolume(v) => {
                self.volume = v.min(100);
                self.muted = false;
                fx.push(Effect::Gain(self.gain()));
                fx.push(Effect::Osd(format!("음량 {}%", self.volume)));
                return fx;
            }
            Command::ToggleMute => {
                self.muted = !self.muted;
                fx.push(Effect::Gain(self.gain()));
                let text = if self.muted {
                    "음소거".to_string()
                } else {
                    format!("음량 {}%", self.volume)
                };
                fx.push(Effect::Osd(text));
                return fx;
            }
            _ => {}
        }

        if !self.has_media() {
            return fx;
        }

        match cmd {
            Command::TogglePlay => match self.state {
                State::Playing => {
                    self.state = State::Paused;
                    fx.push(Effect::Pause(true));
                }
                State::Paused => {
                    self.state = State::Playing;
                    fx.push(Effect::Pause(false));
                }
                State::Ended => self.seek(0.0, true, &mut fx),
                _ => {}
            },
            Command::ToggleRepeat => {
                self.repeat = !self.repeat;
                let text = if self.repeat {
                    "반복 켜짐"
                } else {
                    "반복 꺼짐"
                };
                fx.push(Effect::Osd(text.into()));
                if self.repeat && self.state == State::Ended {
                    self.seek(0.0, true, &mut fx);
                }
            }
            Command::SeekBy(delta) => {
                let base = self.pending_seek.unwrap_or(position);
                let target = self.clamp(base + delta);
                self.seek(target, true, &mut fx);
                let secs = delta.abs().round() as i64;
                let sign = if delta < 0.0 { '-' } else { '+' };
                fx.push(Effect::Osd(format!("{sign}{secs}초")));
            }
            Command::SeekTo { pos, precise } => {
                let target = self.clamp(pos);
                self.seek(target, precise, &mut fx);
            }
            Command::Home => {
                self.seek(0.0, true, &mut fx);
                fx.push(Effect::Osd("처음으로".into()));
            }
            Command::SpeedUp => {
                self.set_speed((self.speed_index + 1).min(SPEEDS.len() - 1), &mut fx)
            }
            Command::SpeedDown => self.set_speed(self.speed_index.saturating_sub(1), &mut fx),
            Command::SpeedReset => self.set_speed(NORMAL_SPEED, &mut fx),
            Command::SpeedCycle => {
                let cur = self.speed();
                let next = SPEED_CYCLE
                    .iter()
                    .copied()
                    .find(|&s| s > cur + 1e-9)
                    .unwrap_or(1.0);
                let idx = SPEEDS
                    .iter()
                    .position(|&s| s == next)
                    .unwrap_or(NORMAL_SPEED);
                self.set_speed(idx, &mut fx);
            }
            _ => {}
        }
        fx
    }

    fn clamp(&self, t: f64) -> f64 {
        if self.duration > 0.0 {
            t.clamp(0.0, self.duration)
        } else {
            t.max(0.0)
        }
    }

    fn seek(&mut self, target: f64, precise: bool, fx: &mut Vec<Effect>) {
        self.pending_seek = Some(target);
        fx.push(Effect::Seek { target, precise });
        if self.state == State::Ended {
            self.state = State::Playing;
            fx.push(Effect::Pause(false));
        }
    }

    fn set_speed(&mut self, idx: usize, fx: &mut Vec<Effect>) {
        self.speed_index = idx;
        fx.push(Effect::Speed(self.speed()));
        fx.push(Effect::Osd(format_speed(self.speed())));
    }
}

/// `1.5×` 형태의 배속 표시.
pub fn format_speed(speed: f64) -> String {
    let s = format!("{speed:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    let s = if s.contains('.') {
        s.to_string()
    } else {
        format!("{s}.0")
    };
    format!("{s}×")
}

/// `12:34` 또는 `1:23:45` 형태의 시간 표시. `hours`가 참이면 시간 자리를 늘 씁니다.
pub fn format_time(secs: f64, hours: bool) -> String {
    let t = secs.max(0.0) as u64;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if hours || h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(duration: f64) -> Player {
        let mut p = Player::new();
        p.opening();
        p.opened(duration);
        p
    }

    #[test]
    fn starts_empty_with_defaults() {
        let p = Player::new();
        assert_eq!(p.state(), State::Empty);
        assert_eq!(p.speed(), 1.0);
        assert_eq!(p.volume(), 100);
    }

    #[test]
    fn commands_without_media_do_nothing() {
        let mut p = Player::new();
        assert!(p.handle(Command::TogglePlay, 0.0).is_empty());
        assert!(p.handle(Command::SeekBy(5.0), 0.0).is_empty());
    }

    #[test]
    fn toggle_play_pauses_and_resumes() {
        let mut p = playing(100.0);
        assert_eq!(
            p.handle(Command::TogglePlay, 1.0),
            vec![Effect::Pause(true)]
        );
        assert_eq!(p.state(), State::Paused);
        assert_eq!(
            p.handle(Command::TogglePlay, 1.0),
            vec![Effect::Pause(false)]
        );
        assert_eq!(p.state(), State::Playing);
    }

    #[test]
    fn repeated_seeks_accumulate_from_pending_target() {
        let mut p = playing(100.0);
        p.handle(Command::SeekBy(5.0), 10.0);
        // 아직 첫 탐색이 끝나지 않아 화면은 10초에 머물러 있습니다.
        let fx = p.handle(Command::SeekBy(5.0), 10.0);
        assert_eq!(
            fx[0],
            Effect::Seek {
                target: 20.0,
                precise: true
            }
        );
        p.seek_settled();
        let fx = p.handle(Command::SeekBy(5.0), 20.0);
        assert_eq!(
            fx[0],
            Effect::Seek {
                target: 25.0,
                precise: true
            }
        );
    }

    #[test]
    fn seek_is_clamped_to_duration() {
        let mut p = playing(30.0);
        let fx = p.handle(Command::SeekBy(-30.0), 10.0);
        assert_eq!(
            fx[0],
            Effect::Seek {
                target: 0.0,
                precise: true
            }
        );
        assert_eq!(fx[1], Effect::Osd("-30초".into()));
        p.seek_settled();
        let fx = p.handle(Command::SeekBy(30.0), 10.0);
        assert_eq!(
            fx[0],
            Effect::Seek {
                target: 30.0,
                precise: true
            }
        );
    }

    #[test]
    fn play_after_end_restarts() {
        let mut p = playing(30.0);
        assert_eq!(p.ended(), vec![Effect::Pause(true)]);
        assert_eq!(p.state(), State::Ended);
        let fx = p.handle(Command::TogglePlay, 30.0);
        assert_eq!(
            fx,
            vec![
                Effect::Seek {
                    target: 0.0,
                    precise: true
                },
                Effect::Pause(false)
            ]
        );
        assert_eq!(p.state(), State::Playing);
    }

    #[test]
    fn ended_is_ignored_while_paused() {
        let mut p = playing(30.0);
        p.handle(Command::TogglePlay, 1.0);
        p.handle(Command::ToggleRepeat, 1.0);
        assert!(p.ended().is_empty());
        assert_eq!(p.state(), State::Paused);
    }

    #[test]
    fn repeat_rewinds_each_time_until_switched_off() {
        let mut p = playing(30.0);
        assert_eq!(
            p.handle(Command::ToggleRepeat, 15.0),
            vec![Effect::Osd("반복 켜짐".into())]
        );
        assert!(p.repeat());
        for _ in 0..3 {
            assert_eq!(
                p.ended(),
                vec![Effect::Seek {
                    target: 0.0,
                    precise: true
                }]
            );
            assert!(p.is_playing());
            assert_eq!(p.pending_seek(), Some(0.0));
            p.seek_settled();
        }
        assert_eq!(
            p.handle(Command::ToggleRepeat, 10.0),
            vec![Effect::Osd("반복 꺼짐".into())]
        );
        assert!(!p.repeat());
        assert_eq!(p.ended(), vec![Effect::Pause(true)]);
        assert_eq!(p.state(), State::Ended);
    }

    #[test]
    fn enabling_repeat_after_end_restarts_only_this_file() {
        let mut p = playing(30.0);
        p.ended();
        assert_eq!(
            p.handle(Command::ToggleRepeat, 30.0),
            vec![
                Effect::Osd("반복 켜짐".into()),
                Effect::Seek {
                    target: 0.0,
                    precise: true
                },
                Effect::Pause(false),
            ]
        );
        assert!(p.is_playing());
        p.opening();
        p.opened(10.0);
        assert_eq!(p.ended(), vec![Effect::Pause(true)]);
    }

    #[test]
    fn speed_steps_stop_at_ends() {
        let mut p = playing(30.0);
        for _ in 0..20 {
            p.handle(Command::SpeedUp, 0.0);
        }
        assert_eq!(p.speed(), 4.0);
        for _ in 0..20 {
            p.handle(Command::SpeedDown, 0.0);
        }
        assert_eq!(p.speed(), 0.5);
        p.handle(Command::SpeedReset, 0.0);
        assert_eq!(p.speed(), 1.0);
    }

    #[test]
    fn speed_cycle_order() {
        let mut p = playing(30.0);
        let mut seen = vec![];
        for _ in 0..5 {
            p.handle(Command::SpeedCycle, 0.0);
            seen.push(p.speed());
        }
        assert_eq!(seen, vec![1.25, 1.5, 2.0, 1.0, 1.25]);
        // 순환에 없는 값에서는 그보다 큰 다음 값으로 갑니다.
        p.handle(Command::SpeedReset, 0.0);
        p.handle(Command::SpeedDown, 0.0);
        p.handle(Command::SpeedCycle, 0.0);
        assert_eq!(p.speed(), 1.0);
        for _ in 0..8 {
            p.handle(Command::SpeedUp, 0.0);
        }
        p.handle(Command::SpeedCycle, 0.0);
        assert_eq!(p.speed(), 1.0);
    }

    #[test]
    fn volume_is_clamped_and_unmutes() {
        let mut p = Player::new();
        p.handle(Command::ToggleMute, 0.0);
        assert_eq!(p.gain(), 0.0);
        let fx = p.handle(Command::VolumeBy(5), 0.0);
        assert!(!p.muted());
        assert_eq!(p.volume(), 100);
        assert_eq!(fx[1], Effect::Osd("음량 100%".into()));
        for _ in 0..30 {
            p.handle(Command::VolumeBy(-5), 0.0);
        }
        assert_eq!(p.volume(), 0);
    }

    #[test]
    fn speed_text() {
        assert_eq!(format_speed(1.0), "1.0×");
        assert_eq!(format_speed(1.5), "1.5×");
        assert_eq!(format_speed(1.25), "1.25×");
        assert_eq!(format_speed(0.75), "0.75×");
        assert_eq!(format_speed(4.0), "4.0×");
    }

    #[test]
    fn time_text() {
        assert_eq!(format_time(0.0, false), "0:00");
        assert_eq!(format_time(754.9, false), "12:34");
        assert_eq!(format_time(5025.0, false), "1:23:45");
        assert_eq!(format_time(65.0, true), "0:01:05");
        assert_eq!(format_time(-3.0, false), "0:00");
    }
}
