//! CPAL microphone capture for the v2.1 native desktop path. The callback only
//! converts into a bounded raw queue; resampling, framing and network enqueue
//! run on the owning worker thread. The v2.2 zero-allocation callback replaces
//! this compatibility implementation after v2.1 feature parity.

use anyhow::{Context, Result, anyhow};
use cpal::{
    SampleFormat,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::mpsc as tokio_mpsc;
use translator_core::session_port::NoticeText;

pub const PCM_SAMPLE_RATE: u32 = 16_000;
pub const PCM_FRAME_SAMPLES: usize = 640;
pub const PCM_FRAME_BYTES: usize = PCM_FRAME_SAMPLES * 2;
const RAW_QUEUE_CHUNKS: usize = 12;
const EVENT_QUEUE_CAPACITY: usize = 4;

#[derive(Debug)]
pub enum AudioTaskEvent {
    Ready,
    Failed(NoticeText),
    Stopped,
}

struct RawChunk {
    samples: Vec<f32>,
}

enum RawEvent {
    Data(RawChunk),
    Failed,
}

struct Resampler {
    from: u32,
    position: f64,
    input: VecDeque<f32>,
    lowpass: f32,
}

impl Resampler {
    fn new(from: u32) -> Self {
        Self {
            from: from.max(1),
            position: 0.0,
            input: VecDeque::new(),
            lowpass: 0.0,
        }
    }

    fn push_interleaved(&mut self, data: &[f32], channels: u16) -> Vec<f32> {
        let channels = usize::from(channels.max(1));
        let cutoff = (PCM_SAMPLE_RATE as f32 * 0.45).min(self.from as f32 * 0.45);
        let coefficient = (std::f32::consts::TAU * cutoff
            / (self.from as f32 + std::f32::consts::TAU * cutoff))
            .clamp(0.0, 1.0);
        for frame in data.chunks_exact(channels) {
            let sample = frame.iter().copied().sum::<f32>() / channels as f32;
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            if self.from > PCM_SAMPLE_RATE {
                self.lowpass += coefficient * (sample - self.lowpass);
                self.input.push_back(self.lowpass);
            } else {
                self.input.push_back(sample);
            }
        }
        let step = self.from as f64 / PCM_SAMPLE_RATE as f64;
        let mut output = Vec::new();
        while self.position + 1.0 < self.input.len() as f64 {
            let index = self.position as usize;
            let fraction = (self.position - index as f64) as f32;
            output.push(self.input[index] * (1.0 - fraction) + self.input[index + 1] * fraction);
            self.position += step;
        }
        let consumed = (self.position as usize).min(self.input.len().saturating_sub(1));
        self.input.drain(..consumed);
        self.position -= consumed as f64;
        output
    }
}

struct PcmFramer {
    resampler: Resampler,
    channels: u16,
    samples: VecDeque<f32>,
}

impl PcmFramer {
    fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            resampler: Resampler::new(sample_rate),
            channels,
            samples: VecDeque::with_capacity(PCM_FRAME_SAMPLES * 2),
        }
    }

    fn push(&mut self, samples: &[f32], output: &tokio_mpsc::Sender<Vec<u8>>) {
        self.samples
            .extend(self.resampler.push_interleaved(samples, self.channels));
        while self.samples.len() >= PCM_FRAME_SAMPLES {
            let mut pcm = Vec::with_capacity(PCM_FRAME_BYTES);
            for sample in self.samples.drain(..PCM_FRAME_SAMPLES) {
                let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                pcm.extend_from_slice(&value.to_le_bytes());
            }
            if output.try_send(pcm).is_err() && output.is_closed() {
                break;
            }
        }
    }
}

pub struct NativeAudioTask {
    stop: Arc<AtomicBool>,
    events: Receiver<AudioTaskEvent>,
    join: Option<JoinHandle<()>>,
}

