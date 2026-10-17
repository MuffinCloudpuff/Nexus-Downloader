import assert from "node:assert/strict";
import {build} from "esbuild";
import {readFile} from "node:fs/promises";

const background = await readFile("src/background.ts", "utf8");
assert.match(background, /await handoffAndClearBrowserDownload\(\s*downloadItem,\s*resourceBridge\.handoffBrowserDownload/);
assert.ok(background.indexOf("await cancelDownload(downloadItem.id)") < background.indexOf("await handoffAndClearBrowserDownload("));
assert.doesNotMatch(background, /eraseFromHistory/); // No disabled-by-default cleanup path.

let erase;
globalThis.chrome = {
  storage: {local: {}}, runtime: {},
  downloads: {
    erase: (query, callback) => erase(query, callback),
    removeFile: () => assert.fail("history cleanup must never delete a file"),
    setUiOptions: () => assert.fail("history cleanup must not hide other downloads"),
  },
};
const bundled = await build({entryPoints: ["src/background/browser-download-handoff.ts"],
  bundle: true, platform: "node", format: "cjs", write: false});
const module = {exports: {}};
new Function("module", "exports", bundled.outputFiles[0].text)(module, module.exports);
const {handoffAndClearBrowserDownload} = module.exports;
const item = Object.freeze({id: 41, filename: "E:/用户指定的位置/压缩包 (2).zip",
  url: "https://example.test/package.zip", state: "in_progress"});
const cancelled = {id: 41, state: "interrupted", error: "USER_CANCELED"};
const normal = {id: 42, state: "in_progress", url: item.url};
const completed = {id: 43, state: "complete"};
const failed = {id: 44, state: "interrupted", error: "NETWORK_FAILED"};
const history = new Map([cancelled, normal, completed, failed].map(record => [record.id, record]));
const operations = [];
erase = (query, callback) => {
  operations.push("erase");
  assert.deepEqual(query, {id: 41}); // No broad filters or empty queries.
  history.delete(query.id);
  callback([query.id]);
};
let accept;
let dispatched = 0;
const pending = handoffAndClearBrowserDownload(item, async received => {
  dispatched++;
  operations.push("handoff-started");
  assert.equal(received, item);
  assert.equal(received.filename, "E:/用户指定的位置/压缩包 (2).zip");
  await new Promise(resolve => accept = resolve);
  operations.push("handoff-accepted");
});
assert.deepEqual(operations, ["handoff-started"]);
assert.equal(history.has(41), true, "do not clear while awaiting backend acceptance");
accept();
assert.deepEqual(await pending, {historyCleared: true});
assert.deepEqual(operations, ["handoff-started", "handoff-accepted", "erase"]);
assert.equal(history.has(41), false);
assert.deepEqual([...history.values()], [normal, completed, failed]);
assert.equal(dispatched, 1);

// HTTP/native dispatch rejection preserves the original cancelled entry.
history.set(41, cancelled);
erase = () => assert.fail("rejected handoffs must not clear history");
await assert.rejects(handoffAndClearBrowserDownload(item, async () => {
  throw new Error("backend rejected download");
}), /backend rejected/);
assert.equal(history.get(41), cancelled);

// A Chrome history error is nonfatal after successful task creation.
erase = (_query, callback) => {
  chrome.runtime.lastError = {message: "history temporarily unavailable"};
  callback([]);
  delete chrome.runtime.lastError;
};
let acceptedCount = 0;
const warning = await handoffAndClearBrowserDownload(item, async () => { acceptedCount++; });
assert.equal(warning.historyCleared, false);
assert.match(warning.cleanupError.message, /history temporarily unavailable/);
assert.equal(acceptedCount, 1, "cleanup failure must not dispatch another task");
assert.equal(history.get(41), cancelled);
assert.equal(item.filename, "E:/用户指定的位置/压缩包 (2).zip");

// Capture the original ID before asynchronous handoff code runs.
const mutable = {...item};
erase = (query, callback) => { assert.deepEqual(query, {id: 41}); callback([41]); };
assert.deepEqual(await handoffAndClearBrowserDownload(mutable, async received => {
  received.id = 42;
}), {historyCleared: true});
console.log("handoff cleanup tests passed: acceptance-before-erase, exact ID only, unaffected downloads/files/UI, original save path, rejected handoff, nonfatal cleanup errors");
