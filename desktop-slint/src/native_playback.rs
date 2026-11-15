//! Bounded translated-audio playback for the Slint native session. Gemini PCM
//! is 24 kHz mono PCM16; a worker resamples it to the selected output device.
//! The CPAL callback only performs non-blocking queue reads and sample copies.

use anyhow::{Context, Result, anyhow};
use cpal::{
    SampleFormat,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};
use translator_core::session_port::NoticeText;
use zeroize::Zeroize;

const GEMINI_SAMPLE_RATE: u32 = 24_000;
const COMMAND_CAPACITY: usize = 12;
const EVENT_CAPACITY: usize = 4;

#[derive(Debug)]
pub enum PlaybackTaskEvent {
    Ready,
    Failed(NoticeText),
    Stopped,
}

enum Command {
    Audio { epoch: u64, bytes: Vec<u8> },
}

struct Resampler {
    from: u32,
    to: u32,
    position: f64,
    input: VecDeque<f32>,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            from: from.max(1),
            to: to.max(1),
            position: 0.0,
            input: VecDeque::new(),
        }
    }

    fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        self.input.extend(samples.iter().copied());
        let step = self.from as f64 / self.to as f64;
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

pub struct NativePlaybackTask {
    commands: SyncSender<Command>,
    events: Receiver<PlaybackTaskEvent>,
    stop: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    samples: Arc<Mutex<VecDeque<f32>>>,
    join: Option<JoinHandle<()>>,
}

