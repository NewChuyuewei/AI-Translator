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

## 开放验证项

- `ASSURANCE-UI-001`：加入可注入内存适配器后验证 Slint 未接入/失败状态不会启动采集；
- `ASSURANCE-COV-001`：选定并鉴定或人工核查覆盖率工具，建立需求覆盖和结构覆盖报告；
- `ASSURANCE-INDEP-001`：在具备第二位合格评审者时记录验证独立性；
- `ASSURANCE-TARGET-001`：Windows/macOS 真机结果必须链接到对应需求，不能用 Linux CI 替代。
