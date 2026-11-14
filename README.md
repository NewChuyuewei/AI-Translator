# Gemini Live Translator

> **版权声明**：Copyright © 2026 chuyuewei. All rights reserved. 本软件及项目所有者创作的源代码、界面设计、文档和发布产物归 chuyuewei 所有；未经事先书面许可，不得复制、修改、再分发、出售或用于其他商业产品。第三方组件、服务及商标仍归各自权利人所有并适用其各自许可。完整声明见 [`COPYRIGHT`](COPYRIGHT)。

Tauri 2 + Rust 的实时语音翻译与转写**开发原型**：支持麦克风、系统播放音频和两者混合，向 Gemini Live 发送音频。翻译模型显示原文／译文并可选播放译音；转写模型只显示原文。桌面端有控制面板和置顶透明字幕窗；Android 使用原生 Kotlin 前台采集服务和可选的系统悬浮字幕。Android 正分阶段迁移为 Kotlin + Jetpack Compose UI，并保留 Rust 安全核心，计划见 [`docs/ANDROID_KOTLIN_MIGRATION.md`](docs/ANDROID_KOTLIN_MIGRATION.md)。

> **验证范围**：这是开发原型，开发环境没有真实 Gemini API Key，也没有 Windows、macOS 或 Android 真机；新模型、系统代理、音频和界面尚未完成真实服务端及目标设备端到端验证，不承诺任意网络或音频来源均可用。
>
> **Android 已知严重问题**：用户报告 0.1.3 ARM64 调试 APK 在 Android 15+（AOSP 和 Xiaomi HyperOS）安装后启动即闪退；尚无崩溃堆栈，根因未知，**0.1.4 及当前 0.2.0 源码都不应被视为已修复闪退**。排查需要手机系统错误报告，详见下方「测试与排障」和 `AI-README.md`。

## 功能与使用

