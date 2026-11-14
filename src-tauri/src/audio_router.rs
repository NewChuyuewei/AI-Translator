//! PCM routing shared by desktop and Android.
//! Input callbacks never block on networking. Bounded queues and a 40 ms
//! mixing clock cap memory use while keeping capture-to-network latency low.
use anyhow::Result;
#[cfg(not(target_os = "android"))]
use anyhow::{anyhow, Context};
use crossbeam_channel::{bounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::{atomic::{AtomicBool, Ordering}, Arc}, thread, time::{Duration, Instant}};
use tokio::sync::mpsc;

pub const SAMPLE_RATE: u32 = 16_000;
pub const FRAME_DURATION: Duration = Duration::from_millis(40);
pub const FRAME_SAMPLES: usize = 640;
// Retain at most 160 ms per source. If a producer outruns the network, prefer
// recent speech over stale audio so latency cannot grow without bound.
const MAX_BUFFERED_SAMPLES: usize = FRAME_SAMPLES * 4;
const SILENCE_RMS_THRESHOLD: f32 = 0.003;
const SILENCE_HANGOVER_FRAMES: u16 = 25; // 1 s preserves server VAD turn endings

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSourceMode { MicrophoneOnly, SystemAudioOnly, Mixed }
#[cfg(not(target_os = "android"))]
impl AudioSourceMode {
    fn wants_mic(self) -> bool { matches!(self, Self::MicrophoneOnly | Self::Mixed) }
    fn wants_system(self) -> bool { matches!(self, Self::SystemAudioOnly | Self::Mixed) }
}

#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioConfig {
    pub mode: AudioSourceMode,
    #[serde(default)] pub microphone_device: Option<String>,
    #[serde(default)] pub system_device: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo { pub id: String, pub name: String, pub is_default: bool }
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevices {
    pub microphones: Vec<AudioDeviceInfo>, pub system_sources: Vec<AudioDeviceInfo>,
    pub playback_devices: Vec<AudioDeviceInfo>, pub platform: String,
}

#[derive(Clone, Copy, Debug)]
pub enum Source {
    Mic,
    // Linux has no system-capture producer yet, but the shared mixer still
    // handles this source for Windows, macOS and Android.
    #[cfg_attr(not(any(target_os = "windows", target_os = "macos", target_os = "android")), allow(dead_code))]
    System,
}
pub struct RawAudio { pub source: Source, pub sample_rate: u32, pub channels: u16, pub data: Vec<f32> }

/// Stateful streaming interpolator. The one-pole low-pass attenuates aliasing
/// during downsampling; phase and filter state are retained across callbacks.
pub struct Resampler { from: u32, to: u32, position: f64, input: VecDeque<f32>, lowpass: f32 }
impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self { from: from.max(1), to: to.max(1), position: 0.0, input: VecDeque::new(), lowpass: 0.0 }
    }
    fn push_samples(&mut self, samples: impl Iterator<Item = f32>) -> Vec<f32> {
        let cutoff = (self.to as f32 * 0.45).min(self.from as f32 * 0.45);
        let a = (std::f32::consts::TAU * cutoff / (self.from as f32 + std::f32::consts::TAU * cutoff)).clamp(0.0, 1.0);
        for s in samples {
            let s = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
            if self.from > self.to { self.lowpass += a * (s - self.lowpass); self.input.push_back(self.lowpass); }
            else { self.input.push_back(s); }
        }
        let step = self.from as f64 / self.to as f64;
        let mut result = Vec::new();
        while self.position + 1.0 < self.input.len() as f64 {
            let index = self.position as usize;
            let frac = (self.position - index as f64) as f32;
            result.push(self.input[index] * (1.0 - frac) + self.input[index + 1] * frac);
            self.position += step;
        }
        let consumed = (self.position as usize).min(self.input.len().saturating_sub(1));
        self.input.drain(..consumed);
        self.position -= consumed as f64;
        result
    }
    pub fn push(&mut self, mono: &[f32]) -> Vec<f32> {
        self.push_samples(mono.iter().copied())
    }
    fn push_interleaved(&mut self, data: &[f32], channels: u16) -> Vec<f32> {
        let channels = usize::from(channels.max(1));
        self.push_samples(data.chunks_exact(channels).map(|frame|
            frame.iter().copied().sum::<f32>() / channels as f32
        ))
    }
}
fn trim(q: &mut VecDeque<f32>) {
    if q.len() > MAX_BUFFERED_SAMPLES { q.drain(..q.len()-MAX_BUFFERED_SAMPLES); }
}
fn pcm16(samples: impl IntoIterator<Item=f32>) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME_SAMPLES * 2);
    for sample in samples {
        let v = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}
fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() { return 0.0; }
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

#[derive(Default)]
struct SilenceGate { quiet_frames: u16 }
impl SilenceGate {
    fn should_send(&mut self, samples: &[f32], enabled: bool) -> bool {
        if !enabled { return true; }
        if frame_rms(samples) >= SILENCE_RMS_THRESHOLD {
            self.quiet_frames = 0;
            true
        } else {
            self.quiet_frames = self.quiet_frames.saturating_add(1);
            self.quiet_frames <= SILENCE_HANGOVER_FRAMES
        }
    }
}

fn process_audio(mode: AudioSourceMode, rx: Receiver<RawAudio>, tx: mpsc::Sender<Vec<u8>>, stop: Arc<AtomicBool>, token_saver: bool) {
    let mut mic = VecDeque::new(); let mut system = VecDeque::new();
    let mut mic_resample: Option<(u32, Resampler)> = None;
    let mut sys_resample: Option<(u32, Resampler)> = None;
    let mut next = Instant::now() + FRAME_DURATION;
    let mut silence_gate = SilenceGate::default();
    while !stop.load(Ordering::Acquire) {
        let wait = next.saturating_duration_since(Instant::now()).min(Duration::from_millis(20));
        if let Ok(chunk) = rx.recv_timeout(wait) {
            let (slot, queue) = match chunk.source {
                Source::Mic => (&mut mic_resample, &mut mic),
                Source::System => (&mut sys_resample, &mut system),
            };
            if slot.as_ref().map(|x| x.0) != Some(chunk.sample_rate) {
                *slot = Some((chunk.sample_rate, Resampler::new(chunk.sample_rate, SAMPLE_RATE)));
            }
            if let Some((_, resampler)) = slot {
                queue.extend(resampler.push_interleaved(&chunk.data, chunk.channels)); trim(queue);
            }
        }
        if Instant::now() >= next {
            let mut out = Vec::with_capacity(FRAME_SAMPLES);
            for _ in 0..FRAME_SAMPLES {
                let m = mic.pop_front().unwrap_or(0.0);
                let s = system.pop_front().unwrap_or(0.0);
                let v = match mode {
                    AudioSourceMode::MicrophoneOnly => m,
                    AudioSourceMode::SystemAudioOnly => s,
                    // Convex weights plus hard clipping protect against overload.
                    AudioSourceMode::Mixed => 0.5 * m + 0.5 * s,
                };
                out.push(v.clamp(-1.0, 1.0));
            }
            // AI Studio quotas are project-wide and server-enforced. Omitting
            // sustained near-silence cannot bypass the quota, but avoids paying
            // audio-token and bandwidth cost while nobody is speaking.
            if silence_gate.should_send(&out, token_saver)
                && tx.try_send(pcm16(out)).is_err() && tx.is_closed() { break; }
            next += FRAME_DURATION;
            // Don't send a burst after a scheduler stall. One late frame is
            // useful; replaying a backlog only increases perceived latency.
            if Instant::now().duration_since(next.min(Instant::now())) > FRAME_DURATION * 3 {
                next = Instant::now() + FRAME_DURATION;
            }
        }
    }
}

pub struct AudioRouter { pub raw_sender: Sender<RawAudio>, stop: Arc<AtomicBool>, threads: Vec<thread::JoinHandle<()>> }
impl AudioRouter {
    pub fn new(mode: AudioSourceMode, pcm: mpsc::Sender<Vec<u8>>, token_saver: bool) -> Self {
        // Twelve typical chunks are enough to absorb scheduler jitter without
        // allowing seconds of stale microphone data to accumulate.
        let (raw_sender, rx) = bounded(12);
        let stop = Arc::new(AtomicBool::new(false)); let worker_stop = stop.clone();
        let worker = thread::spawn(move || process_audio(mode, rx, pcm, worker_stop, token_saver));
        Self { raw_sender, stop, threads: vec![worker] }
    }
    #[cfg(not(target_os = "android"))]
    pub fn start_desktop(&mut self, config: AudioConfig, errors: mpsc::Sender<String>) -> Result<()> {
        let raw = self.raw_sender.clone(); let stopping = self.stop.clone();
        let (ready_tx, ready_rx) = bounded::<Result<()>>(1);
        let worker = thread::spawn(move || {
            let start = desktop::open_streams(&config, &raw, &errors);
            match start {
                Ok(streams) => {
                    let _ = ready_tx.send(Ok(()));
                    while !stopping.load(Ordering::Acquire) { thread::sleep(Duration::from_millis(40)); }
                    drop(streams);
                }
                Err(e) => { let _ = ready_tx.send(Err(e)); }
            }
        });
        self.threads.push(worker);
        ready_rx.recv_timeout(Duration::from_secs(20)).context("等待音频设备初始化超时")??;
        Ok(())
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for handle in self.threads.drain(..) { let _ = handle.join(); }
    }
}
impl Drop for AudioRouter { fn drop(&mut self) { self.stop(); } }

