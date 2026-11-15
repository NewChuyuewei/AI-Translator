//! Credential-safe proxy endpoint parsing independent of transport and platform resolution.

use anyhow::{Context, Result, anyhow};
use percent_encoding::percent_decode_str;
use url::{Host, Url};
use zeroize::Zeroize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    Http,
    Socks5,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyCredentials {
    pub username: Vec<u8>,
    pub password: Vec<u8>,
}

impl Drop for ProxyCredentials {
    fn drop(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyEndpoint {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub credentials: Option<ProxyCredentials>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyDestination {
    Direct,
    Proxy(ProxyEndpoint),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyRoute {
    pub destination: ProxyDestination,
    label: &'static str,
}

impl ProxyRoute {
    pub fn explicit_direct() -> Self {
        Self {
            destination: ProxyDestination::Direct,
            label: "系统网络（不使用显式代理；仍可能经过 VPN）",
        }
    }

    pub fn automatic_direct() -> Self {
        Self {
            destination: ProxyDestination::Direct,
            label: "系统网络（未检测到显式代理；可能经过 VPN）",
        }
    }

    pub fn manual(value: &str) -> Result<Self> {
        Ok(Self::via(ProxyEndpoint::parse(value)?, false))
    }

    /// Direct/manual modes resolve immediately. Auto requires exactly one
    /// platform-selected value and therefore returns `None` here.
    pub fn immediate(mode: crate::ProxyMode, address: &str) -> Result<Option<Self>> {
        match mode {
            crate::ProxyMode::Direct => Ok(Some(Self::explicit_direct())),
            crate::ProxyMode::Manual => Self::manual(address).map(Some),
            crate::ProxyMode::Auto => Ok(None),
        }
    }

    /// Parse the single route selected by a platform resolver. Callers must
    /// not silently try later DIRECT entries after a selected proxy fails.
    pub fn automatic_resolver_value(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("direct://") || value.eq_ignore_ascii_case("direct:///") {
            return Ok(Self::automatic_direct());
        }
        let endpoint =
            ProxyEndpoint::parse(value).context("系统代理类型或地址无法使用；请尝试手动代理")?;
        Ok(Self::via(endpoint, true))
    }

    pub fn via(endpoint: ProxyEndpoint, automatic: bool) -> Self {
        let label = endpoint.safe_label(automatic);
        Self {
            destination: ProxyDestination::Proxy(endpoint),
            label,
        }
    }

    pub fn label(&self) -> &'static str {
        self.label
    }
}

impl ProxyEndpoint {
    /// SWR-PXY-001..003: parse an untrusted manual/system endpoint without
    /// reflecting its original text or parser details into errors.
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() || value.len() > 2_048 || value.chars().any(char::is_control) {
            return Err(anyhow!("请填写有效的代理地址"));
        }
        let url = Url::parse(value)
            .map_err(|_| anyhow!("代理地址格式无效；示例 http://127.0.0.1:7890"))?;
        let kind = match url.scheme() {
            "http" => ProxyKind::Http,
            "socks" | "socks5" | "socks5h" => ProxyKind::Socks5,
            "https" => {
                return Err(anyhow!(
                    "暂不支持加密的 HTTPS 代理入口；请改用 HTTP CONNECT 或 SOCKS5 入口"
                ));
            }
            _ => {
                return Err(anyhow!("只支持 http://、socks5:// 或 socks5h:// 代理地址"));
            }
        };
        if url.host().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(anyhow!("代理地址只能包含主机、端口及可选的用户名密码"));
        }
        let host = normalized_host(url.host().ok_or_else(|| anyhow!("代理地址缺少主机"))?);
        let port = url
            .port_or_known_default()
            .filter(|port| *port > 0)
            .ok_or_else(|| anyhow!("代理地址需要端口"))?;
        let credentials = parse_credentials(&url)?;
        if kind == ProxyKind::Socks5
            && credentials
                .as_ref()
                .is_some_and(|credentials| credentials.username.is_empty())
        {
            return Err(anyhow!("SOCKS5 用户名不能为空"));
        }
        Ok(Self {
            kind,
            host,
            port,
            credentials,
        })
    }

    /// A log/UI label containing no endpoint or credential material.
    pub fn safe_label(&self, automatic: bool) -> &'static str {
        match (automatic, self.kind) {
            (true, ProxyKind::Http) => "系统/环境 HTTP 代理",
            (true, ProxyKind::Socks5) => "系统/环境 SOCKS5 代理",
            (false, ProxyKind::Http) => "手动 HTTP 代理",
            (false, ProxyKind::Socks5) => "手动 SOCKS5 代理",
        }
    }
}

fn normalized_host(host: Host<&str>) -> String {
    match host {
        Host::Domain(value) => value.to_owned(),
        Host::Ipv4(value) => value.to_string(),
        Host::Ipv6(value) => value.to_string(),
    }
}

