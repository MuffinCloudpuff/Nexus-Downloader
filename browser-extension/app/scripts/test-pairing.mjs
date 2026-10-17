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
await assert.rejects(pending, /自动配对超时/);
assert.equal(stalled.buildSnapshot().connectionState, "disconnected");
console.log("pairing tests passed: first connection, concurrent pairing, native error, timeout");
