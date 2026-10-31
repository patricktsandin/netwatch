# 9. Default targets and probe cadence

Date: 2026-10-31

## Status

Accepted

## Context

netwatch should be useful on first run without any configuration. Some endpoints exist
specifically for connectivity checks: they are stable, cheap to serve, and have
well-defined responses. Each of the following was checked from the development host:
- `https://www.gstatic.com/generate_204` and `https://cp.cloudflare.com/generate_204`:
  return 204 with an empty body.
- `https://one.one.one.one/cdn-cgi/trace`: returns 200, and the body includes `colo=`,
  the Cloudflare point of presence that served the request.

The cadence options were sub-second (catches microbursts, but heavy), 1s, or 5–10s
(light, but misses short glitches).

## Decision

- **Default targets**, all editable in the UI:
  - ICMP and DNS to 1.1.1.1, 8.8.8.8, and 9.9.9.9, and their IPv6 counterparts.
  - DNS through the system resolver: the addresses in `/etc/resolv.conf`, or the upstream
    servers that systemd-resolved is using.
  - HTTPS to the three endpoints above. `colo=` is stored as an identity attribute, so a
    change in it shows that anycast routing changed.
- An HTTP probe checks the expected status code (and body, where applicable). A
  captive-portal page that answers 200 counts as a failure.
- **Cadence:**
  - Every 1s: kernel, NIC, local interface, and hops; counters are also read every 1s.
  - Every 5s: ICMP and DNS targets.
  - Every 10s: HTTP(S) targets.

## Consequences

- Useful out of the box, with a negligible load on the default target operators.
- 1s resolution misses events shorter than a second. Sub-second probing can be
  revisited for individual layers if needed.
