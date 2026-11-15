# 需求双向追踪

| 需求 | 设计/实现 | 验证证据 |
|---|---|---|
| SWR-CORE-001 | `translator_core::SessionPhase`、`SessionEvent`、Tauri typed phase adapter | `swr_core_001_lifecycle_accepts_only_defined_nominal_transitions` |
| SWR-CORE-002 | `SessionPhase::transition` 返回 `TransitionError` | `swr_core_002_invalid_lifecycle_transitions_are_rejected` |
| SWR-CORE-003 | `SessionOptions::validate`、`checked_overlay_scale` | `swr_core_003_options_enforce_model_and_untrusted_input_invariants`、`scale_is_bounded_and_normalized` |
| SWR-CORE-004 | `validate_manual_proxy`、Tauri `Trace::redact` | `swr_core_004_proxy_errors_never_echo_credentials` 及旧 Tauri 脱敏测试 |
| SWR-CORE-005 | serde rename/alias 契约 | `swr_core_005_serialized_contract_remains_compatible_with_existing_settings`、前端 mock 测试 |
| SWR-UI-001 | `desktop-slint/src/main.rs` 的适配器未接入安全拒绝路径 | Slint workspace 编译；UI 集成测试待办 `ASSURANCE-UI-001` |
| SWR-NUI-001 | Slint `thinking-index`/`transcription-index`、设置恢复和活动会话禁用门 | `swr_nui_001_protocol_option_indices_round_trip`、属性绑定代码评审 |
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
| SWR-PORT-002 | `SessionPort`、`MemorySessionPort`、Slint `NativeSessionPort` | `swr_port_002_memory_port_enforces_lifecycle_and_publishes_typed_events` |
| SWR-PORT-003 | `UiEvent`、`CaptionSnapshot::bounded`、`NoticeText` | `swr_port_003_ui_text_is_bounded_and_contains_no_pcm_variant` |
| SWR-PORT-004 | `MemorySessionPort::start` 的模型状态约束 | `swr_port_004_transcription_mode_never_emits_translation_playback_state` |
| SWR-DIAG-001 | `diagnostics::DiagnosticStore` | `swr_diag_001_store_is_bounded_monotonic_and_clearable`、前端水位线 mock 测试 |
| SWR-DIAG-002 | `diagnostics::Redactor::new`、`Drop` | `swr_diag_002_redactor_masks_encoded_and_decoded_credentials`、旧 WebSocket 日志测试 |
| SWR-DIAG-003 | `sanitize_message`、`redact_secret` | `swr_diag_003_messages_are_bounded_and_short_tokens_do_not_garble_words` |
| SWR-DIAG-004 | `LogLevel`、`LogEntry`、Tauri `Trace` 适配器 | `swr_diag_004_level_serialization_remains_frontend_compatible`、UI mock 测试 |
| SWR-NDIAG-001 | Slint `NativeDiagnostics::begin_boundary`/`record`、`diagnostic-model` | `swr_ndiag_001_boundary_redacts_ui_lines_and_copy_text`、UI 绑定代码评审 |
| SWR-NDIAG-002 | `NativeDiagnostics::copy_text` 64 Ki 字符上限、核心 `clear` 水位线 | `swr_ndiag_002_copy_is_bounded_and_clear_uses_core_watermark` |
| SWR-NCAP-001 | `CaptionWindow` 的 `always-on-top`、`no-frame`、透明背景与受 `locked` 控制的 `WindowMoveArea` | Slint 编译、属性绑定代码评审；真机待办 `ASSURANCE-TARGET-006` |
| SWR-NCAP-002 | 鼠标穿透保持关闭直至平台适配器成功并完成验收 | 故障注入待办、真机待办 `ASSURANCE-TARGET-006` |
| SWR-SET-001 | `settings_for_storage`、`encode_settings` | `swr_set_001_encode_never_contains_sensitive_values`、旧 Tauri 设置测试 |
| SWR-SET-002 | `PersistedSettings`、`decode_settings` | `swr_set_002_current_format_round_trips_with_compatible_fields`、前端 mock 测试 |
| SWR-SET-003 | `DecodedSettings::migration_required`、Tauri 原子写适配器 | `swr_set_003_legacy_migration_is_explicit_and_erases_secrets`、代码评审 |
| SWR-SET-004 | `MAX_SETTINGS_BYTES`、`decode_settings` | `swr_set_004_rejects_oversize_corruption_and_unknown_versions` |
| SWR-VAULT-001 | `vault::encrypt_secrets`、`decrypt_secrets`、固定 AAD | `swr_vault_001_round_trip_preserves_legacy_format_without_plaintext`、旧 Tauri 回归 |
| SWR-VAULT-002 | `SystemRandom` salt/nonce、`Zeroize` 清理 | `swr_vault_002_random_nonce_prevents_repeat_ciphertext`、代码评审 |
| SWR-VAULT-003 | `decrypt_secrets` 认证和安全错误映射 | `swr_vault_003_wrong_password_and_tampering_share_safe_error` |
| SWR-VAULT-004 | `MAX_VAULT_BYTES`、格式和 KDF 边界检查 | `swr_vault_004_bounds_password_input_and_format_parameters` |
| SWR-NCFG-001 | Slint `ConfigStore::discover`、`load_settings`、`vault_exists` | `swr_ncfg_001_store_round_trips_settings_and_vault_separately`、路径代码评审 |
| SWR-NCFG-002 | Slint `write_private`、Windows `replace_windows` | `swr_ncfg_002_replacement_does_not_leave_plaintext_or_temp_files`、平台代码评审 |
| SWR-NCFG-003 | `read_bounded`、Unix 权限分支 | `swr_ncfg_003_unix_files_are_private`、核心超限测试 |
| SWR-NCFG-004 | Slint 保存/解锁/删除回调和 `Zeroize` | 安全失败代码评审；交互测试待办 `ASSURANCE-UI-002` |
| SWR-DEV-001 | `devices::AudioDevices::bounded`、`AudioDeviceInfo::bounded` | `swr_dev_001_snapshot_is_bounded_and_control_characters_are_removed` |
| SWR-DEV-002 | `devices::normalize` | `swr_dev_002_duplicate_ids_and_defaults_are_deterministically_normalized` |
| SWR-DEV-003 | `AudioDevices::validate_selection` | `swr_dev_003_stale_and_cross_class_selections_are_rejected`、旧 Tauri 枚举回归 |
| SWR-DEV-004 | `DeviceProvider`、`MemoryDeviceProvider`、Slint 快照替换分支 | `swr_dev_004_memory_provider_has_deterministic_recovery`、失败保留代码评审 |
| SWR-NDEV-001 | Slint `NativeDeviceProvider::refresh`（仅 CPAL 枚举 trait） | workspace/Windows/macOS 编译；API 代码评审；真机待办 `ASSURANCE-TARGET-002` |
| SWR-NDEV-002 | Slint `apply_devices`、`selected_device`、启动前 `validate_selection` | `swr_ndev_002_ui_indices_preserve_backend_identifiers`、`swr_dev_003_stale_and_cross_class_selections_are_rejected`、真机待办 `ASSURANCE-TARGET-002` |
| SWR-PXY-001 | `proxy::ProxyEndpoint`、`ProxyKind` | `swr_pxy_001_supported_endpoints_are_canonical_and_typed`、旧代理隧道测试 |
| SWR-PXY-002 | `ProxyCredentials`、`parse_credentials`、`Drop` | `swr_pxy_002_credentials_are_decoded_bounded_and_never_echoed`、代码评审 |
| SWR-PXY-003 | `ProxyEndpoint::parse` 安全错误映射 | `swr_pxy_003_structure_and_credential_limits_are_enforced`、旧代理脱敏测试 |
| SWR-PXY-004 | `ProxyEndpoint::safe_label`、核心无运行时边界 | `swr_pxy_004_labels_are_fixed_and_credential_free`、依赖评审 |
| SWR-TUN-001 | `proxy_handshake::http_connect_request`、`SensitiveBytes` | `swr_tun_001_http_request_is_exact_and_bounded`、旧 HTTP 隧道测试 |
| SWR-TUN-002 | `parse_http_connect_response`、`MAX_HTTP_CONNECT_HEADER_BYTES` | `swr_tun_002_http_response_is_bounded_and_status_allowlisted` |
| SWR-TUN-003 | SOCKS greeting/method/auth/connect 纯函数 | `swr_tun_003_socks_auth_and_remote_dns_are_exact`、旧 SOCKS5 远端 DNS 测试 |
| SWR-TUN-004 | `parse_socks_reply_prefix`、`socks_reply_tail_len` | `swr_tun_004_socks_reply_shapes_are_deterministic`、旧 IPv6 响应测试 |
| SWR-ROUTE-001 | `ProxyRoute::immediate`、`ProxyDestination` | `swr_route_001_direct_and_manual_modes_resolve_without_platform_access` |
| SWR-ROUTE-002 | `ProxyRoute::automatic_resolver_value`、Tauri 首选值策略 | `swr_route_002_auto_requires_one_platform_selected_value`、旧无直连回退隧道测试 |
| SWR-ROUTE-003 | 自动值安全上下文映射 | `swr_route_003_invalid_auto_value_has_fixed_context_without_secrets` |
| SWR-NTRANS-001 | Slint `native_transport::probe_endpoint`、`SessionDriver` | `swr_ntrans_001_probe_sends_header_setup_and_end_but_no_pcm` |
| SWR-NTRANS-002 | `WebSocketConfig` 上限及分阶段 timeout | workspace 编译、代码评审；故障注入待办 `ASSURANCE-NET-001` |
| SWR-NTRANS-003 | `resolve_route`、`open_route`、核心握手函数 | 核心 SWR-ROUTE/TUN 测试、旧异步隧道测试 |
| SWR-NTRANS-004 | Slint `probe-transport` 后台线程、`probe-running` 门和安全错误映射 | 代码评审；UI 驱动待办 `ASSURANCE-UI-003` |
| SWR-NTRANS-005 | `ProbeCancel`、分阶段 `tokio::select!`、可取消平台代理等待 | `swr_ntrans_005_setup_wait_is_cancellable`、退出回调代码评审 |
| SWR-NTRANS-006 | `session_blocking`、共享 `establish_session`/`receive_message`/`close_session` | `swr_ntrans_006_and_007_session_sends_audio_then_forwards_and_stops`、无 PCM 探测测试 |
| SWR-NTRANS-007 | `session_with_audio_blocking`、有界 Tokio 接收器、biased cancel、`SessionDriver::encode_audio` | `swr_ntrans_006_and_007_session_sends_audio_then_forwards_and_stops` |
| SWR-NPORT-001 | Slint `NativeSessionPort` 的音源显式安全门 | `swr_nport_001_unverified_audio_modes_remain_fail_closed` |
| SWR-NPORT-002 | `TransportTask`、`BoundedEventQueue<WorkerEvent, 32>`、原子取消与延迟 join | `swr_nport_002_worker_mailbox_is_bounded_and_freshness_first`、生命周期代码评审 |
| SWR-NPORT-003 | `NativeSessionPort::handle_worker_event`、Slint 25 ms/最多 32 项定时排空 | `swr_nport_003_worker_events_map_to_typed_ui_lifecycle`、UI 适配代码评审 |
| SWR-NAUDIO-001 | `NativeAudioTask`、setupComplete 后启动、音频/传输取消顺序 | 原生端口状态测试、生命周期代码评审；真机待办 `ASSURANCE-TARGET-003` |
| SWR-NAUDIO-002 | CPAL 回调有界 `sync_channel(12)`、工作线程 `Resampler`/`PcmFramer` | `swr_naudio_001_resampling_is_chunk_boundary_independent`、回调代码评审 |
| SWR-NAUDIO-003 | `PCM_QUEUE_FRAMES=6`、16 kHz/640 样本/PCM16 | `swr_naudio_002_framer_emits_exact_bounded_pcm_frames` |
| SWR-NAUDIO-004 | `SilenceGate`、RMS 阈值、25 个 40 ms hangover 帧 | `swr_naudio_004_token_saver_preserves_hangover_and_resumes_on_speech` |
| SWR-NAUDIO-005 | Windows CPAL/WASAPI render loopback、`AudioMixer` 双源 40 ms 时钟和 160 ms 源缓存 | `swr_naudio_005_mixed_frames_use_bounded_convex_weights`、真机待办 `ASSURANCE-TARGET-005` |
| SWR-NPLAY-001 | `NativePlaybackTask`、`playback_required` 双就绪门及失败回滚 | `swr_nport_003_worker_events_map_to_typed_ui_lifecycle`、真机待办 `ASSURANCE-TARGET-004` |
| SWR-NPLAY-002 | 固定 12 项 `Command` 队列、播放工作线程重采样、CPAL `try_lock` 回调 | `swr_nplay_001_resampler_is_chunk_boundary_independent`、回调代码评审 |
| SWR-NPLAY-003 | 原子 epoch、`clear`、480 ms 样本上限及 `Zeroize` | `swr_nplay_002_clear_invalidates_queued_audio_immediately`、代码评审 |

