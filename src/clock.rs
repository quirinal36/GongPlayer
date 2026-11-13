//! 재생 시계.
//!
//! 기본은 벽시계(경과 시간 × 배속)이고, 소리가 나오는 동안은 소리 시계의 관측값으로
//! 계속 보정합니다. 그래서 소리가 없거나 소리 장치가 사라져도 같은 시계로 재생이
//! 이어집니다. 시간은 모두 인자로 받으므로 Windows API를 부르지 않고 테스트할 수 있습니다.

/// 이보다 크게 어긋나면 보정하지 않고 바로 맞춥니다 (초).
const HARD_SYNC: f64 = 0.15;
/// 작게 어긋날 때 한 번에 줄이는 비율.
const SLEW: f64 = 0.1;
/// 이보다 작은 오차는 무시합니다. 관측값의 떨림이 화면에 전해지지 않게 합니다.
const DEAD_ZONE: f64 = 0.004;
/// 소리 관측값이 이보다 오래되면 믿지 않습니다 (초).
const OBSERVATION_TTL: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clock {
    base_media: f64,
    base_time: f64,
    speed: f64,
    running: bool,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub const fn new() -> Self {
        Self {
            base_media: 0.0,
            base_time: 0.0,
            speed: 1.0,
            running: false,
        }
    }

    /// 시각 `t`(벽시계, 초)의 미디어 시각.
    pub fn now(&self, t: f64) -> f64 {
        if self.running {
            self.base_media + (t - self.base_time) * self.speed
        } else {
            self.base_media
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// 미디어 시각을 바로 정합니다. 탐색이나 첫 프레임 표시 때 씁니다.
    pub fn set(&mut self, media: f64, t: f64) {
        self.base_media = media;
        self.base_time = t;
    }

    pub fn pause(&mut self, t: f64) {
        if self.running {
            self.base_media = self.now(t);
            self.running = false;
        }
    }

    pub fn resume(&mut self, t: f64) {
        if !self.running {
            self.base_time = t;
            self.running = true;
        }
    }

    pub fn set_speed(&mut self, speed: f64, t: f64) {
        self.base_media = self.now(t);
        self.base_time = t;
        self.speed = speed;
    }

    /// 기준 시계(소리)의 값 `master`에 맞춥니다. 크게 어긋나면 바로 맞추고,
    /// 작게 어긋나면 조금씩 줄입니다. 돌려주는 값은 보정 전의 오차입니다.
    pub fn sync(&mut self, master: f64, t: f64) -> f64 {
        let diff = master - self.now(t);
        if !self.running {
            return diff;
        }
        if diff.abs() > HARD_SYNC {
            self.set(master, t);
        } else if diff.abs() > DEAD_ZONE {
            self.base_media += diff * SLEW;
        }
        diff
    }
}

/// 소리 출력 콜백이 남기는 관측값: "벽시계 `at`에 미디어 시각 `media`의 소리가
/// 스피커에서 나오고 있었다". 그 사이 배속은 `speed`였습니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioObservation {
    pub media: f64,
    pub at: f64,
    pub speed: f64,
}

impl AudioObservation {
    /// 관측값을 시각 `t`로 늘여 본 기준 시각. 너무 오래된 관측값이면 `None`입니다.
    pub fn extrapolate(&self, t: f64) -> Option<f64> {
        let age = t - self.at;
        if !(-OBSERVATION_TTL..=OBSERVATION_TTL).contains(&age) {
            return None;
        }
        Some(self.media + age * self.speed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn stopped_clock_does_not_advance() {
        let c = Clock::new();
        assert_eq!(c.now(10.0), 0.0);
    }

    #[test]
    fn running_clock_advances_with_speed() {
        let mut c = Clock::new();
        c.set(5.0, 100.0);
        c.resume(100.0);
        assert!(close(c.now(101.0), 6.0));
        c.set_speed(2.0, 101.0);
        assert!(close(c.now(102.0), 8.0));
    }

    #[test]
    fn pause_freezes_and_resume_continues() {
        let mut c = Clock::new();
        c.resume(0.0);
        c.pause(3.0);
        assert!(close(c.now(50.0), 3.0));
        c.resume(50.0);
        assert!(close(c.now(51.0), 4.0));
    }

    #[test]
    fn large_drift_snaps() {
        let mut c = Clock::new();
        c.resume(0.0);
        c.sync(10.0, 1.0);
        assert!(close(c.now(1.0), 10.0));
    }

    #[test]
    fn small_drift_slews() {
        let mut c = Clock::new();
        c.resume(0.0);
        let before = c.now(1.0);
        c.sync(1.05, 1.0);
        let after = c.now(1.0);
        assert!(after > before && after < 1.05);
        // 계속 맞추면 기준에 수렴합니다.
        for _ in 0..100 {
            c.sync(1.05, 1.0);
        }
        assert!((c.now(1.0) - 1.05).abs() < DEAD_ZONE + 1e-6);
    }

    #[test]
    fn tiny_drift_is_ignored() {
        let mut c = Clock::new();
        c.resume(0.0);
        c.sync(1.001, 1.0);
        assert!(close(c.now(1.0), 1.0));
    }

    #[test]
    fn paused_clock_is_not_synced() {
        let mut c = Clock::new();
        c.set(2.0, 0.0);
        c.sync(9.0, 1.0);
        assert!(close(c.now(1.0), 2.0));
    }

    #[test]
    fn observation_extrapolates_and_expires() {
        let o = AudioObservation {
            media: 10.0,
            at: 100.0,
            speed: 1.5,
        };
        assert!(close(o.extrapolate(100.1).unwrap(), 10.15));
        assert!(o.extrapolate(101.0).is_none());
    }
}
