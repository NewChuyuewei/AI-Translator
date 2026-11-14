# Native Android application

This standalone Android application is the replacement for the generated Tauri Android target. It uses Kotlin, Jetpack Compose, JDK 25, AGP 9.4.1, Gradle 9.9.1, and Kotlin 2.4.21. It does not link the desktop Rust core or load a WebView.

## Current rewrite status

Implemented in the native module:

- Compose control and caption UI;
- screenshot protection for API keys and transcripts;
- Android Keystore-backed AES-256-GCM API-key storage;
- bounded Gemini WebSocket messages and redacted transport failures;
- native `AudioRecord` microphone capture using 40 ms PCM16 frames and silence suppression with a one-second VAD hangover;
- bounded 240 ms audio queues and bounded WebSocket send backlog;
- lifecycle-independent foreground service with notification stop control;
- bounded native `AudioTrack` playback for translated 24 kHz PCM;
- JVM-target and configuration validation tests.

Remaining production work:

- MediaProjection system-audio capture;
- Compose system overlay;
- proxy selection and 20k-token/minute budget enforcement;
- emulator and physical-device validation, including Android 15+.

## Build

Use a stable JDK 25 and Android SDK 37:

```bash
gradle -p android testDebugUnitTest assembleDebug
```

CI additionally inspects `MainActivity.class` and `SessionConfig.class` and rejects output whose class-file major version is not 69 (Java 25).
