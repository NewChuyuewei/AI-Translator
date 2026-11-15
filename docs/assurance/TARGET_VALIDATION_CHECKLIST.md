# Windows/macOS v2.1.0 target validation checklist

Status: **OPEN — 0/8 evidence records are present.** This checklist is executable release criteria, not a claim that target validation has occurred.

| Target | Required physical-target cases |
|---|---|
| ASSURANCE-TARGET-001 | Windows 10/11 x64 and macOS 13+ launch, control/caption windows, permissions, clean shutdown |
| ASSURANCE-TARGET-002 | Device names/defaults, persisted selection, hot-plug refresh, permission denial |
| ASSURANCE-TARGET-003 | Microphone default/selected device, 16 kHz uplink, removal/failure, deterministic stop |
| ASSURANCE-TARGET-004 | Playback default/selected device, 24 kHz conversion, interruption clear, failure and stop |
| ASSURANCE-TARGET-005 | Windows WASAPI loopback default/selected render endpoint, mixed underflow, removal and stop |
| ASSURANCE-TARGET-006 | Caption topmost/transparency/frameless drag/lock/pass-through and recovery |
| ASSURANCE-TARGET-007 | Tray/menu/F8/single-instance/close-to-tray/explicit quit/no residual process |
| ASSURANCE-TARGET-008 | macOS 13+ ScreenCaptureKit TCC denial/grant, stereo system audio, mixed mode, self-audio exclusion, stream loss and stop |

For every case:

1. Test the exact release commit and record OS/hardware identifiers outside sensitive logs.
2. Inject denied permission, missing device, device removal, network cancellation, and repeated stop where applicable.
3. Confirm no false `Running`, unbounded growth, stale playback, credential/plaintext diagnostics, or residual process.
4. Measure cold start, idle RSS, caption latency, audio continuity, and an eight-hour soak against the recorded baseline.
5. Archive sanitized evidence, calculate SHA-256, and obtain review by a person other than the tester.
6. Add the corresponding JSON record under `docs/assurance/evidence/` and run the strict evidence gate.

No agent or CI runner may mark an item passed without physical-target evidence and independent review.
