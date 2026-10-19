# Browser download ownership

Download interception has no file-extension, MIME-type or minimum-size allowlist.
ZIP, installers, documents, images, audio, unknown extensions, extensionless and
empty files follow the same policy. The user's browser-confirmed absolute path
always wins over category-directory defaults.

For replayable HTTP(S) downloads, hold filename determination briefly while the
backend is checked and the browser transfer is paused. Do not choose a filename
or cancel Save As at this stage. Once the browser confirms the path, the existing
Rust accepts the task without writing, the extension clears the cancelled
browser record, and an explicit activation starts the Rust transfer to that
exact destination. This ordering prevents Chrome's cancelled-record cleanup
from racing the new file. Resume only pauses owned
by this extension if a takeover is declined. Do not touch user-paused transfers.

Blob/data/file downloads do not provide an HTTP source for Rust to re-fetch.
Likewise, POST-generated exports cannot safely be replayed as GET. Let the
browser save these files, then register the existing file through authenticated
`POST /api/browser-downloads/import`. This also handles HTTP files that finished
before an interception event was processed. This is an import, not a claim that
Rust performed a nonexistent network transfer: no copying, transcoding or second
fragment directory is involved.

Only a completed regular file of the expected size is accepted. Imports are
idempotent, retain the filename, bytes and category, and use the backend's normal
task/event/persistence stream so the tray, React UI and extension agree. Data-URL
bodies are never stored in task metadata. Imported files cannot be retried (a
revoked blob may have no source); reject retry before deleting any output.
Explicit task deletion removes the file and the task through normal cleanup.

Erase only the matching browser history entry after Rust accepts the task. On
any failure, keep the original file and browser record, and record the failure
for diagnostics. Never adopt old browser history through a broad search.

Backend `browser_download_import` and `browser_download_deferred_start`
capabilities are required. An old backend leaves
local browser files untouched; upgrading only the extension is insufficient.

Checks: extension `npm run test:coverage`, `test:downloads`, `test:handoff`, plus
Rust `browser_file_import_preserves_bytes_paths_and_task_lifecycle` and existing
browser-save-path/media regressions.
