# 50. Every request is checked for its site

Date: 2026-11-02

## Status

Accepted. Extends ADR-0035's guard from writes to every request.

## Context

ADR-0035 guards the write endpoints against other web pages the browser shows: a
`Host` that is an IP literal or `localhost` defeats DNS rebinding, and an `Origin`
that matches the `Host` rejects cross-site requests. Reads were left open, on the
grounds that a cross-site page can't read a response without CORS.

That holds for `fetch`, but not for WebSockets, which CORS doesn't cover. Any page
the user visits can open `ws://127.0.0.1:8732/api/live` and read the live stream:
the local address, router addresses and names, kernel log lines, and public
address changes. A DNS-rebound name also reads every `GET` endpoint, because to the
browser it is the same origin.

## Decision

- **Every request passes the same site check** as a write does: the `Host` is an IP
  literal or `localhost`, and the `Origin`, if present, matches it. Anything else is
  refused with 403. Writes still also need `X-Netwatch` and a JSON body.
- The check runs as one middleware in front of every route, the UI's files
  included, so no new route can be left out.

## Consequences

- The UI is reached by an IP literal or `localhost`, as the write guard already
  required. Remote access stays out of scope (ADR-0010).
- Tools without an `Origin`, such as `curl`, still work against `127.0.0.1`.
