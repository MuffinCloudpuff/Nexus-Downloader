import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {build} from "esbuild";
import React from "react";
import {renderToStaticMarkup} from "react-dom/server";

const stored = {featureTabState: {webrtc: [7], search: [7], catch: [8]}};
const injections = [];
globalThis.chrome = {
  runtime: {
    getManifest: () => ({version: "test"}),
    getURL: (path) => `chrome-extension://test/${path}`,
  },
  storage: {local: {
    get: async (defaults) => ({...defaults, ...stored}),
    set: async (values) => Object.assign(stored, values),
  }},
  scripting: {executeScript: async (options) => { injections.push(options); }},
};

const bundled = await build({
  stdin: {
    contents: `
      export {App} from "./src/popup/App";
      export {Header} from "./src/popup/components/Header";
      export {AdvancedPage} from "./src/popup/components/AdvancedPage";
      export {ADVANCED_FEATURES} from "./src/shared/constants";
      export {CAT_CATCH_SCRIPT_FEATURES} from "./src/shared/cat-catch";
      export {createFeatureBridge} from "./src/background/feature-bridge";
    `,
    resolveDir: process.cwd(),
  },
  bundle: true, platform: "node", format: "cjs", packages: "external",
  target: "node22", jsx: "automatic", write: false, metafile: true,
});
const module = {exports: {}};
new Function("require", "module", "exports", `${bundled.outputFiles[0].text}\n//# sourceURL=popup-test-bundle.cjs`)(
  createRequire(import.meta.url), module, module.exports,
);
const {App, Header, AdvancedPage, ADVANCED_FEATURES, CAT_CATCH_SCRIPT_FEATURES, createFeatureBridge} = module.exports;
const noop = () => {};
const header = renderToStaticMarkup(React.createElement(Header, {
  currentView: "tasks", connectionState: "connected", connectionMessage: "已连接",
  mediaDownloadOverlayEnabled: true, interceptEnabled: true,
  onViewChange: noop, onMediaDownloadOverlayToggle: noop, onInterceptToggle: noop,
}));
assert.match(header, /下载任务/);
assert.match(header, /高级功能/);
assert.match(header, /设置/);
assert.doesNotMatch(header, /资源嗅探|value="resources"/);
assert.ok(!Object.keys(bundled.metafile.inputs).some((file) => file.endsWith("/ResourcesPage.tsx")),
  "hidden resource page must not be mounted or bundled into the popup");

assert.deepEqual(ADVANCED_FEATURES.map((feature) => feature.key),
  ["recorder", "recorder2", "mobileUserAgent", "search", "catch"]);
assert.equal("webrtc" in CAT_CATCH_SCRIPT_FEATURES, false);
const advanced = renderToStaticMarkup(React.createElement(AdvancedPage, {
  featureStates: Object.fromEntries(ADVANCED_FEATURES.map(({key}) => [key, false])),
  isFeatureBusy: () => false, onFeatureToggle: noop,
  mediaItems: [], mediaPlaybackState: {
    available: false, message: "", tabId: null, mediaIndex: -1,
    currentTime: 0, duration: 0, progress: 0, volume: 1,
    paused: true, loop: false, muted: false, speed: 1,
  },
  onMediaItemChange: noop, onMediaAction: noop,
}));
assert.doesNotMatch(advanced, /WebRTC|webrtc/);
for (const feature of ADVANCED_FEATURES) assert.ok(advanced.includes(feature.title));
assert.match(advanced, /需刷新/); // Keep truthful warnings for retained features.
const initialPopup = renderToStaticMarkup(React.createElement(App, {
  themePreference: "system", onThemePreferenceChange: noop,
}));
assert.doesNotMatch(initialPopup, /资源嗅探/);
assert.match(initialPopup, /下载此媒体/);

// Retired settings from an older version cannot silently reactivate WebRTC.
const bridge = createFeatureBridge();
await bridge.loadPersistentState();
assert.equal("webrtc" in stored.featureTabState, false);
assert.equal("webrtc" in bridge.createFeatureStateMap(7), false);
assert.equal(bridge.createFeatureStateMap(7).search, true);
assert.equal(bridge.createFeatureStateMap(8).catch, true);
await assert.rejects(bridge.toggleFeature("webrtc", 7), /停用|不受支持/);
await bridge.toggleFeature("recorder", 7);
assert.ok(injections.some((options) => options.files?.includes("catch-script/recorder.js")));
assert.ok(!injections.some((options) => options.files?.some((file) => file.includes("webrtc"))));

// Hiding the resource UI must not remove the actual capture/download backend.
const background = await build({
  entryPoints: ["src/background.ts"], bundle: true, platform: "browser",
  format: "esm", write: false, metafile: true,
});
assert.ok(Object.keys(background.metafile.inputs).some((file) => file.endsWith("/background/resource-bridge.ts")));
console.log("popup tests passed: hidden resource page, retained controls/capture, retired WebRTC and old settings");
