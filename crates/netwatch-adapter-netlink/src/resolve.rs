//! `RTM_GETROUTE` and `RTM_GETLINK` requests, parsed into an [`EgressPath`].

use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;

use netlink_packet_core::{NLM_F_REQUEST, NetlinkHeader, NetlinkMessage, NetlinkPayload};
use netlink_packet_route::link::{InfoKind, LinkAttribute, LinkInfo, LinkMessage};
use netlink_packet_route::route::{RouteAddress, RouteAttribute, RouteMessage};
use netlink_packet_route::{AddressFamily, RouteNetlinkMessage};
use netlink_sys::{Socket, SocketAddr, protocols::NETLINK_ROUTE};
use netwatch_shared::{InterfaceIndex, InterfaceName};
use netwatch_topology::{EgressPath, InterfaceKind};

/// The destination whose route defines the global roles (ADR-0022). Any address
/// that the default route covers would do. This one is also a default target.
const REFERENCE: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));

/// Asks the kernel how it would route to the reference destination right now.
/// `None` means there is no route at all, for example when every link is down.
pub fn resolve_egress() -> io::Result<Option<EgressPath>> {
    let socket = connect()?;
    let Some(route) = get_route(&socket, REFERENCE)? else {
        return Ok(None);
    };
    let mut oif = None;
    let mut source = None;
    for attribute in &route.attributes {
        match attribute {
            RouteAttribute::Oif(index) => oif = NonZeroU32::new(*index),
            RouteAttribute::PrefSource(address) => source = ip(address),
            _ => {}
        }
    }
    let Some(oif) = oif else {
        return Ok(None);
    };
    let link = get_link(&socket, oif)?;
    let mut name = None;
    let mut kind = InterfaceKind::Physical;
    for attribute in &link.attributes {
        match attribute {
            LinkAttribute::IfName(n) => {
                name = Some(InterfaceName::parse(n.as_bytes()).map_err(invalid_data)?);
            }
            LinkAttribute::LinkInfo(infos)
                if infos.iter().any(|info| {
                    matches!(info, LinkInfo::Kind(InfoKind::Wireguard | InfoKind::Tun))
                }) =>
            {
                kind = InterfaceKind::Tunnel;
            }
            _ => {}
        }
    }
    let name = name.ok_or_else(|| invalid_data("RTM_NEWLINK reply has no IFLA_IFNAME"))?;
    Ok(Some(EgressPath {
        index: InterfaceIndex::new(oif),
        name,
        kind,
        source,
    }))
}

/// The device counters (`IFLA_STATS64`) of interface `index`, by `Link.Name`, for the
/// NIC layer's health (ADR-0008) and throughput (ADR-0032).
pub fn link_counters(index: NonZeroU32) -> io::Result<BTreeMap<String, u64>> {
    let link = get_link(&connect()?, index)?;
    let stats = link
        .attributes
        .iter()
        .find_map(|a| match a {
            LinkAttribute::Stats64(stats) => Some(stats),
            _ => None,
        })
        .ok_or_else(|| invalid_data("RTM_NEWLINK reply has no IFLA_STATS64"))?;
    Ok(BTreeMap::from(
        [
            ("RxBytes", stats.rx_bytes),
            ("TxBytes", stats.tx_bytes),
            ("RxErrors", stats.rx_errors),
            ("TxErrors", stats.tx_errors),
            ("RxDropped", stats.rx_dropped),
            ("TxDropped", stats.tx_dropped),
            ("Collisions", stats.collisions),
            ("RxCrcErrors", stats.rx_crc_errors),
            ("RxFrameErrors", stats.rx_frame_errors),
            ("RxFifoErrors", stats.rx_fifo_errors),
            ("RxMissedErrors", stats.rx_missed_errors),
            ("TxCarrierErrors", stats.tx_carrier_errors),
        ]
        .map(|(name, value)| (format!("Link.{name}"), value)),
    ))
}

fn connect() -> io::Result<Socket> {
    let mut socket = Socket::new(NETLINK_ROUTE)?;
    socket.bind_auto()?;
    socket.connect(&SocketAddr::new(0, 0))?;
    Ok(socket)
}

fn get_route(socket: &Socket, destination: IpAddr) -> io::Result<Option<RouteMessage>> {
    let mut message = RouteMessage::default();
    message.header.address_family = match destination {
        IpAddr::V4(_) => AddressFamily::Inet,
        IpAddr::V6(_) => AddressFamily::Inet6,
    };
    message.header.destination_prefix_length = match destination {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    message
        .attributes
        .push(RouteAttribute::Destination(RouteAddress::from(destination)));
    match request(socket, RouteNetlinkMessage::GetRoute(message))? {
        Reply::Message(RouteNetlinkMessage::NewRoute(route)) => Ok(Some(route)),
        Reply::Error(code) if code == libc::ENETUNREACH || code == libc::EHOSTUNREACH => Ok(None),
        Reply::Error(code) => Err(io::Error::from_raw_os_error(code)),
        Reply::Message(other) => Err(invalid_data(format!(
            "unexpected reply to RTM_GETROUTE: {other:?}"
        ))),
    }
}

fn get_link(socket: &Socket, index: NonZeroU32) -> io::Result<LinkMessage> {
    let mut message = LinkMessage::default();
    message.header.index = index.get();
    match request(socket, RouteNetlinkMessage::GetLink(message))? {
        Reply::Message(RouteNetlinkMessage::NewLink(link)) => Ok(link),
        Reply::Error(code) => Err(io::Error::from_raw_os_error(code)),
        Reply::Message(other) => Err(invalid_data(format!(
            "unexpected reply to RTM_GETLINK: {other:?}"
        ))),
    }
}

enum Reply {
    Message(RouteNetlinkMessage),
    /// A positive errno.
    Error(i32),
}

/// Sends one request and reads its single reply.
fn request(socket: &Socket, payload: RouteNetlinkMessage) -> io::Result<Reply> {
    let mut header = NetlinkHeader::default();
    header.flags = NLM_F_REQUEST;
    let mut message = NetlinkMessage::new(header, NetlinkPayload::from(payload));
    message.finalize();
    let mut buffer = vec![0; message.buffer_len()];
    message.serialize(&mut buffer);
    socket.send(&buffer, 0)?;

    let (reply, _) = socket.recv_from_full()?;
    let reply = NetlinkMessage::<RouteNetlinkMessage>::deserialize(&reply).map_err(invalid_data)?;
    match reply.payload {
        NetlinkPayload::InnerMessage(inner) => Ok(Reply::Message(inner)),
        NetlinkPayload::Error(error) => match error.code {
            Some(code) => Ok(Reply::Error(code.get().abs())),
            None => Err(invalid_data("unexpected ACK instead of a reply")),
        },
        other => Err(invalid_data(format!(
            "unexpected netlink payload: {other:?}"
        ))),
    }
}

fn ip(address: &RouteAddress) -> Option<IpAddr> {
    match address {
        RouteAddress::Inet(v4) => Some(IpAddr::V4(*v4)),
        RouteAddress::Inet6(v6) => Some(IpAddr::V6(*v6)),
        _ => None,
    }
}

fn invalid_data(error: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
