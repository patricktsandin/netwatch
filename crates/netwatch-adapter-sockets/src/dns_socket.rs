//! A pinned UDP socket for DNS probes.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;

use netwatch_probing::{ProbeSequence, SendFailure};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::dns;
use crate::icmp::Family;
use crate::socket::{
    Received, enable_recverr, is_pending_icmp_error, read_error, register, send_once,
};

/// One DNS probe stream's socket: connected to one server on port 53, pinned to one
/// interface (ADR-0024). Connecting means the kernel only delivers datagrams from
/// that server. `IP_RECVERR` / `IPV6_RECVERR` is enabled, so an ICMP error for a
/// query arrives on the error queue with the query it answers (ADR-0049).
#[derive(Debug)]
pub struct DnsSocket {
    socket: AsyncFd<Socket>,
    server: IpAddr,
    family: Family,
}

impl DnsSocket {
    /// Opens a socket to `server`, pinned to interface `pin`.
    pub fn open(server: IpAddr, pin: NonZeroU32) -> io::Result<Self> {
        let (domain, family) = if server.is_ipv4() {
            (Domain::IPV4, Family::V4)
        } else {
            (Domain::IPV6, Family::V6)
        };
        let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
        if server.is_ipv4() {
            socket.bind_device_by_index_v4(Some(pin))?;
        } else {
            socket.bind_device_by_index_v6(Some(pin))?;
        }
        enable_recverr(&socket, family)?;
        socket.connect(&SocketAddr::new(server, 53).into())?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket: register(socket, Interest::READABLE | Interest::ERROR)?,
            server,
            family,
        })
    }

    /// Sends one query with `sequence` as its ID.
    pub fn send(&self, sequence: ProbeSequence) -> Result<(), SendFailure> {
        send_once(|| self.socket.get_ref().send(&dns::query(sequence)))
    }

    /// Waits for the next response to one of our queries, or ICMP error that
    /// answers one. Anything else is skipped.
    pub async fn recv(&self) -> io::Result<Received> {
        loop {
            let mut guard = self
                .socket
                .ready(Interest::READABLE | Interest::ERROR)
                .await?;
            let received = guard.try_io(|socket| {
                let own = |quote: &[u8]| dns::parse_own_query(quote).map(|s| (s, None));
                match read_error(socket.get_ref(), self.family, own) {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        self.read_response(socket.get_ref())
                    }
                    other => other,
                }
            });
            match received {
                Ok(Ok(Some(received))) => return Ok(received),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => return Err(error),
                Err(_would_block) => {}
            }
        }
    }

    fn read_response(&self, socket: &Socket) -> io::Result<Option<Received>> {
        let mut buffer = [0u8; 1500];
        let mut reader = socket;
        match io::Read::read(&mut reader, &mut buffer) {
            Ok(len) => Ok(dns::parse_response(&buffer[..len]).map(|reply| Received {
                reply,
                responder: Some(self.server),
            })),
            Err(e) if is_pending_icmp_error(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
