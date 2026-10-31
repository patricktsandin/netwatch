//! A pinned, unprivileged ICMP datagram socket that also receives ICMP errors.

use std::io;
use std::mem::MaybeUninit;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::os::fd::AsRawFd;

use netwatch_probing::{ProbeIndex, ProbeSequence, Reply, SendFailure};
use netwatch_shared::HopTtl;
use socket2::{Domain, MaybeUninitSlice, MsgHdrMut, Protocol, Socket, Type};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::errqueue;
use crate::icmp::{self, Family};

/// What came back for a probe, and who sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Received {
    /// The reply, matched to its probe by sequence number and token.
    pub reply: Reply,
    /// The host that answered: the destination for an echo reply, or the router
    /// that sent an ICMP error. A hop's identity (ADR-0025).
    pub responder: Option<IpAddr>,
}

/// One probe stream's socket. It sends to a single destination, through a single
/// interface, optionally with a limited TTL (a hop probe).
///
/// Pinned with `SO_BINDTOIFINDEX` (ADR-0024). `IP_RECVERR` / `IPV6_RECVERR` is enabled,
/// so ICMP errors that answer our probes (time exceeded, unreachable) arrive on the
/// error queue together with our own original packet, and are matched by its exact
/// sequence number and token.
#[derive(Debug)]
pub struct ProbeSocket {
    socket: AsyncFd<Socket>,
    destination: SocketAddr,
    family: Family,
}

impl ProbeSocket {
    /// Opens a socket to `destination`, pinned to interface `pin`, with the hop
    /// limit `ttl` if given. Fails if ICMP datagram sockets aren't permitted
    /// (`net.ipv4.ping_group_range`).
    pub fn open(destination: IpAddr, pin: NonZeroU32, ttl: Option<HopTtl>) -> io::Result<Self> {
        let (domain, protocol, family) = match destination {
            IpAddr::V4(_) => (Domain::IPV4, Protocol::ICMPV4, Family::V4),
            IpAddr::V6(_) => (Domain::IPV6, Protocol::ICMPV6, Family::V6),
        };
        let socket = Socket::new(domain, Type::DGRAM, Some(protocol))?;
        match family {
            Family::V4 => socket.bind_device_by_index_v4(Some(pin))?,
            Family::V6 => socket.bind_device_by_index_v6(Some(pin))?,
        }
        if let Some(ttl) = ttl {
            let hops = u32::from(ttl.get().get());
            match family {
                Family::V4 => socket.set_ttl_v4(hops)?,
                Family::V6 => socket.set_unicast_hops_v6(hops)?,
            }
        }
        enable_recverr(&socket, family)?;
        socket.set_nonblocking(true)?;
        let socket = register(socket, Interest::READABLE | Interest::ERROR)?;
        Ok(Self {
            socket,
            destination: SocketAddr::new(destination, 0),
            family,
        })
    }

    /// Sends one echo request. A failure is returned as the domain's
    /// [`SendFailure`], never as an error: a failed send is an outcome (ADR-0015).
    pub fn send(&self, sequence: ProbeSequence, token: ProbeIndex) -> Result<(), SendFailure> {
        let packet = icmp::echo_request(self.family, sequence, token);
        self.socket
            .get_ref()
            .send_to(&packet, &self.destination.into())
            .map(drop)
            .map_err(send_failure)
    }

    /// Waits for the next reply or ICMP error that answers one of our probes.
    /// Anything else is skipped.
    pub async fn recv(&self) -> io::Result<Received> {
        loop {
            let mut guard = self
                .socket
                .ready(Interest::READABLE | Interest::ERROR)
                .await?;
            let received = guard.try_io(|socket| match self.read_error(socket.get_ref()) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.read_reply(socket.get_ref())
                }
                other => other,
            });
            match received {
                Ok(Ok(Some(received))) => return Ok(received),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => return Err(error),
                Err(_would_block) => {}
            }
        }
    }

    fn read_reply(&self, socket: &Socket) -> io::Result<Option<Received>> {
        let mut buffer = [0u8; 1500];
        // `Read` for `&Socket` fills an initialized buffer, which avoids socket2's
        // `MaybeUninit`-based `recv`.
        let mut reader = socket;
        let len = io::Read::read(&mut reader, &mut buffer)?;
        Ok(
            icmp::parse_echo_reply(self.family, &buffer[..len]).map(|reply| Received {
                reply,
                responder: Some(self.destination.ip()),
            }),
        )
    }

    fn read_error(&self, socket: &Socket) -> io::Result<Option<Received>> {
        let mut data = [MaybeUninit::<u8>::uninit(); 256];
        let mut control = [MaybeUninit::<u8>::uninit(); 256];
        let mut buffers = [MaybeUninitSlice::new(&mut data)];
        let mut message = MsgHdrMut::new()
            .with_buffers(&mut buffers)
            .with_control(&mut control);
        let len = socket.recvmsg(&mut message, libc::MSG_ERRQUEUE)?;
        let control_len = message.control_len();
        let data: Vec<u8> = initialized(&data, len);
        let control: Vec<u8> = initialized(&control, control_len);
        let Some(error) = errqueue::parse(&control) else {
            return Ok(None);
        };
        let Some(kind) = icmp::error_kind(self.family, error.icmp_type, error.icmp_code) else {
            return Ok(None);
        };
        Ok(
            icmp::parse_own_request(self.family, &data).map(|(sequence, token)| Received {
                reply: Reply {
                    sequence,
                    token: Some(token),
                    kind,
                },
                responder: error.offender,
            }),
        )
    }
}

/// The domain's reason for a failed send (ADR-0015: a failed send is an outcome).
pub(crate) fn send_failure(error: io::Error) -> SendFailure {
    match error.raw_os_error() {
        Some(libc::ENODEV | libc::ENXIO) => SendFailure::InterfaceGone,
        Some(libc::ENETUNREACH | libc::EHOSTUNREACH) => SendFailure::NoRoute,
        Some(errno) => SendFailure::Other(errno),
        None => SendFailure::Other(0),
    }
}

/// Wraps a socket for readiness notifications.
pub(crate) fn register(socket: Socket, interest: Interest) -> io::Result<AsyncFd<Socket>> {
    // SAFETY: the `AsyncFd` takes ownership of the `Socket`, and nothing else holds
    // its descriptor, so the descriptor stays open and refers to the same socket
    // for as long as the `AsyncFd` exists, which is all `register` requires.
    #[allow(unsafe_code)]
    unsafe { AsyncFd::register_with_interest(socket, interest) }.map_err(io::Error::from)
}

/// The first `len` bytes of a buffer the kernel has just filled.
fn initialized(buffer: &[MaybeUninit<u8>], len: usize) -> Vec<u8> {
    // SAFETY: callers pass the length the kernel reported writing, so these bytes
    // are initialized.
    #[allow(unsafe_code)]
    buffer[..len.min(buffer.len())]
        .iter()
        .map(|b| unsafe { b.assume_init() })
        .collect()
}

fn enable_recverr(socket: &Socket, family: Family) -> io::Result<()> {
    let (level, option) = match family {
        Family::V4 => (libc::SOL_IP, libc::IP_RECVERR),
        Family::V6 => (libc::SOL_IPV6, libc::IPV6_RECVERR),
    };
    let on: libc::c_int = 1;
    // SAFETY: the descriptor is a valid, open socket owned by `socket`; the option
    // value points to a live `c_int`, and its exact size is passed.
    #[allow(unsafe_code)]
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            level,
            option,
            (&raw const on).cast(),
            libc::socklen_t::try_from(size_of::<libc::c_int>()).unwrap_or(4),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