fn parse_credentials(url: &Url) -> Result<Option<ProxyCredentials>> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(None);
    }
    let decode = |value: &str| -> Result<Vec<u8>> {
        let bytes = percent_decode_str(value).collect::<Vec<_>>();
        if bytes.len() > 255 || bytes.iter().any(u8::is_ascii_control) {
            return Err(anyhow!("代理用户名或密码过长或含控制字符"));
        }
        Ok(bytes)
    };
    let username = decode(url.username())?;
    let password = decode(url.password().unwrap_or(""))?;
    if username.contains(&b':') {
        return Err(anyhow!("代理用户名不能包含冒号"));
    }
    Ok(Some(ProxyCredentials { username, password }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swr_pxy_001_supported_endpoints_are_canonical_and_typed() {
        let http = ProxyEndpoint::parse("http://127.0.0.1:7890").expect("HTTP fixture");
        assert_eq!(http.kind, ProxyKind::Http);
        assert_eq!(http.host, "127.0.0.1");
        assert_eq!(http.port, 7890);
        let socks = ProxyEndpoint::parse("socks5h://[::1]:1080").expect("SOCKS fixture");
        assert_eq!(socks.kind, ProxyKind::Socks5);
        assert_eq!(socks.host, "::1");
    }

    #[test]
    fn swr_pxy_002_credentials_are_decoded_bounded_and_never_echoed() {
        let endpoint = ProxyEndpoint::parse("http://alice:p%40ss%3Aword@localhost:8080")
            .expect("credential fixture");
        let credentials = endpoint.credentials.as_ref().expect("credentials");
        assert_eq!(credentials.username, b"alice");
        assert_eq!(credentials.password, b"p@ss:word");
        for invalid in [
            "http://alice:secret@example.com:80/path",
            "https://alice:secret@example.com:443",
            "http://alice%3Aadmin:secret@example.com:80",
            "nope://alice:secret@example.com:1",
        ] {
            let error = ProxyEndpoint::parse(invalid)
                .expect_err("invalid endpoint")
                .to_string();
            assert!(!error.contains("alice"));
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn swr_pxy_003_structure_and_credential_limits_are_enforced() {
        assert!(ProxyEndpoint::parse("http://localhost:80/?query=1").is_err());
        assert!(ProxyEndpoint::parse("socks5://:password@localhost:1080").is_err());
        let long = "x".repeat(256);
        assert!(ProxyEndpoint::parse(&format!("http://{long}:p@localhost:8080")).is_err());
        assert!(ProxyEndpoint::parse(&format!("http://u:{long}@localhost:8080")).is_err());
    }

    #[test]
    fn swr_pxy_004_labels_are_fixed_and_credential_free() {
        let endpoint =
            ProxyEndpoint::parse("http://alice:secret@localhost:8080").expect("label fixture");
        assert_eq!(endpoint.safe_label(false), "手动 HTTP 代理");
        assert_eq!(endpoint.safe_label(true), "系统/环境 HTTP 代理");
    }

    #[test]
    fn swr_route_001_direct_and_manual_modes_resolve_without_platform_access() {
        let direct = ProxyRoute::immediate(crate::ProxyMode::Direct, "ignored")
            .expect("direct route")
            .expect("immediate route");
        assert_eq!(direct.destination, ProxyDestination::Direct);
        let manual = ProxyRoute::immediate(
            crate::ProxyMode::Manual,
            "socks5://alice:secret@localhost:1080",
        )
        .expect("manual route")
        .expect("immediate route");
        assert!(matches!(manual.destination, ProxyDestination::Proxy(_)));
        assert!(!manual.label().contains("alice"));
        assert!(!manual.label().contains("secret"));
    }

    #[test]
    fn swr_route_002_auto_requires_one_platform_selected_value() {
        assert!(
            ProxyRoute::immediate(crate::ProxyMode::Auto, "ignored")
                .expect("auto dispatch")
                .is_none()
        );
        assert_eq!(
            ProxyRoute::automatic_resolver_value("direct://")
                .expect("auto direct")
                .destination,
            ProxyDestination::Direct
        );
        let proxy =
            ProxyRoute::automatic_resolver_value("http://localhost:7890").expect("auto proxy");
        assert!(matches!(proxy.destination, ProxyDestination::Proxy(_)));
    }

    #[test]
    fn swr_route_003_invalid_auto_value_has_fixed_context_without_secrets() {
        let error =
            ProxyRoute::automatic_resolver_value("https://alice:secret@proxy.example:443/private")
                .expect_err("unsupported resolver value")
                .to_string();
        assert!(error.contains("系统代理类型或地址无法使用"));
        assert!(!error.contains("alice"));
        assert!(!error.contains("secret"));
        assert!(!error.contains("proxy.example"));
    }
}
