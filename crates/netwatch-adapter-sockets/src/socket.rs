//! A pinned, unprivileged ICMP datagram socket.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;

use netwatch_probing::{ProbeIndex, ProbeSequence, Reply, SendFailure};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::unix::AsyncFd;

use crate::icmp::{self, Family};

/// One echo probe stream's socket. It sends to a single destination, through a single
/// interface.
///
/// The socket is bound to the pinned interface with `SO_BINDTOIFINDEX`, which needs no
/// privileges since Linux 5.7, so the kernel can't send it anywhere else (ADR-0024).
/// If the interface disappears, sends fail, and the failure becomes the
/// `SendFailed` outcome, which is true.
#[derive(Debug)]
pub struct EchoSocket {
    socket: AsyncFd<Socket>,
    destination: SocketAddr,
    family: Family,
}

impl EchoSocket {
    /// Opens a socket to `destination`, pinned to interface `pin`. Fails if ICMP
    /// datagram sockets aren't permitted (`net.ipv4.ping_group_range`).
    pub fn open(destination: IpAddr, pin: NonZeroU32) -> io::Result<Self> {
        let (domain, protocol, family) = match destination {
            IpAddr::V4(_) => (Domain::IPV4, Protocol::ICMPV4, Family::V4),
            IpAddr::V6(_) => (Domain::IPV6, Protocol::ICMPV6, Family::V6),
        };
        let socket = Socket::new(domain, Type::DGRAM, Some(protocol))?;
        match family {
            Family::V4 => socket.bind_device_by_index_v4(Some(pin))?,
            Family::V6 => socket.bind_device_by_index_v6(Some(pin))?,
        }
        socket.set_nonblocking(true)?;
        // SAFETY: the `AsyncFd` takes ownership of the `Socket`, and nothing else holds
        // its descriptor, so the descriptor stays open and refers to the same socket
        // for as long as the `AsyncFd` exists, which is all `register` requires.
        #[allow(unsafe_code)]
        let socket = unsafe { AsyncFd::register(socket) }.map_err(io::Error::from)?;
        Ok(Self {
            socket,
            destination: SocketAddr::new(destination, 0),
            family,
        })
    }

    /// Sends one echo request. A failure is returned as the domain's
    /// [`SendFailure`], never as an error: a failed send is an outcome
    /// (ADR-0015).
    pub fn send(&self, sequence: ProbeSequence, token: ProbeIndex) -> Result<(), SendFailure> {
        let packet = icmp::echo_request(self.family, sequence, token);
        match self
            .socket
            .get_ref()
            .send_to(&packet, &self.destination.into())
        {
            Ok(_) => Ok(()),
            Err(error) => Err(match error.raw_os_error() {
                Some(libc::ENODEV | libc::ENXIO) => SendFailure::InterfaceGone,
                Some(libc::ENETUNREACH | libc::EHOSTUNREACH) => SendFailure::NoRoute,
                Some(errno) => SendFailure::Other(errno),
                None => SendFailure::Other(0),
            }),
        }
    }

    /// Waits for the next packet that parses as an echo reply. Other packets are
    /// skipped.
    pub async fn recv(&self) -> io::Result<Reply> {
        let mut buffer = [0u8; 1500];
        loop {
            let mut guard = self.socket.readable().await?;
            let received = guard.try_io(|socket| {
                // `Read` for `&Socket` fills an initialized buffer, which avoids
                // socket2's `MaybeUninit`-based `recv`.
                io::Read::read(&mut socket.get_ref(), &mut buffer)
            });
            match received {
                Ok(Ok(len)) => {
                    if let Some(reply) = icmp::parse_echo_reply(self.family, &buffer[..len]) {
                        return Ok(reply);
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(_would_block) => {}
            }
        }
    }
}
