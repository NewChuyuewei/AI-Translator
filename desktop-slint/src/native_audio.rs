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
    time::{Duration, Instant},
};
use tokio::sync::mpsc as tokio_mpsc;
use translator_core::{AudioSourceMode, session_port::NoticeText};

pub const PCM_SAMPLE_RATE: u32 = 16_000;
pub const PCM_FRAME_SAMPLES: usize = 640;
pub const PCM_FRAME_BYTES: usize = PCM_FRAME_SAMPLES * 2;
const RAW_QUEUE_CHUNKS: usize = 12;
const EVENT_QUEUE_CAPACITY: usize = 4;
const MAX_BUFFERED_SAMPLES: usize = PCM_FRAME_SAMPLES * 4;
const FRAME_DURATION: Duration = Duration::from_millis(40);
const SILENCE_RMS_THRESHOLD: f32 = 0.003;
const SILENCE_HANGOVER_FRAMES: u16 = 25;

#[derive(Debug)]
pub enum AudioTaskEvent {
    Ready,
    Failed(NoticeText),
    Stopped,
}

#[derive(Clone, Copy)]
enum CaptureSource {
    Microphone,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    System,
}

struct RawChunk {
    source: CaptureSource,
    sample_rate: u32,
    channels: u16,
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

#[derive(Default)]
struct SilenceGate {
    quiet_frames: u16,
}

impl SilenceGate {
    fn should_send(&mut self, samples: &[f32], enabled: bool) -> bool {
        if !enabled {
            return true;
        }
        let mean_square =
            samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len().max(1) as f32;
        if mean_square.sqrt() >= SILENCE_RMS_THRESHOLD {
            self.quiet_frames = 0;
            true
        } else {
            self.quiet_frames = self.quiet_frames.saturating_add(1);
            self.quiet_frames <= SILENCE_HANGOVER_FRAMES
        }
    }
}

struct SourceBuffer {
    sample_rate: u32,
    resampler: Resampler,
    samples: VecDeque<f32>,
}

impl SourceBuffer {
    fn new() -> Self {
        Self {
            sample_rate: 0,
            resampler: Resampler::new(PCM_SAMPLE_RATE),
            samples: VecDeque::with_capacity(MAX_BUFFERED_SAMPLES),
        }
    }

    fn push(&mut self, chunk: &RawChunk) {
        if self.sample_rate != chunk.sample_rate {
            self.sample_rate = chunk.sample_rate;
            self.resampler = Resampler::new(chunk.sample_rate);
            self.samples.clear();
        }
        self.samples.extend(
            self.resampler
                .push_interleaved(&chunk.samples, chunk.channels),
        );
        if self.samples.len() > MAX_BUFFERED_SAMPLES {
            self.samples
                .drain(..self.samples.len() - MAX_BUFFERED_SAMPLES);
        }
    }

    fn next(&mut self) -> f32 {
        self.samples.pop_front().unwrap_or(0.0)
    }
}

struct AudioMixer {
    mode: AudioSourceMode,
    microphone: SourceBuffer,
    system: SourceBuffer,
    silence_gate: SilenceGate,
    token_saver: bool,
}

impl AudioMixer {
    fn new(mode: AudioSourceMode, token_saver: bool) -> Self {
        Self {
            mode,
            microphone: SourceBuffer::new(),
            system: SourceBuffer::new(),
            silence_gate: SilenceGate::default(),
            token_saver,
        }
    }

    fn push(&mut self, chunk: RawChunk) {
        match chunk.source {
            CaptureSource::Microphone => self.microphone.push(&chunk),
            CaptureSource::System => self.system.push(&chunk),
        }
    }