#[cfg(not(target_os = "android"))]
mod desktop {
    use super::*;
    use cpal::{traits::{DeviceTrait, HostTrait, StreamTrait}, SampleFormat};
    pub struct Streams {
        _cpal: Vec<cpal::Stream>,
        #[cfg(target_os = "macos")] _mac: Option<crate::mac_system_audio::MacSystemAudio>,
    }
    pub fn devices() -> Result<AudioDevices> {
        let host = cpal::default_host();
        let input_default = host.default_input_device().and_then(|d| d.name().ok());
        let output_default = host.default_output_device().and_then(|d| d.name().ok());
        let microphones = host.input_devices().context("枚举麦克风失败")?.filter_map(|d| d.name().ok()).map(|name| AudioDeviceInfo { id: name.clone(), is_default: input_default.as_ref()==Some(&name), name }).collect();
        let playback_devices: Vec<AudioDeviceInfo> = host.output_devices().context("枚举输出设备失败")?.filter_map(|d| d.name().ok()).map(|name| AudioDeviceInfo { id: name.clone(), is_default: output_default.as_ref()==Some(&name), name }).collect();
        #[cfg(target_os = "windows")]
        let system_sources = playback_devices.clone();
        #[cfg(target_os = "macos")]
        let system_sources = vec![AudioDeviceInfo { id: "screen-capture-kit".into(), name: "系统音频 · ScreenCaptureKit".into(), is_default: true }];
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let system_sources = Vec::new();
        Ok(AudioDevices { microphones, system_sources, playback_devices, platform: std::env::consts::OS.into() })
    }
    fn input_device(name: Option<&str>) -> Result<cpal::Device> {
        let host = cpal::default_host();
        match name.filter(|s| !s.is_empty()) {
            Some(n) => host.input_devices()?.find(|d| d.name().ok().as_deref()==Some(n)).ok_or_else(|| anyhow!("找不到输入设备 {n}")),
            None => host.default_input_device().ok_or_else(|| anyhow!("没有默认麦克风")),
        }
    }
    #[cfg(target_os = "windows")]
    fn output_device(name: Option<&str>) -> Result<cpal::Device> {
        let host = cpal::default_host();
        match name.filter(|s| !s.is_empty()) {
            Some(n) => host.output_devices()?.find(|d| d.name().ok().as_deref()==Some(n)).ok_or_else(|| anyhow!("找不到系统输出设备 {n}")),
            None => host.default_output_device().ok_or_else(|| anyhow!("没有默认系统输出设备")),
        }
    }
    fn open_cpal(device: cpal::Device, output_endpoint: bool, source: Source, raw: Sender<RawAudio>, errors: mpsc::Sender<String>) -> Result<cpal::Stream> {
        let supported = if output_endpoint { device.default_output_config()? } else { device.default_input_config()? };
        let format = supported.sample_format(); let rate = supported.sample_rate().0; let channels = supported.channels();
        let config = supported.config();
        macro_rules! build {
            ($t:ty, $f:expr) => {{
                let raw = raw.clone(); let err = errors.clone();
                device.build_input_stream(&config,
                    move |data: &[$t], _| {
                        let samples: Vec<f32> = data.iter().map($f).collect();
                        let _ = raw.try_send(RawAudio { source, sample_rate: rate, channels, data: samples });
                    }, move |e| { let _ = err.try_send(format!("音频采集失败: {e}")); }, None)?
            }};
        }
        // On Windows, CPAL opens input on a render endpoint using WASAPI loopback.
        let stream = match format {
            SampleFormat::F32 => build!(f32, |x: &f32| *x),
            SampleFormat::F64 => build!(f64, |x: &f64| *x as f32),
            SampleFormat::I16 => build!(i16, |x: &i16| *x as f32 / 32768.0),
            SampleFormat::U16 => build!(u16, |x: &u16| (*x as f32 - 32768.0) / 32768.0),
            SampleFormat::I32 => build!(i32, |x: &i32| *x as f32 / 2147483648.0),
            SampleFormat::U32 => build!(u32, |x: &u32| (*x as f64 / 2147483648.0 - 1.0) as f32),
            f => return Err(anyhow!("不支持的设备采样格式 {f:?}")),
        };
        stream.play().context("启动音频采集失败")?; Ok(stream)
    }
    pub fn open_streams(config: &AudioConfig, raw: &Sender<RawAudio>, errors: &mpsc::Sender<String>) -> Result<Streams> {
        let mut streams = Vec::new();
        if config.mode.wants_mic() {
            streams.push(open_cpal(input_device(config.microphone_device.as_deref())?, false, Source::Mic, raw.clone(), errors.clone())?);
        }
        #[cfg(target_os = "macos")]
        let mut mac = None;
        if config.mode.wants_system() {
            #[cfg(target_os = "windows")]
            streams.push(open_cpal(output_device(config.system_device.as_deref())?, true, Source::System, raw.clone(), errors.clone())?);
            #[cfg(target_os = "macos")]
            { mac = Some(crate::mac_system_audio::MacSystemAudio::start(raw.clone())?); }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            return Err(anyhow!("系统内录目前仅实现 Windows WASAPI / macOS ScreenCaptureKit / Android PlaybackCapture"));
        }
        Ok(Streams { _cpal: streams, #[cfg(target_os = "macos")] _mac: mac })
    }
}

pub fn list_devices() -> Result<AudioDevices> {
    #[cfg(not(target_os = "android"))] { desktop::devices() }
    #[cfg(target_os = "android")] {
        Ok(AudioDevices { microphones: vec![], system_sources: vec![], playback_devices: vec![], platform: "android".into() })
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn resampling_is_chunk_boundary_independent() {
        let input: Vec<f32> = (0..4800).map(|i| ((i as f32) * 0.03).sin()).collect();
        let mut a = Resampler::new(48_000, 16_000); let reference = a.push(&input);
        let mut b = Resampler::new(48_000, 16_000); let mut divided = Vec::new();
        for chunk in input.chunks(113) { divided.extend(b.push(chunk)); }
        assert_eq!(reference.len(), divided.len());
        assert!(reference.iter().zip(divided.iter()).all(|(a,b)| (a-b).abs()<1e-6));
        assert!((1598..=1601).contains(&reference.len()));
    }
    #[test] fn low_latency_frame_is_40_ms() {
        assert_eq!(FRAME_SAMPLES as u32 * 1_000 / SAMPLE_RATE, 40);
        assert_eq!(FRAME_DURATION, Duration::from_millis(40));
    }
    #[test] fn token_saver_keeps_speech_and_suppresses_sustained_silence() {
        assert_eq!(u32::from(SILENCE_HANGOVER_FRAMES) * 40, 1_000);
        let mut gate = SilenceGate::default();
        let silence = vec![0.0; FRAME_SAMPLES];
        for _ in 0..SILENCE_HANGOVER_FRAMES { assert!(gate.should_send(&silence, true)); }
        assert!(!gate.should_send(&silence, true));
        assert!(gate.should_send(&vec![0.02; FRAME_SAMPLES], true));
        assert!(gate.should_send(&silence, true));
        let mut disabled = SilenceGate::default();
        for _ in 0..100 { assert!(disabled.should_send(&silence, false)); }
    }
    #[test] fn stereo_downmix_and_clipping() {
        let mut resampler = Resampler::new(SAMPLE_RATE, SAMPLE_RATE);
        assert_eq!(resampler.push_interleaved(&[1.0,-1.0,0.5,0.5,0.0,0.0], 2), vec![0.0,0.5]);
        let bytes = pcm16([10.0,-10.0,0.0]);
        assert_eq!(i16::from_le_bytes([bytes[0],bytes[1]]), 32767);
        assert_eq!(i16::from_le_bytes([bytes[2],bytes[3]]), -32767);
    }
}
