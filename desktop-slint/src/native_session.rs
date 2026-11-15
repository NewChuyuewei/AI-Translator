//! Native session lifecycle adapter. The transport worker owns network I/O and
//! publishes only bounded, typed events. Audio activation remains fail-closed
//! until the capture path is integrated and independently verified.

use crate::{
    native_audio::{AudioTaskEvent, NativeAudioTask},
    native_playback::{NativePlaybackTask, PlaybackTaskEvent},
    native_transport::{self, ProbeCancel, SessionTransportEvent},
};
use anyhow::{Result, anyhow};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
use translator_core::{
    AudioSourceMode, SessionEvent, SessionOptions, SessionPhase, SessionStatus,
    server_protocol::ParsedMessage,
    session_port::{BoundedEventQueue, CaptionSnapshot, NoticeText, SessionPort, UiEvent},
};

const WORKER_EVENT_CAPACITY: usize = 32;
const UI_EVENT_CAPACITY: usize = 32;
const PCM_QUEUE_FRAMES: usize = 6;

#[derive(Debug)]
enum WorkerEvent {
    Transport(SessionTransportEvent),
    Completed,
    Failed(NoticeText),
}

struct TransportTask {
    cancel: ProbeCancel,
    events: Arc<Mutex<BoundedEventQueue<WorkerEvent, WORKER_EVENT_CAPACITY>>>,
    finished: Arc<AtomicBool>,
    audio_sender: tokio::sync::mpsc::Sender<Vec<u8>>,
    join: Option<JoinHandle<()>>,
}

impl TransportTask {
    fn spawn(options: SessionOptions) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let events = Arc::new(Mutex::new(BoundedEventQueue::default()));
        let finished = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let completion_cancel = Arc::clone(&cancel);
        let worker_events = Arc::clone(&events);
        let worker_finished = Arc::clone(&finished);
        let (audio_sender, audio_receiver) = tokio::sync::mpsc::channel(PCM_QUEUE_FRAMES);
        let join = std::thread::spawn(move || {
            let result = native_transport::session_with_audio_blocking(
                options,
                worker_cancel,
                audio_receiver,
                |event| {
                    push_worker_event(&worker_events, WorkerEvent::Transport(event));
                },
            );
            let completion = match result {
                Ok(_) => WorkerEvent::Completed,
                Err(_) if completion_cancel.load(Ordering::Acquire) => WorkerEvent::Completed,
                Err(error) => WorkerEvent::Failed(NoticeText::bounded(&error.to_string())),
            };
            worker_finished.store(true, Ordering::Release);
            push_worker_event(&worker_events, completion);
        });
        Self {
            cancel,
            events,
            finished,
            audio_sender,
            join: Some(join),
        }
    }

    fn audio_sender(&self) -> tokio::sync::mpsc::Sender<Vec<u8>> {
        self.audio_sender.clone()
    }

    fn request_stop(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    fn poll(&mut self) -> Option<WorkerEvent> {
        let event = lock_events(&self.events).pop();
        if self.finished.load(Ordering::Acquire)
            && let Some(join) = self.join.take()
        {
            let _ = join.join();
        }
        event
    }
}

impl Drop for TransportTask {
    fn drop(&mut self) {
        self.request_stop();
    }
}

fn lock_events<T, const CAPACITY: usize>(
    events: &Mutex<BoundedEventQueue<T, CAPACITY>>,
) -> std::sync::MutexGuard<'_, BoundedEventQueue<T, CAPACITY>> {
    events
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn push_worker_event(
    events: &Mutex<BoundedEventQueue<WorkerEvent, WORKER_EVENT_CAPACITY>>,
    event: WorkerEvent,
) {
    lock_events(events).push(event);
}

#[derive(Default)]
pub struct NativeSessionPort {
    status: SessionStatus,
    events: BoundedEventQueue<UiEvent, UI_EVENT_CAPACITY>,
    task: Option<TransportTask>,
    audio: Option<NativeAudioTask>,
    playback: Option<NativePlaybackTask>,
    mode: AudioSourceMode,
    microphone_device: Option<String>,
    system_device: Option<String>,
    playback_device: Option<String>,
    playback_required: bool,
    microphone_ready: bool,
    playback_ready: bool,
    token_saver: bool,
}

