import assert from "node:assert/strict";
import {build} from "esbuild";
const listeners = {};
const records = new Map();
const operations = [];
globalThis.chrome = {
  runtime: {}, downloads: {
    search: ({id}, cb) => queueMicrotask(() => cb(records.has(id) ? [{...records.get(id)}] : [])),
    pause: (id, cb) => { operations.push(`pause:${id}`); records.get(id).paused = true; cb(); },
    resume: (id, cb) => { operations.push(`resume:${id}`); if (records.has(id)) records.get(id).paused = false; cb(); },
    onCreated: {addListener: fn => listeners.created = fn},
    onChanged: {addListener: fn => listeners.changed = fn},
    onDeterminingFilename: {addListener: fn => listeners.filename = fn},
    onErased: {addListener: fn => listeners.erased = fn},
  },
};
const result = await build({stdin: {contents: `
export * from "./src/background/browser-download-target";
export * from "./src/background/browser-download-request-policy";
`, resolveDir: process.cwd(), loader: "ts"}, bundle: true, platform: "node", format: "cjs", write: false});
const module = {exports: {}};
new Function("module", "exports", result.outputFiles[0].text)(module, module.exports);
const {listenForConfirmedDownloadTargets, createBrowserDownloadRequestPolicy} = module.exports;
const policy = createBrowserDownloadRequestPolicy();
const taken = [], imported = [];
let ready = true, reject = false, race;
listenForConfirmedDownloadTargets(async item => {
  if (race) { await race(item); return false; }
  if (!ready || reject) return false;
  if (!policy.canReplay(item)) return false;
  taken.push(item);
  records.get(item.id).state = "interrupted";
  return true;
}, {
  prepare: async item => ready && policy.canReplay(item),
  completed: async item => imported.push(item),
});
const flush = async () => { for (let i = 0; i < 6; i++) await new Promise(r => setImmediate(r)); };
let id = 0;
function file(ext, extra = {}) {
  const next = {id: ++id, filename: "", url: `https://files.test/download?id=${id}`, mime: "",
    state: "in_progress", paused: false, fileSize: 1, totalBytes: 1, ...extra};
  records.set(id, next);
  return next;
}
// Tiny, unknown-extension, zero-byte and extensionless files all enter the same gate.
for (const ext of ["zip", "7z", "rar", "exe", "pdf", "docx", "txt", "jpg", "png", "wav", "m4a", "mp4", "unusual", ""]) {
  const item = file(ext, {totalBytes: ext === "" ? 0 : 1});
  let suggested = 0;
  const ret = listeners.filename(item, () => { operations.push(`suggest:${item.id}`); suggested++; });
  assert.equal(ret, true);
  await flush();
  assert.equal(suggested, 1);
  assert.ok(operations.indexOf(`pause:${item.id}`) < operations.indexOf(`suggest:${item.id}`));
  assert.equal(taken.some(x => x.id === item.id), false, "never use a pre-Save-As suggested path");
  assert.equal(item.paused, true, "hold while the user is choosing a destination");
  item.filename = `E:/用户指定/原始名字${ext ? '.' + ext : ''}`;
  listeners.changed({id: item.id, filename: {current: item.filename}});
  listeners.changed({id: item.id, filename: {current: item.filename}});
  await flush();
  assert.equal(taken.filter(x => x.id === item.id).length, 1);
  assert.equal(taken.at(-1).filename, item.filename);
}
// onCreated with a final path, but no subsequent filename delta.
const created = file("zip", {filename: "D:/chosen/no-delta.zip"});
listeners.created(created); await flush();
assert.ok(taken.some(x => x.id === created.id));
// Completion races a capability check: the completion event is coalesced, not lost.
const fast = file("txt", {filename: "D:/chosen/fast.txt"});
race = async item => { records.get(item.id).state = "complete"; listeners.changed({id: item.id, state: {current: "complete"}}); };
listeners.created(fast); await flush(); race = undefined;
assert.equal(imported.filter(x => x.id === fast.id).length, 1);
// Browser-only memory files and POST responses are registered after saving, not cancelled/replayed.
for (const url of ["blob:https://site.test/id", "data:audio/wav;base64,AAAA", "file:///D:/source.dat"]) {
  const local = file("wav", {filename: "D:/chosen/audio.wav", url});
  listeners.created(local); await flush();
  assert.equal(taken.some(x => x.id === local.id), false);
  local.state = "complete";
  listeners.changed({id: local.id, state: {current: "complete"}}); await flush();
  assert.equal(imported.filter(x => x.id === local.id).length, 1);
  listeners.changed({id: local.id, filename: {current: local.filename}}); await flush();
  assert.equal(imported.filter(x => x.id === local.id).length, 1);
}
policy.record("https://files.test/export", "POST");
policy.record("https://files.test/export", "GET"); // Conservative: never discard a POST observation.
const post = file("xlsx", {url: "https://files.test/export", filename: "D:/chosen/export.xlsx"});
assert.equal(policy.canReplay(post), false);
listeners.created(post); await flush();
assert.equal(taken.some(x => x.id === post.id), false);
post.state = "complete"; listeners.changed({id: post.id, state: {current: "complete"}}); await flush();
assert.ok(imported.some(x => x.id === post.id));
// A backend outage does not pause normal browser downloads; a declined handoff releases our own pause.
ready = false;
const offline = file("zip"); let calls = 0;
listeners.filename(offline, () => calls++); await flush();
assert.equal(calls, 1); assert.equal(offline.paused, false);
ready = true; reject = true;
const declined = file("zip"); listeners.filename(declined, () => {}); await flush();
declined.filename = "D:/chosen/fallback.zip"; listeners.changed({id: declined.id, filename: {current: declined.filename}}); await flush();
assert.equal(declined.paused, false);
assert.ok(operations.includes(`resume:${declined.id}`));
// Never resume a download the user paused independently.
const userPaused = file("zip", {filename: "D:/chosen/manual.zip", paused: true});
listeners.created(userPaused); await flush();
assert.equal(userPaused.paused, true);
console.log("download coverage passed: all formats/sizes, pause-before-suggest, exact Save As path, duplicate/created/completion races, blob/data/file/POST import, offline fallback and owned-pause safety");
