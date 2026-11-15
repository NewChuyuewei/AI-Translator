//! Native Gemini transport probe. It validates routing, TLS, WebSocket and
//! setup ordering, then closes without opening audio devices or sending PCM.

use anyhow::{Context, Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_tungstenite::{
    client_async_tls_with_config,
    tungstenite::{
        Error as WsError, Message, client::IntoClientRequest, http::HeaderValue,
        protocol::WebSocketConfig,
    },
};
use translator_core::{
    ProxyMode, SessionOptions,
    protocol::LiveProtocolConfig,
    proxy::{ProxyDestination, ProxyRoute},
    proxy_handshake::{
        MAX_HTTP_CONNECT_HEADER_BYTES, SocksAuthMethod, SocksReplyAddress, http_connect_request,
        parse_http_connect_response, parse_socks_auth_response, parse_socks_method,
        parse_socks_reply_prefix, socks_auth_request, socks_connect_request, socks_greeting,
        socks_reply_tail_len,
    },
    server_protocol::{DataFrameKind, MAX_SERVER_MESSAGE_BYTES},
    session_driver::{DriverEvent, SessionDriver},
};

const ENDPOINT: &str = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";
const PROXY_TARGET: &str = "https://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

pub struct ProbeReport {
    pub route_label: &'static str,
    pub elapsed_ms: u128,
}

pub type ProbeCancel = Arc<AtomicBool>;

pub fn probe_blocking(options: SessionOptions, cancel: ProbeCancel) -> Result<ProbeReport> {
    options.validate()?;
    let route = resolve_route(options.proxy_mode, &options.proxy_url, &cancel)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("无法启动原生网络运行时")?;
    runtime.block_on(probe_endpoint(options, route, ENDPOINT, cancel))
}

fn resolve_route(mode: ProxyMode, address: &str, cancel: &ProbeCancel) -> Result<ProxyRoute> {
    if cancel.load(Ordering::Acquire) {
        return Err(anyhow!("网络验证已取消"));
    }
    if let Some(route) = ProxyRoute::immediate(mode, address)? {
        return Ok(route);
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(resolve_automatic_route());
    });
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(anyhow!("网络验证已取消"));
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow!("系统代理检测任务失败"));
            }
        }
    }
}

fn resolve_automatic_route() -> Result<ProxyRoute> {
    let mut resolver = proxyres::resolver::Resolver::new();
    resolver
        .get_proxies_for_url(PROXY_TARGET)
        .map_err(|_| anyhow!("无法读取系统代理；请检查系统代理设置或选择手动代理"))?;
    if !resolver.wait(Some(Duration::from_secs(10))) {
        return Err(anyhow!(
            "系统代理/PAC 检测超时；可选择手动代理或系统网络模式"
        ));
    }
    if resolver.error().is_some() {
        return Err(anyhow!(
            "系统代理/PAC 检测失败；请检查代理设置或选择手动代理"
        ));
    }
    let selected = resolver
        .next_proxy()
        .ok_or_else(|| anyhow!("系统未返回可用的代理路径；请尝试手动代理"))?;
    ProxyRoute::automatic_resolver_value(&selected)
}

