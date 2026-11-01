//! The public identity (ADR-0039): Cloudflare's trace page, read through the egress
//! interface every 5 minutes, for the header and the report.

use std::time::Duration;

use netwatch_host_health::facts::{PublicIdentity, parse_trace};
use netwatch_shared::Identity;
use netwatch_topology::Binding;
use tokio::sync::{mpsc, watch};

const EVERY: Duration = Duration::from_secs(300);
/// Until the first read succeeds (the egress path may not be known yet at startup).
const UNTIL_KNOWN: Duration = Duration::from_secs(10);

/// Starts reading the public identity. Sends one each time it is read; a failed read
/// sends nothing, so the last known identity stands.
pub fn spawn(egress: watch::Receiver<Option<Binding>>) -> mpsc::Receiver<PublicIdentity> {
    let (tx, rx) = mpsc::channel(2);
    tokio::spawn(async move {
        let mut known = false;
        loop {
            let pin = egress.borrow().as_ref().and_then(|b| match &b.identity {
                Identity::Interface { name, .. } => Some(*name),
                Identity::Address(_) => None,
            });
            let Some(pin) = pin else {
                tokio::time::sleep(UNTIL_KNOWN).await;
                continue;
            };
            match netwatch_adapter_sockets::http::fetch_text(
                "one.one.one.one",
                "/cdn-cgi/trace",
                pin,
            )
            .await
            {
                Ok(text) => match parse_trace(&text) {
                    Some(identity) => {
                        known = true;
                        let _ = tx.send(identity).await;
                    }
                    None => tracing::debug!("the trace page named no address"),
                },
                Err(error) => tracing::debug!(%error, "cannot read the public identity"),
            }
            tokio::time::sleep(if known { EVERY } else { UNTIL_KNOWN }).await;
        }
    });
    rx
}
