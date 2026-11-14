# Android Kotlin + Compose migration plan

The Android target is moving toward a native Kotlin/Jetpack Compose presentation layer while retaining the audited Rust core for Gemini WebSocket, proxy routing, encryption, transcript parsing, and bounded audio mixing.

## Architecture boundary

- **Kotlin/Compose:** activities and screens, runtime permissions, `MediaProjection`, foreground-service lifecycle, notification controls, `AudioRecord`, and the system caption overlay.
- **Rust:** validated session options, secrets vault, TLS/WebSocket transport, proxy tunnelling, protocol parsing, resampling/mixing, diagnostics, and session state machine.
- **Bridge:** typed, bounded commands and state events only. Raw PCM continues to use JNI and must never travel through WebView JSON IPC.

This split avoids duplicating security-sensitive network and cryptographic code while allowing Android UI and lifecycle behavior to become fully native.

## Delivery stages

### Stage 1 — latency and safety baseline (current)

- 40 ms end-to-end PCM frames instead of 100 ms frames.
- Bounded Rust, JNI, and network queues that discard stale audio under pressure.
- Android audio-priority reader threads and a smaller `AudioRecord` buffer floor.
- Removal of the unusable privileged `CAPTURE_AUDIO_OUTPUT` permission; playback capture relies only on user-approved `MediaProjection`.
- Bounded local configuration reads and prompt erasure of temporary vault key/plaintext buffers.
- Existing Android capture, permissions, service, overlay, and bridge code remains Kotlin.

### Stage 2 — native Compose shell

- Add a native Compose activity with Material 3 design tokens and adaptive phone/tablet layouts.
- Expose typed Rust commands for settings, status, diagnostics, device capabilities, start, and stop.
- Replace the Android WebView control panel; desktop keeps the existing Tauri UI.
- Keep API keys in password fields and prevent them from entering Compose saved state, logs, screenshots, or clipboard.

### Stage 3 — native Compose overlay and lifecycle hardening

- Replace the current Kotlin `View` overlay with a Compose overlay hosted by a lifecycle-aware service.
- Add notification stop action, process-recreation recovery, audio-focus behavior, and instrumentation tests for permission cancellation and rapid start/stop.
- Add Macrobenchmark measurements for cold start, frame delivery, recomposition, and memory.

### Stage 4 — 1.0 validation

The project uses `0.2.0` during the migration and remains pre-1.0 until desktop and Android CI, physical-device tests, real Gemini sessions, upgrade tests, release signing, accessibility checks, and privacy/security review pass. Only then should all package, Cargo, Tauri, Android, installer, and documentation versions be changed together to `1.0.0`.

## Performance budgets

- Capture frame: 40 ms.
- Queued PCM after mixing: at most 240 ms.
- UI work: no audio, network, proxy, or cryptography on the main thread.
- Caption state: latest-value delivery; no unbounded event backlog.
- Logs/configuration: bounded input sizes and no secret/transcript payloads.
