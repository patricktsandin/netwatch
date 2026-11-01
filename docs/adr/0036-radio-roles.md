# 36. Radio roles: Wi-Fi and cellular, carrying traffic or standing by

Date: 2026-11-01

## Status

Accepted. Extends ADR-0002 and ADR-0032.

## Context

ADR-0002 planned Wi-Fi and cellular statistics as attributes of the NIC, shown when
the NIC is wireless. This host's NIC is Ethernet. Its Wi-Fi interface and its 5G modem
are backups, and the question worth answering about a backup is whether failover
would work right now: is the modem registered, and how strong is the signal? Under
ADR-0002 that is never shown until the backup is already carrying traffic.

## Decision

- **Each radio is a role**: `wifi` and `cellular` (`Role::Wifi`, `Role::Cellular`),
  appended after the existing variants so the existing order is unchanged. Each binds
  to its device whether or not it carries the egress path. If there are several of a
  kind, the one carrying the path wins, otherwise the lowest interface index (Wi-Fi)
  or the first modem (cellular).
- **Radio roles are off the path.** They are neither upstream nor downstream of any
  other role, so a weak backup is never anyone's probable cause. When a radio is the
  NIC, the NIC role covers the path.
- **Gauges** (ADR-0032), read every 5 s:
  - Wi-Fi, from nl80211 station info: signal (dBm), transmit and receive bitrate
    (bit/s)
  - cellular, from ModemManager over D-Bus: signal quality (%), plus RSRP (dBm), RSRQ
    (dB), and SNR (dB) for LTE or 5G NR when the modem reports extended signal. The
    daemon asks for extended signal (`Signal.Setup`). If that is refused, quality
    alone is graphed.
- **Grades** (Host health, pure):
  - Wi-Fi: not associated is `unknown` (standing by). Associated, a signal of -67 dBm
    or better is healthy, below that is degraded, and below -80 dBm is failing.
  - Cellular: disabled is `unknown`. Enabled but not registered is failing.
    Registered, quality of 30 % or more is healthy, 10–29 % degraded, and under 10 %
    failing.

  Radio incidents follow from these grades like any other role's.
- **What a radio reports** besides gauges, in its row's layer report: the state
  (associated, registered, disabled), the access point's BSSID or the operator and
  access technology. A change of access point or access technology is an event on the
  radio's row. Identifiers of the subscriber or device (IMSI, IMEI, ICCID) are never
  read.
- **Adapters**: nl80211 goes through `wl-nl80211`, in the netlink adapter.
  ModemManager goes through `zbus`, in `netwatch-adapter-modem`. Neither needs a new
  capability. If ModemManager isn't running, or nl80211 has no wireless interface,
  the role doesn't exist. That's absence, not a disabled source.

## Consequences

- A backup radio's readiness is visible before it is needed, and its signal history
  explains a bad failover after the fact.
- A disabled modem or a Wi-Fi interface that isn't associated shows as `unknown`,
  with the reason in its row. Neither raises incidents.
- `Role` grows two variants. Every match on it now handles them.