async fn probe_endpoint(
    options: SessionOptions,
    route: ProxyRoute,
    endpoint: &str,
    cancel: ProbeCancel,
) -> Result<ProbeReport> {
    let started = Instant::now();
    let mut request = endpoint
        .into_client_request()
        .context("无效的 WebSocket 地址")?;
    request.headers_mut().insert(
        "x-goog-api-key",
        HeaderValue::from_str(options.api_key.trim()).context("API Key 中含无效字符")?,
    );
    let host = request
        .uri()
        .host()
        .ok_or_else(|| anyhow!("WebSocket 目标缺少主机"))?
        .to_owned();
    let port = request.uri().port_u16().unwrap_or(443);
    let route_label = route.label();

    let handshake = async {
        let tcp = open_route(&route, &host, port).await?;
        let mut websocket = WebSocketConfig::default();
        websocket.max_message_size = Some(MAX_SERVER_MESSAGE_BYTES);
        websocket.max_frame_size = Some(MAX_SERVER_MESSAGE_BYTES);
        client_async_tls_with_config(request, tcp, Some(websocket), None)
            .await
            .map_err(|error| {
                anyhow!(
                    "Gemini WebSocket/TLS 握手失败：{}",
                    describe_ws_error(&error)
                )
            })
    };
    let (mut socket, _) = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(25), handshake) => {
            result.map_err(|_| anyhow!("连接 Gemini 超时；请检查网络和代理"))??
        }
        () = cancelled(&cancel) => return Err(anyhow!("网络验证已取消")),
    };

    let config = LiveProtocolConfig {
        model: options.model,
        thinking_level: options.thinking_level,
        transcription_mode: options.transcription_mode,
        target_language: options.target_language.clone(),
        bidirectional: options.bidirectional,
        other_language: options.other_language.clone(),
    };
    let mut driver = SessionDriver::new(&config);
    let setup = driver.setup_message().to_owned();
    tokio::select! {
        result = tokio::time::timeout(
            Duration::from_secs(10),
            socket.send(Message::Text(setup.into())),
        ) => {
            result
                .map_err(|_| anyhow!("发送 setup 超时"))?
                .map_err(|error| anyhow!("发送 setup 失败：{}", describe_ws_error(&error)))?;
        }
        () = cancelled(&cancel) => return Err(anyhow!("网络验证已取消")),
    }

    let setup_wait = async {
        while let Some(message) = socket.next().await {
            let message = message.map_err(|error| {
                anyhow!(
                    "等待 setupComplete 时连接断开：{}",
                    describe_ws_error(&error)
                )
            })?;
            let event = match message {
                Message::Text(text) => driver.receive(DataFrameKind::Text, text.as_bytes())?,
                Message::Binary(bytes) => driver.receive(DataFrameKind::Binary, &bytes)?,
                Message::Close(_) => return Err(anyhow!("Gemini 在 setupComplete 前关闭了连接")),
                Message::Ping(value) => {
                    socket.send(Message::Pong(value)).await.map_err(|error| {
                        anyhow!("回复 WebSocket Ping 失败：{}", describe_ws_error(&error))
                    })?;
                    continue;
                }
                Message::Pong(_) | Message::Frame(_) => continue,
            };
            if matches!(event, DriverEvent::SetupComplete) {
                return Ok(());
            }
        }
        Err(anyhow!("Gemini 在 setupComplete 前关闭了连接"))
    };
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(25), setup_wait) => {
            result.map_err(|_| anyhow!("等待 Gemini setupComplete 超时"))??;
        }
        () = cancelled(&cancel) => return Err(anyhow!("网络验证已取消")),
    }

    let end = driver.close()?;
    socket
        .send(Message::Text(end.into()))
        .await
        .map_err(|error| anyhow!("发送流结束失败：{}", describe_ws_error(&error)))?;
    let _ = socket.close(None).await;
    Ok(ProbeReport {
        route_label,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

async fn cancelled(cancel: &ProbeCancel) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn open_route(route: &ProxyRoute, host: &str, port: u16) -> Result<TcpStream> {
    match &route.destination {
        ProxyDestination::Direct => TcpStream::connect((host.trim_matches(['[', ']']), port))
            .await
            .context("连接目标失败；请检查 VPN 或选择手动代理"),
        ProxyDestination::Proxy(proxy) => {
            let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port))
                .await
                .context("无法连接所选代理；请确认代理软件已启动、地址和端口正确")?;
            match proxy.kind {
                translator_core::proxy::ProxyKind::Http => {
                    let request = http_connect_request(proxy, host, port)?;
                    stream
                        .write_all(request.as_slice())
                        .await
                        .context("向 HTTP 代理发送 CONNECT 失败")?;
                    let mut header = Vec::with_capacity(512);
                    while !header.ends_with(b"\r\n\r\n") {
                        if header.len() >= MAX_HTTP_CONNECT_HEADER_BYTES {
                            return Err(anyhow!("HTTP 代理响应头过长"));
                        }
                        header.push(stream.read_u8().await.context("读取 HTTP 代理响应失败")?);
                    }
                    parse_http_connect_response(&header)?;
                }
                translator_core::proxy::ProxyKind::Socks5 => {
                    stream
                        .write_all(socks_greeting(proxy.credentials.is_some()))
                        .await?;
                    let mut greeting = [0_u8; 2];
                    stream.read_exact(&mut greeting).await?;
                    if parse_socks_method(greeting, proxy.credentials.is_some())?
                        == SocksAuthMethod::UsernamePassword
                    {
                        let credentials = proxy.credentials.as_ref().ok_or_else(|| {
                            anyhow!("SOCKS5 代理要求用户名和密码；请使用手动代理")
                        })?;
                        let request = socks_auth_request(credentials)?;
                        stream.write_all(request.as_slice()).await?;
                        let mut response = [0_u8; 2];
                        stream.read_exact(&mut response).await?;
                        parse_socks_auth_response(response)?;
                    }
                    stream
                        .write_all(&socks_connect_request(host, port)?)
                        .await?;
                    let mut prefix = [0_u8; 4];
                    stream.read_exact(&mut prefix).await?;
                    let address = parse_socks_reply_prefix(prefix)?;
                    let domain_length = if address == SocksReplyAddress::DomainLengthPending {
                        Some(stream.read_u8().await?)
                    } else {
                        None
                    };
                    let mut tail = vec![0_u8; socks_reply_tail_len(address, domain_length)?];
                    stream.read_exact(&mut tail).await?;
                }
            }
            Ok(stream)
        }
    }
}

