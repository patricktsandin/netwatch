# 57. Follow the current pin, and touch no real state

Date: 2026-11-02

## Status

Accepted. Refines ADR-0056's outcome windows, ADR-0055's no way out, and ADR-0056's
trace paths; supersedes the implicit `/etc/netwatch/netwatch.toml` of ADR-0035.

## Context

- ADR-0056 kept a role's window to its newest epoch by comparing epoch numbers. A
  target's sample carries its pin's epoch, and when a tunnel goes down the pin goes
  back to the NIC, whose epoch is older. Every outcome after that was dropped, and the
  targets went unknown through any outage that followed.
- With no way out, incidents asked for episode traces that couldn't run, spending
  each target's cooldown, and routine traces fell due and were pushed back 6 h.
- The report's baseline and normal paths took the latest routine trace whatever path
  it ran on.
- The report's summary listed a target once per epoch.
- The namespace integration test ran its daemon as root without a configuration of
  its own. It found the installed service's, and wrote 24 test runs into the real
  database, which had to be repaired. A development build would do the same, and the
  web dev server proxied to the installed service, whose history its UI can flush.

## Decision

- **A window holds one epoch's outcomes and starts afresh on any other.** Which
  outcomes are current is decided by the pin, not by epoch numbers: a target's
  outcome is graded only if it went by the current pin.
- **With no way out, nothing is asked of the trace schedule.** The incidents that
  open meanwhile are held back and given to it once there is a way out; routine
  traces that fall due wait for one.
- **Report traces follow their path**: an episode's baseline is a routine trace of
  the path its episode trace ran on, and a target's normal path is the routine trace
  of the path it was last traced on.
- **The summary has a row per target.** Probes and availability are counts and add up
  across epochs; median and p95 are of the path it was probed on most, named.
- **Nothing finds the service's state implicitly.** The configuration is the file
  `NETWATCH_CONFIG` names, else the built-in defaults; the installed unit and the
  container name theirs. Tests give their daemons their own configuration and
  database, and remove them even when they fail. The web dev server proxies to a dev
  daemon on 8733. Privileged tests build as the user and run only the test binary as
  root.

## Consequences

- Running `netwatch` by hand, without `NETWATCH_CONFIG`, uses the built-in defaults
  and `netwatch.db` in the working directory.
- An incident that opens with no way out and continues once there is one is traced
  then, not at once.
