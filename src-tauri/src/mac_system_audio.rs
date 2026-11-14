//! macOS 13+ system playback via ScreenCaptureKit; no video frames are stored.
use anyhow::{anyhow, Result};
use crossbeam_channel::Sender;
use screencapturekit::prelude::*;
use crate::audio_router::{RawAudio, Source};

pub struct MacSystemAudio { stream: SCStream }
impl MacSystemAudio {
    pub fn start(tx: Sender<RawAudio>) -> Result<Self> {
        let content = SCShareableContent::get()?;
        let display = content.displays().into_iter().next().ok_or_else(|| anyhow!("未找到屏幕；请检查屏幕录制权限"))?;
        let filter = SCContentFilter::create().with_display(&display).with_excluding_windows(&[]).build()?;
        let config = SCStreamConfiguration::new().with_width(2).with_height(2)
            .with_captures_audio(true).with_sample_rate(48_000).with_channel_count(2)
            .with_excludes_current_process_audio(true);
        let mut stream = SCStream::new(&filter, &config)?;
        stream.add_output_handler(move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
            if !matches!(kind, SCStreamOutputType::Audio) { return; }
            if let Ok(list) = sample.audio_buffer_list() {
                // CoreMedia may deliver either separate planes or an interleaved buffer.
                let mut planes: Vec<Vec<f32>> = Vec::new();
                for i in 0..list.num_buffers() {
                    if let Some(buf) = list.get(i) {
                        let samples: Vec<f32> = buf.data().chunks_exact(4).map(|v| f32::from_ne_bytes([v[0],v[1],v[2],v[3]])).collect();
                        let channels = (buf.number_channels as usize).max(1);
                        let mono: Vec<f32> = samples.chunks_exact(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect();
                        planes.push(mono);
                    }
                }
                if let Some(first) = planes.first() {
                    let mut mixed = vec![0.0; first.len()];
                    for plane in &planes { for (dst, src) in mixed.iter_mut().zip(plane) { *dst += *src / planes.len() as f32; } }
                    let _ = tx.try_send(RawAudio { source: Source::System, sample_rate: 48_000, channels: 1, data: mixed });
                }
            }
        }, SCStreamOutputType::Audio)?;
        stream.start_capture()?;
        Ok(Self { stream })
    }
}
impl Drop for MacSystemAudio { fn drop(&mut self) { let _ = self.stream.stop_capture(); } }
