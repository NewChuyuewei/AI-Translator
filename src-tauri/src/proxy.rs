//! Select a route without changing the machine's network settings. Proxy
//! credentials never enter logs, errors, the Gemini URL, or the TLS handshake.
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::{net::IpAddr, time::Duration};
use tauri::AppHandle;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, sync::watch};
use url::Host;
pub use translator_core::ProxyMode;
use translator_core::{
    proxy::{ProxyEndpoint as Proxy, ProxyKind as Kind},
    proxy_handshake::{
        SocksAuthMethod, SocksReplyAddress, http_connect_request, parse_http_connect_response,
        parse_socks_auth_response, parse_socks_method, parse_socks_reply_prefix,
        socks_auth_request, socks_connect_request, socks_greeting, socks_reply_tail_len,
        MAX_HTTP_CONNECT_HEADER_BYTES,
    },
};

// PAC/ProxySelector expect the HTTPS form of the actual WebSocket destination;
// no API key or other secret is included in the URL supplied to the resolver.
pub const PROXY_TARGET: &str = "https://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

#[derive(Clone)]
enum Destination { Direct, Proxy(Proxy) }

#[derive(Clone)]
pub struct Route { destination: Destination, label: &'static str }
impl Route {
    fn direct(label: &'static str) -> Self { Self { destination: Destination::Direct, label } }
    fn via(proxy: Proxy, automatic: bool) -> Self {
        let label = proxy.safe_label(automatic);
        Self { destination: Destination::Proxy(proxy), label }
    }
    pub fn label(&self) -> &'static str { self.label }
}

fn parse_proxy(value: &str) -> Result<Proxy> {
    Proxy::parse(value)
}

pub fn validate(mode: ProxyMode, address: &str) -> Result<()> {
    if matches!(mode, ProxyMode::Manual) { parse_proxy(address)?; }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn from_resolver_value(value: &str) -> Result<Route> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("direct://") || value.eq_ignore_ascii_case("direct:///") {
        return Ok(Route::direct("系统网络（未检测到显式代理；可能经过 VPN）"));
    }
    Ok(Route::via(parse_proxy(value).context("系统代理类型或地址无法使用；请尝试手动代理")?, true))
}

#[cfg(not(target_os = "android"))]
fn desktop_proxy() -> Result<Route> {
    let mut resolver = proxyres::resolver::Resolver::new();
    resolver.get_proxies_for_url(PROXY_TARGET)
        .map_err(|_| anyhow!("无法读取系统代理；请检查系统代理设置或选择手动代理"))?;
    if !resolver.wait(Some(Duration::from_secs(10))) {
        return Err(anyhow!("系统代理/PAC 检测超时；可选择手动代理或系统网络模式"));
    }
    if resolver.error().is_some() {
        return Err(anyhow!("系统代理/PAC 检测失败；请检查代理设置或选择手动代理"));
    }
    // Use only the first OS-selected route. Do not silently fall back to a
    // later DIRECT entry if an explicitly selected proxy is unreachable.
    let selected = resolver.next_proxy().ok_or_else(|| anyhow!("系统未返回可用的代理路径；请尝试手动代理"))?;
    from_resolver_value(&selected)
}

#[cfg(any(target_os = "android", test))]
#[derive(Deserialize)]
struct AndroidProxy { kind: String, host: Option<String>, port: Option<u16> }

#[cfg(any(target_os = "android", test))]
fn android_proxy(value: serde_json::Value) -> Result<Route> {
    let selected: AndroidProxy = serde_json::from_value(value).map_err(|_| anyhow!("Android 系统代理信息无效；请尝试手动代理"))?;
    if selected.kind == "direct" { return Ok(Route::direct("系统网络（未检测到显式代理；可能经过 VPN）")); }
    let kind = match selected.kind.as_str() {
        "http" => Kind::Http,
        "socks5" => Kind::Socks5,
        _ => return Err(anyhow!("Android 返回了不支持的系统代理类型；请尝试手动代理")),
    };
    let host = selected.host.ok_or_else(|| anyhow!("Android 系统代理缺少主机"))?;
    // Host::parse requires brackets for IPv6. Do not perform DNS resolution.
    let parsed = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => ip.to_string(),
        Err(_) => match Host::parse(&host) {
            Ok(Host::Domain(domain)) if !domain.is_empty() => domain,
            Ok(Host::Ipv4(ip)) => ip.to_string(),
            Ok(Host::Ipv6(ip)) => ip.to_string(),
            _ => return Err(anyhow!("Android 系统代理主机无效")),
        },
    };
    let port = selected.port.filter(|p| *p > 0).ok_or_else(|| anyhow!("Android 系统代理端口无效"))?;
    Ok(Route::via(Proxy { kind, host: parsed, port, credentials: None }, true))
}

pub async fn resolve(app: &AppHandle, mode: ProxyMode, address: &str, cancel: &mut watch::Receiver<bool>) -> Result<Route> {
    match mode {
        ProxyMode::Direct => Ok(Route::direct("系统网络（不使用显式代理；仍可能经过 VPN）")),
        ProxyMode::Manual => Ok(Route::via(parse_proxy(address)?, false)),
        ProxyMode::Auto => {
            #[cfg(not(target_os = "android"))]
            {
                let _ = app;
                // Platform proxy APIs and PAC can block; keep them off the async
                // runtime, with a bound and a cancellable session. Resolver Drop
                // requests cancellation if its own ten-second wait expires.
                let worker = tokio::task::spawn_blocking(desktop_proxy);
                tokio::select! {
                    result = tokio::time::timeout(Duration::from_secs(12), worker) => {
                        result.context("系统代理检测超时；请尝试手动代理")?.map_err(|_| anyhow!("系统代理检测任务失败"))?
                    },
                    _ = crate::gemini_client::cancelled(cancel) => Err(anyhow!("已停止")),
                }
            }
            #[cfg(target_os = "android")]
            {
                tokio::select! {
                    result = tokio::time::timeout(Duration::from_secs(12), tauri_plugin_mobile_audio::system_proxy(app, PROXY_TARGET)) => {
                        let value = result.context("Android 系统代理检测超时；请尝试手动代理")?
                            .map_err(|_| anyhow!("无法读取 Android 系统代理；请尝试手动代理"))?;
                        android_proxy(value)
                    },
                    _ = crate::gemini_client::cancelled(cancel) => Err(anyhow!("已停止")),
                }
            }
        }
    }
}

async fn http_tunnel(stream: &mut TcpStream, proxy: &Proxy, host: &str, port: u16) -> Result<()> {
    let request = http_connect_request(proxy, host, port)?;
    stream.write_all(request.as_slice()).await.context("向 HTTP 代理发送 CONNECT 失败")?;
    let mut header = Vec::with_capacity(512);
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= MAX_HTTP_CONNECT_HEADER_BYTES { return Err(anyhow!("HTTP 代理响应头过长")); }
        header.push(stream.read_u8().await.context("读取 HTTP 代理响应失败")?);
    }
    parse_http_connect_response(&header)
}

