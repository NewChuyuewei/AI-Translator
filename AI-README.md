# AI 交接：Gemini Live Translator

> 新开对话时可把**整个源码包**和本文件交给开发助手，并说「先阅读 AI-README.md，继续这个项目」。本文件记录截至 2026-11-14 的状态；应以实际源码和后续测试结果为准。项目不是生产成品。Copyright © 2026 chuyuewei. All rights reserved；完整权利声明见 `COPYRIGHT`，第三方组件仍适用各自许可。不要把用户的 API Key、代理密码、Android 签名密钥或完整未经检查的系统报告放进项目、日志或聊天。

## 1. 当前目标与不可变更的产品约定

- 桌面端当前仍使用 Tauri 2 + Rust；Android 已改为独立的 Kotlin + JDK 25 + Jetpack Compose 工程，不复用 WebView、Rust 或 JNI。Linux 用于开发测试。
- 用户已批准桌面端按 `docs/PC_RUST_SLINT_ROADMAP.md` 分阶段迁移：v2.1.0 Rust + Slint，v2.2.0 无分配回调/SPSC，v2.3.0 平台原生音频，v2.4.0 混音时钟，v2.5.0 VAD，v2.6.0 内部帧，最终 v3.0.0 生产化；当前仅制定计划，尚未开始迁移或推进版本。
- 保留现有 `Gemini 3.8 Live` 指令式翻译、`Gemini 3.8 Live Extended Thinking`（LOW/MEDIUM/HIGH）及原有系统代理、音源、字幕窗、Android 服务和连接诊断。
- 增加 `gemini-3.5-live-translate-preview`：每个会话固定**一个目标语言**，原文及译文字幕，可选择播放译音；不是双向翻译。当前 `echoTargetLanguage=false`：输入已是目标语言时不会复述。
- 增加 `gemini-3.5-transcribe-live`：用户明确选择**只实时转写原文，不做翻译，也不播放译音**；不得私自串接收费的二次翻译服务。目标语言、双向和译音控制在该模式不可用。默认 VERBATIM；可选 SMART，它只整理原文，可能删除口头语或修改格式。
- 控制台、桌面字幕窗、Android 悬浮窗和前台通知应真实区分「翻译」与「转写」，不能在转写模式显示等待译文或启动播放器。
- 稳定技术栈已于 2026-11-14 审计：Vite 8.3.8、jsdom 31.0.1、Tauri Rust 2.12.5／JS 2.12.4、Node 22.22.2+ 或 24.15.0+、Android 构建 JDK 25、Kotlin Gradle Plugin 2.4.21、Java/Kotlin target 25；精确版本和因风险暂缓的升级见 `docs/TECH_STACK.md`。
- 当前源码版本 **1.0.0**；历史 0.1.3 的 APK / Windows 包**没有** 3.5 功能，不能冒充新版交付。1.0.0 尚未生成或验证安装包。

## 2. 最重要的未解决问题：Android 启动闪退

用户报告：安装 0.1.3 ARM64 debug APK 后，Android 15+ 的 AOSP 和 Xiaomi HyperOS 手机上启动即闪退。**还没有用户崩溃堆栈，没有确认根因，0.1.4 和当前 1.0.0 源码都没有针对该闪退的已证实修复，也未在用户手机验证。**不要声称启动问题已经解决。

既往检查了 APK manifest、ABI、JNI 符号、ELF/ZIP 16 KB 对齐、签名和 Tauri/Tao 的 Android context 初始化，未发现可确定的原因；静态检查通过不等于运行正常。尝试在无 KVM 的 x86_64 主机使用 Android 15 Google APIs 模拟器及 ARM native bridge，启动极慢且在结束尝试前未完成系统 boot，**没有安装或运行 APK 的有效结果**；这也不能代替 ARM 真机。

用户只有手机，不要求其使用电脑或 ADB。下一步请用户先复现，再从 Android「开发者选项」生成系统错误报告（厂商路径不同），自行检查及删去隐私信息，只提供与包名 `com.gemini.live.translator`、崩溃时间相关的 `FATAL EXCEPTION` / `Caused by`、`Fatal signal` / tombstone 片段。报告可能含设备 ID、账号、网络、文件名等私人信息；不要索取 API Key 或未检查的整份报告。启动前崩溃不会进入应用的连接诊断控制台。官方参考：https://developer.android.com/studio/debug/bug-report 。

## 3. 模型与 WebSocket 协议

代码：`src-tauri/src/gemini_client.rs`。使用 Gemini Developer API 的 v1beta `BidiGenerateContent` WSS；API Key 通过 `x-goog-api-key` 请求头传送，不写 URL、不写日志。TLS 使用 rustls 的受信根并验证主机；只在 `setupComplete`（文本或 UTF-8 JSON 二进制帧）后发送音频。

