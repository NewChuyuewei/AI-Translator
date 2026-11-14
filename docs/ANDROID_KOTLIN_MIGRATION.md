# Android Kotlin + JDK 25 rewrite plan

The `1.0.0` version milestone does not validate the existing Android target. Android will be rewritten separately as a fully native Kotlin application built with JDK 25. The rewrite will not reuse the current Tauri/WebView or Rust/JNI Android runtime path.

## Target architecture

- **UI:** Jetpack Compose and Material 3 for activities, settings, diagnostics, captions, and adaptive phone/tablet layouts.
- **Lifecycle:** Kotlin services and coroutines for runtime permissions, `MediaProjection`, foreground-service lifecycle, notification controls, process recreation, and audio focus.
- **Audio:** Kotlin/Android `AudioRecord`, playback capture, bounded channels, resampling, mixing, and backpressure.
- **Networking:** Kotlin coroutines with a native JVM WebSocket/TLS client, proxy routing, reconnect policy, Gemini protocol handling, and rate limiting.
- **Security:** Android Keystore-backed secret storage, bounded configuration parsing, redacted logs, screenshot protection, and explicit secret cleanup.
- **Build:** stable JDK 25, stable Kotlin, stable Android Gradle Plugin/Gradle, Java source/target 25, and Kotlin JVM target 25. No alpha, beta, or RC toolchain releases.

Desktop platforms keep their existing Tauri/Rust implementation. Android shares protocol fixtures and behavioral tests rather than native Rust code.

## Delivery stages

### Stage 1 — isolate the new Android application

- Create a standalone Android module so the rewrite does not depend on Tauri-generated Gradle files.
- Establish Compose navigation, dependency injection boundaries, settings models, and fake Gemini/audio implementations.
- Add unit tests and CI checks that inspect Java and Kotlin class-file targets before D8/R8 packaging.

### Stage 2 — native audio and lifecycle

- Implement microphone and user-approved playback capture with 40 ms frames.
- Use bounded coroutine channels and discard stale audio under pressure.
- Add foreground-service notification controls, permission cancellation, rapid start/stop, and process-recreation tests.

### Stage 3 — Gemini transport and security

- Implement Gemini Live WebSocket transport, proxy tunnelling, model discovery, reconnect behavior, and the 20k-token/minute budget.
- Store API keys and proxy credentials with Android Keystore-backed encryption.
- Keep secrets, raw audio, and transcript payloads out of logs, saved Compose state, screenshots, and clipboard by default.

### Stage 4 — Compose captions and validation

- Implement the control UI and lifecycle-aware Compose caption overlay.
- Validate cold start, latency, memory, accessibility, upgrade behavior, Android 8/API 26 minimum compatibility, and Android 15+ startup on physical devices.
- Produce a signed release only after CI, D8/R8, emulator, and physical-device checks pass.

## Performance budgets

- Capture frame: 40 ms.
- Queued PCM after mixing: at most 240 ms.
- UI work: no audio, network, proxy, or cryptography on the main thread.
- Caption state: latest-value delivery; no unbounded event backlog.
- Logs/configuration: bounded input sizes and no secret/transcript payloads.
