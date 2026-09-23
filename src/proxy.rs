//! Proxy support for the WhatsApp connection, media, and update checks.
//!
//! The proxy comes from Settings, or from `ALL_PROXY` / `HTTPS_PROXY` when
//! the setting is empty. `http://`, `socks5://`, and `socks5h://` proxies
//! are supported, with an optional `user:password@`.

use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use whatsapp_rust::transport::{
    Connector, Transport, TransportEvent, TransportFactory, default_tls_connector, from_websocket,
};
use whatsapp_rust::wacore::net::{WHATSAPP_WEB_ORIGIN, WHATSAPP_WEB_WS_URL};

/// Time allowed to reach the proxy and open the tunnel.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest proxy reply header accepted for an HTTP tunnel.
const MAX_REPLY: usize = 8 * 1024;
const ENV_VARS: [&str; 4] = ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    Http,
    /// SOCKS5 with names resolved on this computer.
    Socks5,
    /// SOCKS5 with names resolved by the proxy.
    Socks5h,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Proxy {
    scheme: Scheme,
    host: String,
    port: u16,
    auth: Option<(String, String)>,
    /// Normalized URL for ureq and reqwest.
    url: String,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.redacted())
    }
}

impl Proxy {
    /// Parses a proxy URL. A bare `host:port` is an HTTP proxy.
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        let with_scheme = if value.contains("://") {
            value.to_owned()
        } else {
            format!("http://{value}")
        };
        let url = reqwest::Url::parse(&with_scheme)
            .map_err(|_| "Enter a proxy address like socks5://127.0.0.1:1080".to_owned())?;
        let scheme = match url.scheme() {
            "http" => Scheme::Http,
            "socks5" | "socks" => Scheme::Socks5,
            "socks5h" => Scheme::Socks5h,
            _ => return Err("Use an http://, socks5://, or socks5h:// proxy".to_owned()),
        };
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| "The proxy address needs a host".to_owned())?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        if !matches!(url.path(), "" | "/") || url.query().is_some() {
            return Err("A proxy address has no path".to_owned());
        }
        let port = match scheme {
            Scheme::Http => url.port_or_known_default().unwrap_or(80),
            Scheme::Socks5 | Scheme::Socks5h => url.port().unwrap_or(1080),
        };
        let auth = (!url.username().is_empty()).then(|| {
            (
                percent_decode(url.username()),
                percent_decode(url.password().unwrap_or("")),
            )
        });
        let scheme_name = match scheme {
            Scheme::Http => "http",
            Scheme::Socks5 => "socks5",
            Scheme::Socks5h => "socks5h",
        };
        let credentials = match url.password() {
            Some(password) if !url.username().is_empty() => {
                format!("{}:{password}@", url.username())
            }
            _ if !url.username().is_empty() => format!("{}@", url.username()),
            _ => String::new(),
        };
        let host_part = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.clone()
        };
        Ok(Self {
            scheme,
            url: format!("{scheme_name}://{credentials}{host_part}:{port}"),
            host,
            port,
            auth,
        })
    }

    /// The URL without its password, for logs and Settings.
    pub fn redacted(&self) -> String {
        match (&self.auth, self.url.split_once('@')) {
            (Some((user, _)), Some((_, rest))) => {
                let scheme = self.url.split("://").next().unwrap_or("http");
                format!("{scheme}://{user}:***@{rest}")
            }
            _ => self.url.clone(),
        }
    }

    /// Opens a tunnel through the proxy to `host:port`.
    pub async fn connect(&self, host: &str, port: u16) -> std::io::Result<TcpStream> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port)).await?;
        stream.set_nodelay(true)?;
        match self.scheme {
            Scheme::Http => self.http_connect(&mut stream, host, port).await?,
            Scheme::Socks5 | Scheme::Socks5h => {
                self.socks5_connect(&mut stream, host, port).await?
            }
        }
        Ok(stream)
    }

    async fn http_connect<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        stream: &mut S,
        host: &str,
        port: u16,
    ) -> std::io::Result<()> {
        let target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
        if let Some((user, password)) = &self.auth {
            use base64::Engine as _;
            let token =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
            request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;

        // Read byte by byte so no tunnel data is consumed with the header.
        let mut reply = Vec::with_capacity(256);
        while !reply.ends_with(b"\r\n\r\n") {
            if reply.len() >= MAX_REPLY {
                return Err(proxy_error("The proxy sent an oversized reply"));
            }
            reply.push(stream.read_u8().await?);
        }
        let status_line = String::from_utf8_lossy(&reply);
        let status = status_line
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok());
        match status {
            Some(200..=299) => Ok(()),
            Some(407) => Err(proxy_error("The proxy rejected the user name or password")),
            Some(code) => Err(proxy_error(&format!(
                "The proxy refused the tunnel ({code})"
            ))),
            None => Err(proxy_error("The proxy did not answer as an HTTP proxy")),
        }
    }

    async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        stream: &mut S,
        host: &str,
        port: u16,
    ) -> std::io::Result<()> {
        // Greeting: offer user/password only when there are credentials.
        let greeting: &[u8] = if self.auth.is_some() {
            &[5, 2, 0, 2]
        } else {
            &[5, 1, 0]
        };
        stream.write_all(greeting).await?;
        let mut choice = [0u8; 2];
        stream.read_exact(&mut choice).await?;
        if choice[0] != 5 {
            return Err(proxy_error("The proxy did not answer as a SOCKS5 proxy"));
        }
        match (choice[1], &self.auth) {
            (0, _) => {}
            (2, Some((user, password))) => {
                let (user, password) = (user.as_bytes(), password.as_bytes());
                if user.len() > 255 || password.len() > 255 {
                    return Err(proxy_error("The proxy user name or password is too long"));
                }
                let mut login = vec![1, user.len() as u8];
                login.extend_from_slice(user);
                login.push(password.len() as u8);
                login.extend_from_slice(password);
                stream.write_all(&login).await?;
                let mut status = [0u8; 2];
                stream.read_exact(&mut status).await?;
                if status[1] != 0 {
                    return Err(proxy_error("The proxy rejected the user name or password"));
                }
            }
            (2, None) => return Err(proxy_error("The proxy needs a user name and password")),
            _ => return Err(proxy_error("The proxy offered no usable sign-in method")),
        }

        let mut request = vec![5, 1, 0];
        match self.scheme {
            Scheme::Socks5 => {
                let address = tokio::net::lookup_host((host, port))
                    .await?
                    .next()
                    .ok_or_else(|| proxy_error(&format!("Could not resolve {host}")))?;
                match address.ip() {
                    std::net::IpAddr::V4(ip) => {
                        request.push(1);
                        request.extend_from_slice(&ip.octets());
                    }
                    std::net::IpAddr::V6(ip) => {
                        request.push(4);
                        request.extend_from_slice(&ip.octets());
                    }
                }
            }
            _ => {
                let name = host.as_bytes();
                if name.len() > 255 {
                    return Err(proxy_error("The host name is too long for SOCKS5"));
                }
                request.push(3);
                request.push(name.len() as u8);
                request.extend_from_slice(name);
            }
        }
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request).await?;

        let mut head = [0u8; 4];
        stream.read_exact(&mut head).await?;
        if head[0] != 5 {
            return Err(proxy_error("The proxy did not answer as a SOCKS5 proxy"));
        }
        if head[1] != 0 {
            return Err(proxy_error(socks_reply(head[1])));
        }
        // Skip the bound address and port.
        let skip = match head[3] {
            1 => 4 + 2,
            4 => 16 + 2,
            3 => stream.read_u8().await? as usize + 2,
            _ => return Err(proxy_error("The proxy sent an unknown address type")),
        };
        let mut rest = vec![0u8; skip];
        stream.read_exact(&mut rest).await?;
        Ok(())
    }
}

