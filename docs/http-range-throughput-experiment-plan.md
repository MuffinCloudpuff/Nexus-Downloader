# HTTP Range Throughput Experiment Plan

## Goal

Find a stable strategy for large generic HTTP downloads that avoids two failure
modes:

- tail starvation: only a few slow ranges remain near the end;
- request pressure: small leases create too many short range requests and trigger
  low throughput or `429 Too Many Requests`.

## Current Findings

- The 60 GiB `cdn1.trashbytes.to` link initially returned `206` and the correct
  total size.
- `4 MiB` leases at 32-64 workers only reached about 9-10 MiB/s in diagnostics.
- The same host later returned `429`, which means retrying aggressively is part
  of the problem, not the repair.
- A synthetic browser-like header set returned `403`, so the daemon must not
  guess request headers. The extension has to pass the real browser request
  headers captured by `webRequest`.
- Browser download handoff previously resolved header snapshots by exact full
  URL only. Signed download URLs can differ between `webRequest`, redirects,
  and `chrome.downloads.DownloadItem`, which explains `header_count=1` handoffs
  that only carried a fallback `referer`.

## Hypotheses

1. Missing real browser request headers can put the daemon on a different CDN
   authorization or throttling path than Edge/IDM.
2. `4 MiB` is too small for this CDN. It spends too much time in request startup
   and increases request pressure.
3. Large fixed segments cause tail starvation, but small fixed leases cause
   request pressure.
4. The correct policy is adaptive:
   - use larger leases during steady state;
   - shrink leases only when the queue is near tail or a segment repeatedly
     short-reads;
   - back off per host after `429`.

## Diagnostic Requirements

`http-range-diagnose` must support:

- start offset or tail-byte mode;
- per-worker timeout;
- concise aggregate output;
- preserving full signed URL secrecy in user-facing summaries.

## Passing Criteria

For `cdn1.trashbytes.to`, a strategy is acceptable when a small diagnostic run
can show either:

- throughput close to the observed external downloader class for the current
  network, or
- clear evidence that the host is rate-limiting this client shape, with a
  graceful backoff strategy and no task corruption.

The production downloader must also pass:

- unit tests for lease size selection and `429` classification;
- `cargo test --lib`;
- `cargo check`;
- release build.

## Implementation Direction

1. Add tail/offset diagnostics first.
2. Run one-variable-at-a-time tests:
   - 8, 16, 32, 64 workers;
   - 4, 8, 16, 32 MiB range size;
   - head vs tail positions.
3. Implement dynamic production policy:
   - start with medium lease, not tiny lease;
   - reduce lease for tail or repeated short reads;
   - enter host cooldown on `429`;
   - avoid updating site history from rate-limited samples.
4. Fix browser download handoff:
   - match header snapshots by exact URL, normalized signed URL, then same
     origin/path newest snapshot;
   - log only header names and match strategy, never header values;
   - keep encrypted cookies separate from plain request headers.
5. Harden probe handling:
   - do not treat non-success parser probes as valid file sizes;
   - require `206 Partial Content` before creating a chunked range plan;
   - surface `429/403` as real remote rejection instead of a corrupt tiny file.
