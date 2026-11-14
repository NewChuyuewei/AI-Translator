//! Select a route without changing the machine's network settings. Proxy
//! credentials never enter logs, errors, the Gemini URL, or the TLS handshake.
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use std::{net::IpAddr, time::Duration};
use tauri::AppHandle;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, sync::watch};
use url::{Host, Url};

// PAC/ProxySelector expect the HTTPS form of the actual WebSocket destination;
// no API key or other secret is included in the URL supplied to the resolver.
pub const PROXY_TARGET: &str = "https://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode { #[default] Auto, Manual, Direct }

#[derive(Clone)]
struct Credentials { username: Vec<u8>, password: Vec<u8> }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { Http, Socks5 }
#[derive(Clone)]
struct Proxy { kind: Kind, host: String, port: u16, credentials: Option<Credentials> }
#[derive(Clone)]
enum Destination { Direct, Proxy(Proxy) }

#[derive(Clone)]
pub struct Route { destination: Destination, label: &'static str }
impl Route {
    fn direct(label: &'static str) -> Self { Self { destination: Destination::Direct, label } }
    fn via(proxy: Proxy, auto: bool) -> Self {
        let label = match (auto, proxy.kind) {
            (true, Kind::Http) => "系统/环境 HTTP 代理",
            (true, Kind::Socks5) => "系统/环境 SOCKS5 代理",
            (false, Kind::Http) => "手动 HTTP 代理",
            (false, Kind::Socks5) => "手动 SOCKS5 代理",
        };
        Self { destination: Destination::Proxy(proxy), label }
    }
    pub fn label(&self) -> &'static str { self.label }
}

fn normalized_host(host: Host<&str>) -> String {
    match host { Host::Domain(s) => s.to_owned(), Host::Ipv4(v) => v.to_string(), Host::Ipv6(v) => v.to_string() }
}
fn credentials(url: &Url) -> Result<Option<Credentials>> {
    if url.username().is_empty() && url.password().is_none() { return Ok(None); }
    let decode = |s: &str| -> Result<Vec<u8>> {
        let bytes = percent_decode_str(s).collect::<Vec<_>>();
        if bytes.len() > 255 || bytes.iter().any(|b| b.is_ascii_control()) {
            return Err(anyhow!("代理用户名或密码过长或含控制字符"));
        }
        Ok(bytes)
    };
    let username = decode(url.username())?;
    let password = decode(url.password().unwrap_or(""))?;
    if username.contains(&b':') { return Err(anyhow!("代理用户名不能包含冒号")); }
    Ok(Some(Credentials { username, password }))
}

// The supported proxy endpoint itself is plaintext HTTP CONNECT or SOCKS5;
// WSS (including the API key) remains inside a separate verified TLS tunnel.
fn parse_proxy(value: &str) -> Result<Proxy> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(anyhow!("请填写有效的代理地址"));
    }
    // Never include the original value or url::ParseError in an error message:
    // the URL may contain a username and password.
    let url = Url::parse(value).map_err(|_| anyhow!("代理地址格式无效；示例 http://127.0.0.1:7890"))?;
    let kind = match url.scheme() {
        "http" => Kind::Http,
        "socks" | "socks5" | "socks5h" => Kind::Socks5,
        "https" => return Err(anyhow!("暂不支持加密的 HTTPS 代理入口；请改用 HTTP CONNECT 或 SOCKS5 入口")),
        _ => return Err(anyhow!("只支持 http://、socks5:// 或 socks5h:// 代理地址")),
    };
    if url.host().is_none() || url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err(anyhow!("代理地址只能包含主机、端口及可选的用户名密码"));
    }
    let host = normalized_host(url.host().unwrap());
    let port = url.port_or_known_default().ok_or_else(|| anyhow!("代理地址需要端口"))?;
    if port == 0 { return Err(anyhow!("代理端口无效")); }
    let credentials = credentials(&url)?;
    if kind == Kind::Socks5 && credentials.as_ref().is_some_and(|c| c.username.is_empty()) {
        return Err(anyhow!("SOCKS5 用户名不能为空"));
    }
    Ok(Proxy { kind, host, port, credentials })
}

pub fn validate(mode: ProxyMode, address: &str) -> Result<()> {
    if matches!(mode, ProxyMode::Manual) { parse_proxy(address)?; }
    Ok(())
}

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

fn authority(host: &str, port: u16) -> String {
    if host.trim_matches(['[', ']']).parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{}]:{port}", host.trim_matches(['[', ']']))
    } else { format!("{host}:{port}") }
}

