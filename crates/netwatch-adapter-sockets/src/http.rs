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

use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use netwatch_probing::ExchangeStage;
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
    /// Exactly this status, with a body containing this text within its first
    /// [`BODY_LIMIT`] bytes. The text is not empty.
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
    /// The exchange failed beyond this host, before any response (ADR-0052).
    Failed(ExchangeStage, io::Error),
    /// This host couldn't make the request: the socket couldn't be created or pinned.
    NotSent(io::Error),
}

/// How a fetch failed: at a stage of the exchange, or on this host (`None`).
type Failure = (Option<ExchangeStage>, io::Error);

/// Which addresses a fetch may connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Families {
    /// The first address the resolver gives, of either family.
    Any,
    /// The first IPv4 address.
    Ipv4,
}

/// How much of the body is read while checking for the expected text. A longer body
/// is not an error: the rest is left unread.
pub const BODY_LIMIT: usize = 16 * 1024;

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

/// GETs `https://{host}{path}` over IPv4 through interface `pin` and returns the body
/// as text, whatever the status: for pages that state facts about the IPv4 egress,
/// such as Cloudflare's trace (ADR-0039). Gives up after `limit`.
pub async fn fetch_text(
    host: &str,
    path: &str,
    pin: InterfaceName,
    limit: Duration,
) -> io::Result<String> {
    let fetched = tokio::time::timeout(limit, fetch(host, path, &pin, Families::Ipv4))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no answer in time"))?
        .map_err(|(_, error)| error)?;
    Ok(String::from_utf8_lossy(&fetched.body).into_owned())
}

/// Probes `target` once, through interface `pin`. The caller bounds its time: the
/// probe stream's timeout aborts it (ADR-0028).
pub async fn probe(target: HttpsTarget, pin: InterfaceName) -> HttpsResult {
    match exchange(target, pin).await {
        Ok(result) => result,
        Err((Some(stage), error)) => HttpsResult::Failed(stage, error),
        Err((None, error)) => HttpsResult::NotSent(error),
    }
}

/// A failure of the HTTP request, after the connection was up.
fn request_failed(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Failure {
    (Some(ExchangeStage::Request), io::Error::other(error))
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
    body: Vec<u8>,
}

/// GETs `https://{host}{path}` through interface `pin`, verifying the certificate,
/// and reads at most [`BODY_LIMIT`] of the body. A failure says where it happened.
async fn fetch(
    host: &str,
    path: &str,
    pin: &InterfaceName,
    families: Families,
) -> Result<Fetched, Failure> {
    let at = |stage| move |error| (Some(stage), error);
    let start = Instant::now();
    let addresses: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, 443))
        .await
        .map_err(at(ExchangeStage::Resolve))?
        .filter(|a| families == Families::Any || a.is_ipv4())
        .collect();
    let resolved = Instant::now();

    // In the resolver's order, the next address when one can't be connected to,
    // such as an IPv6 address on an egress with no IPv6 route.
    let mut failed = (
        Some(ExchangeStage::Resolve),
        io::Error::new(io::ErrorKind::NotFound, "no address to connect to"),
    );
    let mut connection = None;
    for address in addresses {
        let local = |error| (None, error);
        let socket = if address.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }
        .map_err(local)?;
        socket.bind_device(Some(pin.as_bytes())).map_err(local)?;
        match socket.connect(address).await {
            Ok(tcp) => {
                connection = Some(tcp);
                break;
            }
            Err(error) => failed = (Some(ExchangeStage::Connect), error),
        }
    }
    let tcp = connection.ok_or(failed)?;
    let connected = Instant::now();

    let name = ServerName::try_from(host.to_owned())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
        .map_err(at(ExchangeStage::Tls))?;
    let stream = tls()
        .connect(name, tcp)
        .await
        .map_err(at(ExchangeStage::Tls))?;
    let secured = Instant::now();

    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
            .await
            .map_err(request_failed)?;
    let driver = tokio::spawn(connection);
    let request = hyper::Request::get(path)
        .header(hyper::header::HOST, host)
        .header(hyper::header::USER_AGENT, "netwatch")
        .body(Empty::new())
        .map_err(request_failed)?;
    let response = sender.send_request(request).await.map_err(request_failed)?;
    let first_byte = Instant::now();
    let status = response.status().as_u16();
    let body = head(response.into_body())
        .await
        .map_err(at(ExchangeStage::Request))?;
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

/// The body's first [`BODY_LIMIT`] bytes, or all of it if it is shorter.
async fn head(mut body: hyper::body::Incoming) -> io::Result<Vec<u8>> {
    let mut head = Vec::new();
    while head.len() < BODY_LIMIT {
        let Some(frame) = body.frame().await else {
            break;
        };
        if let Ok(data) = frame.map_err(io::Error::other)?.into_data() {
            let room = BODY_LIMIT - head.len();
            head.extend_from_slice(&data[..data.len().min(room)]);
        }
    }
    Ok(head)
}

async fn exchange(target: HttpsTarget, pin: InterfaceName) -> Result<HttpsResult, Failure> {
    let Fetched {
        start,
        resolved,
        connected,
        secured,
        first_byte,
        done,
        status,
        body,
    } = fetch(&target.host, &target.path, &pin, Families::Any).await?;
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