    fn emit(&mut self, output: &tokio_mpsc::Sender<Vec<u8>>) {
        let mut frame = Vec::with_capacity(PCM_FRAME_SAMPLES);
        for _ in 0..PCM_FRAME_SAMPLES {
            let microphone = self.microphone.next();
            let system = self.system.next();
            let sample = match self.mode {
                AudioSourceMode::MicrophoneOnly => microphone,
                AudioSourceMode::SystemAudioOnly => system,
                AudioSourceMode::Mixed => 0.5 * microphone + 0.5 * system,
            };
            frame.push(sample.clamp(-1.0, 1.0));
        }
        if !self.silence_gate.should_send(&frame, self.token_saver) {
            return;
        }
        let mut pcm = Vec::with_capacity(PCM_FRAME_BYTES);
        for sample in frame {
            let value = (sample * 32767.0).round() as i16;
            pcm.extend_from_slice(&value.to_le_bytes());
        }
        let _ = output.try_send(pcm);
    }
}

pub struct NativeAudioTask {
    stop: Arc<AtomicBool>,
    events: Receiver<AudioTaskEvent>,
    join: Option<JoinHandle<()>>,
}

impl NativeAudioTask {
    pub fn spawn(
        mode: AudioSourceMode,
        microphone_device: Option<String>,
        system_device: Option<String>,
        token_saver: bool,
        output: tokio_mpsc::Sender<Vec<u8>>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (event_sender, events) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let join = std::thread::spawn(move || {
            if let Err(error) = run_capture(
                mode,
                microphone_device.as_deref(),
                system_device.as_deref(),
                token_saver,
                output,
                &worker_stop,
                &event_sender,
            ) {
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
    mode: AudioSourceMode,
    microphone_device: Option<&str>,
    system_device: Option<&str>,
    token_saver: bool,
    output: tokio_mpsc::Sender<Vec<u8>>,
    stop: &AtomicBool,
    events: &SyncSender<AudioTaskEvent>,
) -> Result<()> {
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = system_device;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    if mode.wants_system_audio() {
        return Err(anyhow!(
            "此平台的 Slint 原生系统音频尚未完成验证；未打开任何音频设备"
        ));
    }
    let (raw_sender, raw_receiver) = mpsc::sync_channel(RAW_QUEUE_CHUNKS);
    #[cfg(target_os = "macos")]
    let mac_capture_failed = Arc::new(AtomicBool::new(false));
    #[cfg(target_os = "macos")]
    let mac_system_capture = if mode.wants_system_audio() {
        if system_device
            .filter(|value| !value.is_empty())
            .is_some_and(|value| value != "screen-capture-kit")
        {
            return Err(anyhow!("所选 ScreenCaptureKit 系统音频源已不可用"));
        }
        Some(macos_system_audio::MacSystemCapture::start({
            let raw_sender = raw_sender.clone();
            let capture_failed = Arc::clone(&mac_capture_failed);
            move |event| {
                match event {
                    macos_system_audio::CaptureEvent::Data(chunk) => {
                        let _ = raw_sender.try_send(RawEvent::Data(RawChunk {
                            source: CaptureSource::System,
                            sample_rate: chunk.sample_rate,
                            channels: chunk.channels,
                            samples: chunk.samples,
                        }));
                    }
                    macos_system_audio::CaptureEvent::Failed => {
                        capture_failed.store(true, Ordering::Release);
                    }
                }
            }
        })?)
    } else {
        None
    };
    let mut streams = Vec::new();
    if mode.wants_microphone() {
        let device = input_device(microphone_device)?;
        let supported = device
            .default_input_config()
            .context("无法读取麦克风默认格式")?;
        streams.push(build_supported_stream(
            &device,
            supported,
            CaptureSource::Microphone,
            raw_sender.clone(),
        )?);
    }
    #[cfg(target_os = "windows")]
    if mode.wants_system_audio() {
        let device = output_device(system_device)?;
        let supported = device
            .default_output_config()
            .context("无法读取系统音频默认格式")?;
        streams.push(build_supported_stream(
            &device,
            supported,
            CaptureSource::System,
            raw_sender.clone(),
        )?);
    }
    for stream in &streams {
        stream.play().context("无法启动音频采集")?;
    }
    events
        .try_send(AudioTaskEvent::Ready)
        .map_err(|_| anyhow!("音频状态队列不可用"))?;
    let mut mixer = AudioMixer::new(mode, token_saver);
    let mut next_frame = Instant::now() + FRAME_DURATION;
    while !stop.load(Ordering::Acquire) {
        #[cfg(target_os = "macos")]
        if mac_capture_failed.load(Ordering::Acquire) {
            return Err(anyhow!("ScreenCaptureKit 系统音频流已停止或返回无效格式"));
        }
        let wait = next_frame
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(20));
        match raw_receiver.recv_timeout(wait) {
            Ok(RawEvent::Data(chunk)) => mixer.push(chunk),
            Ok(RawEvent::Failed) => return Err(anyhow!("音频采集回调失败")),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("音频采集意外停止"));
            }
        }
        if Instant::now() >= next_frame {
            mixer.emit(&output);
            next_frame += FRAME_DURATION;
            if Instant::now().saturating_duration_since(next_frame) > FRAME_DURATION * 3 {
                next_frame = Instant::now() + FRAME_DURATION;
            }
        }
    }
    drop(streams);
    #[cfg(target_os = "macos")]
    drop(mac_system_capture);
    let _ = events.try_send(AudioTaskEvent::Stopped);
    Ok(())
}

fn input_device(device_id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    match device_id.filter(|value| !value.is_empty()) {
        Some(id) => host
            .input_devices()
            .context("无法读取麦克风设备")?
            .find(|device| device.name().ok().as_deref() == Some(id))
            .ok_or_else(|| anyhow!("所选麦克风已不可用，请刷新设备列表")),
        None => host
            .default_input_device()
            .ok_or_else(|| anyhow!("系统没有默认麦克风")),
    }
}

#[cfg(target_os = "windows")]
fn output_device(device_id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    match device_id.filter(|value| !value.is_empty()) {
        Some(id) => host
            .output_devices()
            .context("无法读取系统音频设备")?
            .find(|device| device.name().ok().as_deref() == Some(id))
            .ok_or_else(|| anyhow!("所选系统音频设备已不可用，请刷新设备列表")),
        None => host
            .default_output_device()
            .ok_or_else(|| anyhow!("系统没有默认音频输出设备")),
    }
}

fn build_supported_stream(
    device: &cpal::Device,
    supported: cpal::SupportedStreamConfig,
    source: CaptureSource,
    samples: SyncSender<RawEvent>,
) -> Result<cpal::Stream> {
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let config = supported.config();
    let errors = samples.clone();
    match supported.sample_format() {
        SampleFormat::F32 => build_stream::<f32>(
            device,
            &config,
            source,
            sample_rate,
            channels,
            samples,
            errors,
            |value| *value,
        ),
        SampleFormat::F64 => build_stream::<f64>(
            device,
            &config,
            source,
            sample_rate,
            channels,
            samples,
            errors,
            |value| *value as f32,
        ),
        SampleFormat::I16 => build_stream::<i16>(
            device,
            &config,
            source,
            sample_rate,
            channels,
            samples,
            errors,
            |value| *value as f32 / 32768.0,
        ),
        SampleFormat::U16 => build_stream::<u16>(
            device,
            &config,
            source,
            sample_rate,
            channels,
            samples,
            errors,
            |value| (*value as f32 - 32768.0) / 32768.0,
        ),
        SampleFormat::I32 => build_stream::<i32>(
            device,
            &config,
            source,
            sample_rate,
            channels,
            samples,
            errors,
            |value| *value as f32 / 2_147_483_648.0,
        ),
        SampleFormat::U32 => build_stream::<u32>(
            device,
            &config,
            source,
            sample_rate,
            channels,
            samples,
            errors,
            |value| (*value as f64 / 2_147_483_648.0 - 1.0) as f32,
        ),
        _ => Err(anyhow!("音频采样格式不受支持")),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_stream<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    source: CaptureSource,
    sample_rate: u32,
    channels: u16,
    samples: SyncSender<RawEvent>,
    errors: SyncSender<RawEvent>,
    convert: impl Fn(&T) -> f32 + Send + 'static,
) -> Result<cpal::Stream> {
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let converted = data.iter().map(&convert).collect();
                let _ = samples.try_send(RawEvent::Data(RawChunk {
                    source,
                    sample_rate,
                    channels,
                    samples: converted,
                }));
            },
            move |_| {
                let _ = errors.try_send(RawEvent::Failed);
            },
            None,
        )
        .context("无法创建音频采集流")
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

