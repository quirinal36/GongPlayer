//! 소리 링 버퍼. 단일 생산자(소리 디코드 스레드), 단일 소비자(소리 출력 콜백).
//!
//! 소비자 쪽은 잠금도 할당도 하지 않습니다. 샘플과 함께 "이 위치의 샘플은 미디어 시각
//! 몇 초이고 배속은 얼마였다"는 표시를 남겨, 콜백이 지금 내보내는 소리의 미디어 시각을
//! 알 수 있게 합니다. 위치는 모두 프레임(채널 묶음) 단위의 누적값입니다.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const MARKERS: usize = 256;

struct Marker {
    pos: AtomicU64,
    media: AtomicU64,
    speed: AtomicU64,
}

pub struct AudioRing {
    samples: Box<[AtomicU32]>,
    /// 프레임 단위 용량 (2의 거듭제곱).
    frames: u64,
    channels: usize,
    rate: f64,
    write: AtomicU64,
    read: AtomicU64,
    discard_until: AtomicU64,
    markers: Box<[Marker]>,
    marker_write: AtomicU64,
    marker_read: AtomicU64,
}

/// 소비자가 읽은 덩어리의 정보.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadInfo {
    pub frames: usize,
    /// 읽은 첫 프레임의 미디어 시각과 그때의 배속. 표시가 없으면 `None`.
    pub media: Option<(f64, f64)>,
}

impl AudioRing {
    /// `min_frames` 이상을 담는 링을 만듭니다.
    pub fn new(min_frames: usize, channels: usize, rate: u32) -> Self {
        let frames = min_frames.max(2).next_power_of_two() as u64;
        let samples = (0..frames as usize * channels)
            .map(|_| AtomicU32::new(0))
            .collect();
        let markers = (0..MARKERS)
            .map(|_| Marker {
                pos: AtomicU64::new(0),
                media: AtomicU64::new(0),
                speed: AtomicU64::new(0),
            })
            .collect();
        Self {
            samples,
            frames,
            channels,
            rate: rate as f64,
            write: AtomicU64::new(0),
            read: AtomicU64::new(0),
            discard_until: AtomicU64::new(0),
            markers,
            marker_write: AtomicU64::new(0),
            marker_read: AtomicU64::new(0),
        }
    }

    // ---- 생산자 쪽 ----

    /// 지금 더 넣을 수 있는 프레임 수.
    pub fn free_frames(&self) -> usize {
        let w = self.write.load(Ordering::Relaxed);
        let r = self
            .read
            .load(Ordering::Acquire)
            .max(self.discard_until.load(Ordering::Relaxed));
        (self.frames - (w - r.min(w))) as usize
    }

    /// 아직 소비되지 않은 프레임 수.
    pub fn queued_frames(&self) -> usize {
        self.frames as usize - self.free_frames()
    }

    /// 다음에 쓸 프레임의 미디어 시각을 표시합니다. 표시 칸이 모자라면 건너뜁니다.
    pub fn mark(&self, media: f64, speed: f64) {
        let mw = self.marker_write.load(Ordering::Relaxed);
        let mr = self.marker_read.load(Ordering::Acquire);
        if mw - mr >= MARKERS as u64 - 1 {
            return;
        }
        let m = &self.markers[(mw % MARKERS as u64) as usize];
        m.pos
            .store(self.write.load(Ordering::Relaxed), Ordering::Relaxed);
        m.media.store(media.to_bits(), Ordering::Relaxed);
        m.speed.store(speed.to_bits(), Ordering::Relaxed);
        self.marker_write.store(mw + 1, Ordering::Release);
    }

    /// 인터리브된 샘플을 들어가는 만큼 넣고, 넣은 프레임 수를 돌려줍니다.
    pub fn write(&self, interleaved: &[f32]) -> usize {
        let want = interleaved.len() / self.channels;
        let n = want.min(self.free_frames());
        let w = self.write.load(Ordering::Relaxed);
        for i in 0..n {
            let slot = ((w + i as u64) & (self.frames - 1)) as usize * self.channels;
            for c in 0..self.channels {
                self.samples[slot + c].store(
                    interleaved[i * self.channels + c].to_bits(),
                    Ordering::Relaxed,
                );
            }
        }
        self.write.store(w + n as u64, Ordering::Release);
        n
    }

    /// 지금까지 넣은 것을 모두 버리게 합니다. 실제로 건너뛰는 일은 소비자가 합니다.
    pub fn discard(&self) {
        self.discard_until
            .store(self.write.load(Ordering::Relaxed), Ordering::Release);
    }