1. 在 [Google AI Studio](https://aistudio.google.com/api-keys) 创建可使用 Gemini Live 的 API Key；启动应用，在控制面板填写。
2. 选择模型与音源：
   - **Gemini 3.8 Live / Extended Thinking**：指令式翻译，可启用依靠模型判断的双向自动切换；Extended Thinking 的低／中／高推理深度可能影响延迟。
   - **Gemini 3.5 Live Translate**：专用语音翻译，每次会话固定一个目标语言，显示原文和译文，可选播放译音；不支持双向自动切换。输入已经是目标语言时，不复述该语音。
   - **Gemini 3.5 Transcribe Live**：只将语音识别成原文，不翻译也不输出译音；自动检测语言，可选默认逐字转写（VERBATIM）或智能整理（SMART，仍为原文，可能删除口头语并修改格式）。会话最长 10 分钟，到期需手动重新开始。
   选择麦克风、系统音频或混合音源；翻译模型如需播放译音，打开「播放翻译语音」。
3. 点击「开始翻译」或「开始转写」。Android 需按系统提示授权麦克风，使用系统音频时还需**每次会话**同意 MediaProjection。
4. 桌面端字幕窗默认系统级鼠标穿透：在控制面板点「解锁字幕窗以拖动」，拖动窗口上方把手，再点「锁定并穿透」。完全穿透时把手也无法接收鼠标，这是操作系统窗口机制的限制。控制面板的「字幕窗缩放」可在 80%–160% 间同步调整窗口和字幕字号；该偏好会自动保存。持续增长的字幕会自动滚动到最新内容。
5. Android 若要在其他应用上方显示字幕，先点「授权系统悬浮窗」并在系统设置中允许；启动后可解锁、拖动、重新锁定。悬浮窗不是锁屏显示的保证。
6. 桌面端 **F8** 或面板按钮停止；再次开始会建立**新的会话**。F8 的“暂停”是停止采集和连接，不保留原会话或恢复上下文。更改音源、设备、语言或模型会停止并重新连接；Android 系统内录可能再次请求授权。桌面关闭控制面板会退出应用。

提示：F8 可能被其他应用、操作系统或笔记本媒体键占用；Linux Wayland 对全局快捷键和穿透也可能有限制。Android 没有全局 F8 功能。

### 本地设置与敏感信息

0.1.5 起，模型、语言、音源、播放和网络模式等非敏感偏好会自动写入操作系统提供的应用私有配置目录，并在下次启动时恢复。API Key 和手动代理地址永远不会进入普通 `settings.json`；如需跨启动保留，须明确勾选「加密保存 API Key 和手动代理地址」，设置至少 8 个字符的密码并点击保存。下次启动需再次输入密码解锁；应用不保存密码。取消勾选后会删除本地加密保险库，但当前会话仍可继续使用内存中的值。

保险库采用 **AES-256-GCM** 认证加密，密钥使用 **PBKDF2-HMAC-SHA256（210,000 次）**、随机盐和用户密码派生，每次保存使用随机 nonce；加密文件同时依赖应用数据隔离和文件权限。忘记密码后无法恢复。该功能降低配置文件或单独备份泄露造成的风险，但运行中密码和 API Key 仍存在进程内存，无法抵御已控制设备、恶意进程、root／越狱或键盘记录，因此仍不是硬件钥匙串。设置不会同步到云端，也不会进入诊断日志。检测到早期开发快照的明文敏感配置时会自动清除，用户需要重新输入并加密保存。

## 连接诊断控制台（0.1.3）

控制面板的「连接诊断控制台」会显示网络路径选择、TCP/代理隧道、TLS/WebSocket 握手、`setup` / `setupComplete`、消息帧类型、状态代码、耗时以及音频准备等阶段。出现连接问题时：

1. 点击「清空」，然后再次点击「开始翻译」并等待出现错误。
2. 点击「复制日志」，检查内容后粘贴给协助排查的人；若按钮不可用，可在日志文本框中手动全选、复制。Android 使用系统剪贴板，桌面使用 WebView 的剪贴板或选中文本备用方式。
3. 不要发送 API Key、代理地址中的用户名/密码、截图里的密钥或其他私人信息。复制后日志会暂时留在系统剪贴板，可能被其他应用读取；不再需要时可用其他文字覆盖。

应用只保存最近 300 条诊断到进程内存，退出后清除；不会主动建立诊断日志文件。日志只输出固定的阶段、数据帧类型、允许的错误状态及数字代码，不记录 API Key、代理凭据、音频、字幕或服务端原始消息／关闭原因；对于服务端错误只给出有限的通用提示。启动于终端时也会把相同的诊断写到标准错误，Android 系统或终端可能自行收集这些输出。分享前仍请人工检查，切勿主动粘贴密钥或私人信息。

0.1.3 修复初始化时只处理文本帧的问题：Gemini 的 `setupComplete` 也可能以 UTF-8 JSON 二进制帧到达，旧版会忽略该消息，进而错误地等待超时。本版在初始化和会话中统一处理文本／二进制 JSON；无效数据会显示安全错误而不是静默忽略。该问题是用户报告的超时的**可能原因之一**，网络、代理、权限、服务或模型可用性也可能造成失败。

## 系统代理、VPN 与手动代理

本功能只影响**本应用到 Gemini Live API 的连接**，不会修改系统网络设置，也不会安装或提供代理节点，不能改变 Google 账户、区域、模型权限或服务配额；Google AI Studio 网页则由你使用的浏览器及其网络设置控制。

1. 先启动你已有的代理软件，启用其系统代理，或者启动 VPN 并确保 VPN 的分应用规则包含本应用。
2. 控制面板「网络连接」默认选择 **自动：使用系统代理 / VPN**。Windows 从当前用户系统代理配置选择到目标 HTTPS URL 的路径，支持常见的静态代理、绕过规则和 PAC/WPAD；Android 通过系统 `ProxySelector` 查询当前网络的 HTTP/PAC 选择。VPN 通常作为系统网络路由使用，不一定表现为 HTTP 代理。macOS 使用系统代理解析，Linux 使用支持的 GNOME/KDE 设置或环境变量（取决于桌面环境）。
3. 点击「开始翻译」。面板中的「选定路径」仅说明本应用选择了哪类路由，**不能证明 VPN、出口地区或 Google 访问已经可用**。代理连接失败后不会自动跳过所选代理重试直连，会显示错误。
4. 如果自动检测未找到代理，选择 **手动：HTTP / SOCKS5 代理**，按你的软件提供的地址输入，例如 `http://127.0.0.1:7890` 或 `socks5://127.0.0.1:1080`。`127.0.0.1` 指运行本应用的那台电脑或手机，端口必须与该设备的代理软件一致。支持 HTTP CONNECT 和 SOCKS5（目标域名交给 SOCKS5 代理解析），不支持 SOCKS4、HTTPS 加密代理入口及 NTLM/Kerberos 等企业代理认证。
5. 若使用纯 VPN 或希望不使用显式应用代理，可选择 **系统网络：不使用显式代理**。这是使用操作系统网络，不是关闭系统 VPN；如果 VPN 没有覆盖本应用，则可能使用普通网络。

系统 PAC/绕过规则可能主动返回 `DIRECT`，此时自动模式会按系统规则使用系统网络，它不等于验证过 VPN。**若你要求不能绕过代理，应选手动代理**；所选代理失败后本应用不会转为直连。自动模式只使用系统返回的首个路径，不会依次尝试 PAC 列表中的其他代理。系统代理或 PAC 不可读、超时或不兼容时请改用手动地址；某些代理软件仅设置浏览器扩展、未设置系统代理，或者 VPN 排除了本应用，都不会被自动修复。Windows 同时配置手动代理与 WPAD 时，当前解析器可能优先手动代理；复杂企业网络及拨号/VPN 独立配置需自行测试。Linux 对 PAC 和环境变量的支持取决于桌面环境及解析器配置。

如代理需要用户名/密码，可使用 `http://用户名:密码@主机:端口` 或 `socks5://用户名:密码@主机:端口`，特殊字符需 URL 百分号编码。**HTTP CONNECT / SOCKS5 的代理认证信息在到代理的这段连接上不加密**，只应连接可信代理，优先使用本机代理；地址输入框可能在屏幕上可见，不要截图或分享。手动代理地址和 API Key 默认只存在于进程内存；只有明确启用并设置密码后才写入独立的加密保险库。代理协商完成后 Gemini WSS 使用 rustls 验证服务器证书与主机名，API Key 只放在加密连接的请求头，不放在代理 CONNECT 请求或 URL 中。Android 系统代理接口不提供代理凭据，需认证时请在手动模式填写。

WSS 使用内置的 WebPKI 根证书，而非自动采用企业自建根证书；本版没有自定义 CA 的界面。如果网络代理解密并重签 TLS，证书可能不被信任而连接失败，不应通过关闭证书校验来规避。

桌面系统代理解析使用 [proxyres 1.5.7](https://crates.io/crates/proxyres/1.5.7)（`MIT AND MPL-2.0`；源码及许可可从其 crate 页面获取）；其他第三方依赖遵循各自许可。

## 模型与音频协议

- **3.8 译员模式**：`models/gemini-3.8-live`，使用系统指令要求只翻译、不回答，不发送 `thinkingConfig`。
- **Extended Thinking 模式**：`models/gemini-3.8-live-extended-thinking`，使用同样的译员指令，在 `generationConfig.thinkingConfig.thinkingLevel` 中发送 `LOW`、`MEDIUM` 或 `HIGH`；不发送 `translationConfig`。`turnComplete` 只标记一次发言，不终止连接；`interactionStatus` 为 `IN_PROGRESS` 时仍继续接收后续字幕和音频。
- 上面两种是**通用实时对话模型而非专用翻译模型**，可能回答、添加内容或识别错语言；双向自动切换依赖模型判断，不是两条独立译流。
- **3.5 Live Translate**：`models/gemini-3.5-live-translate-preview`。`generationConfig.responseModalities=["AUDIO"]`，`generationConfig.translationConfig.targetLanguageCode` 为唯一目标语言，`echoTargetLanguage=false`。输入和输出音频转写对象位于 `setup` 顶层。界面「中文（简体）」对该模型映射为 `zh-Hans`；不发送思考配置或 3.8 译员指令。
- **3.5 Transcribe Live**：`models/gemini-3.5-transcribe-live`。请求 `TEXT` 和顶层 `inputAudioTranscription: {languageCodes: [], mode: "VERBATIM" 或 "SMART"}`，空列表表示自动语言检测；不请求译音、输出转写或翻译配置。服务端 `interimInputTranscription` 是可替换的猜测，`inputTranscription` 是权威最终分段；当前字幕只显示最新分段，不保存完整记录。相同消息内同时出现两者时最终结果优先。转写模式的译音播放器不会启动，后端也拒绝不适用的播放／双向参数。
- 输入音频经 WSS 发送至 Google，统一为 **16 kHz 单声道 PCM16 little-endian**，约 40 ms 一帧；翻译模型的译音为 **24 kHz 单声道 PCM16**。WebSocket 等待 `setupComplete` 后才发送音频；收到 `interrupted` 会清空译音缓冲。
- 输入设备不同采样率、声道数会在 Rust 中降混和重采样；混合模式为两路各 50%，不足的帧补静音。队列有上限，网络或设备跟不上时可能丢帧；这是低延迟简化实现，不是高保真混音、精密时钟同步或声学回声消除器。
- Google AI Studio 的 Token/min 配额按 Google 项目在服务端统计，客户端不能绕过或精确替代服务端计数。默认开启“静音时节省 Token”：连续近静音约 1 秒 后停止上传 PCM，检测到讲话后立即恢复，以减少空闲音频的 Token 和带宽消耗。噪声环境可能使静音门保持开启；担心漏掉轻声时可以关闭。收到 HTTP 429 / `RESOURCE_EXHAUSTED` 时应用会显示配额提示，请等待 Google 的配额窗口恢复，不应通过快速重连规避限制。
- 会话接近到期时服务端可能发送 `goAway`。3.5 Transcribe 官方会话上限为 10 分钟，应用约在连接后 9 分钟提醒；当前版本**不自动续接或恢复上下文**，断开后请手动重新开始。翻译模型字幕分片的合并是启发式处理。转写模型不支持说话人分离和逐词时间戳。模型可用性、延迟、配额和费用取决于 Google 账户及服务。

## 平台音频与权限

| 平台 | 麦克风 | 系统音频 | 关键限制 |
|---|---|---|---|
| Windows | CPAL 默认或指定输入设备 | CPAL 在输出端点上建立 WASAPI loopback | 需要目标 Windows 设备实测；回录整个选定输出端点，可能录到本应用译音及提示音。受保护音频可能不可捕获。 |
| macOS 13+ | CPAL / CoreAudio | ScreenCaptureKit，配置为不捕获本进程 | 须授权麦克风及屏幕录制／系统音频；当前实现按请求的 48 kHz、float PCM 格式处理，实际采样格式和设备兼容性需在 Mac 验证。透明窗口使用 Tauri 的 macOS 私有 API，可能不适合 Mac App Store。 |
| Android 8+ | `AudioRecord`，`RECORD_AUDIO` | Android 10/API 29+ `MediaProjection` + `AudioPlaybackCapture` | 需要用户每次同意系统投影；播放来源须允许捕获、在同一用户 profile，且音频用途须符合 Android 的捕获规则。通话、受保护或禁止捕获的音频可能不可捕获；设备列表目前使用系统默认设备。 |
| Linux | CPAL / ALSA（仅作开发辅助） | 未实现 | 本项目目标是 Windows、macOS、Android；Linux 系统音频模式会提示不支持。 |

### Android 重要说明

插件清单声明 `RECORD_AUDIO`、`FOREGROUND_SERVICE`、`FOREGROUND_SERVICE_MICROPHONE`、`FOREGROUND_SERVICE_MEDIA_PROJECTION`、`POST_NOTIFICATIONS`、`WAKE_LOCK` 和 `SYSTEM_ALERT_WINDOW` 等。内录只走官方允许的用户授权 `MediaProjection` + `AudioPlaybackCapture`；不请求普通第三方应用无法获得的 `CAPTURE_AUDIO_OUTPUT` 特权权限，也不绕过其他应用的捕获策略。

前台服务在可见 Activity 中由用户启动，显示持续通知；Android 13+ 若通知权限被拒绝，其可见性取决于系统。Android 14+ 对麦克风 while-in-use 和投影前台服务的启动顺序、后台启动有严格限制。服务为非 sticky，并持有最多 30 分钟的 wake lock：切应用、锁屏、被系统回收、厂商省电或强制停止后**不保证持续运行或自动恢复**。需用真实设备测试，且不能保证在锁屏界面显示悬浮字幕。

**避免反馈**：Windows 的端点回录会录到同一输出设备上的译音，即使用耳机也可能形成数字回录。应关闭译音或将译音播放到不被回录的独立输出设备；麦克风拾取扬声器声音时则可使用耳机。Android 尝试排除本应用 UID；macOS 尝试排除本进程，但都仍应实测。

## 项目结构

```text
AI-README.md                  新对话继续开发的状态与交接
src/
  control.html, control.js     控制面板、设备和会话设置
  index.html, overlay.js       桌面字幕窗
  styles.css                   样式
src-tauri/
  Cargo.toml, Cargo.lock       Rust / Tauri 依赖
  tauri.conf.json              桌面双窗口与打包设置
  tauri.android.conf.json      Android 单窗口覆盖
  Info.plist, Entitlements.plist  macOS 权限及签名配置
  capabilities/default.json   前端能力权限
  src/lib.rs                  会话控制、命令、事件、F8
  src/audio_router.rs         输入采集、混音、重采样、PCM 分帧
  src/audio_player.rs         译音播放、缓冲与中断
  src/gemini_client.rs        Gemini Live WebSocket 协议与 TLS
  src/diagnostics.rs          有界内存诊断、事件与敏感信息防护
  src/proxy.rs                系统代理选择、HTTP CONNECT / SOCKS5 隧道
  src/mac_system_audio.rs     macOS ScreenCaptureKit
packaging/windows-installer.nsi  Windows 当前用户安装包脚本
packaging/windows-portable-readme.txt  免安装版随附说明
plugins/mobile-audio/
  src/lib.rs                  Tauri Android 插件和 JNI 接口
  android/src/main/AndroidManifest.xml
  android/src/main/java/com/gemini/livetranslator/audio/
    AudioPlugin.kt            权限、MediaProjection、Android 系统代理查询与 Tauri 回调
    AudioCaptureService.kt    前台服务、采集线程、悬浮窗
    NativeAudioBridge.kt      PCM 到 Rust 的 JNI 桥
```

音频通过 JNI 直接进入 Rust 有界队列，不经过 WebView JSON。API Key 在界面和进程内存中使用；默认不持久化，用户明确启用并提供密码后可写入独立的 AES-256-GCM 加密保险库，但永远不应写入普通设置、源码、诊断日志或发布包。握手通过 `x-goog-api-key` 请求头传递。不要在公开仓库、日志或截图中暴露密钥。本项目是用户自行输入密钥的直接连接原型，**不是生产级密钥保护方案**；面向公共客户端时应考虑服务端签发 [ephemeral tokens](https://ai.google.dev/gemini-api/docs/live-api/ephemeral-tokens)、配额与滥用控制。

## 通用依赖与桌面构建

当前稳定技术栈版本、升级依据和暂缓的破坏性升级见 [`docs/TECH_STACK.md`](docs/TECH_STACK.md)。

安装 Node.js 22.22.2+（22.x）或 24.15.0+、npm、Rust stable（项目声明最低 Rust 1.90，锁文件在 Rust 1.100 测试），以及 [Tauri 2 平台依赖](https://v2.tauri.app/start/prerequisites/)。在项目根目录：

```bash
npm ci
npm run build
npm run tauri dev       # 开发运行；需要图形桌面
npm run tauri build     # 本机平台 release 构建和打包
```

- **Windows**：安装 Microsoft C++ Build Tools（“使用 C++ 的桌面开发”）和 WebView2 Runtime，建议在 Windows 主机上构建并验证 WASAPI。
- **macOS**：安装 Xcode 和 Command Line Tools。`src-tauri/Info.plist` 会合并到应用 Info.plist；`Entitlements.plist` 配有麦克风及网络 entitlement。首次使用时在系统设置「隐私与安全性」中允许麦克风及屏幕录制，必要时重启应用。签名、公证和捕获行为须在 Mac 上验证。
- **Ubuntu/Debian 开发参考**：安装 `build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev`，其余依赖以 Tauri 官方文档为准。

浏览器直接打开前端只能看界面，不会运行 Tauri 命令、音频采集或 Gemini 会话。

### 本次 Windows 交付物

Windows **0.2.0** 源码面向 **Windows 10/11 x64**，需要 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。安装版为当前用户安装到本地应用目录，可选择路径，创建开始菜单快捷方式和卸载入口，无需管理员权限。免安装版须完整解压后运行其中的 `Gemini-Live-Translator.exe`，**`WebView2Loader.dll` 必须与主程序位于同一目录**。安装版与主程序均未签名，可能触发 Windows SmartScreen 提示。

本构建使用 Linux 到 `x86_64-pc-windows-gnu` 的交叉编译，已静态检查 PE 格式、导入项、模型 ID 和安装包载荷；没有 Windows 真机验证，也没有真实 API Key 联调。特别是麦克风、WASAPI 系统回录、悬浮窗、F8 和 WebView2 行为仍需在目标设备验证。

如需在 Linux 上重建上述 GNU 便携文件及安装程序，安装 MinGW-w64 x86-64 GCC/G++ 和 NSIS，然后在项目根目录运行（内存充裕时可使用标准 release 配置）：

```bash
rustup target add x86_64-pc-windows-gnu
npm ci && npm run build
export CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_OPT_LEVEL=1
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256 CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_LTO=false
cargo build --manifest-path src-tauri/Cargo.toml --locked --release \
  --target x86_64-pc-windows-gnu --bin gemini-live-translator
# 根据自定义 CARGO_TARGET_DIR 调整下列路径。
P="$PWD/packaging/payload"
B="$PWD/src-tauri/target/x86_64-pc-windows-gnu/release"
mkdir -p "$P"
cp "$B/gemini-live-translator.exe" "$P/Gemini-Live-Translator.exe"
cp "$B/WebView2Loader.dll" "$P/WebView2Loader.dll"
LC_ALL=C.UTF-8 makensis -INPUTCHARSET UTF8 -DINPUT_DIR="$P" \
  -DOUTPUT_FILE="$PWD/Gemini-Live-Translator-Windows-Setup.exe" \
  packaging/windows-installer.nsi
```

GNU 版本必须携带 DLL；上面的安装程序包含该 DLL，但不包含 WebView2 Runtime。跨平台构建并非对 Tauri 所有版本都受官方支持，正式发布建议在 Windows 主机使用 MSVC、代码签名并完成实机测试。

### GitHub Actions 自动构建与发布

- `.github/workflows/ci.yml`：推送到 `main`／`arena/**`、Pull Request 或手动触发时，执行前端构建、JSDOM 测试和锁定依赖的 Rust 单元测试。
- `.github/workflows/release.yml`：推送 `v*` 标签或手动填写标签时，在原生 runner 构建 Windows x64 NSIS 安装包与便携 ZIP、macOS Apple Silicon DMG、Linux x64 DEB/AppImage，以及 Android ARM64 debug APK。手动运行可选择 `build_only`（只保留 Actions 构建产物）、`prerelease`（发布 GitHub 预发布）或 `release`（发布正式版本）；推送标签默认发布正式版本。发布模式会在全部平台成功后生成 SHA-256 校验文件并创建或更新 GitHub Release。

当前源码基准版本为 `0.2.0`。手动运行默认选择安全的 `build_only`，不会创建 GitHub Release；确认产物后可用同一版本选择预发布或正式发布。发布工作流支持自定义严格三段式版本：在 Actions 手动运行时填写 `vMAJOR.MINOR.PATCH`（例如 `v0.2.1`），工作流会在各个隔离的构建任务中运行 `node scripts/set-version.mjs MAJOR.MINOR.PATCH`，同步 npm、Cargo、Tauri、Android、NSIS 和便携版产物版本；不会把临时版本改写提交回仓库。也可以推送同格式标签触发。正式源码升级可在本地运行该脚本后提交。预发布后缀目前不支持，以免生成无效的 Windows 四段数字版本资源。当前工作流没有代码签名证书或 Android release keystore，因此桌面产物未签名，Android 产物明确为 **debug 开发包**；不得冒充应用商店或生产发布包。首次使用应在仓库 Actions 页面观察完整运行结果，目标平台失败时按 runner 日志调整依赖或签名配置。

## Android 构建与安装

当前源码版本为 **0.2.0**；发布工作流生成仅含 `arm64-v8a` 的 Android debug APK，最低 Android 8.0/API 26，系统音频捕获需要 Android 10/API 29 及以上。APK 的实际 `versionCode` 由当前 Tauri Android 模板生成，应以构建产物清单为准。调试签名可能随构建环境变化；签名不同的 APK 无法覆盖安装，卸载会删除应用数据。

**重要：0.1.3 在部分 Android 15+ 手机上启动即闪退的原因尚未知；当前 0.2.0 源码未在这些手机上验证，也不能保证解决。**建议保留旧版和相关数据，并提供脱敏后的系统崩溃堆栈供进一步排查。

这是开发用 debug 包，可调试且允许明文网络流量配置，不应用于正式发布。Gemini 音频连接本身使用加密 WSS。源码包不包含调试 keystore：自行在另一台电脑构建的 APK 通常会使用不同的签名，因而可能无法覆盖此前安装的版本。

源码包不包含生成目录 `src-tauri/gen/`、已下载的 SDK/NDK、`node_modules/` 或构建产物；这些要在本机生成。Actions 初始化工程后会运行 `scripts/configure-android-gradle.mjs`：删除已弃用且降低校验强度的 `android.proguard.failOnMissingFiles=false`，将生成应用的 `android {}` 迁移到公开 `ApplicationExtension`，并仅抑制 Tauri 2 当前仍需要的 AGP 兼容选项／旧 variant DSL 迁移提示，不会全局关闭 Gradle 或 Kotlin 警告。项目自己的 mobile-audio 插件已改用公开 `LibraryExtension` 和类型安全的 Kotlin `compilerOptions`。GitHub Actions 使用 JDK 25 运行 Gradle；初始化后会把 Tauri 模板自带的 Kotlin Gradle Plugin 2.2.10 升级到稳定版 2.4.21。AGP 9.3.1 的 classloader 仍会向构建脚本暴露不含 `JVM_25` 的 Kotlin 2.2 枚举，因此应用与插件使用局部弃用豁免的字符串兼容 API `kotlinOptions.jvmTarget = "25"`，避免链接不存在的静态字段，同时仍由 Kotlin 2.4 编译器输出 Java/Kotlin 25 字节码。生成的 Tauri Kotlin 文件只对其文件内的旧 Android 兼容调用使用 `DEPRECATION` 豁免，项目 Kotlin 代码的其他警告仍保持可见；JDK 25 编译 Tauri 的 Java 8 兼容模块所产生的 source/target 提示按 AGP 官方属性精确抑制。Cargo 依赖内 Tauri 自身的 Kotlin 源码仍可能报告上游生命周期 API／Java 类型提示，需等待 Tauri 模板升级，项目不会修改 Cargo registry 中的第三方源码来隐藏它。准备 Android Studio、JDK 25、SDK Platform-Tools、SDK 平台、Build Tools 和 NDK。使用本项目锁定的 Tauri CLI 初始化时，本次生成的工程使用 `compileSdk/targetSdk 37`，插件使用 `compileSdk 37`；建议在 SDK Manager 中安装 API 36、37 和相应 Build Tools，以及 NDK 28.2.13676358。Android 版本号由 `src-tauri/tauri.android.conf.json` 覆盖；版本或 Tauri 模板升级后以生成工程为准。

macOS / Linux 示例（按本机目录调整）：

```bash
export JAVA_HOME=/path/to/jdk-25
export ANDROID_HOME="$HOME/Android/Sdk"
export NDK_HOME="$ANDROID_HOME/ndk/28.2.13676358"
export PATH="$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$PATH"
# 可通过 Android Studio SDK Manager 安装，或：
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
  "platform-tools" "platforms;android-36" "platforms;android-37.0" \
  "build-tools;36.0.0" "build-tools;37.0.0" "ndk;28.2.13676358"
rustup target add aarch64-linux-android
npm ci
npx tauri android init --ci
npx tauri android build --debug --apk --target aarch64 --ci
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

Windows PowerShell 环境变量示例（版本和路径按实际修改，随后运行同样的 `rustup`、`npm`、`npx` 和 `adb` 命令）：

```powershell
$env:JAVA_HOME = "C:\Program Files\Android\Android Studio\jbr"
$env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
$env:NDK_HOME = "$env:ANDROID_HOME\ndk\28.2.13676358"
$env:PATH = "$env:JAVA_HOME\bin;$env:ANDROID_HOME\platform-tools;$env:PATH"
```

`--target aarch64` 只包含 ARM64，即使输出文件名含 `universal`；其他 ABI 需安装对应 Rust target 并指定相应构建目标。调试 APK 使用调试签名，**不应作为正式发布包**。正式发布需要自行配置版本号、持久保管的 release keystore、签名与 Play 政策材料；参见 [Tauri Android 签名文档](https://v2.tauri.app/distribute/sign/android/)。每次重新生成 `gen/android` 后，针对生成工程的手工签名配置需重新应用或脚本化。

## 测试与排障

```bash
npm run build
npm run test:ui              # JSDOM + 模拟 Tauri 事件，并非真实窗口测试
cd src-tauri
cargo test --lib --locked
```

0.1.4 已执行：前端构建和 JSDOM 控制台／字幕窗模拟交互测试通过；Linux Rust 测试 **30/30** 通过，包含两种 3.5 模型 setup、转写临时／最终分段、转写模式安全限制、模拟 WebSocket、文本与二进制 `setupComplete`、代理、取消、TLS 证书和错误脱敏。Windows x64 release 交叉编译和安装包载荷检查通过；Android ARM64 Rust、Kotlin/Gradle debug APK 已构建，签名、架构、版本和 16 KB ZIP 对齐已静态检查。这些只能验证本机代码、打包与模拟协议，不能替代真实设备及 Gemini 服务端联调；目标平台的系统代理/VPN、音频、窗口、系统授权、Android 剪贴板和 Gemini 端到端均**未验证**。Android 15+ 闪退问题仍待真实崩溃堆栈。

- **Android 15+ 安装后立即闪退**：用户已报告 0.1.3 问题，尚无堆栈和根因，当前 0.2.0 源码尚不能视为修复。如果只有手机，可先触发闪退，然后在系统「开发者选项」中生成完整错误报告（不同厂商菜单不同）。报告可能包含设备标识、账号及其他隐私信息；先自行检查并遮盖，只分享 `com.gemini.live.translator` 相关的 `FATAL EXCEPTION`、`Caused by`、`Fatal signal` 或 tombstone 附近堆栈。应用没打开时控制面板的连接诊断日志无法捕获启动前闪退。请勿发送 API Key、代理密码或完整未经检查的报告。
- **连接失败 / 模型不可用**：检查网络、API Key、所在区域、账户模型权限和当前 [Live API 文档](https://ai.google.dev/gemini-api/docs/live-api)。本项目没有自动切换模型。
- **自动代理未识别或不能连接**：确认代理软件确实开启了系统代理，或 VPN 的分应用规则包含本程序；查看面板的选定路径。PAC 检测失败可手动填 HTTP/SOCKS5 地址；手机上的 `127.0.0.1` 不是电脑。确认代理监听端口、本机防火墙、凭据和目标站点规则；如果证书失败请检查代理是否做 TLS 解密，不要关闭证书校验。
- **Android 构建提示找不到 `TauriActivity`**：若重建过 `gen/android` 或多个项目共用了 `CARGO_TARGET_DIR`，旧的 Tauri/Wry 缓存可能没有把 Kotlin 文件生成到新目录。运行 `cargo clean --manifest-path src-tauri/Cargo.toml -p tauri -p wry --target aarch64-linux-android`，然后重新执行 Android 构建；其他 ABI 改用对应 Rust target。
- **Android 系统音频无声**：确认 Android 10+、授权了本次投影、源应用允许 AudioPlaybackCapture、源音频用途允许捕获，并且源/捕获应用位于同一用户 profile。`CAPTURE_AUDIO_OUTPUT` 的声明不会突破这些限制。
- **系统授权弹窗被取消**：重新开始并完成授权；若停止发生在弹窗打开时，先关闭该弹窗再重试，以避免并发覆盖系统回调。
- **字幕有但听不到译音**：确认已开启译音、输出设备和系统音量；某些设备格式可能不支持。关闭译音不影响字幕。
- **macOS 看不到音频或字幕窗不透明**：检查系统授权并在真正的 `.app` 包中测试；ScreenCaptureKit 音频格式和透明窗口行为尚未经目标机验证。
- **桌面退出后只有字幕窗**：本版本关闭控制面板会退出应用；正常操作可在面板底部点击「退出应用」。

## 参考文档

- [Gemini Live API 与 WebSocket](https://ai.google.dev/gemini-api/docs/live-api/get-started-websocket)
- [Gemini 3.5 Live Translate](https://ai.google.dev/gemini-api/docs/live-api/live-translate)
- [Gemini 3.5 Transcribe Live](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe)
- [Gemini Live Extended Thinking 与会话状态](https://ai.google.dev/gemini-api/docs/live-api/thinking)
- [Android 系统错误报告](https://developer.android.com/studio/debug/bug-report)
- [Android 音频播放捕获](https://developer.android.com/media/platform/av-capture)
- [Android 权限参考](https://developer.android.com/reference/android/Manifest.permission)
- [Android ProxySelector](https://developer.android.com/reference/java/net/ProxySelector)
- [Windows WinHTTP PAC 解析](https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpgetproxyforurl)
- [proxyres 跨平台代理解析库](https://github.com/proxyres/proxyres-rs)
- [Tauri 2 Android 开发](https://v2.tauri.app/start/prerequisites/)