| UI 值 | 模型 ID | setup 核心 | 模型边界 |
| --- | --- | --- | --- |
| `general_live` | `models/gemini-3.8-live` | `AUDIO` + 译员 `systemInstruction` + 输入/输出转写 | 双向是指令式判断，不是两条独立译流；无 thinkingConfig |
| `extended_thinking` | `models/gemini-3.8-live-extended-thinking` | 同上 + `generationConfig.thinkingConfig.thinkingLevel` | `turnComplete` 不应结束连接；留意 IN_PROGRESS / IDLE |
| `live_translate_35` | `models/gemini-3.5-live-translate-preview` | `AUDIO`、`generationConfig.translationConfig.targetLanguageCode`、`echoTargetLanguage=false`、setup 顶层输入/输出转写 | 单目标；UI 的 `zh` 映射 `zh-Hans`；不发 thinkingConfig |
| `transcribe_live_35` | `models/gemini-3.5-transcribe-live` | `TEXT`、顶层 `inputAudioTranscription: {languageCodes: [], mode: "VERBATIM"/"SMART"}` | 仅识别原文；不发输出转写、译员指令或 translationConfig |

不要复用 `live_translate` 作为 3.5 的序列化值：历史兼容别名 `live_translate` 目前指向 `ExtendedThinking`；改动它会破坏旧配置语义。两种 3.5 模型的 setup 均有模拟断言，真实服务端行为仍需用用户自己有权限的 Key 验证。

输入音频：16 kHz 单声道 PCM16 little-endian，约 40 ms／帧；默认静音节省模式会在连续近静音约 1 秒 后暂停发送并在检测到讲话后恢复，以降低 AI Studio 空闲音频 Token／带宽消耗。Google 的 20k Token/min 等配额按项目在服务端强制执行，客户端不能绕过，也不应把本地静音门描述成精确计费器。`realtimeInput.audio.mimeType=audio/pcm;rate=16000`；翻译音频输出为 24 kHz 单声道 PCM16。服务端 `interimInputTranscription` 是可替换猜测，`inputTranscription` 为最终权威分段，两个字段的 camelCase 和 snake_case 均支持。`lib.rs::replace_transcript` 显示最新一句，猜测不断替换、最终文本优先，不将假设和最终值追加在一起；它不是完整会议记录或带说话人分离的转录系统。

3.5 转写官方会话最长 10 分钟；约连接后 9 分钟提醒，当前**不自动续接、也不恢复上下文**。未实现说话人分离、逐词时间戳、指定输入语言、热词或文本转译。翻译模型的片段合并仍是启发式，真实服务事件可能出现未覆盖边界。

协议依据（日期可能变化，继续开发时核对）：
- https://ai.google.dev/gemini-api/docs/live-api/live-translate
- https://ai.google.dev/gemini-api/docs/live-api/live-transcribe
- https://ai.google.dev/gemini-api/docs/live-api/get-started-websocket
- https://ai.google.dev/gemini-api/docs/models/gemini-3.8-live-extended-thinking

## 4. 代码地图与跨层流向

- `src-tauri/src/lib.rs`：会话状态、取消及重连、字幕事件、文本模式强约束、9 分钟提醒、播放器和 Android 桥。IPC `StartOptions` 后端再次验证；`play_translation()` 是转写模式的额外防护。`Status.transcriptionOnly` 和 `Subtitle.transcriptionOnly` 供前端反映实际运行会话，而不是仅反映新选项。
- `src-tauri/src/gemini_client.rs`：模型枚举、每种模型的 setup、WSS、消息解码、输入/输出与 interim 转写。日志只记录安全类别，不记录原始内容。
- `src-tauri/src/audio_router.rs`：混音、重采样和有界队列；`audio_player.rs`：可选 24 kHz 输出和中断清空；`mac_system_audio.rs`：ScreenCaptureKit。
- `src-tauri/src/proxy.rs`、`diagnostics.rs`：系统/手动代理和有限的内存诊断，不保存密钥或字幕。选定路径不是 VPN 可用性的证明；选中代理失败不会自动绕过。
- `src/control.html`、`src/control.js`、`src/styles.css`：模型选择、逐字/智能转写、状态、选项、字幕预览与设备/代理；`src/index.html`、`src/overlay.js`：桌面字幕窗。桌面字幕的原文／译文区域会在内容增长时自动跟随到底部；控制台可将窗口和字号在 80%–160% 间缩放，比例保存在普通设置中。
- 0.1.5 增加本地设置恢复与加密保险库：普通偏好保存到 Tauri 应用私有目录的 `settings.json`，API Key／手动代理地址始终在普通序列化前清空。用户明确启用并提供至少 8 字符密码后，敏感字段使用 AES-256-GCM + PBKDF2-HMAC-SHA256（210,000 次、随机盐和 nonce）写入 `secrets.enc.json`；密码不保存，重启后需解锁。Unix 尝试将目录设为 0700、文件设为 0600。不要将其描述为可抵御已控制设备或 root 的硬件钥匙串。
- `plugins/mobile-audio/src/lib.rs`、`plugins/mobile-audio/android/.../AudioPlugin.kt`、`AudioCaptureService.kt`：Android JNI 音频、权限、前台服务、通知和悬浮窗。`transcriptionOnly` 从 Rust start 参数传至服务，决定通知和原生悬浮窗；避免误显示「等待翻译」。
- `tests/ui_mock_test.mjs`、`tests/overlay_mock_test.mjs` 和 Rust 单元测试：模拟 UI、协议、分段、网络及无敏感信息日志；测试不代表真实服务或真实设备可用。
- `README.md` 是用户手册；`packaging/` 含 Windows NSIS 脚本与便携版说明。版本号同时出现在 `package*.json`、`src-tauri/Cargo.toml` / `Cargo.lock`、`tauri*.conf.json` 和 NSIS 脚本，更新时保持一致。

