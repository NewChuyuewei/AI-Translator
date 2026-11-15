# 需求双向追踪

| 需求 | 设计/实现 | 验证证据 |
|---|---|---|
| SWR-CORE-001 | `translator_core::SessionPhase`、`SessionEvent`、Tauri typed phase adapter | `swr_core_001_lifecycle_accepts_only_defined_nominal_transitions` |
| SWR-CORE-002 | `SessionPhase::transition` 返回 `TransitionError` | `swr_core_002_invalid_lifecycle_transitions_are_rejected` |
| SWR-CORE-003 | `SessionOptions::validate`、`checked_overlay_scale` | `swr_core_003_options_enforce_model_and_untrusted_input_invariants`、`scale_is_bounded_and_normalized` |
| SWR-CORE-004 | `validate_manual_proxy`、Tauri `Trace::redact` | `swr_core_004_proxy_errors_never_echo_credentials` 及旧 Tauri 脱敏测试 |
| SWR-CORE-005 | serde rename/alias 契约 | `swr_core_005_serialized_contract_remains_compatible_with_existing_settings`、前端 mock 测试 |
| SWR-UI-001 | `desktop-slint/src/main.rs` 的适配器未接入安全拒绝路径 | Slint workspace 编译；UI 集成测试待办 `ASSURANCE-UI-001` |
| SWR-CFG-001 | `scripts/set-version.mjs`、`scripts/check-version.mjs` | `tests/version_script_test.mjs`、CI version gate |
| SWR-PROTO-001 | `translator_core::protocol::setup`、`LiveProtocolConfig` | `swr_proto_001_setup_is_model_specific_and_contains_no_credentials`、旧 WebSocket setup 测试 |
| SWR-PROTO-002 | `protocol::setup` 的模型专属分支 | `swr_proto_002_translation_and_transcription_semantics_are_distinct`、旧精确 setup 回归测试 |
| SWR-PROTO-003 | `protocol::audio_frame`、`MAX_AUDIO_BYTES_PER_MESSAGE` | `swr_proto_003_audio_frames_are_bounded_and_exactly_encoded` |
| SWR-PROTO-004 | `protocol::stream_end` | `swr_proto_004_stream_end_has_no_implicit_audio_payload` |
| SWR-RX-001 | `server_protocol::decode_data_frame`、`DataFrameKind` | `swr_rx_001_text_and_binary_decoding_is_bounded_and_content_safe`、旧 malformed-frame 测试 |
| SWR-RX-002 | `server_protocol::ParsedMessage`、`parse_server_value` | `swr_rx_002_parses_naming_styles_and_typed_state`、旧转写/WebSocket 测试 |
| SWR-RX-003 | `MAX_TRANSCRIPT_BYTES`、`MAX_AUDIO_BYTES_PER_MESSAGE` | `swr_rx_003_audio_and_transcripts_are_bounded_before_growth`、旧 oversized-message 测试 |
| SWR-RX-004 | `safe_error_hint`、`server_error`、`close_description` | `swr_rx_004_server_errors_are_allowlisted_and_redacted`、旧服务端错误脱敏测试 |
| SWR-SESSION-001 | `session_driver::SessionDriver`、`DriverEvent` | `swr_session_001_memory_session_runs_without_ui_or_socket` |
| SWR-SESSION-002 | `SessionDriver::receive`、`encode_audio` | `swr_session_002_rejects_out_of_order_and_duplicate_events` |
| SWR-SESSION-003 | `SessionDriver::close`、`DriverState::Closed` | `swr_session_003_close_is_deterministic_and_idempotence_is_rejected` |
| SWR-PORT-001 | `BoundedEventQueue<T, CAPACITY>` | `swr_port_001_queue_is_bounded_and_freshness_first` |
| SWR-PORT-002 | `SessionPort`、`MemorySessionPort`、Slint `PendingNativeSessionPort` | `swr_port_002_memory_port_enforces_lifecycle_and_publishes_typed_events` |
| SWR-PORT-003 | `UiEvent`、`CaptionSnapshot::bounded`、`NoticeText` | `swr_port_003_ui_text_is_bounded_and_contains_no_pcm_variant` |
| SWR-PORT-004 | `MemorySessionPort::start` 的模型状态约束 | `swr_port_004_transcription_mode_never_emits_translation_playback_state` |
| SWR-DIAG-001 | `diagnostics::DiagnosticStore` | `swr_diag_001_store_is_bounded_monotonic_and_clearable`、前端水位线 mock 测试 |
| SWR-DIAG-002 | `diagnostics::Redactor::new`、`Drop` | `swr_diag_002_redactor_masks_encoded_and_decoded_credentials`、旧 WebSocket 日志测试 |
| SWR-DIAG-003 | `sanitize_message`、`redact_secret` | `swr_diag_003_messages_are_bounded_and_short_tokens_do_not_garble_words` |
| SWR-DIAG-004 | `LogLevel`、`LogEntry`、Tauri `Trace` 适配器 | `swr_diag_004_level_serialization_remains_frontend_compatible`、UI mock 测试 |

## 开放验证项

- `ASSURANCE-UI-001`：加入可注入内存适配器后验证 Slint 未接入/失败状态不会启动采集；
- `ASSURANCE-COV-001`：选定并鉴定或人工核查覆盖率工具，建立需求覆盖和结构覆盖报告；
- `ASSURANCE-INDEP-001`：在具备第二位合格评审者时记录验证独立性；
- `ASSURANCE-TARGET-001`：Windows/macOS 真机结果必须链接到对应需求，不能用 Linux CI 替代。
