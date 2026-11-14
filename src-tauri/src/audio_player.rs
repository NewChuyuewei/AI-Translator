//! Plays Gemini's 24 kHz PCM through a bounded, interruptible CPAL buffer.
use anyhow::{anyhow, Context, Result};
use cpal::{traits::{DeviceTrait, HostTrait, StreamTrait}, SampleFormat};
use crossbeam_channel::{bounded, Sender};
use std::{collections::VecDeque, sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}}, thread, time::Duration};
use tokio::sync::mpsc;
use crate::audio_router::Resampler;

enum Command { Audio { epoch: u64, bytes: Vec<u8> }, Stop }
pub struct AudioPlayer {
    tx: Sender<Command>, thread: Option<thread::JoinHandle<()>>,
    queue: Arc<Mutex<VecDeque<f32>>>, epoch: Arc<AtomicU64>,
}
impl AudioPlayer {
    pub fn start(device_name: Option<String>, errors: mpsc::UnboundedSender<String>) -> Result<Self> {
        let (tx, rx) = bounded::<Command>(48); let (ready_tx, ready_rx) = bounded::<Result<()>>(1);
        let queue = Arc::new(Mutex::new(VecDeque::<f32>::new())); let worker_queue = queue.clone();
        let epoch = Arc::new(AtomicU64::new(0)); let worker_epoch = epoch.clone();
        let worker = thread::spawn(move || {
            let init = (|| -> Result<(cpal::Stream, Arc<Mutex<VecDeque<f32>>>, u32)> {
                let host = cpal::default_host();
                let device = match device_name.as_deref().filter(|v| !v.is_empty()) {
                    Some(name) => host.output_devices()?.find(|d| d.name().ok().as_deref()==Some(name)).ok_or_else(|| anyhow!("找不到译音输出设备: {name}"))?,
                    None => host.default_output_device().ok_or_else(|| anyhow!("没有默认播放设备"))?,
                };
                let supported = device.default_output_config().context("获取播放设备配置失败")?;
                let rate = supported.sample_rate().0; let channels = supported.channels() as usize;
                let config = supported.config(); let queue = worker_queue;
                macro_rules! build {
                    ($t:ty, $convert:expr, $silence:expr) => {{
                        let q = queue.clone(); let err = errors.clone();
                        device.build_output_stream(&config, move |output: &mut [$t], _| {
                            if let Ok(mut samples) = q.try_lock() {
                                for frame in output.chunks_mut(channels.max(1)) {
                                    let sample = samples.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
                                    for value in frame { *value = ($convert)(sample); }
                                }
                            } else { output.fill($silence); }
                        }, move |e| { let _ = err.send(format!("译音播放失败: {e}")); }, None)?
                    }};
                }
                let stream = match supported.sample_format() {
                    SampleFormat::F32 => build!(f32, |x: f32| x, 0.0),
                    SampleFormat::I16 => build!(i16, |x: f32| (x*32767.0) as i16, 0),
                    SampleFormat::U16 => build!(u16, |x: f32| ((x+1.0)*32767.5) as u16, 32768),
                    f => return Err(anyhow!("不支持的播放格式 {f:?}")),
                };
                stream.play().context("无法开始播放译音")?;
                Ok((stream, queue, rate))
            })();
            let (_stream, queue, rate) = match init { Ok(v) => { let _ = ready_tx.send(Ok(())); v }, Err(e) => { let _ = ready_tx.send(Err(e)); return; } };
            let mut resampler = Resampler::new(24_000, rate);
            let mut resampler_epoch = worker_epoch.load(Ordering::Acquire);
            while let Ok(command) = rx.recv() {
                match command {
                    Command::Stop => break,
                    Command::Audio { epoch, bytes } => {
                        // An interruption invalidates even chunks still queued
                        // ahead of the playback worker, without waiting for it.
                        if epoch != worker_epoch.load(Ordering::Acquire) { continue; }
                        if resampler_epoch != epoch { resampler = Resampler::new(24_000, rate); resampler_epoch = epoch; }
                        let input: Vec<f32> = bytes.chunks_exact(2).map(|x| i16::from_le_bytes([x[0],x[1]]) as f32 / 32768.0).collect();
                        let output = resampler.push(&input);
                        if let Ok(mut q) = queue.lock() {
                            if epoch != worker_epoch.load(Ordering::Acquire) { continue; }
                            q.extend(output);
                            let max = (rate as usize) * 2;
                            if q.len() > max { let count = q.len() - max; q.drain(..count); }
                        }
                    }
                }
            }
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).context("播放设备初始化超时")??;
        Ok(Self { tx, thread: Some(worker), queue, epoch })
    }
    pub fn push(&self, pcm: Vec<u8>) {
        let epoch = self.epoch.load(Ordering::Acquire);
        let _ = self.tx.try_send(Command::Audio { epoch, bytes: pcm });
    }
    pub fn clear(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut queue) = self.queue.lock() { queue.clear(); }
    }
}
impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(h) = self.thread.take() { let _ = h.join(); }
    }
}