impl NativeAudioTask {
    pub fn spawn(device_id: Option<String>, output: tokio_mpsc::Sender<Vec<u8>>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (event_sender, events) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let join = std::thread::spawn(move || {
            if let Err(error) =
                run_capture(device_id.as_deref(), output, &worker_stop, &event_sender)
            {
                let _ = event_sender.try_send(AudioTaskEvent::Failed(NoticeText::bounded(
                    &error.to_string(),
                )));
            }
        });
        Self {
            stop,
            events,
            join: Some(join),
        }
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn poll(&mut self) -> Option<AudioTaskEvent> {
        let event = self.events.try_recv().ok();
        if matches!(
            event,
            Some(AudioTaskEvent::Stopped | AudioTaskEvent::Failed(_))
        ) && let Some(join) = self.join.take()
        {
            let _ = join.join();
        }
        event
    }
}

impl Drop for NativeAudioTask {
    fn drop(&mut self) {
        self.request_stop();
    }
}

fn run_capture(
    device_id: Option<&str>,
    output: tokio_mpsc::Sender<Vec<u8>>,
    stop: &AtomicBool,
    events: &SyncSender<AudioTaskEvent>,
) -> Result<()> {
    let host = cpal::default_host();
    let device = match device_id.filter(|value| !value.is_empty()) {
        Some(id) => host
            .input_devices()
            .context("无法读取麦克风设备")?
            .find(|device| device.name().ok().as_deref() == Some(id))
            .ok_or_else(|| anyhow!("所选麦克风已不可用，请刷新设备列表"))?,
        None => host
            .default_input_device()
            .ok_or_else(|| anyhow!("系统没有默认麦克风"))?,
    };
    let supported = device
        .default_input_config()
        .context("无法读取麦克风默认格式")?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let config = supported.config();
    let (raw_sender, raw_receiver) = mpsc::sync_channel(RAW_QUEUE_CHUNKS);
    let error_sender = raw_sender.clone();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => {
            build_stream::<f32>(&device, &config, raw_sender, error_sender, |value| *value)?
        }
        SampleFormat::F64 => {
            build_stream::<f64>(&device, &config, raw_sender, error_sender, |value| {
                *value as f32
            })?
        }
        SampleFormat::I16 => {
            build_stream::<i16>(&device, &config, raw_sender, error_sender, |value| {
                *value as f32 / 32768.0
            })?
        }
        SampleFormat::U16 => {
            build_stream::<u16>(&device, &config, raw_sender, error_sender, |value| {
                (*value as f32 - 32768.0) / 32768.0
            })?
        }
        SampleFormat::I32 => {
            build_stream::<i32>(&device, &config, raw_sender, error_sender, |value| {
                *value as f32 / 2_147_483_648.0
            })?
        }
        SampleFormat::U32 => {
            build_stream::<u32>(&device, &config, raw_sender, error_sender, |value| {
                (*value as f64 / 2_147_483_648.0 - 1.0) as f32
            })?
        }
        _ => return Err(anyhow!("麦克风采样格式不受支持")),
    };
    stream.play().context("无法启动麦克风采集")?;
    events
        .try_send(AudioTaskEvent::Ready)
        .map_err(|_| anyhow!("音频状态队列不可用"))?;
    let mut framer = PcmFramer::new(sample_rate, channels);
    while !stop.load(Ordering::Acquire) {
        match raw_receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(RawEvent::Data(chunk)) => framer.push(&chunk.samples, &output),
            Ok(RawEvent::Failed) => return Err(anyhow!("麦克风采集回调失败")),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("麦克风采集意外停止"));
            }
        }
    }
    drop(stream);
    let _ = events.try_send(AudioTaskEvent::Stopped);
    Ok(())
}

fn build_stream<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: SyncSender<RawEvent>,
    errors: SyncSender<RawEvent>,
    convert: impl Fn(&T) -> f32 + Send + 'static,
) -> Result<cpal::Stream> {
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let converted = data.iter().map(&convert).collect();
                let _ = samples.try_send(RawEvent::Data(RawChunk { samples: converted }));
            },
            move |_| {
                let _ = errors.try_send(RawEvent::Failed);
            },
            None,
        )
        .context("无法创建麦克风采集流")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swr_naudio_001_resampling_is_chunk_boundary_independent() {
        let input: Vec<f32> = (0..4_800)
            .map(|index| ((index as f32) * 0.03).sin())
            .collect();
        let mut whole = Resampler::new(48_000);
        let expected = whole.push_interleaved(&input, 1);
        let mut chunked = Resampler::new(48_000);
        let actual: Vec<_> = input
            .chunks(113)
            .flat_map(|chunk| chunked.push_interleaved(chunk, 1))
            .collect();
        assert_eq!(expected, actual);
        assert!((1_598..=1_601).contains(&expected.len()));
    }

    #[tokio::test]
    async fn swr_naudio_002_framer_emits_exact_bounded_pcm_frames() {
        let (sender, mut receiver) = tokio_mpsc::channel(2);
        let mut framer = PcmFramer::new(PCM_SAMPLE_RATE, 1);
        framer.push(&vec![0.5; PCM_FRAME_SAMPLES + 1], &sender);
        let frame = receiver.recv().await.expect("PCM frame");
        assert_eq!(frame.len(), PCM_FRAME_BYTES);
        assert!(receiver.try_recv().is_err());
        assert_eq!(i16::from_le_bytes([frame[0], frame[1]]), 16_384);
    }
}