## 开放验证项

- `ASSURANCE-UI-001`：加入可注入内存适配器后验证 Slint 未接入/失败状态不会启动采集；
- `ASSURANCE-UI-002`：使用 Slint 交互驱动验证密码属性即时清空、保险库失败状态及保存/解锁/删除按钮状态；
- `ASSURANCE-UI-003`：验证网络探测重复点击门、后台完成回调及窗口关闭后的安全丢弃；
- `ASSURANCE-NET-001`：对 DNS、TCP、代理、TLS、WebSocket、setup 发送和 setupComplete 等待逐阶段注入超时/取消故障；
- `ASSURANCE-COV-001`：选定并鉴定或人工核查覆盖率工具，建立需求覆盖和结构覆盖报告；
- `ASSURANCE-INDEP-001`：在具备第二位合格评审者时记录验证独立性；
- `ASSURANCE-TARGET-001`：Windows/macOS 真机结果必须链接到对应需求，不能用 Linux CI 替代。
- `ASSURANCE-TARGET-002`：在 Windows/macOS 真机验证 Slint 设备名称、默认端点、热插拔刷新、持久化选择恢复和权限失败；Linux CI 仅证明可编译。
- `ASSURANCE-TARGET-003`：在 Windows/macOS 真机验证 Slint 麦克风权限、默认/指定设备、16 kHz PCM 上行、设备失效和确定性停止；完成前不得宣称麦克风生产验收。
- `ASSURANCE-TARGET-004`：在 Windows/macOS 真机验证 Slint 默认/指定播放设备、24 kHz 译音重采样、中断清空、设备失效及停止无残留；完成前不得宣称译音播放生产验收。
- `ASSURANCE-TARGET-005`：在 Windows 真机验证 Slint WASAPI loopback 默认/指定 render endpoint、系统音频、麦克风混合、单路欠载、设备失效和停止；完成前不得宣称 Windows 系统采集生产验收。
- `ASSURANCE-TARGET-006`：在 Windows/macOS 真机验证字幕窗置顶、透明无边框、解锁拖动、锁定禁止拖动，以及鼠标穿透失败时仍可恢复；Linux CI 不能替代窗口管理器验收。