fn describe_ws_error(error: &WsError) -> String {
    match error {
        WsError::Http(response) => format!("HTTP 状态码 {}", response.status().as_u16()),
        WsError::Tls(_) => "TLS 握手失败；请检查证书和代理".into(),
        WsError::Io(error) => match error.raw_os_error() {
            Some(code) => format!("网络 I/O 错误（{:?}，系统代码 {code}）", error.kind()),
            None => format!("网络 I/O 错误（{:?}）", error.kind()),
        },
        WsError::Protocol(_) => "WebSocket 协议异常".into(),
        WsError::Capacity(_) => "WebSocket 消息超过大小限制".into(),
        WsError::Utf8 => "WebSocket 文本不是有效 UTF-8".into(),
        WsError::ConnectionClosed | WsError::AlreadyClosed => "WebSocket 已关闭".into(),
        WsError::WriteBufferFull(_) => "WebSocket 发送缓冲已满".into(),
        WsError::AttackAttempt => "WebSocket 检测到无效握手".into(),
        WsError::Url(_) => "WebSocket 地址无效".into(),
        WsError::HttpFormat(_) => "HTTP 握手格式无效".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::Mutex;
    use translator_core::{AudioSourceMode, ModelKind, ThinkingLevel, TranscriptionMode};

    fn options() -> SessionOptions {
        SessionOptions {
            api_key: "PRIVATE-PROBE-KEY".into(),
            model: ModelKind::GeneralLive,
            target_language: "it".into(),
            thinking_level: ThinkingLevel::Low,
            transcription_mode: TranscriptionMode::Verbatim,
            bidirectional: false,
            other_language: "en".into(),
            mode: AudioSourceMode::MicrophoneOnly,
            microphone_device: None,
            system_device: None,
            playback_enabled: false,
            playback_device: None,
            token_saver: true,
            proxy_mode: ProxyMode::Direct,
            proxy_url: String::new(),
            overlay_scale: 1.0,
        }
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)]
    async fn swr_ntrans_001_probe_sends_header_setup_and_end_but_no_pcm() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let captured = Arc::new(Mutex::new(Vec::<String>::new()));
        let server_capture = Arc::clone(&captured);
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept fixture");
            let mut socket = tokio_tungstenite::accept_hdr_async(
                tcp,
                move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                      response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                    let key = request
                        .headers()
                        .get("x-goog-api-key")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    server_capture
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(key);
                    Ok(response)
                },
            )
            .await
            .expect("WebSocket fixture");
            let setup = socket
                .next()
                .await
                .expect("setup frame")
                .expect("setup message");
            let Message::Text(setup) = setup else {
                panic!("expected setup text")
            };
            assert!(setup.contains("\"setup\""));
            assert!(!setup.contains("PRIVATE-PROBE-KEY"));
            socket
                .send(Message::Text(r#"{"setupComplete":{}}"#.into()))
                .await
                .expect("setup complete");
            let end = socket
                .next()
                .await
                .expect("end frame")
                .expect("end message");
            let Message::Text(end) = end else {
                panic!("expected end text")
            };
            assert_eq!(end.as_str(), r#"{"realtimeInput":{"audioStreamEnd":true}}"#);
        });

        let report = probe_endpoint(
            options(),
            ProxyRoute::explicit_direct(),
            &format!("ws://{address}"),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect("probe fixture");
        assert_eq!(
            report.route_label,
            "系统网络（不使用显式代理；仍可能经过 VPN）"
        );
        server.await.expect("server fixture");
        assert_eq!(
            captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            &["PRIVATE-PROBE-KEY"]
        );
    }

    #[tokio::test]
    async fn swr_ntrans_005_setup_wait_is_cancellable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept fixture");
            let mut socket = tokio_tungstenite::accept_async(tcp)
                .await
                .expect("WebSocket fixture");
            let _ = socket.next().await;
            let _ = socket.next().await;
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&cancel);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger.store(true, Ordering::Release);
        });
        let error = probe_endpoint(
            options(),
            ProxyRoute::explicit_direct(),
            &format!("ws://{address}"),
            cancel,
        )
        .await
        .expect_err("probe must cancel")
        .to_string();
        assert_eq!(error, "网络验证已取消");
        server.await.expect("server fixture");
    }
}
