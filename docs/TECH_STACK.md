# Technology stack baseline

Audited on 2026-11-14. The project follows the latest **stable** releases where they can be upgraded and tested without replacing a generated Tauri build system or making an unverified cross-platform audio/network migration.

## Active baseline

| Area | Baseline |
|---|---|
| Runtime | Node.js `^22.22.2` or `>=24.15.0`; GitHub Actions uses Node 24 |
| Frontend build | Vite 8.3.8 |
| UI tests | jsdom 31.0.1 |
| Desktop shell | Tauri 2.12.5; JS API/CLI 2.12.4 |
| Rust async/TLS | Tokio 1.54.1, rustls 0.23.46, tokio-rustls 0.26.6 |
| Serialization/security | serde 1.0.229, serde_json 1.0.154, ring 0.17.14 |
| Native Android | JDK 25, AGP 9.4.1, Gradle 9.9.1, Kotlin 2.4.21, Compose BOM 2026.09.00, OkHttp 5.5.0 |
| Android SDK | min SDK 26; compile/target SDK 37; Java/Kotlin bytecode target 25 |
| CI runners | Ubuntu 24.04, Windows Server 2022, macOS 26 ARM64; Node 24, JDK 25, Rust stable |
| Actions | checkout v7, setup-node v7, setup-java v6, upload-artifact v7, download-artifact v8, setup-android v4, rust-cache v2 |

> **Java 25 Android policy:** Android is now a standalone Kotlin/Compose project and does not use Tauri-generated Gradle files. CI builds and packages it with stable JDK 25, AGP 9.4.1, Gradle 9.9.1, and Kotlin 2.4.21, then verifies class-file major version 69. Physical-device behavior on API 26 and Android 15+ remains unverified.

`Cargo.lock` remains the reproducible source of exact transitive Rust versions. Direct dependency lower bounds are aligned with the locked, tested stable line rather than left at broad historical minimums.

## Approved desktop migration

The v2.1.0 migration workspace targets the latest stable toolchain verified on 2026-11-14: Rust 1.100.0, edition 2024, and Slint 1.18.1. Versions are pinned in `rust-toolchain.toml` and the workspace manifests so CI and developer builds agree; dependency updates still require tests rather than floating production builds.

The active desktop baseline remains Tauri 2 until the replacement passes feature parity and target-device gates. The approved sequence is Rust + Slint in v2.1.0, allocation-free callbacks/SPSC transport in v2.2.0, native low-latency platform backends in v2.3.0, clocked mixing in v2.4.0, adaptive VAD in v2.5.0, 10 ms internal frames in v2.6.0, and production hardening in v3.0.0. See [`PC_RUST_SLINT_ROADMAP.md`](PC_RUST_SLINT_ROADMAP.md). No version is advanced merely by adding dependencies or compiling a prototype.

## Intentionally deferred breaking upgrades

These are not silently forgotten; they are held because this environment cannot compile Cargo/Gradle or run physical-device audio tests.

- **Tauri 3 alpha:** prerelease and excluded by the stable-only policy. Tauri 2.12.5 is the newest stable Tauri 2 release at the audit date.
- **tokio-tungstenite 0.30:** newer than the current 0.26.2 line but contains several 0.x breaking releases. It requires a dedicated proxy/TLS/WebSocket migration and Rust integration test run.
- **CPAL 0.18.2:** newer than 0.15.3 but changes backend APIs across WASAPI, CoreAudio and ALSA. Upgrade only with Windows/macOS/Linux compile checks and physical capture/playback tests.
- **Android NDK:** the native rewrite no longer links Rust/JNI and therefore does not install or pin an NDK.
- **Ubuntu 26.04 / Windows 2025 runners:** available, but changing release images also changes system compilers, WebKit/GTK, MSVC and packaging behavior. Keep the currently validated images until a separate runner migration proves every bundle.

## Upgrade verification

For JavaScript changes:

```bash
npm ci
npm run build
npm run test:ui
npm run test:version
npm run test:android-config
```

For a complete acceptance run, GitHub Actions must additionally execute locked Rust tests and all native bundle builds. Android and audio dependency migrations require physical-device tests; a successful compile alone is insufficient.
