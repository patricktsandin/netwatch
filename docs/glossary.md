# Glossary

This file is the ubiquitous language of netwatch ([ADR-0013](adr/0013-domain-driven-design.md)).
Code, tests, specs, UI text, and commit messages use these terms exactly as written.
Add a term here before using it in code. To rename a term, rename it everywhere in one
change.

Terms are grouped by bounded context (see [context-map.md](context-map.md)). *Italic*
words are defined elsewhere in this file.

## Words with one meaning only

- **Kernel**: the Linux kernel. Never use it for anything else.
- **Shared crate**: `netwatch-shared`, the value objects that every domain context uses
  (DDD calls this a shared kernel; we don't, see *Kernel*).
- **Verified crate**: `netwatch-verified`, algorithms proven with Verus.
- **Event**: avoid using it alone. Say *observation*, *domain event*, *topology event*,
  or *event marker*.

## Domain core vocabulary (all contexts)

- **Observation**: an input to a domain state machine. Examples: a netlink message
  already translated into domain terms, a received packet, a clock reading. ADR-0012 calls
  these "events"; *observation* is the term.
- **Command**: an output from a domain state machine asking the shell to act (send
  a *probe*, wake at instant T, start *hop discovery*).
- **Fact**: an output from a domain state machine recording something that happened:
  a *sample*, a *counter reading*, or a *domain event*.
- **Domain event**: a *fact* that another context reacts to. *Topology events* are the
  main kind.
- **Instant**: a monotonic clock reading. Durations are only ever differences between
  instants.
- **Wall time**: a UTC timestamp, used only to label *samples* and *domain events*.

## Topology

- **Layer**: one level of the monitored path: *kernel layer*, *NIC layer*, *tunnel layer*,
  *local interface layer*, *hop layer*, *target layer*.
- **Kernel role**: the role of the *kernel layer*. It is bound to the loopback
  interface, and its samples are *loopback probes*: the stack baseline.
- **Role**: a logical position that is measured continuously, like "the NIC", "hop 2 toward
  target X", or "target X". Series are keyed by role.
- **Identity**: the concrete thing behind a *role* at a given moment, such as an interface,
  IP address, MAC address, *responding address*, *BSSID*, *serving cell*, or *point of presence*.
- **Identity epoch**: a span of time during which a *role* keeps the same *identity*.
  A new epoch begins whenever the identity changes. Aggregates never span epochs.
  Only the topology aggregate assigns epochs.
- **Binding**: a *role*'s current *identity* together with its *identity epoch*. A role
  with no identity is *unbound*.
- **Pin**: the *binding* a *probe* is tied to when it is scheduled. The probe travels
  only through the pinned identity, and its *sample* carries the pinned epoch
  (ADR-0020).
- **Probe-identified role**: a *role* whose *identity* comes from its own source,
  not from the kernel's route notifications: a *hop* (who answered) or a radio (the
  device read). The source reports it identified (`ProbeIdentified`) or gone
  (`ProbeVanished`, ADR-0051), and either takes effect at once.
- **Notification**: a change message from the kernel (rtnetlink, nl80211, ModemManager),
  translated into an *observation*. Notifications arrive after the change has already
  taken effect.
- **Notification loss**: notifications dropped because a netlink socket's buffer
  overflowed (`ENOBUFS`). After a loss, the aggregate's view may be stale.
- **Snapshot**: the full current state requested from the kernel by a *resync*.
- **Resync**: requesting a *snapshot* after *notification loss* and reconciling it with
  the current *bindings*.
- **Synchronized / desynchronized**: whether the topology aggregate's view can be
  trusted. It becomes desynchronized on *notification loss*, and synchronized again
  after a *resync*.
- **Egress path**: what the kernel resolves for a destination: egress interface,
  *gateway*, source address, and *tunnel* (if any), as `RTM_GETROUTE` reports it.
- **No way out**: no *egress path* at all, for at least 5 s: nothing can be sent, so
  the local interface and every *target* and *hop* are `Failing` (ADR-0053, ADR-0055).
  The report states the *time without a way out*: the local interface's own outages.
- **Egress interface**: the interface an *egress path* leaves through.
- **NIC**: the physical device under the current *egress path*. If a *tunnel* is
  involved, the NIC is the device that carries the tunnel's traffic.
