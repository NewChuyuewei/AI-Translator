# AI maintenance guide

## Production architecture

- Desktop: Rust workspace + Slint only. Entry crate: `desktop-slint` (`translator-desktop`).
- Core protocol/security/audio: `crates/translator-core`.
- Narrow native adapters: `caption-platform`, `desktop-lifecycle`, `macos-system-audio`.
- Android: independent Kotlin/Compose/JDK 25 project under `android/`.
- There is no Tauri, WebView UI, Vite, jsdom, Rust/JNI Android or generated-Tauri Android path. Do not restore one.

## Non-negotiable invariants

- API keys enter only the WebSocket request header and encrypted compatibility vault.
- Proxy setup errors are terminal; never silently connect directly.
- PCM, transcripts, proxy URLs, server payloads and close reasons never enter diagnostics.
- Transport/audio tasks remain off the UI thread, bounded and cancellable.
- Windows WASAPI and macOS ScreenCaptureKit sources preserve independent resampling, fixed 40 ms mixing, capacity-12 raw queues and six-frame network budget.
- Any capture/playback/device/transport failure cancels sibling resources and cannot report false `Running`.
- macOS system audio requires macOS 13+ and must safely reject TCC denial, no display, invalid/no audio and stream failure.

## Controlled checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
node scripts/check-version.mjs
node tests/version_script_test.mjs
node scripts/check-assurance.mjs
node scripts/check-no-legacy-desktop.mjs
node scripts/check-target-evidence.mjs
node tests/target_evidence_test.mjs
```

`cargo llvm-cov` in CI creates auxiliary structural evidence only. It is not a qualified verification tool.

## Release discipline

The code version remains `1.1.0` until all v2.1.0 physical-target acceptance criteria pass. Build-only Windows/macOS artifacts are not acceptance evidence. Publication runs the strict evidence gate and requires eight independently reviewed records under `docs/assurance/evidence/`.

Never invent target evidence, mark a checklist passed from CI, or equate a native runner build with permission/device/window/installer validation. Large sanitized artifacts remain external; Git records immutable SHA-256 values.

Version sequence is fixed:

1. v2.1.0 Slint migration and target acceptance.
2. v2.2.0 preallocated blocks and lock-free SPSC real-time pipeline.
3. v2.3.0 platform-native low-latency backends.
4. v2.4.0 mixer clock.
5. v2.5.0 sensitive VAD.
6. v2.6.0 internal frame adjustment.
7. v3.0.0 final production hardening and local model deployment such as `canary-0.6b-turbo`.