async fn socks_tunnel(stream: &mut TcpStream, proxy: &Proxy, host: &str, port: u16) -> Result<()> {
    stream.write_all(socks_greeting(proxy.credentials.is_some())).await?;
    let mut greeting = [0; 2];
    stream.read_exact(&mut greeting).await.context("读取 SOCKS5 协商失败")?;
    if parse_socks_method(greeting, proxy.credentials.is_some())? == SocksAuthMethod::UsernamePassword {
        let credentials = proxy.credentials.as_ref().ok_or_else(|| anyhow!("SOCKS5 代理要求用户名和密码；请使用手动代理"))?;
        let request = socks_auth_request(credentials)?;
        stream.write_all(request.as_slice()).await.context("SOCKS5 认证通信失败")?;
        let mut answer = [0; 2];
        stream.read_exact(&mut answer).await.context("读取 SOCKS5 认证响应失败")?;
        parse_socks_auth_response(answer)?;
    }
    let connect = socks_connect_request(host, port)?;
    stream.write_all(&connect).await.context("向 SOCKS5 代理发送连接请求失败")?;
    let mut reply = [0; 4];
    stream.read_exact(&mut reply).await.context("读取 SOCKS5 连接响应失败")?;
    let address = parse_socks_reply_prefix(reply)?;
    let domain_length = if address == SocksReplyAddress::DomainLengthPending {
        Some(stream.read_u8().await.context("读取 SOCKS5 响应域名长度失败")?)
    } else { None };
    let mut bound = vec![0; socks_reply_tail_len(address, domain_length)?];
    stream.read_exact(&mut bound).await.context("读取 SOCKS5 响应地址失败")?;
    Ok(())
}

/// Opens the selected TCP route. No direct fallback occurs on proxy failure.
pub async fn open(route: &Route, host: &str, port: u16) -> Result<TcpStream> {
    match &route.destination {
        Destination::Direct => TcpStream::connect((host.trim_matches(['[', ']']), port)).await
            .context("连接目标失败；请检查 VPN 或选择手动代理"),
        Destination::Proxy(proxy) => {
            let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port)).await
                .context("无法连接所选代理；请确认代理软件已启动、地址和端口正确")?;
            match proxy.kind {
                Kind::Http => http_tunnel(&mut stream, proxy, host, port).await?,
                Kind::Socks5 => socks_tunnel(&mut stream, proxy, host, port).await?,
            }
            Ok(stream)
        }
    }
}

