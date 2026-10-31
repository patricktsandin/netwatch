// -*- mode: Bluespec; -*-
/**
 * # The UI's live stream and backfill
 *
 * ADR-0019 requires that the UI's series has no gaps and no duplicates, whatever
 * happens to its WebSocket. This spec models one role's series end to end:
 *
 * - **The daemon** produces samples (times 1, 2, 3, ...). It broadcasts each one
 *   live, and writes it to SQLite through the batching writer, so the database
 *   lags behind the broadcast (`flushed` <= number produced).
 * - **The WebSocket** can drop at any moment. While it's down, broadcasts are lost.
 * - **The client** merges live samples into its series (a set, because `merge` is a
 *   set union in time order; it is property-tested). When the socket (re)opens, it
 *   backfills: it asks for every stored sample since its latest one.
 *
 * Convergence is stated as a safety invariant: once the socket is up, nothing is in
 * flight, and the writer has caught up, the client has every sample.
 *
 * `FLUSH_BEFORE_BACKFILL` is ADR-0029's decision. With it set to `false`, the
 * simulator finds the gap within a few steps: a sample produced while disconnected
 * hasn't been written yet when the backfill query runs, and nothing fetches it
 * again.
 */
module live_backfill {

  pure val MAX_SAMPLES = 6
  pure val FLUSH_BEFORE_BACKFILL = true

  // The daemon.
  var produced: int        // samples 1..produced exist
  var flushed: int         // samples 1..flushed are in the database
  // The socket.
  var connected: bool
  var inbox: List[int]     // live samples in flight to the client
  // The client.
  var points: Set[int]
  var request: Set[int]    // the `since` of an outstanding backfill request (0 or 1 element)
  var response: Set[Set[int]]  // a backfill response in flight (0 or 1 element)

  pure def latest(ps: Set[int]): int = ps.fold(0, (m, p) => if (p > m) p else m)

  action init = all {
    produced' = 0, flushed' = 0,
    connected' = false, inbox' = [],
    points' = Set(), request' = Set(), response' = Set(),
  }

  action clientIdle: bool = all { points' = points, request' = request, response' = response }

  /// The daemon produces a sample, and broadcasts it if the socket is up.
  action produce: bool = all {
    produced < MAX_SAMPLES,
    produced' = produced + 1,
    flushed' = flushed,
    connected' = connected,
    inbox' = if (connected) inbox.append(produced + 1) else inbox,
    clientIdle,
  }

  /// The writer flushes one more sample to the database.
  action flush: bool = all {
    flushed < produced,
    flushed' = flushed + 1,
    produced' = produced, connected' = connected, inbox' = inbox,
    clientIdle,
  }

  action disconnect: bool = all {
    connected,
    connected' = false,
    inbox' = [],
    produced' = produced, flushed' = flushed,
    clientIdle,
  }

  /// The socket opens, which subscribes the client to the broadcast, and the
  /// client asks for everything since its latest sample. The order matters:
  /// subscribing first means every later sample arrives live.
  action reconnect: bool = all {
    not(connected),
    request.size() == 0 and response.size() == 0,
    connected' = true,
    inbox' = [],
    request' = Set(latest(points)),
    produced' = produced, flushed' = flushed, points' = points, response' = response,
  }

  /// The daemon answers the backfill from the database. With the flush barrier
  /// (ADR-0029), the writer first writes everything produced so far.
  action serveBackfill: bool = all {
    request.size() == 1,
    val since = latest(request)
    val stored = if (FLUSH_BEFORE_BACKFILL) produced else flushed
    all {
      response' = Set(1.to(stored).filter(t => t >= since)),
      flushed' = stored,
    },
    request' = Set(),
    produced' = produced, connected' = connected, inbox' = inbox, points' = points,
  }

  action deliverBackfill: bool = all {
    response.size() == 1,
    points' = response.fold(points, (acc, r) => acc.union(r)),
    response' = Set(),
    request' = request,
    produced' = produced, flushed' = flushed, connected' = connected, inbox' = inbox,
  }

  action deliverLive: bool = all {
    inbox.length() > 0,
    points' = points.union(Set(inbox.head())),
    inbox' = inbox.tail(),
    request' = request, response' = response,
    produced' = produced, flushed' = flushed, connected' = connected,
  }

  action step = any {
    produce, flush, disconnect, reconnect, serveBackfill, deliverBackfill, deliverLive,
  }

  /// The client never holds a sample that wasn't produced.
  val noInventedSamples: bool = points.forall(t => 1 <= t and t <= produced)

  /// **Convergence (ADR-0019).** Once connected, with nothing in flight and the
  /// writer caught up, the client has every sample: no gaps. (Duplicates can't
  /// happen, because the series is merged as a set.)
  val quiescentSeriesIsComplete: bool =
    (connected and inbox.length() == 0 and request.size() == 0 and response.size() == 0
      and flushed == produced)
      implies points == 1.to(produced)

  val invariants: bool = noInventedSamples and quiescentSeriesIsComplete
}
