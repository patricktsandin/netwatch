//! A pinned UDP socket for DNS probes.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;

use netwatch_probing::{ProbeSequence, SendFailure};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::dns;
use crate::socket::{Received, register, send_failure};

/// One DNS probe stream's socket: connected to one server on port 53, pinned to one
/// interface (ADR-0024). Connecting means the kernel only delivers datagrams from
/// that server.
#[derive(Debug)]
pub struct DnsSocket {
    socket: AsyncFd<Socket>,
    server: IpAddr,
}

impl DnsSocket {
    /// Opens a socket to `server`, pinned to interface `pin`.
    pub fn open(server: IpAddr, pin: NonZeroU32) -> io::Result<Self> {
        let domain = if server.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };
        let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
        if server.is_ipv4() {
            socket.bind_device_by_index_v4(Some(pin))?;
        } else {
            socket.bind_device_by_index_v6(Some(pin))?;
        }
        socket.connect(&SocketAddr::new(server, 53).into())?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket: register(socket, Interest::READABLE)?,
            server,
        })
    }

    /// Sends one query with `sequence` as its ID.
    pub fn send(&self, sequence: ProbeSequence) -> Result<(), SendFailure> {
        self.socket
            .get_ref()
            .send(&dns::query(sequence))
            .map(drop)
            .map_err(send_failure)
    }

    /// Waits for the next response to one of our queries.
    pub async fn recv(&self) -> io::Result<Received> {
        let mut buffer = [0u8; 1500];
        loop {
            let mut guard = self.socket.readable().await?;
            let received = guard.try_io(|socket| {
                let mut reader = socket.get_ref();
                io::Read::read(&mut reader, &mut buffer)
            });
            match received {
                Ok(Ok(len)) => {
                    if let Some(reply) = dns::parse_response(&buffer[..len]) {
                        return Ok(Received {
                            reply,
                            responder: Some(self.server),
                        });
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(_would_block) => {}
            }
        }
    }
}
