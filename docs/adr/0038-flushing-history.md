# 38. Flushing all history from the UI

Date: 2026-11-01

## Status

Accepted. Adds the API's second write (ADR-0035).

## Context

People want to start over: after testing, after moving the host to another network,
or after a rule change that left wrong incidents in history (ADR-0034). Until now
that meant stopping the service and deleting the database by hand.

## Decision

- **`DELETE /api/history` flushes history**: samples, readings, both rollup tiers,
  incidents, and events. It carries the same guard as saving settings (ADR-0035): the
  `X-Netwatch` header, an IP or `localhost` host, and a matching origin.
- **The single writer does it** (ADR-0006), as one more message in its queue. It
  discards the batch it holds, then deletes in one transaction, so no sample from
  before the flush can land after it.
- **Epochs survive a flush, enough to stay unique.** Open epochs, and the highest one
  ever allocated, are kept. A restart resumes after it, so new samples never share an
  epoch with old ones that something might still hold (ADR-0003). Closed epochs go.
- **Configuration and current state survive**: targets, bindings, statuses, and open
  incidents carry on. An open incident is written again at its next change.
- **Every open UI starts over.** The daemon broadcasts `cleared`, and each client
  drops what it holds and backfills from the now-empty store.
- The UI asks for confirmation first, because a flush can't be undone.

## Consequences

- One click loses all history, so the button sits in Settings behind a confirmation,
  not in the header.
