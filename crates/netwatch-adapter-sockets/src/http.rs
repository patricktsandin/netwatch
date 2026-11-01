//! HTTPS probes with phase timings (ADR-0028).
//!
//! High-level HTTP clients hide where time goes, so this drives each phase by hand:
//! resolve the name, connect TCP through the pinned interface, perform the TLS
//! handshake, and send `GET` over HTTP/1.1, timing each step. The response must match
//! the target's expected response. A captive portal answering 200 is a failure, not a
//! success (ADR-0009).

use std::io;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use netwatch_shared::InterfaceName;
use tokio::net::TcpSocket;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

/// What an HTTPS target must answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// Exactly this status, and any body.
    Status(u16),
    /// Exactly this status, with a body containing this text.
    StatusWithBody(u16, String),
}

/// One HTTPS endpoint built for connectivity checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsTarget {
    /// The host name, also used for SNI and certificate verification.
    pub host: String,
    /// The path to request, starting with `/`.
    pub path: String,
    /// What it must answer.
    pub expected: Expected,
}

/// Where the time went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseTimings {
    /// Name resolution.
    pub dns: Duration,
    /// The TCP handshake.
    pub connect: Duration,
    /// The TLS handshake.
    pub tls: Duration,
    /// From sending the request to receiving the response head.
    pub first_byte: Duration,
    /// From the start to the end of the body.
    pub total: Duration,
}

/// How an HTTPS probe ended.
#[derive(Debug)]
pub enum HttpsResult {
    /// The expected response arrived.
    Answered(PhaseTimings),
    /// A response arrived, but not the expected one.
    Unexpected {
        /// The status that arrived.
        status: u16,
    },
    /// The exchange failed before any response: name resolution, connection, or TLS.
    Failed(io::Error),
}

/// The largest body read while checking for the expected text.
const BODY_LIMIT: usize = 16 * 1024;

fn tls() -> &'static TlsConnector {
    static CONNECTOR: OnceLock<TlsConnector> = OnceLock::new();
    CONNECTOR.get_or_init(|| {
        let roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let config = ClientConfig::builder_with_provider(Arc::new(
            tokio_rustls::rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth());
        // The ring provider supports the default versions, so this can't fail.
        TlsConnector::from(Arc::new(config.expect("ring supports TLS 1.2 and 1.3")))
    })
}

/// GETs `https://{host}{path}` through interface `pin` and returns the body as text,
/// whatever the status: for pages that state facts, such as Cloudflare's trace
/// (ADR-0039).
pub async fn fetch_text(host: &str, path: &str, pin: InterfaceName) -> io::Result<String> {
    let fetched = fetch(host, path, &pin).await?;
    Ok(String::from_utf8_lossy(&fetched.body).into_owned())
}

/// Probes `target` once, through interface `pin`.
pub async fn probe(target: HttpsTarget, pin: InterfaceName) -> HttpsResult {
    match exchange(target, pin).await {
        Ok(result) => result,
        Err(error) => HttpsResult::Failed(error),
    }
}

/// One GET, timed phase by phase.
struct Fetched {
    start: Instant,
    resolved: Instant,
    connected: Instant,
    secured: Instant,
    first_byte: Instant,
    done: Instant,
    status: u16,
    body: Bytes,
}

/// GETs `https://{host}{path}` over IPv4 through interface `pin`, verifying the
/// certificate, and reads at most [`BODY_LIMIT`] of the body.
async fn fetch(host: &str, path: &str, pin: &InterfaceName) -> io::Result<Fetched> {
    let start = Instant::now();
    let address = tokio::net::lookup_host((host, 443))
        .await?
        .find(std::net::SocketAddr::is_ipv4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no IPv4 address"))?;
    let resolved = Instant::now();

    let socket = TcpSocket::new_v4()?;
    socket.bind_device(Some(pin.as_bytes()))?;
    let tcp = socket.connect(address).await?;
    let connected = Instant::now();

    let name = ServerName::try_from(host.to_owned())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let stream = tls().connect(name, tcp).await?;
    let secured = Instant::now();

    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
            .await
            .map_err(io::Error::other)?;
    let driver = tokio::spawn(connection);
    let request = hyper::Request::get(path)
        .header(hyper::header::HOST, host)
        .header(hyper::header::USER_AGENT, "netwatch")
        .body(Empty::new())
        .map_err(io::Error::other)?;
    let response = sender
        .send_request(request)
        .await
        .map_err(io::Error::other)?;
    let first_byte = Instant::now();
    let status = response.status().as_u16();
    let body = Limited::new(response.into_body(), BODY_LIMIT)
        .collect()
        .await
        .map_err(io::Error::other)?
        .to_bytes();
    let done = Instant::now();
    driver.abort();
    Ok(Fetched {
        start,
        resolved,
        connected,
        secured,
        first_byte,
        done,
        status,
        body,
    })
}

async fn exchange(target: HttpsTarget, pin: InterfaceName) -> io::Result<HttpsResult> {
    let Fetched {
        start,
        resolved,
        connected,
        secured,
        first_byte,
        done,
        status,
        body,
    } = fetch(&target.host, &target.path, &pin).await?;
    let matches = match target.expected {
        Expected::Status(expected) => status == expected,
        Expected::StatusWithBody(expected, ref text) => {
            status == expected && body.windows(text.len()).any(|w| w == text.as_bytes())
        }
    };
    Ok(if matches {
        HttpsResult::Answered(PhaseTimings {
            dns: resolved - start,
            connect: connected - resolved,
            tls: secured - connected,
            first_byte: first_byte - secured,
            total: done - start,
        })
    } else {
        HttpsResult::Unexpected { status }
    })
}
