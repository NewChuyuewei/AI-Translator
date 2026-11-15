//! ScreenCaptureKit system-audio source for macOS 13+.
//!
//! Apple callbacks only copy bounded Float32 PCM into the existing raw ingress;
//! resampling, mixing, framing, and network work remain on the audio worker.

use crate::{AudioChunk, CaptureEvent, interleave_planar, supports_audio_capture};
use anyhow::{Result, anyhow};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::Retained};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
use objc2_core_media::{CMBlockBuffer, CMSampleBuffer, CMTime};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSProcessInfo};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use std::{
    mem::size_of,
    sync::{Arc, Mutex},
    time::Duration,
};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const MAX_AUDIO_BYTES: usize = SAMPLE_RATE as usize * CHANNELS as usize * size_of::<f32>();
const MAX_AUDIO_BUFFERS: usize = 8;
const START_TIMEOUT: Duration = Duration::from_secs(8);

pub struct MacSystemCapture {
    stream: Retained<SCStream>,
    delegate: Retained<Delegate>,
    queue: DispatchRetained<DispatchQueue>,
}

impl MacSystemCapture {
    pub fn start(notify: impl Fn(CaptureEvent) + Send + Sync + 'static) -> Result<Self> {
        let version = NSProcessInfo::processInfo().operatingSystemVersion();
        if !supports_audio_capture((
            version.majorVersion as u64,
            version.minorVersion as u64,
            version.patchVersion as u64,
        )) {
            return Err(anyhow!("ScreenCaptureKit 系统音频需要 macOS 13 或更高版本"));
        }
        let content = shareable_content()?;
        let displays = unsafe { content.displays() };
        if displays.count() == 0 {
            return Err(anyhow!("ScreenCaptureKit 未找到可采集显示器"));
        }
        let display = displays.objectAtIndex(0);
        let excluded = NSArray::<SCWindow>::new();
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &excluded,
            )
        };
        let configuration = unsafe { SCStreamConfiguration::new() };
        unsafe {
            configuration.setCapturesAudio(true);
            configuration.setExcludesCurrentProcessAudio(true);
            configuration.setSampleRate(SAMPLE_RATE as isize);
            configuration.setChannelCount(CHANNELS as isize);
            // ScreenCaptureKit requires a screen stream even when only its
            // audio output is consumed. Keep video work minimal and discard it.
            configuration.setWidth(2);
            configuration.setHeight(2);
            configuration.setMinimumFrameInterval(CMTime::new(1, 1));
            configuration.setQueueDepth(3);
        }

        let delegate = Delegate::new(std::sync::Arc::new(notify));
        let queue = DispatchQueue::new("com.gemini.live.translator.system-audio", None);
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &configuration,
                Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)),
            )
        };
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    objc2::runtime::ProtocolObject::from_ref(&*delegate),
                    SCStreamOutputType::Audio,
                    Some(&queue),
                )
                .map_err(|_| anyhow!("无法连接 ScreenCaptureKit 音频输出"))?;
        }
        start_capture(&stream)?;
        Ok(Self {
            stream,
            delegate,
            queue,
        })
    }
}

impl Drop for MacSystemCapture {
    fn drop(&mut self) {
        self.delegate.disable();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let sender = Mutex::new(Some(sender));
        let handler = RcBlock::new(move |_error: *mut NSError| {
            if let Some(sender) = sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                let _ = sender.send(());
            }
        });
        unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&handler)) };
        let _ = receiver.recv_timeout(Duration::from_millis(800));
        let _ = &self.queue;
    }
}

type EventCallback = dyn Fn(CaptureEvent) + Send + Sync;

struct DelegateIvars {
    callback: Mutex<Option<Arc<EventCallback>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "GeminiTranslatorSystemAudioDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl SCStreamDelegate for Delegate {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn did_stop(&self, _stream: &SCStream, _error: &NSError) {
            if let Some(callback) = self.lock_callback().take() {
                callback(CaptureEvent::Failed);
            }
        }
    }

    unsafe impl SCStreamOutput for Delegate {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            output_type: SCStreamOutputType,
        ) {
            if output_type.0 != SCStreamOutputType::Audio.0 {
                return;
            }
            let event = match extract_audio(sample_buffer) {
                Some(chunk) => CaptureEvent::Data(chunk),
                None => CaptureEvent::Failed,
            };
            if let Ok(callback) = self.ivars().callback.try_lock()
                && let Some(callback) = callback.as_ref()
            {
                callback(event);
            }
        }
    }
);