impl NativeSessionPort {
    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Transport(SessionTransportEvent::SetupComplete) => {
                let Some(task) = self.task.as_ref() else {
                    self.fail("原生传输任务缺失");
                    return;
                };
                self.status.message = "网络已连接，正在打开麦克风".into();
                self.events.push(UiEvent::Status(self.status.clone()));
                self.audio = Some(NativeAudioTask::spawn(
                    self.mode,
                    self.microphone_device.clone(),
                    self.system_device.clone(),
                    self.token_saver,
                    task.audio_sender(),
                ));
                if self.playback_required {
                    self.playback = Some(NativePlaybackTask::spawn(self.playback_device.clone()));
                }
            }
            WorkerEvent::Transport(SessionTransportEvent::Message(message)) => {
                self.publish_message(message);
            }
            WorkerEvent::Completed => {
                if self.status.phase == SessionPhase::Stopping {
                    match self.status.phase.transition(SessionEvent::StopCompleted) {
                        Ok(phase) => {
                            self.status.phase = phase;
                            self.status.message = "已停止".into();
                            self.events.push(UiEvent::Status(self.status.clone()));
                        }
                        Err(error) => self.fail(&error.to_string()),
                    }
                } else if self.status.phase != SessionPhase::Error {
                    self.fail("原生传输意外结束");
                }
            }
            WorkerEvent::Failed(message) => self.fail(message.as_str()),
        }
    }

    fn handle_audio_event(&mut self, event: AudioTaskEvent) {
        match event {
            AudioTaskEvent::Ready => {
                self.microphone_ready = true;
                self.try_mark_running();
            }
            AudioTaskEvent::Failed(message) => {
                if let Some(task) = &self.task {
                    task.request_stop();
                }
                self.fail(message.as_str());
            }
            AudioTaskEvent::Stopped => {
                if self.status.phase != SessionPhase::Stopping
                    && self.status.phase != SessionPhase::Error
                {
                    if let Some(task) = &self.task {
                        task.request_stop();
                    }
                    self.fail("麦克风采集意外停止");
                }
            }
        }
    }

    fn handle_playback_event(&mut self, event: PlaybackTaskEvent) {
        match event {
            PlaybackTaskEvent::Ready => {
                self.playback_ready = true;
                self.try_mark_running();
            }
            PlaybackTaskEvent::Failed(message) => {
                if let Some(task) = &self.task {
                    task.request_stop();
                }
                if let Some(audio) = &self.audio {
                    audio.request_stop();
                }
                self.fail(message.as_str());
            }
            PlaybackTaskEvent::Stopped => {
                if self.status.phase != SessionPhase::Stopping
                    && self.status.phase != SessionPhase::Error
                {
                    if let Some(task) = &self.task {
                        task.request_stop();
                    }
                    if let Some(audio) = &self.audio {
                        audio.request_stop();
                    }
                    self.fail("译音播放意外停止");
                }
            }
        }
    }

    fn try_mark_running(&mut self) {
        if !self.microphone_ready || (self.playback_required && !self.playback_ready) {
            return;
        }
        match self.status.phase.transition(SessionEvent::SetupCompleted) {
            Ok(phase) => {
                self.status.phase = phase;
                self.status.message = "原生麦克风会话正在运行".into();
                self.events.push(UiEvent::Status(self.status.clone()));
            }
            Err(error) => self.fail(&error.to_string()),
        }
    }

    fn publish_message(&mut self, mut message: ParsedMessage) {
        if let Some(playback) = &self.playback {
            if message.interrupted {
                playback.clear();
            }
            for audio in message.audio.drain(..) {
                playback.push(audio);
            }
        }
        let original = message
            .input
            .or(message.interim_input)
            .map_or_else(String::new, |value| value.0);
        let translated = if self.status.transcription_only {
            String::new()
        } else {
            message.output.map_or_else(String::new, |value| value.0)
        };
        if !original.is_empty() || !translated.is_empty() {
            self.events.push(UiEvent::Caption(CaptionSnapshot::bounded(
                &original,
                &translated,
                self.status.transcription_only,
            )));
        }
    }

    fn fail(&mut self, message: &str) {
        self.status.phase = SessionPhase::Error;
        self.status.message = NoticeText::bounded(message).as_str().to_owned();
        self.events.push(UiEvent::Status(self.status.clone()));
    }
}

