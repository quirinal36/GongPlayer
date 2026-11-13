//! 소리 출력 장치. 콜백은 링 버퍼에서 꺼내 음량을 곱하고 장치에 넘깁니다.
//! 콜백 안에서는 잠금, 할당, 파일 접근을 하지 않습니다.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::clock::AudioObservation;
use crate::media::SessionShared;
use crate::ring::AudioRing;

/// 링 버퍼 용량 (초). 실제로 채우는 양은 소리 스레드가 더 작게 유지합니다.
const RING_SECONDS: f64 = 0.4;

pub struct AudioOut {
    stream: cpal::Stream,
    pub ring: Arc<AudioRing>,
    pub rate: u32,
    pub channels: usize,
    pub failed: Arc<AtomicBool>,
}

impl AudioOut {
    pub fn open(s: &Arc<SessionShared>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("소리 출력 장치가 없습니다")?;
        let config = device
            .default_output_config()
            .map_err(|e| format!("소리 장치 설정을 읽을 수 없습니다: {e}"))?;
        if config.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!(
                "지원하지 않는 소리 장치 형식입니다: {:?}",
                config.sample_format()
            ));
        }
        let config: cpal::StreamConfig = config.into();
        let rate = config.sample_rate;
        let channels = config.channels as usize;
        let ring = Arc::new(AudioRing::new(
            (rate as f64 * RING_SECONDS) as usize,
            channels,
            rate,
        ));
        let failed = Arc::new(AtomicBool::new(false));

        let cb_ring = ring.clone();
        let cb_s = s.clone();
        let err_failed = failed.clone();
        let err_s = s.clone();
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    let ts = info.timestamp();
                    let latency = ts.playback.duration_since(ts.callback).as_secs_f64();
                    let r = cb_ring.read(data);
                    let filled = r.frames * channels;
                    data[filled..].fill(0.0);
                    let gain = cb_s.audio.gain();
                    if gain != 1.0 {
                        for x in &mut data[..filled] {
                            *x *= gain;
                        }
                    }
                    if r.frames > 0
                        && let Some((media, speed)) = r.media
                    {
                        cb_s.app.audio_obs.store(AudioObservation {
                            media,
                            at: crate::shared::now() + latency,
                            speed,
                        });
                    }
                },
                move |e| {
                    eprintln!("소리 출력 오류: {e}");
                    err_failed.store(true, Ordering::SeqCst);
                    err_s.audio.wake.notify();
                },
                None,
            )
            .map_err(|e| format!("소리 출력을 열 수 없습니다: {e}"))?;
        Ok(Self {
            stream,
            ring,
            rate,
            channels,
            failed,
        })
    }

    pub fn play(&self) {
        let _ = self.stream.play();
    }

    pub fn pause(&self) {
        let _ = self.stream.pause();
    }

    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
}
