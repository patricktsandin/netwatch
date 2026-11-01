# 35. Configuring targets from the UI

Date: 2026-11-01

## Status

Accepted. Narrows ADR-0010 (the API stays local-only) and adds the API's first write.

## Context

Targets live in `/etc/netwatch/netwatch.toml`. Changing them means editing a file as
root and restarting the service. People asked to do it from the web UI.

The API has only ever been read-only. A write endpoint on `127.0.0.1` can be reached
by any web page the person visits, through a cross-site request or through DNS
rebinding, so it needs protecting even without remote access.

## Decision

- **The UI edits the same file**, the one the daemon loaded, in place with
  `toml_edit`, so comments, formatting, and every setting other than targets survive.
  There is one source of truth, and editing it by hand keeps working. When the daemon
  runs on its built-in defaults, there is no file, and the UI says so instead of
  offering to save.
- **Only targets are editable**: add, rename, change, and remove. The listen address
  and database path stay in the file, because a wrong value from the UI could lock
  the UI out.
- **The daemon assigns ids.** A new target gets one more than the highest id in the
  file or in history, so an id is never reused (ADR-0009). Validation is the same
  `config::parse` the daemon uses at startup, applied to the edited file before it
  is written. Errors come back as the daemon's own message.
- **Saving applies by re-executing.** After writing the file atomically (a temporary
  file, then `rename`), the daemon flushes the writer and `execve`s itself. Every
  probe task starts again from the new targets, ambient capabilities survive
  `execve`, and the UI's live stream reconnects and backfills (ADR-0029).
- **Writes must come from the UI itself.** `PUT /api/config` requires:
  - a JSON body
  - the header `X-Netwatch: 1`, which a cross-site page can't send without a CORS
    preflight, and the daemon answers no preflight
  - a `Host` that is an IP literal or `localhost`, which defeats DNS rebinding
  - an `Origin`, if present, that matches the `Host`
- **The service can write its configuration.** The unit adds
  `ReadWritePaths=/etc/netwatch`, and the RPM makes `/etc/netwatch` and the file
  group-owned by `netwatch` and group-writable.

## Consequences

- A save interrupts probing for about a second. The gap shows as a gap, never as loss.
- The file the daemon rewrites keeps its comments. Comments attached to a removed
  target go with it.
- The service account can change which hosts netwatch probes, but nothing else in
  `/etc`.