fn socks_reply(code: u8) -> &'static str {
    match code {
        1 => "The proxy failed",
        2 => "The proxy does not allow this connection",
        3 => "The proxy cannot reach the network",
        4 => "The proxy cannot reach the host",
        5 => "The host refused the proxy's connection",
        6 => "The proxy connection timed out",
        _ => "The proxy refused the connection",
    }
}

fn proxy_error(message: &str) -> std::io::Error {
    std::io::Error::other(message.to_owned())
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = value
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `NO_PROXY` exempts `host`.
fn bypassed(host: &str, no_proxy: &str) -> bool {
    no_proxy.split(',').map(str::trim).any(|entry| {
        let entry = entry.trim_start_matches("*.").trim_start_matches('.');
        entry == "*"
            || (!entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}"))))
    })
}

fn from_env(host: &str) -> Option<Proxy> {
    let no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    if bypassed(host, &no_proxy) {
        return None;
    }
    ENV_VARS
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty())
        .and_then(|value| match Proxy::parse(&value) {
            Ok(proxy) => Some(proxy),
            Err(error) => {
                log::warn!("ignoring the proxy environment variable: {error}");
                None
            }
        })
}

struct Configured {
    setting: String,
    agent: Option<ureq::Agent>,
}

fn configured() -> &'static RwLock<Configured> {
    static CONFIGURED: OnceLock<RwLock<Configured>> = OnceLock::new();
    CONFIGURED.get_or_init(|| {
        RwLock::new(Configured {
            setting: String::new(),
            agent: None,
        })
    })
}

