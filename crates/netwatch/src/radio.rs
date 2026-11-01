//! Reading the radios (ADR-0036): the Wi-Fi device over nl80211 and the modem over
//! ModemManager, every 5 s, as updates for the main loop.

use std::time::Duration;

use netwatch_adapter_web::{LayerReportView, LiveMessage};
use netwatch_host_health::LayerStatus;
use netwatch_host_health::radio::RadioReport;
use netwatch_probing::Health;
use netwatch_shared::{Identity, InterfaceIndex, InterfaceName, Role};
use netwatch_topology::Binding;
use tokio::sync::{broadcast, mpsc, watch};

const EVERY: Duration = Duration::from_secs(5);

/// One radio's reading.
pub enum Update {
    Wifi(netwatch_adapter_netlink::WifiDevice),
    /// The modem, and its network interface's identity, if it has one.
    Cellular(Option<Identity>, netwatch_adapter_modem::Modem),
}

/// Starts reading the radios. A radio this host doesn't have sends nothing, so its
/// role never appears.
pub fn spawn(egress: watch::Receiver<Option<Binding>>) -> mpsc::Receiver<Update> {
    let (tx, rx) = mpsc::channel(8);
    tokio::spawn(async move {
        let mut modems = match netwatch_adapter_modem::ModemManager::connect().await {
            Ok(manager) => Some(manager),
            Err(error) => {
                tracing::info!(%error, "no system bus, so no cellular radio role");
                None
            }
        };
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            // The Wi-Fi device carrying the egress path wins, if there is one.
            let prefer = egress.borrow().as_ref().and_then(|b| match &b.identity {
                Identity::Interface { index, .. } => Some(index.get()),
                Identity::Address(_) => None,
            });
            match netwatch_adapter_netlink::wifi(prefer).await {
                Ok(Some(device)) => {
                    let _ = tx.send(Update::Wifi(device)).await;
                }
                Ok(None) => {}
                Err(error) => tracing::debug!(%error, "cannot read nl80211"),
            }
            if let Some(manager) = modems.as_mut()
                && let Some(modem) = manager.read().await
            {
                let identity = modem.interface.as_deref().and_then(interface);
                let _ = tx.send(Update::Cellular(identity, modem)).await;
            }
        }
    });
    rx
}

/// The identity of the interface named `name`, if it exists.
fn interface(name: &str) -> Option<Identity> {
    let index = netwatch_adapter_netlink::index_of(name)?;
    Some(Identity::Interface {
        index: InterfaceIndex::new(index),
        name: InterfaceName::parse(name.as_bytes()).ok()?,
    })
}

/// Publishes a radio's report on its row, and returns its grade.
pub fn publish(
    role: &Role,
    report: &RadioReport,
    live: &broadcast::Sender<LiveMessage>,
    layers: &watch::Sender<std::collections::BTreeMap<String, LayerReportView>>,
) -> Health {
    let (status, health) = match report.status {
        LayerStatus::Healthy => ("healthy", Health::Healthy),
        LayerStatus::Degraded => ("degraded", Health::Degraded),
        LayerStatus::Failing => ("failing", Health::Failing),
        LayerStatus::Unknown => ("unknown", Health::Unknown),
    };
    let view = LayerReportView {
        status: status.to_owned(),
        notes: vec![report.summary.clone()],
        increases: Vec::new(),
        resets: Vec::new(),
        pressure: None,
        conntrack_fill: None,
        clock_synchronized: None,
        clock_error_us: None,
        disabled: Vec::new(),
    };
    let _ = live.send(LiveMessage::Layer {
        role: role.key(),
        report: view.clone(),
    });
    layers.send_modify(|all| {
        all.insert(role.key(), view);
    });
    health
}