#[cfg(test)]
pub(crate) fn direct_for_test() -> Route { Route::direct("test") }
#[cfg(test)]
pub(crate) fn manual_for_test(url: &str) -> Result<Route> { Ok(Route::via(parse_proxy(url)?, false)) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parses_manual_proxy_and_does_not_echo_secrets() {
        let http = parse_proxy("http://alice:p%40ss%3Aword@127.0.0.1:7890").unwrap();
        assert!(http.kind == Kind::Http); assert_eq!(http.host, "127.0.0.1"); assert_eq!(http.port, 7890);
        let cred = http.credentials.unwrap(); assert_eq!(cred.username, b"alice"); assert_eq!(cred.password, b"p@ss:word");
        let socks = parse_proxy("socks5h://[::1]:1080").unwrap();
        assert!(socks.kind == Kind::Socks5); assert_eq!(socks.host, "::1");
        for invalid in ["http://alice:secret@example.com:123/path", "https://alice:secret@example.com:444", "socks5://alice:secret@example.com", "nope://alice:secret@example.com:1", "http://alice:secret@example.com:80/?q=secret"] {
            let error = parse_proxy(invalid).err().unwrap().to_string();
            assert!(!error.contains("secret")); assert!(!error.contains("alice"));
        }
    }
    #[test] fn auto_direct_or_supported_routes_only() {
        assert!(matches!(from_resolver_value("direct://").unwrap().destination, Destination::Direct));
        assert!(matches!(from_resolver_value("socks://127.0.0.1:1080").unwrap().destination, Destination::Proxy(_)));
        assert!(from_resolver_value("https://127.0.0.1:9999").is_err());
    }
    #[test] fn android_selector_results_are_checked_without_dns() {
        use serde_json::json;
        assert!(matches!(android_proxy(json!({"kind":"direct"})).unwrap().destination, Destination::Direct));
        let route = android_proxy(json!({"kind":"http","host":"[::1]","port":7890})).unwrap();
        match route.destination { Destination::Proxy(p) => { assert!(p.kind == Kind::Http); assert_eq!(p.host, "::1"); assert_eq!(p.port, 7890); }, _ => panic!("expected proxy") }
        let route = android_proxy(json!({"kind":"socks5","host":"proxy.example","port":1080})).unwrap();
        match route.destination { Destination::Proxy(p) => assert!(p.kind == Kind::Socks5), _ => panic!("expected proxy") }
        assert!(android_proxy(json!({"kind":"http","host":"proxy.example","port":0})).is_err());
        assert!(android_proxy(json!({"kind":"unsupported","host":"proxy.example","port":8080})).is_err());
        assert!(android_proxy(json!({"kind":"http","host":"bad/path","port":8080})).is_err());
    }

    #[tokio::test]
    async fn socks5_uses_remote_dns_and_username_password_authentication() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 4]; conn.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 2, 2, 0]); conn.write_all(&[5, 2]).await.unwrap();
            assert_eq!(conn.read_u8().await.unwrap(), 1);
            let ulen = conn.read_u8().await.unwrap() as usize;
            let mut u = vec![0; ulen]; conn.read_exact(&mut u).await.unwrap();
            let plen = conn.read_u8().await.unwrap() as usize;
            let mut p = vec![0; plen]; conn.read_exact(&mut p).await.unwrap();
            assert_eq!(u, b"alice"); assert_eq!(p, b"s@cret"); conn.write_all(&[1, 0]).await.unwrap();
            let mut head = [0; 4]; conn.read_exact(&mut head).await.unwrap();
            assert_eq!(head, [5, 1, 0, 3]); // domain, not locally resolved IP
            let len = conn.read_u8().await.unwrap() as usize;
            let mut name = vec![0; len]; conn.read_exact(&mut name).await.unwrap();
            assert_eq!(name, b"must-resolve-at-proxy.invalid");
            assert_eq!(conn.read_u16().await.unwrap(), 443);
            conn.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 1]).await.unwrap();
        });
        let route = manual_for_test(&format!("socks5://alice:s%40cret@{address}")).unwrap();
        let _tcp = open(&route, "must-resolve-at-proxy.invalid", 443).await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn failed_http_proxy_does_not_fall_back_or_echo_credentials() {
        use tokio::net::TcpListener;
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target.local_addr().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") { header.push(conn.read_u8().await.unwrap()); }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with(&format!("CONNECT {target_addr} HTTP/1.1\r\n")));
            assert!(header.contains("Proxy-Authorization: Basic "));
            assert!(!header.contains("x-goog-api-key"));
            conn.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        });
        let route = manual_for_test(&format!("http://alice:secret@{proxy_addr}")).unwrap();
        let error = open(&route, &target_addr.ip().to_string(), target_addr.port()).await.err().unwrap();
        let text = format!("{error:#}"); assert!(text.contains("407"));
        assert!(!text.contains("alice") && !text.contains("secret"));
        assert!(tokio::time::timeout(Duration::from_millis(100), target.accept()).await.is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_no_auth_accepts_ipv6_reply() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3]; conn.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]); conn.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 22]; conn.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..4], &[5, 1, 0, 4]);
            assert_eq!(&request[20..], &443u16.to_be_bytes());
            let mut reply = vec![5, 0, 0, 4]; reply.extend_from_slice(&[0; 16]); reply.extend_from_slice(&[0, 0]);
            conn.write_all(&reply).await.unwrap();
        });
        let route = manual_for_test(&format!("socks5://{addr}")).unwrap();
        let _tcp = open(&route, "::1", 443).await.unwrap();
        server.await.unwrap();
    }
}
