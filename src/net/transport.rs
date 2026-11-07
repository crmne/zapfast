//! The direct WhatsApp WebSocket, dialed with Happy Eyeballs.
//!
//! The Tokio transport this replaces resolves the host and dials the first
//! address DNS returns. On a dual-stack network whose IPv6 has a route but no
//! path past the gateway, that address never answers, and every reconnect
//! makes the same choice, so the link stays down while IPv4 works
//! (crmne/zapfast#212). The factory here resolves the host itself on each dial
//! and races what it gets, one address every [`HEAD_START`], so a family that
//! does not answer loses to one that does.
//!
//! Resolving per dial is also what makes a reconnect after a network change
//! pick up the new addresses instead of the ones the link came up with.
//! [`crate::proxy`] is the same factory shape for a configured proxy, which
//! dials the proxy itself rather than the WhatsApp host.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::net::TcpStream;
use whatsapp_rust::transport::{
    Connector, Transport, TransportEvent, TransportFactory, default_tls_connector, from_websocket,
};
use whatsapp_rust::wacore::net::{WHATSAPP_WEB_ORIGIN, WHATSAPP_WEB_WS_URL};

/// How long one address gets before the next one is tried, as RFC 8305 asks.
///
/// Long enough that a working address is never raced against a redundant one,
/// short enough that a family which never answers costs a fraction of a
/// second instead of the whole connect timeout.
pub const HEAD_START: Duration = Duration::from_millis(250);

/// Alternates the families in `addrs`, keeping the order within each one and
/// the family the resolver preferred first.
///
/// The resolver sorts addresses by RFC 6724, which puts IPv6 first whenever
/// the host has a global one, even when nothing gets past the gateway. Trying
/// them in that order spends a full connect timeout on the first address of
/// the broken family before reaching the other one, so the two are
/// interleaved: at most one head start is lost to a family that does not
/// answer.
pub fn interleave(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let mut prefer_v6 = addrs.first().is_some_and(SocketAddr::is_ipv6);
    let mut v6 = VecDeque::new();
    let mut v4 = VecDeque::new();
    for addr in addrs {
        if addr.is_ipv6() {
            v6.push_back(addr);
        } else {
            v4.push_back(addr);
        }
    }
    let mut ordered = Vec::with_capacity(v6.len() + v4.len());
    loop {
        let next = if prefer_v6 {
            v6.pop_front().or_else(|| v4.pop_front())
        } else {
            v4.pop_front().or_else(|| v6.pop_front())
        };
        let Some(addr) = next else {
            return ordered;
        };
        ordered.push(addr);
        prefer_v6 = !prefer_v6;
    }
}

/// Opens a TCP connection to the first of `addrs` that answers.
///
/// Returns the stream together with the address it reached, which is the one
/// worth naming in a log line or an error: it is what says whether the
/// connection is running over IPv4 or IPv6.
pub async fn dial(addrs: Vec<SocketAddr>) -> std::io::Result<(TcpStream, SocketAddr)> {
    race(addrs, HEAD_START, |addr| async move {
        TcpStream::connect(addr).await
    })
    .await
}

/// The attempts a dial has running, aborted when the dial is dropped.
///
/// `create_transport` is awaited under the library's connect timeout, and the
/// trait asks a dial that is cancelled to leave no socket behind. Attempts
/// live in their own tasks, so nothing else would stop them.
#[derive(Default)]
struct Attempts(Vec<tokio::task::JoinHandle<()>>);

impl Attempts {
    fn start(&mut self, handle: tokio::task::JoinHandle<()>) {
        self.0.push(handle);
    }
}