async fn http_tunnel(stream: &mut TcpStream, proxy: &Proxy, host: &str, port: u16) -> Result<()> {
    let target = authority(host, port);
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(c) = &proxy.credentials {
        let mut pair = c.username.clone(); pair.push(b':'); pair.extend_from_slice(&c.password);
        request.push_str(&format!("Proxy-Authorization: Basic {}\r\n", STANDARD.encode(pair)));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.context("向 HTTP 代理发送 CONNECT 失败")?;
    // Consume precisely the CONNECT header, not any subsequent tunnel bytes.
    // The bound prevents an untrusted proxy from exhausting memory.
    let mut header = Vec::with_capacity(512);
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 16 * 1024 { return Err(anyhow!("HTTP 代理响应头过长")); }
        header.push(stream.read_u8().await.context("读取 HTTP 代理响应失败")?);
    }
    let first_line = header.split(|b| *b == b'\n').next().unwrap_or_default();
    let first_line = std::str::from_utf8(first_line).map_err(|_| anyhow!("HTTP 代理响应无效"))?;
    let mut fields = first_line.split_ascii_whitespace();
    let version = fields.next().unwrap_or("");
    let code = fields.next().and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);
    if version != "HTTP/1.0" && version != "HTTP/1.1" { return Err(anyhow!("HTTP 代理响应无效")); }
    match code {
        200..=299 => Ok(()),
        407 => Err(anyhow!("HTTP 代理要求认证（407）；可在手动代理地址中填写用户名和密码")),
        100..=599 => Err(anyhow!("HTTP 代理拒绝连接（状态码 {code}）")),
        _ => Err(anyhow!("HTTP 代理响应无效")),
    }
}

async fn socks_tunnel(stream: &mut TcpStream, proxy: &Proxy, host: &str, port: u16) -> Result<()> {
    if proxy.credentials.is_some() { stream.write_all(&[5, 2, 2, 0]).await?; }
    else { stream.write_all(&[5, 1, 0]).await?; }
    let mut greeting = [0; 2]; stream.read_exact(&mut greeting).await.context("读取 SOCKS5 协商失败")?;
    if greeting[0] != 5 { return Err(anyhow!("代理不是 SOCKS5 服务")); }
    match greeting[1] {
        0 => {},
        2 => {
            let c = proxy.credentials.as_ref().ok_or_else(|| anyhow!("SOCKS5 代理要求用户名和密码；请使用手动代理"))?;
            if c.username.is_empty() || c.username.len() > 255 || c.password.len() > 255 {
                return Err(anyhow!("SOCKS5 用户名或密码长度无效"));
            }
            let mut auth = vec![1, c.username.len() as u8];
            auth.extend_from_slice(&c.username); auth.push(c.password.len() as u8); auth.extend_from_slice(&c.password);
            stream.write_all(&auth).await.context("SOCKS5 认证通信失败")?;
            let mut answer = [0; 2]; stream.read_exact(&mut answer).await.context("读取 SOCKS5 认证响应失败")?;
            if answer != [1, 0] { return Err(anyhow!("SOCKS5 代理认证失败")); }
        }
        0xff => return Err(anyhow!("SOCKS5 代理拒绝认证方式；请检查用户名和密码")),
        _ => return Err(anyhow!("SOCKS5 代理要求不支持的认证方式")),
    }
    let mut connect = vec![5, 1, 0];
    let unbracketed = host.trim_matches(['[', ']']);
    match unbracketed.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => { connect.push(1); connect.extend_from_slice(&ip.octets()); },
        Ok(IpAddr::V6(ip)) => { connect.push(4); connect.extend_from_slice(&ip.octets()); },
        Err(_) => {
            if unbracketed.is_empty() || unbracketed.len() > 255 || !unbracketed.is_ascii() {
                return Err(anyhow!("目标主机名无效"));
            }
            // Proxy resolves the target hostname, avoiding local DNS leakage.
            connect.push(3); connect.push(unbracketed.len() as u8); connect.extend_from_slice(unbracketed.as_bytes());
        },
    }
    connect.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&connect).await.context("向 SOCKS5 代理发送连接请求失败")?;
    let mut reply = [0; 4]; stream.read_exact(&mut reply).await.context("读取 SOCKS5 连接响应失败")?;
    if reply[0] != 5 || reply[2] != 0 { return Err(anyhow!("SOCKS5 连接响应无效")); }
    if reply[1] != 0 { return Err(anyhow!("SOCKS5 代理拒绝目标连接（代码 {}）", reply[1])); }
    let size = match reply[3] { 1 => 4, 4 => 16, 3 => stream.read_u8().await? as usize, _ => return Err(anyhow!("SOCKS5 响应地址类型无效")) };
    let mut bound = vec![0; size + 2]; stream.read_exact(&mut bound).await.context("读取 SOCKS5 响应地址失败")?;
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
        assert_eq!(authority("::1", 443), "[::1]:443");
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
