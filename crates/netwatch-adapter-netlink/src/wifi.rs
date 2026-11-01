//! The Wi-Fi radio role's source (ADR-0036): nl80211, through `wl-nl80211`.
//!
//! Each call opens its own generic-netlink connection, lists the wireless interfaces,
//! picks one, and reads its station info: for a client, the one station is its
//! access point. nl80211's attributes become a [`WifiLink`] here, and go no further.

use std::io;
use std::num::NonZeroU32;

use futures_util::TryStreamExt;
use netwatch_host_health::radio::{Association, WifiLink};
use netwatch_shared::{InterfaceIndex, InterfaceName};
use wl_nl80211::{Nl80211Attr, Nl80211InterfaceType, Nl80211RateInfo, Nl80211StationInfo};

/// The Wi-Fi device the radio role binds to, and its link.
#[derive(Debug, Clone, PartialEq)]
pub struct WifiDevice {
    /// The interface's index.
    pub index: InterfaceIndex,
    /// The interface's name.
    pub name: InterfaceName,
    /// What it's associated with, and how well.
    pub link: WifiLink,
}

fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// Reads the Wi-Fi device: the one with index `prefer` if that is a wireless client
/// (it carries the egress path), otherwise the lowest-numbered one. `None` if this
/// host has no wireless client interface.
pub async fn wifi(prefer: Option<NonZeroU32>) -> io::Result<Option<WifiDevice>> {
    let (connection, handle, _) = wl_nl80211::new_connection()?;
    let driver = tokio::spawn(connection);
    let result = read(&handle, prefer).await;
    driver.abort();
    result
}

async fn read(
    handle: &wl_nl80211::Nl80211Handle,
    prefer: Option<NonZeroU32>,
) -> io::Result<Option<WifiDevice>> {
    let interfaces: Vec<_> = handle
        .interface()
        .get(Vec::new())
        .execute()
        .await
        .try_collect()
        .await
        .map_err(other)?;
    let mut clients: Vec<(u32, String)> = interfaces
        .iter()
        .filter_map(|message| {
            let attributes = &message.payload.attributes;
            let station = attributes
                .iter()
                .any(|a| matches!(a, Nl80211Attr::IfType(Nl80211InterfaceType::Station)));
            let index = attributes.iter().find_map(|a| match a {
                Nl80211Attr::IfIndex(i) => Some(*i),
                _ => None,
            })?;
            let name = attributes.iter().find_map(|a| match a {
                Nl80211Attr::IfName(n) => Some(n.clone()),
                _ => None,
            })?;
            station.then_some((index, name))
        })
        .collect();
    clients.sort();
    let chosen = clients
        .iter()
        .find(|(i, _)| Some(*i) == prefer.map(NonZeroU32::get))
        .or_else(|| clients.first());
    let Some((index, name)) = chosen.cloned() else {
        return Ok(None);
    };
    let (Some(index), Ok(name)) = (
        NonZeroU32::new(index),
        InterfaceName::parse(name.as_bytes()),
    ) else {
        return Ok(None);
    };
    // A station dump of a client lists its access point, if it's associated. A
    // device that is down answers with an error, which is "not associated" too.
    let stations: Vec<_> = handle
        .station()
        .dump(index.get())
        .execute()
        .await
        .try_collect()
        .await
        .unwrap_or_default();
    let associated = stations.iter().find_map(|message| {
        let attributes = &message.payload.attributes;
        let bssid = attributes.iter().find_map(|a| match a {
            Nl80211Attr::Mac(mac) => Some(*mac),
            _ => None,
        })?;
        let info = attributes.iter().find_map(|a| match a {
            Nl80211Attr::StationInfo(info) => Some(info),
            _ => None,
        })?;
        Some(association(bssid, info))
    });
    Ok(Some(WifiDevice {
        index: InterfaceIndex::new(index),
        name,
        link: WifiLink { associated },
    }))
}

fn association(bssid: [u8; 6], info: &[Nl80211StationInfo]) -> Association {
    let signal = info
        .iter()
        .find_map(|i| match i {
            Nl80211StationInfo::SignalAvg(s) => Some(*s),
            _ => None,
        })
        .or_else(|| {
            info.iter().find_map(|i| match i {
                Nl80211StationInfo::Signal(s) => Some(*s),
                _ => None,
            })
        });
    let rate = |rates: &[Nl80211RateInfo]| {
        // Both are in units of 100 kbit/s; the 32-bit one exists for rates above
        // 6.5 Gbit/s, so it wins when present.
        let units = rates
            .iter()
            .find_map(|r| match r {
                Nl80211RateInfo::Bitrate32(b) => Some(*b),
                _ => None,
            })
            .or_else(|| {
                rates.iter().find_map(|r| match r {
                    Nl80211RateInfo::Bitrate(b) => Some(u32::from(*b)),
                    _ => None,
                })
            })?;
        Some(f64::from(units) * 100_000.0)
    };
    Association {
        bssid,
        signal_dbm: signal,
        tx_bitrate_bps: info.iter().find_map(|i| match i {
            Nl80211StationInfo::TxBitrate(r) => rate(r),
            _ => None,
        }),
        rx_bitrate_bps: info.iter().find_map(|i| match i {
            Nl80211StationInfo::RxBitrate(r) => rate(r),
            _ => None,
        }),
    }
}