impl Drop for Attempts {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// Runs `open` over `addrs`, starting a new attempt every `head_start` while
/// the ones already running are still in flight, and returns the first
/// success with the address that produced it.
///
/// A failed attempt is replaced at once, so a refused connection does not wait
/// out the head start, and the first success drops every attempt still
/// running, which closes the sockets they had opened. When none answers, the
/// error names every address that was tried and why it failed, which is the
/// diagnostic a bug report needs: it says which family could not be reached.
async fn race<A, T, F, Fut>(addrs: Vec<A>, head_start: Duration, open: F) -> std::io::Result<(T, A)>
where
    A: Copy + Send + std::fmt::Display + 'static,
    T: Send + 'static,
    F: Fn(A) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = std::io::Result<T>> + Send + 'static,
{
    let (answers, mut answered) =
        tokio::sync::mpsc::channel::<(A, std::io::Result<T>)>(addrs.len().max(1));
    let open = Arc::new(open);
    let mut remaining = addrs.into_iter();
    let mut running = Attempts::default();
    let mut failures: Vec<String> = Vec::new();
    let mut last_kind = std::io::ErrorKind::Other;
    let mut in_flight = 0usize;
    let mut exhausted = false;
    let mut next_start = tokio::time::Instant::now();

    loop {
        while !exhausted && tokio::time::Instant::now() >= next_start {
            let Some(addr) = remaining.next() else {
                exhausted = true;
                break;
            };
            in_flight += 1;
            next_start = tokio::time::Instant::now() + head_start;
            let answers = answers.clone();
            let open = Arc::clone(&open);
            running.start(tokio::spawn(async move {
                let result = open(addr).await;
                let _ = answers.send((addr, result)).await;
            }));
        }
        if in_flight == 0 {
            if exhausted {
                return Err(no_address_answered(&failures, last_kind));
            }
            next_start = tokio::time::Instant::now();
            continue;
        }
        tokio::select! {
            biased;
            answer = answered.recv() => match answer {
                Some((addr, Ok(value))) => {
                    log::debug!("connected to {addr}");
                    return Ok((value, addr));
                }
                Some((addr, Err(error))) => {
                    log::info!("could not connect to {addr}: {error}");
                    last_kind = error.kind();
                    failures.push(format!("{addr}: {error}"));
                    in_flight -= 1;
                    next_start = tokio::time::Instant::now();
                }
                // Every attempt has reported and dropped its sender, so no
                // answer can still arrive.
                None => in_flight = 0,
            },
            _ = tokio::time::sleep_until(next_start) => {}
        }
    }
}

/// The error a dial that tried every address reports.
fn no_address_answered(failures: &[String], kind: std::io::ErrorKind) -> std::io::Error {
    if failures.is_empty() {
        return std::io::Error::new(kind, "the host resolved to no address");
    }
    std::io::Error::new(
        kind,
        format!("no address answered: {}", failures.join("; ")),
    )
}

/// The direct WhatsApp WebSocket, dialed with Happy Eyeballs.
pub struct HappyEyeballsTransportFactory {
    url: String,
    /// Built on the first dial and kept, so the TLS session store inside it
    /// outlives a reconnect. A fresh connector per dial leaves that store
    /// permanently empty and resumption can never fire, which is why the
    /// Tokio factory keeps one too.
    connector: OnceLock<Connector>,
}

impl HappyEyeballsTransportFactory {
    pub fn new() -> Self {
        Self {
            url: WHATSAPP_WEB_WS_URL.to_owned(),
            connector: OnceLock::new(),
        }
    }
}

impl Default for HappyEyeballsTransportFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[whatsapp_rust::async_trait]
impl TransportFactory for HappyEyeballsTransportFactory {
    async fn create_transport(&self) -> Result<Link, anyhow::Error> {
        let uri: http::Uri = self.url.parse()?;
        let host = uri.host().unwrap_or("web.whatsapp.com").to_owned();
        let port = uri.port_u16().unwrap_or(443);
        // Resolved here, on every dial, so a reconnect after the network
        // changed does not go back to the addresses the link came up with.
        let addrs = interleave(
            tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|error| anyhow::anyhow!("Could not resolve {host}: {error}"))?
                .collect(),
        );
        if addrs.is_empty() {
            anyhow::bail!("{host} resolved to no address");
        }
        let (stream, addr) = dial(addrs)
            .await
            .map_err(|error| anyhow::anyhow!("Could not reach {host}: {error}"))?;
        websocket(&self.connector, uri, &host, stream, &format!("to {addr}")).await
    }
}

