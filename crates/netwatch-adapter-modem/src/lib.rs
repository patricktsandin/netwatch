//! The cellular radio role's source (ADR-0036): ModemManager, over the system D-Bus.
//!
//! One `GetManagedObjects` call returns every modem with all its properties. The first
//! modem's become a [`CellularLink`] here, and go no further. Only state, signal,
//! access technology, operator name, and the network interface are read: never the
//! IMEI, IMSI, ICCID, or phone number.

use std::collections::HashMap;

use netwatch_host_health::radio::{CellularLink, ModemState, RadioAccess};
use zbus::{OwnedObjectPath, OwnedVariant as OwnedValue};

const SERVICE: &str = "org.freedesktop.ModemManager1";
const ROOT: &str = "/org/freedesktop/ModemManager1";
const MODEM: &str = "org.freedesktop.ModemManager1.Modem";
const MODEM_3GPP: &str = "org.freedesktop.ModemManager1.Modem.Modem3gpp";
const SIGNAL: &str = "org.freedesktop.ModemManager1.Modem.Signal";
/// `MM_MODEM_PORT_TYPE_NET`: the port that is a network interface.
const PORT_NET: u32 = 2;
/// How often ModemManager should refresh extended signal, in seconds.
const SIGNAL_RATE_S: u32 = 5;

type Properties = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Properties>>;

/// The modem the radio role binds to: its network interface's name, if it has one,
/// and its link.
#[derive(Debug, Clone, PartialEq)]
pub struct Modem {
    /// The network interface it provides, such as `wwan0`.
    pub interface: Option<String>,
    /// Its state and signal.
    pub link: CellularLink,
}

/// A connection to ModemManager, kept open between readings.
pub struct ModemManager {
    connection: zbus::Connection,
    /// The modem object extended signal was asked of. A modem detected again (after
    /// a suspend, a USB reset, or a ModemManager restart) is a new object, with its
    /// signal reporting off again, so it is asked again.
    signal_requested: Option<String>,
}

impl ModemManager {
    /// Connects to the system bus. Fails if there is no system bus; a missing
    /// ModemManager shows up later, as no modem.
    pub async fn connect() -> zbus::Result<Self> {
        Ok(Self {
            connection: zbus::Connection::system().await?,
            signal_requested: None,
        })
    }

    /// Reads the first modem. `None` if ModemManager isn't running or has no modem.
    pub async fn read(&mut self) -> Option<Modem> {
        let reply = self
            .connection
            .call_method(
                Some(SERVICE),
                ROOT,
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await
            .ok()?;
        let objects: Objects = reply.body().deserialize().ok()?;
        let mut modems: Vec<_> = objects.into_iter().collect();
        modems.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        let (path, interfaces) = modems.into_iter().find(|(_, i)| i.contains_key(MODEM))?;
        if self.signal_requested.as_deref() != Some(path.as_str()) {
            // Extended signal (RSRP, RSRQ, SNR) is only reported after a client asks
            // for it. Without permission, quality alone is graphed (ADR-0036).
            self.signal_requested = Some(path.as_str().to_owned());
            if let Err(error) = self
                .connection
                .call_method(
                    Some(SERVICE),
                    path.as_str(),
                    Some(SIGNAL),
                    "Setup",
                    &(SIGNAL_RATE_S,),
                )
                .await
            {
                tracing::info!(%error, "the modem won't report extended signal; graphing quality only");
            }
        }
        Some(modem(&interfaces))
    }
}

fn get<T: TryFrom<OwnedValue>>(properties: Option<&Properties>, name: &str) -> Option<T> {
    properties?.get(name)?.try_clone().ok()?.try_into().ok()
}

/// `MMModemState`, folded to what grading needs.
fn state(code: i32) -> ModemState {
    match code {
        -1 => ModemState::Failed,
        3 => ModemState::Disabled,
        6 | 7 => ModemState::Searching,
        8 | 9 => ModemState::Registered,
        10 | 11 => ModemState::Connected,
        _ => ModemState::Busy,
    }
}

/// The most capable technology in `MMModemAccessTechnology` flags.
fn access(flags: u32) -> Option<RadioAccess> {
    const NR5G: u32 = 1 << 15;
    const LTE: u32 = 1 << 14 | 1 << 16 | 1 << 17;
    const UMTS: u32 = 0b1_1111 << 5;
    const GSM: u32 = 0b1111 << 1;
    [
        (NR5G, RadioAccess::Nr5g),
        (LTE, RadioAccess::Lte),
        (UMTS, RadioAccess::Umts),
        (GSM, RadioAccess::Gsm),
    ]
    .into_iter()
    .find(|(mask, _)| flags & mask != 0)
    .map(|(_, access)| access)
}

fn modem(interfaces: &HashMap<String, Properties>) -> Modem {
    let modem = interfaces.get(MODEM);
    let gpp = interfaces.get(MODEM_3GPP);
    let signal = interfaces.get(SIGNAL);
    let quality: Option<(u32, bool)> = get(modem, "SignalQuality");
    // 5G NR's values when it reports them, LTE's otherwise.
    let extended = |name: &str| -> Option<f64> {
        ["Nr5g", "Lte"].into_iter().find_map(|technology| {
            let values: HashMap<String, OwnedValue> = get(signal, technology)?;
            let value: f64 = values.get(name)?.try_clone().ok()?.try_into().ok()?;
            // ModemManager reports unknown values as -inf or NaN.
            value.is_finite().then_some(value)
        })
    };
    let ports: Vec<(String, u32)> = get(modem, "Ports").unwrap_or_default();
    Modem {
        interface: ports
            .into_iter()
            .find(|(_, kind)| *kind == PORT_NET)
            .map(|(name, _)| name),
        link: CellularLink {
            state: state(get(modem, "State").unwrap_or(0)),
            quality_percent: quality.and_then(|(percent, _)| u8::try_from(percent).ok()),
            access: get(modem, "AccessTechnologies").and_then(access),
            operator: get::<String>(gpp, "OperatorName").filter(|o| !o.is_empty()),
            rsrp_dbm: extended("rsrp"),
            rsrq_db: extended("rsrq"),
            snr_db: extended("snr"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modem_states_fold_to_grading_states() {
        assert_eq!(state(3), ModemState::Disabled);
        assert_eq!(state(7), ModemState::Searching);
        assert_eq!(state(8), ModemState::Registered);
        assert_eq!(state(11), ModemState::Connected);
        assert_eq!(state(-1), ModemState::Failed);
        assert_eq!(state(2), ModemState::Busy);
    }

    #[test]
    fn the_most_capable_access_technology_wins() {
        assert_eq!(access(1 << 15 | 1 << 14), Some(RadioAccess::Nr5g));
        assert_eq!(access(1 << 14), Some(RadioAccess::Lte));
        assert_eq!(access(1 << 8), Some(RadioAccess::Umts));
        assert_eq!(access(1 << 1), Some(RadioAccess::Gsm));
        assert_eq!(access(0), None);
    }
}
