# HTTP Range Lease Scheduler Plan

## Problem

The resilient subworker queue fixed whole-task failure on short HTTP range
reads, but it still lets a worker hold one large segment until that segment
finishes or fails. Near the tail, the queue can run out of pending segments
while a small number of slow active segments remain. The configured connection
ceiling is then irrelevant because there is no work left to assign.

Symptom: large HTTP downloads run fast in the middle and collapse near the last
few percent.

Direct trigger: only a few large or slow range segments remain active.

Root cause: range scheduling is segment-bound instead of lease-bound. Slow
segments are split only after recoverable failures, not as a normal part of
work distribution.

## Target

Keep the public protocol, task model, resume manifest format, and direct sparse
file writes unchanged.

Backend-only change:

- `range_segments` owns segment lease lifecycle.
- `downloader` requests only a bounded byte lease from a segment.
- A completed lease updates durable progress and returns the segment to the
  queue if bytes remain.
- Large remaining tails are split after normal lease completion or recoverable
  requeue, so idle workers can keep consuming the tail.

## Execution Model

1. Load or create direct-write manifest chunks as before.
2. Seed `SegmentQueue` with incomplete chunks.
3. Worker pops a segment and computes a lease:
   - lease start is the segment's durable `next_offset`;
   - lease end is `min(next_offset + lease_bytes - 1, segment.end)`.
4. Worker requests only that lease range and writes bytes at absolute offsets.
5. On lease completion:
   - if the segment is complete, mark it completed;
   - otherwise persist progress as pending and return the segment to the queue;
   - split a large remaining tail when enough bytes remain.
6. On recoverable request/read failure:
   - persist current progress as pending;
   - return the remaining segment to the queue;
   - split large remaining tails.

## Policy

- Small files still use one range path through the existing small-file guard.
- Generic HTTP uses short leases so unstable CDNs do not hold large ranges.
- Googlevideo can keep using the same lease scheduler with a smaller policy.
- Splitting remains coverage-preserving: original chunk keeps the lower range;
  new chunk starts at the split offset and is appended to the manifest.

## Observability

Add structured logs:

- `http range segment lease completed`
- `http range segment split`
- `http range segment requeued`

These logs must not include full signed URLs, cookies, or tokens.

## Verification

- Unit test lease bounds.
- Unit test normal lease completion requeues progress and splits large tails.
- Unit test recoverable requeue still preserves progress.
- Run `cargo test downloader --lib`, `cargo test range_segments --lib`,
  `cargo check`, and release build before replacing the portable backend.
