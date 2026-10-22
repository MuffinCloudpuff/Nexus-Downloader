import assert from "node:assert/strict";
import {build} from "esbuild";

const stored = {};
let nativeCall;
let nativeCount = 0;
globalThis.self = {setTimeout, clearTimeout};
globalThis.chrome = {
  storage: {local: {
    get: async (defaults) => ({...defaults, ...stored}),
    set: async (values) => Object.assign(stored, values),
  }},
  runtime: {
    getManifest: () => ({version: "test"}),
    getURL: (path) => `chrome-extension://test/${path}`,
    sendNativeMessage: (host, message, callback) => {
      assert.equal(host, "com.gd3.nexusdownloader");
      assert.equal(message.action, "get_token");
      nativeCount++;
      nativeCall(callback);
    },
  },
};
globalThis.fetch = async (_url, options) => {
  assert.equal(options.headers.get("Authorization"), "Bearer test-native-token");
  return Response.json({tasks: [{task_id: "shared-task", status: "completed"}]});
};
const bundled = await build({
  entryPoints: ["src/background/daemon-client.ts"], bundle: true,
  platform: "node", format: "cjs", target: "node22", write: false,
});
const module = {exports: {}};
new Function("module", "exports", `${bundled.outputFiles[0].text}\n//# sourceURL=daemon-client-test-bundle.cjs`)(module, module.exports);
const {createDaemonClient} = module.exports;

// A fresh client automatically obtains a native token before requesting tasks.
nativeCall = (callback) => queueMicrotask(() => callback({token: "test-native-token"}));
const fresh = createDaemonClient();
await fresh.loadPersistentState();
assert.equal(fresh.hasToken(), false);
assert.equal((await fresh.connect())[0].task_id, "shared-task");
assert.equal(fresh.buildSnapshot().connectionState, "connected");
assert.equal(nativeCount, 1);
await fresh.connect();
assert.equal(nativeCount, 1);

// Concurrent connection attempts share a single native pairing operation.
const concurrent = createDaemonClient();
const before = nativeCount;
assert.deepEqual(await Promise.all([concurrent.requestPairing(), concurrent.requestPairing()]),
  ["test-native-token", "test-native-token"]);
assert.equal(nativeCount, before + 1);

// Preserve the actual native error rather than reporting an empty task snapshot
// and masking a failed handshake as the normal "waiting for pairing" state.
nativeCall = (callback) => {
  chrome.runtime.lastError = {message: "Specified native messaging host not found"};
  callback(undefined);
  delete chrome.runtime.lastError;
};
const failed = createDaemonClient();
await assert.rejects(failed.connect(), /native messaging host not found/);
assert.equal(failed.buildSnapshot().connectionState, "disconnected");
assert.match(failed.buildSnapshot().connectionMessage, /native messaging host not found/);

// A nonresponsive host must not leave auto-reconnect stuck in "connecting".
let expire;
self.setTimeout = (callback) => { expire = callback; return 0; };
self.clearTimeout = () => {};
nativeCall = () => {};
const stalled = createDaemonClient();
const pending = stalled.requestPairing();
expire();
await assert.rejects(pending, /等待后台启动/);
assert.equal(stalled.buildSnapshot().connectionState, "disconnected");
console.log("pairing tests passed: first connection, concurrent pairing, native error, timeout");

// A new extension must never cancel a browser download for an older daemon
// that would silently discard the selected save_path.
self.setTimeout = setTimeout;
self.clearTimeout = clearTimeout;
globalThis.fetch = async () => Response.json({browser_download_save_path: true});
assert.equal(await fresh.supportsBrowserSavePath(), false, "an old backend cannot safely defer transfer until browser cleanup");
globalThis.fetch = async () => Response.json({browser_download_save_path: true, browser_download_deferred_start: true});
assert.equal(await fresh.supportsBrowserSavePath(), true);
globalThis.fetch = async () => new Response("Not Found", {status: 404});
assert.equal(await fresh.supportsBrowserSavePath(), false);
globalThis.fetch = async () => Response.json({});
assert.equal(await fresh.supportsBrowserSavePath(), false);
console.log("daemon save-path capability compatibility tests passed");

// Offline native bridge does not create/store a token. A later retry pairs itself.
nativeCall = callback => callback({error: "daemon_unavailable", retryable: true});
const waiting = createDaemonClient();
await assert.rejects(waiting.connect(), /等待后台启动/);
assert.equal(waiting.hasToken(), false);
assert.equal(waiting.buildSnapshot().connectionState, "disconnected");
nativeCall = callback => callback({token: "test-native-token"});
globalThis.fetch = async () => Response.json({tasks: []});
await waiting.connect();
assert.equal(waiting.isReady(), true);

// Revoked/stale token is cleared, and the next automatic attempt repairs it.
globalThis.fetch = async () => new Response("unauthorized", {status: 401});
await assert.rejects(waiting.connect());
assert.equal(waiting.hasToken(), false);
globalThis.fetch = async () => Response.json({tasks: []});
await waiting.connect();
assert.equal(waiting.isReady(), true);

// Persisted state survives service-worker suspension; alarms drive retries.
const resumed = createDaemonClient();
await resumed.loadPersistentState();
await resumed.connect();
assert.equal(resumed.isReady(), true);
globalThis.fetch = async () => { throw new TypeError("Failed to fetch"); };
await assert.rejects(resumed.connect());
assert.match(resumed.buildSnapshot().connectionMessage, /等待后台启动/);
globalThis.fetch = async () => Response.json({tasks: []});
await resumed.connect();
assert.equal(resumed.isReady(), true);
console.log("automatic pairing recovery passed: offline, startup, revoked token, worker reload, restart");

let retryAlarm;
chrome.alarms = {create: (name, options) => { retryAlarm = {name, options}; }};
resumed.ensureReconnectAlarm();
assert.equal(retryAlarm.options.periodInMinutes, 1);
let alarmProbe;
resumed.onReconnectAlarm({name: 'unrelated'}, () => { throw new Error('Unrelated alarm must be ignored'); });
globalThis.fetch = async () => { throw new TypeError('offline'); };
resumed.onReconnectAlarm({name: retryAlarm.name}, () => { alarmProbe = resumed.connect(); });
await assert.rejects(alarmProbe);
assert.match(resumed.buildSnapshot().connectionMessage, /等待后台启动/);
globalThis.fetch = async () => Response.json({tasks: []});
resumed.onReconnectAlarm({name: retryAlarm.name}, () => { alarmProbe = resumed.connect(); });
await alarmProbe;
assert.equal(resumed.isReady(), true);
console.log('reconnect alarm passed: detects offline while previously connected, then recovers');