/// What a [`TransportFactory`] hands the library.
pub type Link = (
    Arc<dyn Transport>,
    whatsapp_rust::async_channel::Receiver<TransportEvent>,
);

/// Runs TLS and the WhatsApp WebSocket handshake over a connected `stream`,
/// shared by the direct dial here and [`crate::proxy`].
///
/// `connector` is the factory's own, built on first use and kept, so the TLS
/// session store inside it outlives a reconnect. `via` names the path in an
/// error, such as "to 157.240.0.1:443" or "through the proxy".
pub async fn websocket(
    connector: &OnceLock<Connector>,
    uri: http::Uri,
    host: &str,
    stream: TcpStream,
    via: &str,
) -> Result<Link, anyhow::Error> {
    let stream = connector
        .get_or_init(default_tls_connector)
        .wrap(host, stream)
        .await
        .map_err(|error| {
            // The library's own line about this is reduced to a category, so
            // the TLS error itself is kept here, under our own target.
            log::warn!("TLS {via} failed: {error}");
            anyhow::anyhow!("TLS {via} failed: {error}")
        })?;
    let (ws, _) = tokio_websockets::ClientBuilder::from_uri(uri)
        .add_header(
            http::header::ORIGIN,
            http::HeaderValue::from_static(WHATSAPP_WEB_ORIGIN),
        )?
        .connect_on(stream)
        .await
        .map_err(|error| {
            log::warn!("WebSocket connect {via} failed: {error}");
            anyhow::anyhow!("WebSocket connect {via} failed: {error}")
        })?;
    Ok(from_websocket(ws))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn v6(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)
    }

    fn v4(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    #[test]
    fn interleaves_the_families() {
        let (v6a, v6b, v4a) = (v6(1), v6(2), v4(3));
        assert_eq!(interleave(vec![v6a, v6b, v4a]), vec![v6a, v4a, v6b]);
        assert_eq!(interleave(vec![v4a, v6a, v6b]), vec![v4a, v6a, v6b]);
        assert_eq!(interleave(vec![v6a, v6b]), vec![v6a, v6b]);
        assert_eq!(interleave(vec![v4a]), vec![v4a]);
        assert!(interleave(Vec::new()).is_empty());
    }

    #[tokio::test]
    async fn dials_the_next_address_when_the_first_refuses() {
        let live = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live_addr = live.local_addr().unwrap();
        // A port nothing listens on: bind one, then let it go.
        let dead_addr = {
            let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
            dead.local_addr().unwrap()
        };

        let (mut stream, addr) = dial(vec![dead_addr, live_addr]).await.unwrap();

        assert_eq!(addr, live_addr);
        // The stream that came back is the live one, not a loser left open.
        let (mut peer, _) = live.accept().await.unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        peer.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
    }

    #[tokio::test]
    async fn starts_the_next_address_before_the_first_gives_up() {
        let slow = v6(1);
        let fast = v4(2);
        let started = Instant::now();

        let (value, addr) = race(vec![slow, fast], HEAD_START, move |addr| async move {
            if addr == slow {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Ok(addr)
        })
        .await
        .unwrap();

        assert_eq!(addr, fast);
        assert_eq!(value, fast);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the second address waited for the first instead of racing it"
        );
    }

    #[tokio::test]
    async fn names_every_address_it_tried_when_none_answers() {
        let (first, second) = (v6(1), v4(2));

        let error = race(vec![first, second], HEAD_START, move |addr| async move {
            Err::<SocketAddr, _>(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("refused {addr}"),
            ))
        })
        .await
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains(&first.to_string()), "{message}");
        assert!(message.contains(&second.to_string()), "{message}");
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
    }
}
