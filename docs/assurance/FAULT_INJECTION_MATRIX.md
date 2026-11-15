# Automated fault-injection matrix

The workspace test suite injects faults at typed boundaries. These tests run in CI on every change; physical-driver faults remain in the target checklist and cannot be credited from simulation.

| Fault class | Automated injection / oracle | Residual physical-target work |
|---|---|---|
| Lifecycle order | invalid/duplicate setup, audio and close events must be rejected deterministically | repeated F8/quit during device transitions |
| Queue pressure | overfill UI, worker, raw-audio and playback queues; assert fixed capacity/freshness policy | sustained CPU pressure and eight-hour soak |
| Transport cancellation | cancel during platform proxy/setup wait; assert bounded completion and unique stream end | Wi-Fi loss, sleep/wake, proxy outage |
| Hostile network data | malformed/oversized binary/text, unsafe server error/close text and proxy credentials | live proxy/TLS appliance interoperability |
| Credential storage | wrong password, modified ciphertext, oversized settings/vault; assert common safe errors and no plaintext | OS credential-store denial and upgrade |
| Audio conversion | partial frames, invalid layouts, planar over-capacity, underflow and saturation boundaries | driver format changes, unplug/hot-plug and clock drift |
| Playback interruption | queued audio followed by epoch clear; stale samples must not play | real output interruption and endpoint removal |
| Caption adapter failure | injected enable/disable failures; reported state must remain safe and recoverable | compositor/accessibility behavior |
| Unsupported source | request unverified system-audio mode; assert fail-closed and never Running | Windows/macOS permissions and capture-service loss |
| Packaging/evidence | absent, duplicate, malformed or non-independent target record blocks publication | independent review of real artifacts |

Test functions use the `swr_*` requirement identifier convention. `cargo test --workspace --all-targets --locked` is the controlled campaign. CI also builds both target operating systems and stores LCOV, but those activities do not replace the final column.
