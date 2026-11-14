# Native Android application

This standalone Android application is the replacement for the generated Tauri Android target. It uses Kotlin, Jetpack Compose, JDK 25, AGP 9.4.1, Gradle 9.9.1, and Kotlin 2.4.21. It does not link the desktop Rust core or load a WebView.

## Current rewrite status

Implemented in the native module:

- Compose control and caption UI;
- screenshot protection for API keys and transcripts;
- Android Keystore-backed AES-256-GCM API-key storage;
- bounded Gemini WebSocket messages, separate bounded control/audio event queues, redacted transport failures, explicit setup-complete state gating, and a 25-second initialization timeout;
- native `AudioRecord` microphone capture and Android 10+ `AudioPlaybackCapture` for device media/game audio;
- selectable microphone-only, device-audio-only, or bounded real-time mixed input using 40 ms PCM16 frames;
- silence suppression with a one-second VAD hangover, bounded 240 ms audio queues, and bounded WebSocket send backlog;
- lifecycle-independent foreground service with notification stop control;
- draggable, collapsible system overlay for live original/translated captions, with source status and stop/hide actions;
- bounded native `AudioTrack` playback for translated 24 kHz PCM;
- JVM-target and configuration validation tests.

Remaining production work:

- proxy selection and 20k-token/minute budget enforcement;
- emulator and physical-device validation, including Android 15+.

## Build

Use a stable JDK 25 and Android SDK 37:

```bash
gradle -p android testDebugUnitTest assembleDebug
```

CI additionally inspects `MainActivity.class` and `SessionConfig.class` and rejects output whose class-file major version is not 69 (Java 25).