- **Tunnel**: a virtual interface (WireGuard, tun/tap) that the *egress path* goes
  through. It is linked to the NIC beneath it.
- **Tunnel endpoint**: the remote address of a *tunnel*. Its own *egress path* identifies
  the NIC underneath.
- **Gateway**: the next-hop router on an *egress path*.
- **Radio role**: `wifi` or `cellular`: a wireless device, whether it carries the
  *egress path* or stands by as a backup (ADR-0036). Radio roles are off the path:
  neither *upstream* nor *downstream* of anything.
- **Public identity**: the address this host's traffic leaves the ISP with, and the
  anycast site (`colo`) and country where it enters Cloudflare, read from Cloudflare's
  trace page (ADR-0039).
- **Link facts**: a NIC's negotiated speed, duplex, driver, MTU, and carrier-change
  count, read from sysfs (ADR-0039).
- **BSSID**: the identity of the Wi-Fi access point a wireless NIC is connected to.
- **Serving cell**: the identity of the cell a cellular NIC is attached to.
- **Point of presence**: the anycast site that answered a *target*, such as Cloudflare's
  `colo=`.
- **Topology event**: a *domain event* recording a change, such as a route change, a
  *gateway* change, a link going up or down, an address being added or removed, a new
  *identity epoch*, a *suspend gap*, or a *config drift*.
  - The first ones defined in code are: `EpochStarted`, `EpochEnded`,
    `Desynchronized`, and `Resynchronized`.
- **Resumed epoch**: a role's first *identity epoch* after the daemon starts, when its
  *identity* is the one it had before the stop. It is a new epoch, but not a change,
  so no *event marker* shows it. Only an epoch that starts within a minute of the
  start can resume one (ADR-0044, ADR-0046).

## Probing

- **Probe**: one measurement attempt with exactly one *probe outcome*. Its lifecycle is
  scheduled → in flight → resolved.
- **Probe kind**: one of *loopback probe*, *link-layer probe*, *hop probe*, *echo probe*,
  *DNS probe*, *HTTP probe*.
- **Loopback probe**: an ICMP echo or UDP echo through `lo`, giving the *kernel layer*
  baseline.
- **Link-layer probe**: an ARP request (IPv4) or neighbor solicitation (IPv6) to the *gateway*.
- **Hop probe**: a TTL-limited probe toward a *target*, answered by the router at that
  *hop*.
- **Echo probe**: an ICMP or ICMPv6 echo request to a *target*.
- **DNS probe**: a DNS query to a specific server.
- **HTTP probe**: an HTTP(S) request with *phase timings*, checked against an *expected
  response*.
- **Phase timings**: the DNS, connect, TLS, time-to-first-byte, and total durations of an
  *HTTP probe*.
- **Expected response**: the status code (and body, where relevant) an *HTTP probe* must
  receive to count as answered. A captive portal page fails it.
- **Probe outcome**: one of:
  - `Answered`, with a round-trip time
  - `TimedOut`
  - `Unreachable { code }`
  - `TimeExceeded`: the probe expired in transit. Only on a stream that doesn't limit
    its TTL; on a *hop probe*'s stream, time exceeded is the hop's answer (ADR-0049).
  - `Refused { code }`
  - `ExchangeFailed { stage }`: an HTTPS exchange failed beyond this host, while
    resolving the name, connecting, in the TLS handshake, or in the request
    (ADR-0052)
  - `SendFailed { cause }`: this host couldn't send it
  - `UnexpectedResponse`

  These are never collapsed into a missing value.
- **Reply matching**: pairing a received packet with its *probe* by identifier, sequence
  number, and the original header embedded in ICMP errors. Never by arrival order.
- **Probe stream**: the fixed-cadence sequence of *probes* for one *role*. Several
  *targets* can share one stream (see *stream sharing*).
- **Stream sharing**: running one *probe stream* for *hops* that resolve identically
  across *targets*. Hop 1 is shared by every target of an address family (ADR-0026),
  and hops 2–3 by every target with the same destination address (ADR-0043). Each
  target's *hop* keeps its own *role*.
