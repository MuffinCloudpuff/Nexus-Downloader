# HTTP Range Resilient Subworker Plan

## Problem

The large HTTP task failure is not a timeout problem. Logs show repeated
`206 Partial Content` responses whose bodies ended before the requested range was
fully received. The current downloader treats that as a chunk failure after a
small fixed retry budget, and `res??` then fails the whole task.

Symptom: a 60 GB download slows near the tail and eventually fails.

Direct trigger: one fixed chunk repeatedly short-read or hit a body decode/read
error.

Root cause: the HTTP Range executor models chunks as one-shot jobs. It does not
have a long-lived recoverable subworker queue where a broken connection resumes
from the current byte offset, and it does not split large remaining slow ranges.

## Target Architecture

Keep the external protocol stable. Desktop UI and browser extension still see a
normal task with progress, speed, cancel, pause, and resume semantics.

Backend-only changes:

- `downloader.rs` remains the public `UniversalDownloader` entry point.
- A new internal `range_segments` module owns queued range state, progress
  accounting, retry health, and dynamic split decisions.
- `resume.rs` manifest format is reused for this iteration. Each chunk's
  `bytes_written` is the durable progress pointer. `Failed` is not a terminal
  in-task state; recoverable network failures return the chunk to the queue.
- `range_tuner.rs` continues to own active connection target selection.

## Execution Model

1. Build or load existing direct-write chunks from the resume manifest.
2. Seed a shared queue with every incomplete chunk.
3. Spawn worker tasks. Each worker:
   - acquires the adaptive limiter permit;
   - pops an incomplete segment;
   - requests `Range: bytes=current_offset-end` with `Accept-Encoding: identity`;
   - writes bytes to the final sparse file at absolute offsets;
   - persists progress periodically;
   - marks the segment complete only when `progress == end + 1`.
4. If the HTTP body ends early or the connection errors:
   - persist current progress as `Pending`;
   - record a recoverable failure metric;
   - put the remaining segment back into the queue unless the task is canceled.
5. The task fails only on unrecoverable conditions:
   - cancel token;
   - non-range response for a range request;
   - authorization/expired URL class errors;
   - disk write error;
   - no progress for too many consecutive recoverable attempts.
6. When a segment has a large remaining tail and the queue is empty, split the
   largest remaining segment so another worker can help. Respect a minimum split
   size guard so small files do not get over-fragmented.

## Split Policy

- Minimum splittable remainder: 16 MiB for generic HTTP.
- Split only if the remaining bytes are at least twice the minimum.
- Split the largest remaining active segment into two ranges:
  - original segment keeps `[current_progress, midpoint]`;
  - new segment gets `[midpoint + 1, original_end]`.
- Newly created split segments are appended to the same manifest stage so resume
  can continue from them.

## Observability

Add structured logs without leaking URLs, cookies, or tokens:

- `http range segment recoverable failure`
  - task id, host, chunk index, range start/end, next offset, attempts, error kind
- `http range segment requeued`
  - task id, chunk index, bytes written, remaining bytes
- `http range segment split`
  - task id, original chunk index, new chunk index, split offset
- `http range worker fatal`
  - task id, chunk index, status/error class

## Verification

Automated:

- Unit test range queue requeues an incomplete segment from its saved progress.
- Unit test dynamic split preserves full coverage without overlap.
- Unit test one recoverable short read does not mark the segment failed.

Manual:

- Run the known large HTTP URL.
- Confirm logs show recoverable segment requeue instead of whole task failure.
- Pause/cancel mid-download and confirm resume starts from stored progress.
- Confirm Bilibili/YouTube media paths still compile because they call the same
  `UniversalDownloader` surface.

## Non-goals

- Do not increase fixed retry counts as the main repair.
- Do not change desktop or browser-extension protocol fields in this step.
- Do not introduce a second permanent downloader path. This is the shared HTTP
  Range engine route.
