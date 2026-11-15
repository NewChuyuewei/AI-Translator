#![deny(unsafe_op_in_unsafe_fn)]

//! Bounded ScreenCaptureKit system-audio adapter.

#[derive(Debug)]
pub struct AudioChunk {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

#[derive(Debug)]
pub enum CaptureEvent {
    Data(AudioChunk),
    Failed,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn supports_audio_capture(version: (u64, u64, u64)) -> bool {
    version >= (13, 0, 0)
}

/// Interleave equally formatted Float32 planes without reading beyond the
/// shortest plane. Bounds are checked before allocation.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn interleave_planar(
    planes: &[&[f32]],
    max_channels: usize,
    max_samples: usize,
) -> Option<Vec<f32>> {
    if planes.is_empty() || planes.len() > max_channels {
        return None;
    }
    let frames = planes.iter().map(|plane| plane.len()).min()?;
    let output_len = frames.checked_mul(planes.len())?;
    if output_len > max_samples {
        return None;
    }
    let mut output = Vec::with_capacity(output_len);
    for frame in 0..frames {
        for plane in planes {
            output.push(plane[frame]);
        }
    }
    Some(output)
}

#[cfg(target_os = "macos")]
mod platform;
#[cfg(target_os = "macos")]
pub use platform::MacSystemCapture;

#[cfg(not(target_os = "macos"))]
pub struct MacSystemCapture;

#[cfg(not(target_os = "macos"))]
impl MacSystemCapture {
    pub fn start(_notify: impl Fn(CaptureEvent) + Send + Sync + 'static) -> anyhow::Result<Self> {
        Err(anyhow::anyhow!("ScreenCaptureKit 仅支持 macOS"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swr_naudio_006_requires_macos_13_for_system_audio() {
        assert!(!supports_audio_capture((12, 7, 6)));
        assert!(supports_audio_capture((13, 0, 0)));
        assert!(supports_audio_capture((26, 1, 0)));
    }

    #[test]
    fn swr_naudio_006_planar_audio_is_bounded_and_interleaved() {
        let left = [1.0, 2.0, 3.0];
        let right = [4.0, 5.0];
        assert_eq!(
            interleave_planar(&[&left, &right], 8, 16),
            Some(vec![1.0, 4.0, 2.0, 5.0])
        );
        assert!(interleave_planar(&[], 8, 16).is_none());
        assert!(interleave_planar(&[&left, &right], 1, 16).is_none());
        assert!(interleave_planar(&[&left, &right], 8, 3).is_none());
    }
}