- **Stream owner**: the *target* whose *hop* *role* a shared *probe stream* runs as,
  and toward whose address it probes: the lowest target with a public address, or the
  lowest if none has one (ADR-0046). A shared *row* draws the owner's samples.
- **Cadence**: the fixed interval at which a *probe stream* sends, measured from the
  schedule, never from the last reply.
- **Timer lateness**: how far after its scheduled *instant* a *probe* was actually sent.
- **Hop**: the router at a given TTL on a *target*'s path. Hop N is a *role*.
- **Responding address**: the address that answered a *hop probe*. It is the hop's
  *identity*.
- **Hop discovery**: finding hops 1–3 for an ICMP or DNS *target*. It runs again immediately whenever
  the target's *egress path* changes.
- **Suspected rate limiting**: loss at a *hop* that doesn't carry through to later hops.
  It is shown as such, never as loss.
- **Path trace**: a one-off TTL sweep toward a *target*'s address, beyond the *hops*
  measured continuously, recording who answered at each TTL (ADR-0040). A *routine
  trace* runs at startup and every 6 h; an *episode trace* is asked for when an
  *incident* opens on the target or on one of its hops beyond hop 1. Each waits in the
  *trace queue*, and records its *trace path*.
- **Trace queue**: where traces wait to run, one at a time, until the *trace path* has
  been the same for 10 s. An *episode trace* whose target has no *incident* open by
  the time it would start is dropped (ADR-0058, ADR-0059).
- **Trace path**: the path a *path trace* ran on: the egress interface and the local
  address. The network map draws only traces of the current one (ADR-0056).
- **Trace schedule**: decides when each *target* is traced: routine traces, episode
  traces at most once per cooldown, and never two at once (`specs/traces.qnt`).
- **Router name**: what a *responding address* is called: its PTR name, and its *origin
  AS* (ADR-0040).
- **Origin AS**: the autonomous system that announces the prefix an address is in:
  whose network a *hop* is in.
- **First public hop**: the first *hop* whose address isn't private, shared, or
  link-local: most likely where the ISP's network begins.
- **Target**: a user-configured endpoint probed by an *echo probe*, *DNS probe*, or
  *HTTP probe*.
- **Timestamp source**: where a receive time came from: a kernel timestamp
  (`SO_TIMESTAMPING`) or userspace. Recorded on every *sample*.

## Host health

- **Telemetry source**: one collector of host data, such as protocol counters, softnet,
  conntrack, qdisc, PSI, clock health, config drift, kernel log, or drop reasons.
- **Disabled source**: a *telemetry source* (or *probe kind*) turned off because a
  capability is missing. It is recorded as a *domain event* and shown in the UI.
- **Counter**: a value that only increases, read from the kernel or a driver. It can
  wrap around or reset.
- **Counter reading**: one *counter* value at one *instant*. Not stored: it becomes a
  *rate* (ADR-0032).
- **Counter reset**: a *counter reading* lower than the previous one, caused by a reboot,
  a module reload, or wraparound. It is detected, never shown as negative traffic.
- **Rate**: the change in a *counter* over a time interval, computed when the counter
  is read and stored as a *reading* (ADR-0032).
- **Gauge**: a named quantity with a fixed unit that can go up or down, such as
  throughput, PSI, or signal strength.
- **Throughput**: the *rate* of bytes a *NIC* receives or transmits, in bit/s.
- **Drop reason**: the kernel's `skb_drop_reason` for a discarded packet, counted by
  the eBPF *telemetry source*.
- **Kernel log line**: a network-relevant line from `/dev/kmsg`, matched by a known
  pattern and attributed to an interface where possible.
- **Suspend gap**: a period when the host was suspended, detected from `CLOCK_BOOTTIME`
  vs. `CLOCK_MONOTONIC`. It is a gap in the data, never loss.
- **Clock health**: synchronization state, estimated error, clock source, and
  *wall time* jumps.
- **Config drift**: a change to a watched network sysctl.

## History

- **Sample**: one *probe outcome*, together with its *role*, *identity*, *identity
  epoch*, *wall time*, and *timestamp source*.
- **Reading**: one *gauge* value, together with its *role*, *identity epoch*, and
  *wall time* (ADR-0032). Stored, retained, and rolled up like a *sample*.
