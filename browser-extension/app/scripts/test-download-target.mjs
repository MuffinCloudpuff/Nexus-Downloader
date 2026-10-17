import assert from "node:assert/strict";
import {build} from "esbuild";
import {createRequire} from "node:module";
import React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {readFile} from "node:fs/promises";

const manifest = JSON.parse(await readFile("public/manifest.json", "utf8"));
assert.match(manifest.content_security_policy.extension_pages, /img-src 'self' http:\/\/127\.0\.0\.1:14371;/);

const listeners = {};
let current;
globalThis.self = globalThis;
globalThis.chrome = {
  storage: {local: {get: async values => values, set: async () => {}}},
  runtime: {getManifest: () => ({version: "test"}), getURL: (p) => `chrome-extension://test/${p}`},
  downloads: {
    onDeterminingFilename: {addListener: fn => listeners.suggest = fn},
    onChanged: {addListener: fn => listeners.changed = fn},
    search: (_query, callback) => queueMicrotask(() => callback(current ? [current] : [])),
  },
};
const result = await build({stdin: {contents: `
  export * from "./src/background/browser-download-target";
  export {daemonTaskToGenericTask} from "./src/shared/protocol/daemon";
  export {taskVisual} from "./src/shared/utils";
  export {TaskCard} from "./src/popup/components/TaskCard";
  export {createResourceBridge} from "./src/background/resource-bridge";
`, resolveDir: process.cwd(), loader: "ts"}, bundle: true, platform: "node", format: "cjs",
  packages: "external", write: false});
const module = {exports: {}};
new Function("module", "exports", "require", result.outputFiles[0].text)(module, module.exports, createRequire(import.meta.url));
const {listenForConfirmedDownloadTargets, isAbsoluteDownloadPath, daemonTaskToGenericTask, taskVisual, TaskCard, createResourceBridge} = module.exports;
for (const path of ["D:\\用户目录\\原图 (2).jpg", "C:/chosen/image.png", "\\\\server\\share\\image.jpg", "/home/user/image.jpg"]) {
  assert.ok(isAbsoluteDownloadPath(path), path);
}
for (const path of ["image.jpg", "D:image.jpg", "D:/folder/", "", "D:/../photo.jpg", "D:/photo.jpg:stream", "\\\\.\\NUL", "\\\\?\\D:\\photo.jpg"]) {
  assert.equal(isAbsoluteDownloadPath(path), false, path);
}
const intercepted = [];
let finish;
listenForConfirmedDownloadTargets(async item => { intercepted.push(item); await new Promise(r => finish = r); });
const flush = () => new Promise(r => setImmediate(r));
// Suggested paths may look absolute. They are not the selected destination.
let suggested = 0;
listeners.suggest({id: 1, filename: "C:/Downloads/suggested.jpg"}, () => suggested++);
await flush();
assert.equal(suggested, 1);
assert.equal(intercepted.length, 0);
// Cancel Save As: no final filename, therefore no takeover.
listeners.changed({id: 1, state: {current: "interrupted"}});
await flush();
assert.equal(intercepted.length, 0);
current = {id: 2, filename: "D:/我的目录/原图.jpg", mime: "image/jpeg", state: "in_progress"};
listeners.changed({id: 2, filename: {current: current.filename}});
listeners.changed({id: 2, filename: {current: current.filename}});
await flush();
assert.equal(intercepted.length, 1);
assert.equal(intercepted[0].filename, "D:/我的目录/原图.jpg");
finish(); await flush();
// Images that finished while the handler was starting stay with the browser.
current = {id: 3, filename: "D:/chosen/fast.png", state: "complete"};
listeners.changed({id: 3, filename: {current: current.filename}});
await flush();
assert.equal(intercepted.length, 1);
// The authoritative output extension wins over a title without an extension.
const photo = daemonTaskToGenericTask({task_id: "photo", title: "photo", output_path: "D:/x.com/photo.jpg",
  output_category: "image", status: "completed", artifact_state: "available"});
assert.equal(photo.fileExt, "jpg");
assert.equal(taskVisual(photo).kind, "image");
const markup = renderToStaticMarkup(React.createElement(TaskCard, {task: photo, onAction: () => {}}));
assert.match(markup, /system-icon\?ext=jpg/);
assert.match(markup, /图片/);
assert.doesNotMatch(markup, /alt="X"/);
const archive = {...photo, taskId: "zip", title: "a.zip", outputPath: "D:/a.zip", outputCategory: "archive"};
assert.match(renderToStaticMarkup(React.createElement(TaskCard, {task: archive, onAction: () => {}})), /system-icon\?ext=zip/);
let dispatched;
const bridge = createResourceBridge({createTask: async req => { dispatched = req; return {ok: true}; }});
await bridge.handoffBrowserDownload({id: 6, filename: "E:/用户选择/renamed.jpg", url: "https://example.test/image?format=png",
  mime: "image/jpeg", state: "in_progress", totalBytes: 100});
assert.equal(dispatched.payload.save_path, "E:/用户选择/renamed.jpg");
assert.equal(dispatched.payload.filename, "renamed.jpg");
assert.equal(dispatched.payload.mime, "image/jpeg");
await assert.rejects(bridge.handoffBrowserDownload({id: 7, filename: "not-confirmed.jpg"}), /完整保存路径/);
console.log("download target tests passed: confirmed path, Save As cancellation, duplicate events, completed files, image classification and Windows icon URLs");
