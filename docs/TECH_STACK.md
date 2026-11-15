# Technical stack

## Desktop production path

- Rust `1.100.0`, edition 2024, Cargo workspace.
- Slint `1.18.1` native UI (`backend-winit`, `renderer-femtovg`).
- Tokio, tokio-tungstenite and rustls for bounded asynchronous transport.
- CPAL `0.15.3` for microphone/playback and Windows render-loopback device streams.
- `windows` APIs for WASAPI endpoint enumeration/loopback, window styles and lifecycle.
- ScreenCaptureKit through `objc2-screen-capture-kit 0.3.2` for macOS 13+ system audio.
- `tray-icon` for native tray integration; native global F8 and OS single-instance adapters.
- `keyring` compatibility vault; redacted bounded in-memory diagnostics.

The desktop production and packaging path has no Tauri, WebView UI, Vite, jsdom or Node runtime. Node executes repository control scripts only.

## Android

- Kotlin, Jetpack Compose and Gradle 9.9.1.
- JDK 25 source/target and Kotlin JVM target 25.
- Android `AudioRecord`/`AudioTrack`, Keystore-backed preferences and native lifecycle.
- No Rust/JNI, Tauri or WebView application shell.

## Packaging and verification

- Windows: Cargo release binary, portable ZIP, NSIS per-user installer.
- macOS: Cargo release binary, `.app` plist/entitlements, code-signing layout verification, DMG; distribution signing/notarization remains a controlled release-environment activity.
- Linux: auxiliary native tarball.
- CI: native Linux/Windows/macOS Rust builds, Android build/tests, Clippy/rustfmt, platform-adapter cross-checks and `cargo-llvm-cov` LCOV artifact.
- Release publication: fail-closed physical-target evidence schema and independent tester/reviewer gate.

See `docs/assurance/` for requirements, traceability, open target validation and the limits of unqualified coverage tooling.