impl SessionPort for NativeSessionPort {
    fn start(&mut self, options: &SessionOptions) -> Result<()> {
        options.validate()?;
        if self.status.phase.is_active() || self.task.is_some() {
            return Err(anyhow!("原生会话已在运行或停止中"));
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        if options.mode != AudioSourceMode::MicrophoneOnly {
            return Err(anyhow!(
                "此平台的 Slint 原生系统音频与混合采集尚未完成验证；未连接或上传音频"
            ));
        }
        let phase = self.status.phase.transition(SessionEvent::StartRequested)?;
        self.status.phase = phase;
        self.status.message = "正在连接原生传输".into();
        self.status.transcription_only = options.model.is_transcription_only();
        self.events.push(UiEvent::Status(self.status.clone()));
        self.mode = options.mode;
        self.microphone_device = options.microphone_device.clone();
        self.system_device = options.system_device.clone();
        self.playback_device = options.playback_device.clone();
        self.playback_required = options.play_translation();
        self.microphone_ready = false;
        self.playback_ready = !self.playback_required;
        self.token_saver = options.token_saver;
        self.task = Some(TransportTask::spawn(options.clone()));
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        let phase = self.status.phase.transition(SessionEvent::StopRequested)?;
        let task = self
            .task
            .as_ref()
            .ok_or_else(|| anyhow!("原生传输任务缺失"))?;
        self.status.phase = phase;
        self.status.message = "正在停止".into();
        self.events.push(UiEvent::Status(self.status.clone()));
        if let Some(playback) = &self.playback {
            playback.request_stop();
        }
        if let Some(audio) = &self.audio {
            audio.request_stop();
        }
        task.request_stop();
        Ok(())
    }

    fn phase(&self) -> SessionPhase {
        self.status.phase
    }

    fn poll_event(&mut self) -> Option<UiEvent> {
        if let Some(event) = self.events.pop() {
            return Some(event);
        }
        let playback_event = self.playback.as_mut().and_then(NativePlaybackTask::poll);
        if let Some(event) = playback_event {
            let terminal = matches!(
                event,
                PlaybackTaskEvent::Stopped | PlaybackTaskEvent::Failed(_)
            );
            self.handle_playback_event(event);
            if terminal {
                self.playback = None;
            }
            return self.events.pop();
        }
        let audio_event = self.audio.as_mut().and_then(NativeAudioTask::poll);
        if let Some(event) = audio_event {
            let terminal = matches!(event, AudioTaskEvent::Stopped | AudioTaskEvent::Failed(_));
            self.handle_audio_event(event);
            if terminal {
                self.audio = None;
            }
            return self.events.pop();
        }
        let worker_event = self.task.as_mut().and_then(TransportTask::poll);
        if let Some(event) = worker_event {
            let terminal = matches!(event, WorkerEvent::Completed | WorkerEvent::Failed(_));
            if terminal {
                if let Some(playback) = &self.playback {
                    playback.request_stop();
                }
                if let Some(audio) = &self.audio {
                    audio.request_stop();
                }
            }
            self.handle_worker_event(event);
            if terminal {
                self.task = None;
            }
        }
        self.events.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use translator_core::{
        AudioSourceMode, ModelKind, ProxyMode, ThinkingLevel, TranscriptionMode,
    };

    fn options() -> SessionOptions {
        SessionOptions {
            api_key: "test-key".into(),
            model: ModelKind::GeneralLive,
            target_language: "it".into(),
            thinking_level: ThinkingLevel::Low,
            transcription_mode: TranscriptionMode::Verbatim,
            bidirectional: false,
            other_language: "en".into(),
            mode: AudioSourceMode::MicrophoneOnly,
            microphone_device: None,
            system_device: None,
            playback_enabled: false,
            playback_device: None,
            token_saver: true,
            proxy_mode: ProxyMode::Direct,
            proxy_url: String::new(),
            overlay_scale: 1.0,
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn swr_nport_001_unverified_audio_modes_remain_fail_closed() {
        let mut port = NativeSessionPort::default();
        let mut options = options();
        options.mode = AudioSourceMode::Mixed;
        let error = port.start(&options).expect_err("audio gate must reject");
        assert_eq!(
            error.to_string(),
            "此平台的 Slint 原生系统音频与混合采集尚未完成验证；未连接或上传音频"
        );
        assert_eq!(port.phase(), SessionPhase::Idle);
        assert!(port.poll_event().is_none());
        assert!(port.task.is_none());
    }

    #[test]
    fn swr_nport_002_worker_mailbox_is_bounded_and_freshness_first() {
        let events = Mutex::new(BoundedEventQueue::default());
        for index in 0..(WORKER_EVENT_CAPACITY + 5) {
            push_worker_event(
                &events,
                WorkerEvent::Failed(NoticeText::bounded(&format!("failure-{index}"))),
            );
        }
        let queue = lock_events(&events);
        assert_eq!(queue.len(), WORKER_EVENT_CAPACITY);
        assert_eq!(queue.dropped(), 5);
    }

    #[test]
    fn swr_nport_003_worker_events_map_to_typed_ui_lifecycle() {
        let mut port = NativeSessionPort::default();
        port.status.phase = SessionPhase::Connecting;
        port.playback_required = true;
        port.handle_audio_event(AudioTaskEvent::Ready);
        assert_eq!(port.phase(), SessionPhase::Connecting);
        port.handle_playback_event(PlaybackTaskEvent::Ready);
        assert_eq!(port.phase(), SessionPhase::Running);
        let message = ParsedMessage {
            input: Some(("hello".into(), true)),
            output: Some(("你好".into(), true)),
            ..ParsedMessage::default()
        };
        port.handle_worker_event(WorkerEvent::Transport(SessionTransportEvent::Message(
            message,
        )));
        assert!(matches!(
            port.events.pop(),
            Some(UiEvent::Status(status)) if status.phase == SessionPhase::Running
        ));
        assert!(matches!(
            port.events.pop(),
            Some(UiEvent::Caption(caption))
                if caption.original == "hello" && caption.translated == "你好"
        ));
        port.status.phase = SessionPhase::Stopping;
        port.handle_worker_event(WorkerEvent::Completed);
        assert_eq!(port.phase(), SessionPhase::Idle);
    }
}