/// Applies the Settings value. An invalid value is ignored, so Settings
/// validates before saving.
pub fn configure(setting: &str) {
    let mut configured = configured().write().unwrap_or_else(|e| e.into_inner());
    if configured.setting != setting.trim() {
        configured.setting = setting.trim().to_owned();
        configured.agent = None;
    }
}

/// The proxy chosen in Settings, if any.
pub fn from_settings() -> Option<Proxy> {
    let setting = configured()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .setting
        .clone();
    (!setting.is_empty())
        .then(|| Proxy::parse(&setting).ok())
        .flatten()
}

/// The proxy for the WhatsApp connection: Settings, then the environment.
pub fn for_whatsapp() -> Option<Proxy> {
    from_settings().or_else(|| from_env("web.whatsapp.com"))
}

/// The ureq proxy: Settings, then the environment's proxy variables.
pub fn ureq_proxy() -> Option<ureq::Proxy> {
    match from_settings().map(|proxy| ureq::Proxy::new(&proxy.url)) {
        Some(Ok(proxy)) => Some(proxy),
        Some(Err(error)) => {
            log::warn!("the proxy is not usable for downloads: {error}");
            None
        }
        None => ureq::Proxy::try_from_env(),
    }
}

/// A shared HTTP agent that uses [`ureq_proxy`].
pub fn agent() -> ureq::Agent {
    if let Some(agent) = &configured().read().unwrap_or_else(|e| e.into_inner()).agent {
        return agent.clone();
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(ureq_proxy())
        .build()
        .into();
    configured()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .agent = Some(agent.clone());
    agent
}

/// The Settings proxy as a reqwest proxy. reqwest reads the environment
/// itself when this is `None`.
pub fn reqwest_proxy() -> Option<reqwest::Proxy> {
    from_settings().and_then(|proxy| reqwest::Proxy::all(&proxy.url).ok())
}

/// Opens the WhatsApp WebSocket through a proxy.
pub struct ProxyTransportFactory {
    proxy: Proxy,
    connector: OnceLock<Connector>,
}

impl ProxyTransportFactory {
    pub fn new(proxy: Proxy) -> Self {
        Self {
            proxy,
            connector: OnceLock::new(),
        }
    }
}

#[whatsapp_rust::async_trait]
impl TransportFactory for ProxyTransportFactory {
    async fn create_transport(
        &self,
    ) -> Result<
        (
            Arc<dyn Transport>,
            whatsapp_rust::async_channel::Receiver<TransportEvent>,
        ),
        anyhow::Error,
    > {
        let uri: http::Uri = WHATSAPP_WEB_WS_URL.parse()?;
        let host = uri.host().unwrap_or("web.whatsapp.com").to_owned();
        let port = uri.port_u16().unwrap_or(443);
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, self.proxy.connect(&host, port))
            .await
            .map_err(|_| anyhow::anyhow!("The proxy {} did not answer", self.proxy.redacted()))?
            .map_err(|error| anyhow::anyhow!("Proxy {}: {error}", self.proxy.redacted()))?;
        let connector = self.connector.get_or_init(default_tls_connector);
        let stream = connector
            .wrap(&host, stream)
            .await
            .map_err(|error| anyhow::anyhow!("TLS through the proxy failed: {error}"))?;
        let (ws, _) = tokio_websockets::ClientBuilder::from_uri(uri)
            .add_header(
                http::header::ORIGIN,
                http::HeaderValue::from_static(WHATSAPP_WEB_ORIGIN),
            )?
            .connect_on(stream)
            .await
            .map_err(|error| anyhow::anyhow!("WebSocket connect failed: {error}"))?;
        Ok(from_websocket(ws))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn parses_supported_schemes() {
        let proxy = Proxy::parse("socks5h://me:p%40ss@127.0.0.1:9050").unwrap();
        assert_eq!(proxy.scheme, Scheme::Socks5h);
        assert_eq!(proxy.port, 9050);
        assert_eq!(proxy.auth, Some(("me".to_owned(), "p@ss".to_owned())));
        assert_eq!(proxy.redacted(), "socks5h://me:***@127.0.0.1:9050");
        assert!(!format!("{proxy:?}").contains("p%40ss"));

        let proxy = Proxy::parse("proxy.lan:3128").unwrap();
        assert_eq!(proxy.scheme, Scheme::Http);
        assert_eq!(proxy.url, "http://proxy.lan:3128");
        assert_eq!(
            Proxy::parse("socks5://[::1]").unwrap().url,
            "socks5://[::1]:1080"
        );
        assert!(Proxy::parse("ftp://proxy.lan").is_err());
        assert!(Proxy::parse("http://proxy.lan/path").is_err());
        assert!(Proxy::parse("").is_err());
    }

    #[test]
    fn no_proxy_matches_domains() {
        assert!(bypassed("web.whatsapp.com", "localhost, .whatsapp.com"));
        assert!(bypassed("web.whatsapp.com", "*"));
        assert!(bypassed("web.whatsapp.com", "web.whatsapp.com"));
        assert!(!bypassed("web.whatsapp.com", "app.com,,"));
        assert!(!bypassed("web.whatsapp.com", ""));
    }

    #[tokio::test]
    async fn socks5_tunnel_sends_the_host_name_and_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 4];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 2, 0, 2]);
            socket.write_all(&[5, 2]).await.unwrap();
            let mut login = [0u8; 7];
            socket.read_exact(&mut login).await.unwrap();
            assert_eq!(&login, b"\x01\x02me\x02pw");
            socket.write_all(&[1, 0]).await.unwrap();
            let mut request = vec![0u8; 5 + "web.whatsapp.com".len() + 2];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..5], &[5, 1, 0, 3, 16]);
            assert_eq!(&request[5..21], b"web.whatsapp.com");
            assert_eq!(&request[21..], &443u16.to_be_bytes());
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .await
                .unwrap();
            socket.write_all(b"hello").await.unwrap();
        });
        let proxy = Proxy::parse(&format!("socks5h://me:pw@{address}")).unwrap();
        let mut stream = proxy.connect("web.whatsapp.com", 443).await.unwrap();
        let mut hello = [0u8; 5];
        stream.read_exact(&mut hello).await.unwrap();
        assert_eq!(&hello, b"hello");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_tunnel_keeps_data_after_the_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("CONNECT web.whatsapp.com:443 HTTP/1.1\r\n"));
            assert!(request.contains("Proxy-Authorization: Basic bWU6cHc=\r\n"));
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\nhello")
                .await
                .unwrap();
        });
        let proxy = Proxy::parse(&format!("http://me:pw@{address}")).unwrap();
        let mut stream = proxy.connect("web.whatsapp.com", 443).await.unwrap();
        let mut hello = [0u8; 5];
        stream.read_exact(&mut hello).await.unwrap();
        assert_eq!(&hello, b"hello");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_tunnel_reports_rejected_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            socket
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await
                .unwrap();
        });
        let proxy = Proxy::parse(&format!("http://{address}")).unwrap();
        let error = proxy.connect("web.whatsapp.com", 443).await.unwrap_err();
        assert!(error.to_string().contains("user name or password"));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Opens the real WhatsApp WebSocket through `ZAPFAST_TEST_PROXY`.
    #[tokio::test]
    #[ignore = "needs network and a proxy in ZAPFAST_TEST_PROXY"]
    async fn reaches_whatsapp_through_the_proxy() {
        let proxy = Proxy::parse(&std::env::var("ZAPFAST_TEST_PROXY").unwrap()).unwrap();
        let factory = ProxyTransportFactory::new(proxy);
        let (transport, _events) = factory.create_transport().await.unwrap();
        transport.disconnect().await;
    }
}
