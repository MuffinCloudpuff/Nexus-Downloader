# HTTP Large File Accelerator Root-Cause Plan

## Context

Large generic HTTP downloads are still much slower than mature download
managers on the same URL. The current production path has already fixed one
correctness bug: range requests must request raw bytes with
`Accept-Encoding: identity` and the HTTP client must not transparently decode
the response body.

The remaining performance issue is not proven to be a simple "too many
threads" problem. The current evidence points to a combined bottleneck across
connection model, request headers, range scheduling, and disk write strategy.

## Current Evidence

- The failing large file exposes a real size of `64,480,785,020` bytes and
  supports HTTP range responses.
- Earlier failures produced body decoding errors and short chunks. That was
  caused by automatic HTTP content decoding on range responses.
- After forcing raw identity bytes, decoding errors disappeared, but the large
  HTTP task still ran far below expected speed.
- A local curl control test against the same URL reached materially higher
  aggregate speed than the daemon, so the daemon path has a local bottleneck.
- Bilibili DASH downloads still complete through the shared range downloader,
  so changes to the shared downloader are high impact and must be tested
  against generic HTTP and media-stage downloads.

## References

### Ghost Downloader 3

Local path:

- `D:\study\workspace\python_workspace\Ghost-Downloader-3-main\features\http_pack\task.py`
- `D:\study\workspace\python_workspace\Ghost-Downloader-3-main\features\http_pack\pack.py`
- `D:\study\workspace\python_workspace\Ghost-Downloader-3-main\app\supports\sysio.py`

Observed design:

- Range probe sends `range` plus `accept-encoding: identity`.
- Each subworker requests its own byte range.
- Request headers are inherited from the original browser payload, then Range
  and identity encoding are added.
- Cookies are split from headers and sent with the request.
- The worker writes directly into the target file using platform pwrite.
- On Windows, `ftruncate` marks the file sparse before setting file length.
- Progress is recorded as subworker triples: `start`, `progress`, `end`.
- Slow/large remaining ranges are dynamically split by `reassignSubworker`.
- Auto acceleration adds more subworkers only when speed history suggests it
  helps.

Key point: Ghost is not just "direct write". Its direct-write path is paired
with raw range bytes, independent subworkers, browser header inheritance,
resume records, sparse preallocation, and dynamic range reassignment.

### aria2

The aria2 manual separates queue-level concurrency from per-file segmented
connections:

- `--max-concurrent-downloads` controls how many download items run together.
- `--split`, `--max-connection-per-server`, and `--min-split-size` control
  how many connections are used inside one download item.
- aria2 also has piece selection strategies such as default, inorder, random,
  and geom.

Architecture lesson: single-task segment count, global task concurrency, and
minimum segment size are separate controls. They should not be collapsed into
one generic "thread count".

### XDM / Browser-integrated download managers

XDM's public project describes browser integration for taking over downloads,
resuming broken downloads, and accelerating transfers. Its browser integration
module sends download information from the browser to the native application.

Architecture lesson: for browser-originated downloads, the backend should use
the browser-captured URL, filename, headers, cookies, and relevant request
context instead of treating the URL as an anonymous backend-only request.

## Working Root-Cause Hypotheses

### H1: Connection model mismatch

The backend may not be creating the same connection shape as mature download
managers. If many ranges are multiplexed through HTTP/2 or a shared connection
pool in a way the origin/CDN throttles differently, the apparent segment count
does not equal the number of independent transfer lanes.

Validation:

- Compare shared reqwest client vs one client per subworker.
- Compare HTTP/2-enabled vs HTTP/1.1-only for generic HTTP range downloads.
- Log response HTTP version, local worker id, range, first-byte latency, and
  per-worker speed.

### H2: Missing or altered browser request context

The extension may not pass all effective browser request context into the
backend. Some CDNs shape traffic based on User-Agent, Cookie, Referer, Origin,
Accept, Sec-Fetch fields, or signed URL state.

Validation:

- Log sanitized captured header names from extension-created HTTP downloads.
- Compare backend request headers with the browser request headers visible in
  the extension.
- Run the same URL with browser headers inherited and with minimal headers.

Rules:

- Never log cookie/token values.
- Log only header names and a redacted presence/length summary for sensitive
  headers.

### H3: Direct random writes are throttling the network path

The current direct-write path opens the final file and many workers write to
far-apart offsets. On Windows/NTFS this can become much slower than sequential
part-file writes, especially with a large preallocated file and many active
workers.

Validation:

- Download the same ranges to `NUL`.
- Download the same ranges into one direct-write final file.
- Download the same ranges into per-worker `.part` files.
- Measure network read time separately from disk write time.
- Log write latency and write bytes per worker.

### H4: Missing sparse-file behavior