    // ---- 소비자 쪽 (잠금·할당 없음) ----

    /// `out`을 채울 만큼 읽습니다. 모자라는 부분은 호출하는 쪽이 무음으로 채웁니다.
    pub fn read(&self, out: &mut [f32]) -> ReadInfo {
        let mut r = self.read.load(Ordering::Relaxed);
        let discard = self.discard_until.load(Ordering::Acquire);
        if r < discard {
            r = discard;
        }
        let w = self.write.load(Ordering::Acquire);
        let n = ((w - r.min(w)) as usize).min(out.len() / self.channels);
        let media = self.media_at(r);
        for i in 0..n {
            let slot = ((r + i as u64) & (self.frames - 1)) as usize * self.channels;
            for c in 0..self.channels {
                out[i * self.channels + c] =
                    f32::from_bits(self.samples[slot + c].load(Ordering::Relaxed));
            }
        }
        self.read.store(r + n as u64, Ordering::Release);
        ReadInfo { frames: n, media }
    }

    /// 위치 `pos`의 미디어 시각. 지나간 표시는 정리합니다.
    fn media_at(&self, pos: u64) -> Option<(f64, f64)> {
        let mw = self.marker_write.load(Ordering::Acquire);
        let mut mr = self.marker_read.load(Ordering::Relaxed);
        if mr >= mw {
            return None;
        }
        // 다음 표시가 이미 지났으면 넘어갑니다.
        while mr + 1 < mw {
            let next = &self.markers[((mr + 1) % MARKERS as u64) as usize];
            if next.pos.load(Ordering::Relaxed) <= pos {
                mr += 1;
            } else {
                break;
            }
        }
        self.marker_read.store(mr, Ordering::Release);
        let m = &self.markers[(mr % MARKERS as u64) as usize];
        let mpos = m.pos.load(Ordering::Relaxed);
        if mpos > pos {
            return None;
        }
        let media = f64::from_bits(m.media.load(Ordering::Relaxed));
        let speed = f64::from_bits(m.speed.load(Ordering::Relaxed));
        Some((media + (pos - mpos) as f64 / self.rate * speed, speed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let ring = AudioRing::new(8, 2, 48000);
        assert_eq!(ring.write(&[1.0, 2.0, 3.0, 4.0]), 2);
        let mut out = [0.0; 8];
        let info = ring.read(&mut out);
        assert_eq!(info.frames, 2);
        assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn write_stops_when_full_and_wraps() {
        let ring = AudioRing::new(4, 1, 48000);
        assert_eq!(ring.write(&[1.0; 6]), 4);
        assert_eq!(ring.free_frames(), 0);
        let mut out = [0.0; 3];
        assert_eq!(ring.read(&mut out).frames, 3);
        assert_eq!(ring.write(&[5.0, 6.0, 7.0]), 3);
        let mut out = [0.0; 4];
        assert_eq!(ring.read(&mut out).frames, 4);
        assert_eq!(out, [1.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn discard_skips_written_frames() {
        let ring = AudioRing::new(8, 1, 48000);
        ring.write(&[1.0, 2.0, 3.0]);
        ring.discard();
        assert_eq!(ring.free_frames(), 8);
        ring.write(&[9.0]);
        let mut out = [0.0; 4];
        let info = ring.read(&mut out);
        assert_eq!(info.frames, 1);
        assert_eq!(out[0], 9.0);
    }

    #[test]
    fn markers_give_media_time() {
        let ring = AudioRing::new(64, 1, 10);
        ring.mark(100.0, 1.0);
        ring.write(&[0.0; 10]);
        ring.mark(200.0, 2.0);
        ring.write(&[0.0; 10]);
        let mut out = [0.0; 5];
        let info = ring.read(&mut out);
        assert_eq!(info.media, Some((100.0, 1.0)));
        let info = ring.read(&mut out);
        assert_eq!(info.media, Some((100.5, 1.0)));
        let info = ring.read(&mut out);
        assert_eq!(info.media, Some((200.0, 2.0)));
        let info = ring.read(&mut out);
        assert_eq!(info.media, Some((201.0, 2.0)));
    }

    #[test]
    fn no_marker_means_no_media_time() {
        let ring = AudioRing::new(8, 1, 10);
        ring.write(&[0.0; 2]);
        let mut out = [0.0; 2];
        assert_eq!(ring.read(&mut out).media, None);
    }
}