## 5. 构建和测试

环境：Node.js 20+、npm、Rust stable、Tauri 2 平台依赖。Linux 需 GTK/WebKitGTK 4.1、ALSA 等开发包，详见 `README.md`。

```bash
npm ci
npm run build
npm run test:ui
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked
npm run tauri dev
# 目标平台本机发布构建：npm run tauri build
```

Android：安装 JDK 25、Android SDK Platform-Tools、API 36/37、Build Tools 和 NDK 28.2.13676358，设置 `JAVA_HOME`、`ANDROID_HOME`、`NDK_HOME` 后：

```bash
rustup target add aarch64-linux-android
npm ci
npx tauri android init --ci
npx tauri android build --debug --apk --target aarch64 --ci
# APK: src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

文件名含 `universal` 但该命令只构建 ARM64。`src-tauri/gen/` 是生成目录，未放入源码包。若重建生成目录但复用旧 Cargo target，Tauri/Wry 可能没有重新生成 Kotlin 文件，可先执行 `cargo clean --manifest-path src-tauri/Cargo.toml -p tauri -p wry --target aarch64-linux-android` 再构建。内存受限时限制 Cargo/Gradle 并发、Gradle 堆和 Kotlin 编译器；不要误删有价值的签名密钥。

Android debug 包仅供开发：要覆盖旧应用需**相同签名**且版本号不低；源码包不包含任何 keystore，新机器通常会使用不同的调试签名。不要把 `~/.android/debug.keystore` 或 release key 打包/分享。正式发布应使用独立、安全保管的 release 签名。

Windows GNU 交叉构建及 NSIS 命令见 `README.md`；GNU 免安装版的 `WebView2Loader.dll` 必须与 EXE 同目录。GitHub Actions 的原生 MSVC 便携 ZIP 始终包含 EXE 和说明，并在构建实际生成动态 `WebView2Loader.dll` 时一并收录。两者都需要系统 WebView2 Runtime。macOS 原生构建和签名需在 Mac 上完成。

## 6. 当前验证边界与下一步

- 1.0.0 当前源码的本地设置／加密保险库及 UI 改动已通过 `npm run build` 和 JSDOM UI／字幕窗测试，并增加普通设置清空敏感字段、加密往返、错误密码拒绝的 Rust 单元测试；当前 Arena 环境没有 `cargo`，因此尚未实际运行更新后的 Rust 测试。`.github/workflows/ci.yml` 和 `release.yml` 已加入并在 GitHub runner 试跑，已根据 Android SDK、macOS runner／SDK 错误继续修正，但完整发布仍待再次验证；发布工作流会生成 Windows NSIS 与便携 ZIP、macOS DMG、Linux DEB/AppImage 和明确标为 debug 的原生 Kotlin Android APK。尚未生成或验证 1.0.0 安装包。
- 0.1.4 前端 `npm run build`、JSDOM 控制台 / 字幕窗模拟测试通过；Linux Rust 单元和本地模拟 WebSocket 测试 **30/30** 通过。Android ARM64 debug APK 已构建，静态确认 0.1.4 / versionCode 1004、16 KB ZIP/ELF 对齐及与 0.1.3 相同的调试签名；这些检查不能证明能在手机上启动。Windows x64 GNU release 已交叉编译，NSIS 安装包与便携 ZIP 已构建并静态检查，**未在 Windows 真机运行**。
- 本次交付文件名：`Gemini-Live-Translator-0.1.4-arm64-debug.apk`、`Gemini-Live-Translator-0.1.4-Windows-Setup.exe`、`Gemini-Live-Translator-0.1.4-Windows-portable.zip`、`Gemini-Live-Translator-0.1.4-source.zip`。源码包内含本文件，但不包含其他安装包或签名密钥；新会话需要继续开发时请提供源码包。
- 没有真实 Gemini API Key，也没有可用真机，**尚未验证** 3.5 模型的真实 setup 接受情况、各语种 interim/final 事件、译音、代理、音频捕获及目标平台 UI。账号、地区和配额可能限制模型。
- 首要待办：取得用户检查并脱敏后的 Android 15+ 启动崩溃堆栈，按实际栈追踪 JNI / Activity / WebView / 原生库等，修复后再在真实手机验证；不要盲目宣称修复。
- 然后使用拥有权限的真实 API 测试两个模型：确认请求/结果结构、音频采集和取消、翻译目标语、转写逐字/智能效果、临时和最终字幕、网络中断及 10 分钟上限；不要记录密钥或音频正文。
- 在 Windows / macOS / Android 验证系统权限、录音、输出反馈、悬浮窗、后台生命周期、不同厂商与不同采样率；必要时改善自动续接和翻译模型字幕算法，但保持用户的仅转写约束。
