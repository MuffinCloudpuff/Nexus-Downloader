# HTTP Range Direct Write Plan

## Context

The daemon currently downloads HTTP range chunks into separate `chunk_*.part`
files and merges them into the final output file after all chunks complete.
This works, but it adds a full-file merge pass and makes large downloads spend
extra disk I/O at the end.

Ghost's original HTTP downloader uses a different model: each range worker
requests raw bytes with `Accept-Encoding: identity` and writes the received
bytes directly into the final file at the worker's byte offset.

## Decision

HTTP range downloads should use direct offset writes:

1. Probe or parse the total file size as before.
2. Build a range chunk plan from `total_size` and `max_chunk_connections`.
3. Create or open the final output file before workers start.
4. Preallocate the final file to `total_size`.
5. For each chunk, request `Range: bytes=start-end`.
6. Force raw bytes with `Accept-Encoding: identity` and a reqwest client with
   automatic content decoding disabled.
7. Write each received buffer to `output_path` at the current chunk offset.
8. Record each chunk's `bytes_written` in the resume manifest.
9. When all chunks are completed, mark the task complete without a merge step.

## Module Boundaries

- `src/core/engine/downloader.rs`
  Owns HTTP range execution, raw-byte request headers, file preallocation,
  direct offset writes, progress reporting, and manifest updates.

- `src/core/resume.rs`
  Keeps the existing chunk manifest data model. `path` remains for backward
  compatibility but is no longer used as the chunk part file path by direct
  write mode.

- `src/services/task_artifact_service.rs`
  Existing cleanup already removes the final output file, `temp/<task_id>`, and
  the resume manifest. This remains sufficient.

## Resume Semantics

Each chunk tracks:

- `start`
- `end`
- `bytes_written`
- `status`

On resume:

- Completed chunks are skipped.
- Incomplete chunks resume at `start + bytes_written`.
- The write offset is always the same value used in the Range request.
- If the output file is missing or smaller/larger than expected, the current
  manifest is not trusted and the chunk plan is recreated.

## Failure Handling

The root correctness rule is that range chunks are file byte ranges. Therefore
range requests must not allow transparent HTTP content decoding. If a chunk
returns a non-206 status, reads fewer bytes than expected, or fails while
streaming, that chunk is retried according to the existing retry policy.

## Scope

This change affects:

- generic HTTP direct downloads;
- Bilibili DASH video/audio stage downloads, because they share the same range
  downloader;
- pause/resume behavior;
- cancellation cleanup.

This change does not require a protocol change for the desktop UI or browser
extension.