impl Delegate {
    fn new(callback: Arc<EventCallback>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars {
            callback: Mutex::new(Some(callback)),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn lock_callback(&self) -> std::sync::MutexGuard<'_, Option<Arc<EventCallback>>> {
        self.ivars()
            .callback
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn disable(&self) {
        self.lock_callback().take();
    }
}

fn extract_audio(sample_buffer: &CMSampleBuffer) -> Option<AudioChunk> {
    let mut needed = 0usize;
    let status = unsafe {
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            &mut needed,
            std::ptr::null_mut(),
            0,
            None,
            None,
            0,
            std::ptr::null_mut(),
        )
    };
    if status != 0 || needed < size_of::<AudioBufferList>() || needed > 4096 {
        return None;
    }

    // AudioBufferList includes pointers, so use u64 backing for alignment.
    let mut storage = vec![0u64; needed.div_ceil(size_of::<u64>())];
    let list = storage.as_mut_ptr().cast::<AudioBufferList>();
    let mut block = std::ptr::null_mut::<CMBlockBuffer>();
    let status = unsafe {
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            std::ptr::null_mut(),
            list,
            needed,
            None,
            None,
            0,
            &mut block,
        )
    };
    if status != 0 {
        return None;
    }
    let _block = unsafe { Retained::from_raw(block) }?;
    let count = unsafe { (*list).mNumberBuffers } as usize;
    if count == 0 || count > MAX_AUDIO_BUFFERS {
        return None;
    }
    let buffers = unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!((*list).mBuffers).cast::<AudioBuffer>(),
            count,
        )
    };
    if buffers
        .iter()
        .any(|buffer| buffer.mDataByteSize as usize > MAX_AUDIO_BYTES)
    {
        return None;
    }

    let (samples, channels) = if let [buffer] = buffers {
        let channels = u16::try_from(buffer.mNumberChannels).ok()?;
        if channels == 0 || channels > MAX_AUDIO_BUFFERS as u16 || buffer.mData.is_null() {
            return None;
        }
        let bytes = buffer.mDataByteSize as usize;
        if bytes % size_of::<f32>() != 0 {
            return None;
        }
        let values = unsafe {
            std::slice::from_raw_parts(buffer.mData.cast::<f32>(), bytes / size_of::<f32>())
        };
        (values.to_vec(), channels)
    } else {
        if buffers.iter().any(|buffer| {
            buffer.mNumberChannels != 1
                || buffer.mData.is_null()
                || buffer.mDataByteSize as usize % size_of::<f32>() != 0
        }) {
            return None;
        }
        let planes: Vec<&[f32]> = buffers
            .iter()
            .map(|buffer| unsafe {
                std::slice::from_raw_parts(
                    buffer.mData.cast::<f32>(),
                    buffer.mDataByteSize as usize / size_of::<f32>(),
                )
            })
            .collect();
        (
            interleave_planar(
                &planes,
                MAX_AUDIO_BUFFERS,
                MAX_AUDIO_BYTES / size_of::<f32>(),
            )?,
            u16::try_from(count).ok()?,
        )
    };
    Some(AudioChunk {
        sample_rate: SAMPLE_RATE,
        channels,
        samples,
    })
}

fn shareable_content() -> Result<Retained<SCShareableContent>> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let sender = Mutex::new(Some(sender));
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, _error: *mut NSError| {
            let result = unsafe { Retained::retain(content) }.map(SendObject);
            if let Some(sender) = sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                let _ = sender.send(result);
            }
        },
    );
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    receiver
        .recv_timeout(START_TIMEOUT)
        .map_err(|_| anyhow!("ScreenCaptureKit 内容查询超时"))?
        .map(|content| content.0)
        .ok_or_else(|| anyhow!("ScreenCaptureKit 内容查询失败或权限被拒绝"))
}

fn start_capture(stream: &SCStream) -> Result<()> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let sender = Mutex::new(Some(sender));
    let handler = RcBlock::new(move |error: *mut NSError| {
        if let Some(sender) = sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = sender.send(error.is_null());
        }
    });
    unsafe { stream.startCaptureWithCompletionHandler(Some(&handler)) };
    match receiver.recv_timeout(START_TIMEOUT) {
        Ok(true) => Ok(()),
        Ok(false) => Err(anyhow!("ScreenCaptureKit 启动失败或权限被拒绝")),
        Err(_) => Err(anyhow!("ScreenCaptureKit 启动超时")),
    }
}

struct SendObject<T>(Retained<T>);
// SAFETY: Objective-C objects are retained while crossing the completion
// queue, and ScreenCaptureKit's shareable content object is immutable here.
unsafe impl<T> Send for SendObject<T> {}
