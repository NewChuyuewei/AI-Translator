Gemini Live Translator 1.1.0 · Windows 64 位免安装版
Copyright © 2026 chuyuewei. All rights reserved. 版权所有，保留所有权利。

1. 将压缩包中的所有文件解压到同一文件夹，不要仅取出 .exe。
2. 双击 Gemini-Live-Translator.exe。若压缩包内含 WebView2Loader.dll，请务必将它与 EXE 保持在同一文件夹。
3. 需要 Windows 10/11 x64 和 Microsoft Edge WebView2 Runtime。若缺少 WebView2，请从 Microsoft 官网安装：
   https://developer.microsoft.com/microsoft-edge/webview2/
4. 先启动已有代理软件并启用系统代理，或启动覆盖本程序的 VPN。网络连接默认“自动”；未检测到时可选手动代理并输入本机 HTTP 或 SOCKS5 地址，例如 http://127.0.0.1:7890。实际端口请查看你的代理软件。本程序不提供代理节点，也不修改全局设置。
5. 系统网络路径可能经过 VPN，也可能直连，应用无法验证 VPN 是否生效。已选代理连接失败时不会自动绕过；PAC 规则本身也可能选择直连。手动代理认证信息到代理的传输未加密，只应使用可信代理。
6. 填写可使用 Gemini Live 的 API Key，选择模型和音源。3.8 两种模型可启用依靠模型判断的双向翻译；Extended Thinking 支持低、中、高推理深度。3.5 Live Translate 每次固定一个翻译目标，不支持双向。3.5 Transcribe Live 仅显示识别原文，不翻译或播放译音；可选择逐字或智能整理，会话最长 10 分钟，断开后手动重启。
7. 翻译模式默认关闭译音；采集系统音频时若播放译音到同一设备，可能产生数字反馈。
8. 连接失败时，在控制面板找到“连接诊断控制台”，先清空、重新尝试，然后点击“复制日志”。日志仅记录连接阶段、消息类型和安全错误代码，不记录密钥、代理凭据、音频或字幕；分享前仍请检查，不要发送密钥或代理密码。

程序支持文本和二进制 UTF-8 JSON 的 setupComplete；网络和服务权限问题仍可能导致连接失败。本程序是未经签名的开发原型，可能触发 Windows 安全提示。未在 Windows 真机或真实 Gemini API 上测试；代理、VPN、音频、字幕窗及模型均需自行验证。音频会发送到 Google Gemini；API Key 和手动代理地址默认仅保留在进程内存中，只有用户明确启用并提供密码后才会写入本地 AES-256-GCM 加密保险库。
