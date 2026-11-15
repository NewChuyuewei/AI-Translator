//! Pure HTTP CONNECT and SOCKS5 handshake construction/parsing.
//! Network I/O, timeouts and cancellation remain adapter responsibilities.

use crate::proxy::{ProxyCredentials, ProxyEndpoint, ProxyKind};
use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::net::{IpAddr, Ipv6Addr};
use zeroize::Zeroize;

pub const MAX_HTTP_CONNECT_HEADER_BYTES: usize = 16 * 1024;

pub struct SensitiveBytes(Vec<u8>);

impl SensitiveBytes {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocksAuthMethod {
    NoAuthentication,
    UsernamePassword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocksReplyAddress {
    Ipv4,
    Ipv6,
    DomainLengthPending,
}

/// SWR-TUN-001: build a bounded CONNECT request while keeping credentials in
/// a zeroizing buffer owned by the transport until write completion.
pub fn http_connect_request(
    proxy: &ProxyEndpoint,
    target_host: &str,
    target_port: u16,
) -> Result<SensitiveBytes> {
    if proxy.kind != ProxyKind::Http {
        return Err(anyhow!("代理握手类型无效"));
    }
    let target = authority(target_host, target_port)?;
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n").into_bytes();
    if let Some(credentials) = &proxy.credentials {
        let mut pair = credentials.username.clone();
        pair.push(b':');
        pair.extend_from_slice(&credentials.password);
        let encoded = STANDARD.encode(&pair);
        pair.zeroize();
        request.extend_from_slice(b"Proxy-Authorization: Basic ");
        request.extend_from_slice(encoded.as_bytes());
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    if request.len() > MAX_HTTP_CONNECT_HEADER_BYTES {
        request.zeroize();
        return Err(anyhow!("HTTP 代理请求头过长"));
    }
    Ok(SensitiveBytes(request))
}

/// SWR-TUN-002: parse only a complete, bounded response header and expose at
/// most an allowlisted status code.
pub fn parse_http_connect_response(header: &[u8]) -> Result<()> {
    if header.len() > MAX_HTTP_CONNECT_HEADER_BYTES || !header.ends_with(b"\r\n\r\n") {
        return Err(anyhow!("HTTP 代理响应无效"));
    }
    let first_line = header
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let first_line = std::str::from_utf8(first_line).map_err(|_| anyhow!("HTTP 代理响应无效"))?;
    let mut fields = first_line.split_ascii_whitespace();
    let version = fields.next().unwrap_or("");
    let code = fields
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(anyhow!("HTTP 代理响应无效"));
    }
    match code {
        200..=299 => Ok(()),
        407 => Err(anyhow!(
            "HTTP 代理要求认证（407）；可在手动代理地址中填写用户名和密码"
        )),
        100..=599 => Err(anyhow!("HTTP 代理拒绝连接（状态码 {code}）")),
        _ => Err(anyhow!("HTTP 代理响应无效")),
    }
}

pub fn socks_greeting(has_credentials: bool) -> &'static [u8] {
    if has_credentials {
        &[5, 2, 2, 0]
    } else {
        &[5, 1, 0]
    }
}

pub fn parse_socks_method(response: [u8; 2], has_credentials: bool) -> Result<SocksAuthMethod> {
    if response[0] != 5 {
        return Err(anyhow!("代理不是 SOCKS5 服务"));
    }
    match response[1] {
        0 => Ok(SocksAuthMethod::NoAuthentication),
        2 if has_credentials => Ok(SocksAuthMethod::UsernamePassword),
        2 => Err(anyhow!("SOCKS5 代理要求用户名和密码；请使用手动代理")),
        0xff => Err(anyhow!("SOCKS5 代理拒绝认证方式；请检查用户名和密码")),
        _ => Err(anyhow!("SOCKS5 代理要求不支持的认证方式")),
    }
}

/// SWR-TUN-003: RFC 1929 credentials remain in a zeroizing write buffer.
pub fn socks_auth_request(credentials: &ProxyCredentials) -> Result<SensitiveBytes> {
    if credentials.username.is_empty()
        || credentials.username.len() > 255
        || credentials.password.len() > 255
    {
        return Err(anyhow!("SOCKS5 用户名或密码长度无效"));
    }
    let mut request =
        Vec::with_capacity(credentials.username.len() + credentials.password.len() + 3);
    request.extend_from_slice(&[1, credentials.username.len() as u8]);
    request.extend_from_slice(&credentials.username);
    request.push(credentials.password.len() as u8);
    request.extend_from_slice(&credentials.password);
    Ok(SensitiveBytes(request))
}

pub fn parse_socks_auth_response(response: [u8; 2]) -> Result<()> {
    if response == [1, 0] {
        Ok(())
    } else {
        Err(anyhow!("SOCKS5 代理认证失败"))
    }
}

/// SOCKS5 domains are intentionally sent to the proxy to avoid local DNS
/// leakage. IP literals use their canonical address forms.
pub fn socks_connect_request(host: &str, port: u16) -> Result<Vec<u8>> {
    if port == 0 {
        return Err(anyhow!("目标端口无效"));
    }
    let mut request = vec![5, 1, 0];
    let host = host.trim_matches(['[', ']']);
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            request.push(1);
            request.extend_from_slice(&address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            request.push(4);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            if host.is_empty() || host.len() > 255 || !host.is_ascii() {
                return Err(anyhow!("目标主机名无效"));
            }
            request.extend_from_slice(&[3, host.len() as u8]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    Ok(request)
}

/// SWR-TUN-004: validate the fixed reply prefix before an adapter reads the
/// remaining bound address and port bytes.
pub fn parse_socks_reply_prefix(prefix: [u8; 4]) -> Result<SocksReplyAddress> {
    if prefix[0] != 5 || prefix[2] != 0 {
        return Err(anyhow!("SOCKS5 连接响应无效"));
    }
    if prefix[1] != 0 {
        return Err(anyhow!("SOCKS5 代理拒绝目标连接（代码 {}）", prefix[1]));
    }
    match prefix[3] {
        1 => Ok(SocksReplyAddress::Ipv4),
        4 => Ok(SocksReplyAddress::Ipv6),
        3 => Ok(SocksReplyAddress::DomainLengthPending),
        _ => Err(anyhow!("SOCKS5 响应地址类型无效")),
    }
}

pub fn socks_reply_tail_len(
    address: SocksReplyAddress,
    domain_length: Option<u8>,
) -> Result<usize> {
    let address_len = match (address, domain_length) {
        (SocksReplyAddress::Ipv4, None) => 4,
        (SocksReplyAddress::Ipv6, None) => 16,
        (SocksReplyAddress::DomainLengthPending, Some(length)) if length > 0 => usize::from(length),
        _ => return Err(anyhow!("SOCKS5 响应地址长度无效")),
    };
    Ok(address_len + 2)
}

fn authority(host: &str, port: u16) -> Result<String> {
    if port == 0 {
        return Err(anyhow!("目标端口无效"));
    }
    let host = host.trim_matches(['[', ']']);
    if host.is_empty() || host.len() > 255 || host.chars().any(char::is_control) {
        return Err(anyhow!("目标主机名无效"));
    }
    if host.parse::<Ipv6Addr>().is_ok() {
        Ok(format!("[{host}]:{port}"))
    } else {
        Ok(format!("{host}:{port}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(kind: ProxyKind, credentials: Option<ProxyCredentials>) -> ProxyEndpoint {
        ProxyEndpoint {
            kind,
            host: "localhost".into(),
            port: 8080,
            credentials,
        }
    }

    fn credentials() -> ProxyCredentials {
        ProxyCredentials {
            username: b"alice".to_vec(),
            password: b"secret".to_vec(),
        }
    }

    #[test]
    fn swr_tun_001_http_request_is_exact_and_bounded() {
        let request =
            http_connect_request(&endpoint(ProxyKind::Http, Some(credentials())), "::1", 443)
                .expect("request fixture");
        let text = std::str::from_utf8(request.as_slice()).expect("ASCII request");
        assert!(text.starts_with("CONNECT [::1]:443 HTTP/1.1\r\nHost: [::1]:443\r\n"));
        assert!(text.contains("Proxy-Authorization: Basic YWxpY2U6c2VjcmV0\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn swr_tun_002_http_response_is_bounded_and_status_allowlisted() {
        assert!(parse_http_connect_response(b"HTTP/1.1 200 OK\r\n\r\n").is_ok());
        assert!(parse_http_connect_response(b"HTTP/1.1 407 Secret\r\n\r\n").is_err());
        assert!(parse_http_connect_response(b"HTTP/1.1 200 OK\r\n").is_err());
        let oversized = vec![b'x'; MAX_HTTP_CONNECT_HEADER_BYTES + 1];
        assert!(parse_http_connect_response(&oversized).is_err());
    }

    #[test]
    fn swr_tun_003_socks_auth_and_remote_dns_are_exact() {
        assert_eq!(socks_greeting(true), [5, 2, 2, 0]);
        assert_eq!(
            parse_socks_method([5, 2], true).expect("auth method"),
            SocksAuthMethod::UsernamePassword
        );
        assert_eq!(
            socks_auth_request(&credentials())
                .expect("auth request")
                .as_slice(),
            &[
                1, 5, b'a', b'l', b'i', b'c', b'e', 6, b's', b'e', b'c', b'r', b'e', b't'
            ]
        );
        let request =
            socks_connect_request("must-resolve-at-proxy.invalid", 443).expect("connect request");
        assert_eq!(&request[..5], &[5, 1, 0, 3, 29]);
        assert_eq!(&request[request.len() - 2..], &443_u16.to_be_bytes());
    }

    #[test]
    fn swr_tun_004_socks_reply_shapes_are_deterministic() {
        assert_eq!(
            socks_reply_tail_len(
                parse_socks_reply_prefix([5, 0, 0, 4]).expect("IPv6 prefix"),
                None
            )
            .expect("IPv6 tail"),
            18
        );
        let domain = parse_socks_reply_prefix([5, 0, 0, 3]).expect("domain prefix");
        assert_eq!(
            socks_reply_tail_len(domain, Some(12)).expect("domain tail"),
            14
        );
        assert!(parse_socks_reply_prefix([5, 5, 0, 1]).is_err());
        assert!(socks_reply_tail_len(domain, Some(0)).is_err());
    }
}