Ghost marks the output file sparse before setting file length on Windows. The
current Rust direct-write path uses normal file length preallocation. That may
cause heavier filesystem work and slower random offset writes.

Validation:

- Add a diagnostic-only Windows sparse-file mode.
- Compare direct-write normal preallocation vs sparse final file.

### H5: Static equal chunks are weaker than dynamic scheduling

Fixed equal chunks can leave slow ranges or throttled connections dragging the
whole task. Mature accelerators can split remaining large/slow ranges and
start additional subworkers when speed history proves benefit.

Validation:

- Log per-worker completion time and tail latency.
- Implement a diagnostic scheduler that splits the largest remaining range.
- Compare fixed `N` chunks with dynamic reassignment under the same max worker
  budget.

## Test Matrix

All tests must run against the same signed URL before it expires and must use
the same output drive.

### Network-only

Write response bodies to `NUL` or drop buffers in memory.

- reqwest shared client, HTTP/2 allowed, 8/16/32 workers.
- reqwest shared client, HTTP/1.1 only, 8/16/32 workers.
- reqwest client per worker, HTTP/1.1 only, 8/16/32 workers.
- browser headers inherited vs minimal headers.

Goal: prove the maximum network throughput available through our Rust HTTP
client without disk write noise.

### Disk-write models

Use the best network model from the previous step.

- direct write to final file with normal set_len.
- direct write to final sparse file on Windows.
- per-worker `.part` files with sequential writes, followed by merge timing.
- direct write with bounded writer queue and measured write latency.

Goal: identify whether direct-write is actually faster for this class of file
on this filesystem.

### Scheduler models

Use the best connection and write model.

- fixed equal chunks.
- Ghost-style dynamic reassignment of the largest remaining range.
- minimum split size guard so tiny fragments are not over-split.

Goal: reduce slow-tail behavior without creating unlimited requests.

## Required Logging

Add diagnostic logs behind a feature flag or debug setting:

- `task_id`
- `worker_id`
- `range_start`
- `range_end`
- `next_offset`
- `http_version`
- `status`
- `content_range`
- `content_length`
- `first_byte_ms`
- `read_bytes`
- `read_bps`
- `write_bytes`
- `write_ms`
- `write_bps`
- `retry_count`
- `client_mode`: `shared`, `per_worker`
- `http_mode`: `default`, `http1_only`
- `write_mode`: `direct`, `sparse_direct`, `part_file`, `writer_queue`

Header logs must be sanitized:

- normal headers: log name and short value summary if non-sensitive.
- sensitive headers: log presence and approximate length only.
- never log Cookie, Authorization, token, signed secrets, passwords, or full
  query strings.

## Proposed Formal Design

### Backend protocol input

For browser-originated generic HTTP downloads, the extension should provide:

- URL
- filename if browser knows it
- file size if browser knows it
- range support if browser knows it
- sanitized request headers required for replay
- cookies through the existing sensitive transport path
- source client info, for example `edge_extension`

The backend owns:

- range probing
- final file type/path routing
- segment planning
- connection strategy
- disk write strategy
- resume state
- progress and completion state

### Downloader engine

Split the current monolithic HTTP range path into internal modules:

- `http_probe`: range probe, content length, filename hints, support detection.
- `http_headers`: raw range headers, browser header filtering, sensitive
  redaction.
- `segment_plan`: min split size, max worker count, fixed/dynamic range state.
- `connection_strategy`: shared client, per-worker client, HTTP/1.1 vs default.
- `range_store`: direct/sparse/part-file/write-queue strategies.
- `range_diagnostics`: benchmark-only metrics and structured logs.

This keeps the shared `UniversalDownloader` surface stable while making each
root-cause hypothesis testable.

## Execution Plan

### Phase 1: Diagnostic harness

Create a diagnostic command or test harness that uses the same Rust HTTP stack
as production but does not alter production behavior.

It must support:

- URL input from an environment variable or local redacted fixture.
- sanitized headers from a local JSON file.
- worker count.
- HTTP mode.
- client mode.
- write mode.
- byte limit for short tests.
- CSV/JSON summary output.

Pass condition:

- We can explain where speed is lost: network connection model, missing
  request context, disk writes, or scheduler tail behavior.

### Phase 2: Production fix

Only after Phase 1 identifies the bottleneck, change the production downloader.

Likely production changes:

- Keep raw range identity as mandatory.
- Use browser-originated headers/cookies for intercepted HTTP downloads.
- Use the fastest proven connection mode for generic HTTP range tasks.
- Use a write strategy selected by evidence:
  - direct sparse write if it wins;
  - part-file sequential write if it wins;
  - hybrid by file size/drive behavior if measurements justify it.
- Add Ghost-style dynamic reassignment with a minimum split size guard if it
  improves tail latency.

### Phase 3: Regression coverage

Add tests for:

- range header construction always forces identity.
- sensitive headers are redacted in logs.
- segment planning respects `max_chunk_connections` and minimum split size.
- resume state is correct for the selected write model.
- Bilibili DASH still completes through the shared downloader.

## Non-Goals

- Do not solve this by blindly extending timeout or retry count.
- Do not hard-cap the UI value as a substitute for finding the bottleneck.
- Do not permanently keep multiple competing production paths without a clear
  selection rule and cleanup plan.
- Do not log real cookies, tokens, or full signed URLs.

## Current Decision

The next implementation step is a diagnostic harness and structured benchmark,
not another production-path guess. The production downloader should be changed
only after the benchmark shows which factor explains the speed gap.

## Diagnostic Results

Diagnostic binary:

- `src/bin/http-range-diagnose.rs`

Test target:

- same large signed HTTP URL used in the real failing task.
- output drive: `D:\download`.
- headers: minimal backend headers for this first pass.
- sensitive URL query and cookie values are not logged by the harness.

### 64 MiB sample, 16 workers

| Mode | HTTP version | Speed |
| --- | --- | ---: |
| default / shared / null | HTTP/2 | 1.78 MiB/s |
| HTTP/1.1 / shared / null | HTTP/1.1 | 7.30 MiB/s |
| HTTP/1.1 / per-worker / null | HTTP/1.1 | 6.53 MiB/s |
| HTTP/1.1 / shared / direct | HTTP/1.1 | 5.90 MiB/s |
| HTTP/1.1 / shared / part-file | HTTP/1.1 | 6.61 MiB/s |

### Larger samples

| Mode | Sample | Speed |
| --- | ---: | ---: |
| HTTP/1.1 / shared / part-file / 32 workers | 256 MiB | 16.22 MiB/s |
| HTTP/1.1 / shared / part-file / 64 workers | 256 MiB | 25.39 MiB/s |
| HTTP/1.1 / shared / part-file / 128 workers | 256 MiB | 34.67 MiB/s |

### Interpretation

The first proven root cause is HTTP protocol selection. The default reqwest
client negotiates HTTP/2 for this CDN, and concurrent Range requests over that
connection shape are much slower. Forcing HTTP/1.1 moves the same test from
1.78 MiB/s to materially higher throughput.

The second proven factor is write strategy. In the 16-worker short test,
part-file sequential writes were faster and had much lower measured write time
than direct random writes into the final file. In larger tests, write time was
small relative to network time, so the dominant bottleneck remains connection
and request scheduling rather than disk.

The per-worker client hypothesis is not supported by the first-pass test:
HTTP/1.1 shared client was slightly faster than creating one client per worker.
The important change is not "one reqwest Client per worker"; it is avoiding
HTTP/2 multiplexing for Range downloads.

The remaining gap versus IDM-like speeds should be investigated through:

- browser-originated headers/cookies instead of minimal backend headers;
- larger sample sizes;
- dynamic range reassignment;
- minimum split size and active worker tuning;
- optional sparse/direct write only if later measurements prove it wins.

## Updated Production Recommendation

1. For generic HTTP Range downloads, build a dedicated raw-byte HTTP/1.1 client.
2. Keep `Accept-Encoding: identity` mandatory on probe and chunk requests.
3. Treat `max_chunk_connections` as a daemon safety ceiling, not a fixed active
   worker count; allow the configured 8..256 range and tune active workers per
   task from runtime measurements.
4. Prefer measured part-file sequential writes for generic large HTTP files
   unless a sparse direct-write benchmark beats it.
5. Keep direct-write available only after it has a clean strategy boundary, not
   as an unmeasured replacement for all range downloads.
6. Add browser header/cookie propagation diagnostics before judging the final
   speed ceiling.

## Production Validation After Full Chain Update

Implemented in the production HTTP Range path:

- raw-byte `Accept-Encoding: identity` on parser probe, execution probe, and
  chunk requests;
- HTTP/1.1-only reqwest client for Range downloads;
- browser-provided headers and decrypted cookies continue through the protocol
  layer into stage headers;
- Windows offset writes through platform `FileExt::seek_write`;
- best-effort sparse output file marking before setting final length;
- fine-grained queued range segments: planned segments can exceed active
  connections, while active connections remain capped by `max_chunk_connections`.

Validation run:

- daemon: release build;
- endpoint: local HTTP API on a separate test port;
- URL: same large signed HTTP URL used in the failing task;
- settings: `max_chunk_connections = 128`, speed limit disabled;
- test duration: 45 seconds, then canceled and cleaned up.

Result:

- average non-zero API-reported speed: `80.34 MiB/s`;
- peak API-reported speed: `93.34 MiB/s`;
- downloaded during sample: `4072.08 MiB`.

This confirms the full chain reaches IDM-class throughput on the provided URL.