    #[test]
    fn swr_naudio_004_token_saver_preserves_hangover_and_resumes_on_speech() {
        let silence = vec![0.0; PCM_FRAME_SAMPLES];
        let speech = vec![0.02; PCM_FRAME_SAMPLES];
        let mut gate = SilenceGate::default();
        for _ in 0..SILENCE_HANGOVER_FRAMES {
            assert!(gate.should_send(&silence, true));
        }
        assert!(!gate.should_send(&silence, true));
        assert!(gate.should_send(&speech, true));
        assert!(gate.should_send(&silence, true));
        let mut disabled = SilenceGate::default();
        for _ in 0..100 {
            assert!(disabled.should_send(&silence, false));
        }
    }

    #[tokio::test]
    async fn swr_naudio_002_framer_emits_exact_bounded_pcm_frames() {
        let (sender, mut receiver) = tokio_mpsc::channel(2);
        let mut mixer = AudioMixer::new(AudioSourceMode::MicrophoneOnly, false);
        mixer.push(RawChunk {
            source: CaptureSource::Microphone,
            sample_rate: PCM_SAMPLE_RATE,
            channels: 1,
            samples: vec![0.5; PCM_FRAME_SAMPLES + 1],
        });
        mixer.emit(&sender);
        let frame = receiver.recv().await.expect("PCM frame");
        assert_eq!(frame.len(), PCM_FRAME_BYTES);
        assert!(receiver.try_recv().is_err());
        assert_eq!(i16::from_le_bytes([frame[0], frame[1]]), 16_384);
    }

    #[tokio::test]
    async fn swr_naudio_005_mixed_frames_use_bounded_convex_weights() {
        let (sender, mut receiver) = tokio_mpsc::channel(1);
        let mut mixer = AudioMixer::new(AudioSourceMode::Mixed, false);
        mixer.push(RawChunk {
            source: CaptureSource::Microphone,
            sample_rate: PCM_SAMPLE_RATE,
            channels: 1,
            samples: vec![0.5; PCM_FRAME_SAMPLES + 1],
        });
        mixer.push(RawChunk {
            source: CaptureSource::System,
            sample_rate: PCM_SAMPLE_RATE,
            channels: 1,
            samples: vec![-0.5; PCM_FRAME_SAMPLES + 1],
        });
        mixer.emit(&sender);
        let frame = receiver.recv().await.expect("mixed PCM frame");
        assert_eq!(frame.len(), PCM_FRAME_BYTES);
        assert!(
            frame
                .as_chunks::<2>()
                .0
                .iter()
                .all(|sample| i16::from_le_bytes(*sample) == 0)
        );
    }
}