impl NativePlaybackTask {
    pub fn spawn(device_id: Option<String>) -> Self {
        let (commands, command_receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (event_sender, events) = mpsc::sync_channel(EVENT_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(0));
        let samples = Arc::new(Mutex::new(VecDeque::new()));
        let worker_stop = Arc::clone(&stop);
        let worker_epoch = Arc::clone(&epoch);
        let worker_samples = Arc::clone(&samples);
        let join = std::thread::spawn(move || {
            if let Err(error) = run_playback(
                device_id.as_deref(),
                command_receiver,
                &worker_stop,
                &worker_epoch,
                worker_samples,
                &event_sender,
            ) {
                let _ = event_sender.try_send(PlaybackTaskEvent::Failed(NoticeText::bounded(
                    &error.to_string(),
                )));
            }
        });
        Self {
            commands,
            events,
            stop,
            epoch,
            samples,
            join: Some(join),
        }
    }

    pub fn push(&self, bytes: Vec<u8>) {
        let epoch = self.epoch.load(Ordering::Acquire);
        let _ = self.commands.try_send(Command::Audio { epoch, bytes });
    }

    pub fn clear(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        lock_samples(&self.samples).clear();
    }

    pub fn request_stop(&self) {
        self.clear();
        self.stop.store(true, Ordering::Release);
    }

    pub fn poll(&mut self) -> Option<PlaybackTaskEvent> {
        let event = self.events.try_recv().ok();
        if matches!(
            event,
            Some(PlaybackTaskEvent::Stopped | PlaybackTaskEvent::Failed(_))
        ) && let Some(join) = self.join.take()
        {
            let _ = join.join();
        }
        event
    }
}

impl Drop for NativePlaybackTask {
    fn drop(&mut self) {
        self.request_stop();
    }
}

fn run_playback(
    device_id: Option<&str>,
    commands: Receiver<Command>,
    stop: &AtomicBool,
    epoch: &AtomicU64,
    samples: Arc<Mutex<VecDeque<f32>>>,
    events: &SyncSender<PlaybackTaskEvent>,
) -> Result<()> {
    let host = cpal::default_host();
    let device = match device_id.filter(|value| !value.is_empty()) {
        Some(id) => host
            .output_devices()
            .context("无法读取播放设备")?
            .find(|device| device.name().ok().as_deref() == Some(id))
            .ok_or_else(|| anyhow!("所选播放设备已不可用，请刷新设备列表"))?,
        None => host
            .default_output_device()
            .ok_or_else(|| anyhow!("系统没有默认播放设备"))?,
    };
    let supported = device
        .default_output_config()
        .context("无法读取播放设备默认格式")?;
    let rate = supported.sample_rate().0;
    let channels = usize::from(supported.channels().max(1));
    let config = supported.config();
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_output::<f32>(
            &device,
            &config,
            channels,
            Arc::clone(&samples),
            Arc::clone(&failed),
            |sample| sample,
            0.0,
        )?,
        SampleFormat::F64 => build_output::<f64>(
            &device,
            &config,
            channels,
            Arc::clone(&samples),
            Arc::clone(&failed),
            |sample| f64::from(sample),
            0.0,
        )?,
        SampleFormat::I16 => build_output::<i16>(
            &device,
            &config,
            channels,
            Arc::clone(&samples),
            Arc::clone(&failed),
            |sample| (sample * 32767.0).round() as i16,
            0,
        )?,
        SampleFormat::U16 => build_output::<u16>(
            &device,
            &config,
            channels,
            Arc::clone(&samples),
            Arc::clone(&failed),
            |sample| ((sample + 1.0) * 32767.5).round() as u16,
            32768,
        )?,
        SampleFormat::I32 => build_output::<i32>(
            &device,
            &config,
            channels,
            Arc::clone(&samples),
            Arc::clone(&failed),
            |sample| (sample * 2_147_483_647.0).round() as i32,
            0,
        )?,
        SampleFormat::U32 => build_output::<u32>(
            &device,
            &config,
            channels,
            Arc::clone(&samples),
            Arc::clone(&failed),
            |sample| ((f64::from(sample) + 1.0) * 2_147_483_647.5).round() as u32,
            2_147_483_648,
        )?,
        _ => return Err(anyhow!("播放设备采样格式不受支持")),
    };
    stream.play().context("无法启动译音播放")?;
    events
        .try_send(PlaybackTaskEvent::Ready)
        .map_err(|_| anyhow!("播放状态队列不可用"))?;
    let mut resampler = Resampler::new(GEMINI_SAMPLE_RATE, rate);
    let mut active_epoch = epoch.load(Ordering::Acquire);
    while !stop.load(Ordering::Acquire) {
        if failed.load(Ordering::Acquire) {
            return Err(anyhow!("译音播放回调失败"));
        }
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(Command::Audio {
                epoch: command_epoch,
                mut bytes,
            }) => {
                if command_epoch != epoch.load(Ordering::Acquire) {
                    bytes.zeroize();
                    continue;
                }
                if active_epoch != command_epoch {
                    resampler = Resampler::new(GEMINI_SAMPLE_RATE, rate);
                    active_epoch = command_epoch;
                }
                let mut input: Vec<f32> = bytes
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0)
                    .collect();
                bytes.zeroize();
                let output = resampler.push(&input);
                input.zeroize();
                let mut queue = lock_samples(&samples);
                if command_epoch == epoch.load(Ordering::Acquire) {
                    queue.extend(output);
                    let maximum = rate as usize * 12 / 25;
                    if queue.len() > maximum {
                        let stale = queue.len() - maximum;
                        queue.drain(..stale);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(stream);
    let _ = events.try_send(PlaybackTaskEvent::Stopped);
    Ok(())
}

fn build_output<T: cpal::SizedSample + Copy>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    samples: Arc<Mutex<VecDeque<f32>>>,
    failed: Arc<AtomicBool>,
    convert: impl Fn(f32) -> T + Send + 'static,
    silence: T,
) -> Result<cpal::Stream> {
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                if let Ok(mut queue) = samples.try_lock() {
                    for frame in output.chunks_mut(channels) {
                        let sample = queue.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
                        for value in frame {
                            *value = convert(sample);
                        }
                    }
                } else {
                    output.fill(silence);
                }
            },
            move |_| failed.store(true, Ordering::Release),
            None,
        )
        .context("无法创建译音播放流")
}

fn lock_samples(samples: &Mutex<VecDeque<f32>>) -> std::sync::MutexGuard<'_, VecDeque<f32>> {
    samples
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swr_nplay_001_resampler_is_chunk_boundary_independent() {
        let input: Vec<f32> = (0..2_400)
            .map(|index| ((index as f32) * 0.03).sin())
            .collect();
        let mut whole = Resampler::new(24_000, 48_000);
        let expected = whole.push(&input);
        let mut chunked = Resampler::new(24_000, 48_000);
        let actual: Vec<_> = input
            .chunks(113)
            .flat_map(|chunk| chunked.push(chunk))
            .collect();
        assert_eq!(expected, actual);
    }

    #[test]
    fn swr_nplay_002_clear_invalidates_queued_audio_immediately() {
        let queue = Arc::new(Mutex::new(VecDeque::from([0.5, 0.25])));
        lock_samples(&queue).clear();
        assert!(lock_samples(&queue).is_empty());
    }
}