- **Reading rollup**: count, sum, min, and max of a *gauge*'s *readings* over one
  *tier* interval within one *identity epoch*.
- **Tier**: a resolution level of stored history: *raw*, *1m*, or *1h*.
- **Rollup**: an aggregate over one *tier* interval within one *identity epoch*: count,
  loss, min, average, max, p50, p95, p99, and jitter. Rollups merge (see *merge*).
- **Merge**: combining two *rollups* of adjacent intervals into one. It is associative
  and commutative, and proven in the *verified crate*.
- **Jitter**: the mean absolute difference between consecutive answered RTTs of one
  *role* (RFC 3550's interarrival jitter, unsmoothed).
- **Loss burst**: a run of consecutive lost *probes* of one *role*. Its length is the
  number of probes in the run.
- **Retention**: how long each *tier* is kept: raw 7 days, 1m 90 days, 1h indefinitely.

## Presentation

- **Row**: one *layer* or *role* on the main screen, with a *status*, a sparkline, and
  current values. A *probe stream* shared by several *hops* (*stream sharing*) is one
  row, whose status is the worst of theirs (ADR-0045).
- **Status**: a *row*'s current condition: `Healthy`, `Degraded`, `Failing`, `Unknown`, or
  `RateLimited`. `Failing` needs at least three outcomes of the role's newest epoch
  (ADR-0056).
- **Time axis**: the single shared horizontal axis that every *row* uses. Zoom and pan
  are synchronized across rows.
- **Event marker**: a vertical line on the *time axis* for a *topology event* or another
  *domain event*, drawn only on the *rows* of the *roles* the event *concerns*.
- **Concerns**: the *roles* a *domain event* is relevant to, decided by the daemon
  (ADR-0031). An *identity epoch* change concerns its *role* and every role
  *downstream* of it, except the *kernel role*'s: no path runs through loopback.
- **Upstream / downstream**: order along one *target*'s path: kernel, NIC, tunnel,
  local interface, hop 1, hop 2, hop 3, then the *target*.
- **Incident**: a period during which one *role*'s *status* was `Degraded` or `Failing`
  (ADR-0031). Its *severity* is *outage* if it was ever `Failing`, otherwise
  *degradation*.
- **Probable cause**: the most *upstream* *role* whose *incident* opened before
  another's, or at most 10 s after it, and was still open (ADR-0034).
- **Episode**: a connected group of overlapping *incidents*, linked by *probable
  cause*: one problem as it showed up across the *roles* it affected (ADR-0033,
  ADR-0034). It is *on this host or LAN*
  when its *probable cause* is at or before hop 1, and *beyond the gateway* otherwise.
- **Incident report**: a printable account of the *episodes* in a time span, with path
  evidence for each, for sending to an ISP (ADR-0033).
- **Ticket details**: the customer's account number, service address, contact, ISP
  ticket reference, and description, typed in for one *incident report*. Never stored
  by the daemon (ADR-0040).
- **Network map**: the *Map* tab's directed acyclic graph of every path from this host
  to every *target*, built by the daemon from *statuses*, *hop* identities, and each
  target's latest *path trace* (ADR-0042).
- **Map node**: one place on the *network map*: a *layer*, a *radio role*, a router
  (a *responding address* at one TTL, shared by every path through it), an unknown or
  *silent* hop, or a *target*. Its *status* is the worst of the *roles* merged into it.
  A node seen only by a trace has none.
- **Map edge**: a step between consecutive positions on one or more *targets*' paths.
  It is *probed*, *traced*, *unmeasured* (hops between its ends are unknown), or
  *off-path* (a *radio role*'s).
- **Sighting**: what one *target*'s latest *path trace* found at a *map node*: the
  probes it sent at that TTL, how many were answered, and the median answer.
- **Incident band**: an *incident* drawn as a shaded period behind its *role*'s graph.
- **Tooltip**: what the UI shows on hover: the value, *incidents*, and *event markers*
  at that time.
- **Settings**: the UI's page for adding, changing, and removing *targets*. It edits
  the daemon's configuration file in place (ADR-0035).
- **Live stream**: *samples* pushed to the UI over WebSocket.
- **Backfill**: fetching history to fill gaps, at startup, on zoom, or after a reconnect.
  It merges with the *live stream* without gaps or duplicates.
