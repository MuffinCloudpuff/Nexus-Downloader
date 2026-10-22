import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {build} from "esbuild";
import React from "react";
import {renderToStaticMarkup} from "react-dom/server";

const extensionBase = "chrome-extension://nexus-test/";
let stored = {};
let calls = [];
let failure = "";
let popupPath = "popup.html";
let openPanel = null;
let forbiddenCreates = 0;
const storage = {
  get: async defaults => {
    if (failure === "storage") throw new Error("storage unavailable");
    return {...defaults, ...structuredClone(stored)};
  },
  set: async values => {
    if (failure === "storage") throw new Error("storage unavailable");
    Object.assign(stored, structuredClone(values));
  },
};
globalThis.chrome = {
  runtime: {
    getURL: path => extensionBase + path,
    getManifest: () => ({version: "test"}),
    sendMessage: async message => {
      assert.equal(message.type, "progress_popup_focus");
      calls.push({kind: "focus", taskId: message.taskId});
      if (openPanel?.type === "progress") {
        openPanel.taskId = message.taskId;
        return {focused: true};
      }
      if (openPanel?.type === "manual") return {focused: false};
      throw new Error("Could not establish connection. Receiving end does not exist.");
    },
  },
  storage: {session: storage, local: storage},
  windows: {create: () => { forbiddenCreates++; assert.fail("never create a standalone window"); }},
  tabs: {create: () => { forbiddenCreates++; assert.fail("never create a progress tab"); }},
  action: {
    setPopup: async ({popup}) => {
      calls.push({kind: "set", popup});
      if (failure === "set" && popup !== "popup.html") throw new Error("setPopup failed");
      await Promise.resolve();
      popupPath = popup;
    },
    openPopup: async () => {
      calls.push({kind: "open", popup: popupPath});
      if (failure === "open") throw new Error("openPopup not allowed");
      await Promise.resolve();
      openPanel = {type: "progress", taskId: progressTaskId(new URL(popupPath, extensionBase).search)};
    },
  },
};
const openPopup = chrome.action.openPopup;

const bundled = await build({
  stdin: {contents: `
    export {progressTaskId, progressPopupPath} from "./src/shared/progress-popup";
    export {createDownloadProgressPopup} from "./src/background/download-progress-popup";
    export {DownloadProgressPage} from "./src/popup/components/DownloadProgressPage";
    export {App} from "./src/popup/App";
  `, resolveDir: process.cwd()},
  bundle: true, platform: "node", format: "cjs", packages: "external",
  target: "node22", jsx: "automatic", write: false,
});
const module = {exports: {}};
new Function("require", "module", "exports", bundled.outputFiles[0].text)(
  createRequire(import.meta.url), module, module.exports,
);
const {progressTaskId, progressPopupPath, createDownloadProgressPopup, DownloadProgressPage, App} = module.exports;
const noop = () => {};

// Task IDs are opaque: query punctuation, Unicode and spaces must round-trip.
for (const id of ["task-1", "task + &?=/你好#1", " padded-id "]) {
  const url = new URL(progressPopupPath(id), extensionBase);
  assert.equal(url.pathname, "/popup.html");
  assert.equal(url.searchParams.get("view"), "progress");
  assert.equal(url.searchParams.get("task"), id);
  assert.equal(progressTaskId(url.search), id);
  assert.equal(url.hash, "");
}
for (const search of ["", "?task=task-1", "?view=tasks&task=task-1", "?view=progress", "?view=progress&task=", "?view=progress&task=%20%20"]) {
  assert.equal(progressTaskId(search), null);
}

const task = (taskId, title, overrides = {}) => ({
  taskId, title, status: "downloading", progress: 0.5, receivedBytes: 1024,
  fileSize: 2048, speed: 512, createdAt: 1, canPause: true,
  canOpenFile: false, canOpenFolder: false, fileExt: "zip", packName: "HTTP",
  outputCategory: "archive", outputPath: `D:\\Selected\\${title}`, ...overrides,
});
const target = task("task-target", "this-download-only.zip");
const history = task("task-history", "private-history-should-stay-hidden.zip");
const similar = task("task-target-other", "similar-id-is-not-the-target.zip");
const baseProps = {
  taskId: target.taskId, tasks: [history, similar, target], connectionState: "connected",
  connectionMessage: "", busy: false, onAction: noop,
};
const renderProgress = props => renderToStaticMarkup(React.createElement(DownloadProgressPage, {...baseProps, ...props}));
const html = renderProgress({});
assert.match(html, /this-download-only\.zip/);
assert.doesNotMatch(html, /private-history-should-stay-hidden|similar-id-is-not-the-target/);
assert.match(html, /role="progressbar"/);
assert.match(html, /暂停/);
assert.doesNotMatch(html, /Nexus Downloader|收起|关闭窗口|保存到：|不会停止下载|查看全部任务/);
assert.match(renderProgress({tasks: [{...target, status: "paused"}]}), /继续/);
assert.match(renderProgress({tasks: [{...target, status: "completed", canOpenFile: true, canOpenFolder: true}]}), /打开文件夹/);
const missing = renderProgress({tasks: [history, similar]});
assert.match(missing, /此任务已删除或不再可用/);
assert.doesNotMatch(missing, /private-history-should-stay-hidden|similar-id-is-not-the-target|role="progressbar"/);
const offline = renderProgress({tasks: [history], connectionState: "disconnected", connectionMessage: "等待后台启动"});
assert.match(offline, /等待后台启动/);
assert.match(renderProgress({tasks: [], connectionState: "disconnected", connectionMessage: ""}), /正在等待此下载的进度/);
assert.doesNotMatch(offline, /private-history-should-stay-hidden/);
const appProgress = renderToStaticMarkup(React.createElement(App, {
  themePreference: "system", onThemePreferenceChange: noop, progressTaskId: target.taskId,
}));
assert.match(appProgress, /本次下载/);
assert.doesNotMatch(appProgress, /高级功能|下载此媒体|role="tablist"/);
const appToolbar = renderToStaticMarkup(React.createElement(App, {
  themePreference: "system", onThemePreferenceChange: noop,
}));
assert.match(appToolbar, /下载任务/);
assert.match(appToolbar, /高级功能/);
assert.doesNotMatch(appToolbar, /Nexus Downloader · 本次下载/);
console.log("progress page passed: compact card only, exact task selection, no history fallback, actions, unchanged toolbar");

// Execute the real component's effect with a deterministic React-effect/timer
// harness. Styling hooks are irrelevant here; the SSR checks above exercise
// the actual Fluent components independently.
const effectBundle = await build({
  entryPoints: ["src/popup/components/DownloadProgressPage.tsx"],
  bundle: true, platform: "node", format: "cjs", packages: "external",
  target: "node22", jsx: "automatic", write: false,
});
const require = createRequire(import.meta.url);
let effects = [];
let effectIndex = 0;
let refs = [];
let refIndex = 0;
const effectModule = {exports: {}};
new Function("require", "module", "exports", effectBundle.outputFiles[0].text)(name => {
  if (name === "react") return {...React, useRef: value => refs[refIndex++] ?? (refs[refIndex - 1] = {current: value}), useEffect: (effect, deps) => {
    const index = effectIndex++;
    const previous = effects[index];
    if (previous && deps.length === previous.deps.length && deps.every((value, i) => Object.is(value, previous.deps[i]))) return;
    previous?.cleanup?.();
    effects[index] = {deps, cleanup: effect()};
  }};
  if (name === "@fluentui/react-components") return {...require(name), makeStyles: () => () => ({root: "test-root"})};
  return require(name);
}, effectModule, effectModule.exports);
const previousWindow = globalThis.window;
const timers = new Map();
let timerId = 0;
let now = 0;
let closed = 0;
globalThis.window = {
  setTimeout: (callback, delay) => {
    const id = ++timerId;
    timers.set(id, {callback, due: now + delay});
    return id;
  },
  clearTimeout: id => timers.delete(id),
  close: () => { closed++; },
};
const renderEffect = props => {
  effectIndex = 0;
  refIndex = 0;
  effectModule.exports.DownloadProgressPage({...baseProps, ...props});
};
const advance = milliseconds => {
  now += milliseconds;
  for (const [id, timer] of [...timers]) if (timer.due <= now) {
    timers.delete(id);
    timer.callback();
  }
};
try {
  renderEffect({tasks: [target, {...history, status: "completed"}]});
  assert.equal(timers.size, 0, "another task's completion must not dismiss this panel");
  renderEffect({tasks: [{...target, status: "completed"}]});
  assert.equal(timers.size, 1);
  const completionTimer = [...timers.keys()][0];
  renderEffect({tasks: [{...target, status: "completed", receivedBytes: 2048}]});
  assert.equal([...timers.keys()][0], completionTimer, "normal snapshots cannot postpone the close indefinitely");
  advance(799);
  assert.equal(closed, 0);
  advance(1);
  assert.equal(closed, 1, "completion dismisses the automatic panel after 800 ms");

  // Status changes/deletion and task switches must cancel the old deadline.
  for (const status of ["downloading", "paused", "failed"]) {
    renderEffect({tasks: [target]});
    renderEffect({tasks: [{...target, status: "completed"}]});
    assert.equal(timers.size, 1);
    renderEffect({tasks: [{...target, status}]});
    assert.equal(timers.size, 0, `${status} must cancel the pending auto-close`);
    advance(1000);
    assert.equal(closed, 1);
  }
  renderEffect({tasks: [{...target, status: "completed"}]});
  const oldTaskTimer = [...timers.keys()][0];
  renderEffect({taskId: history.taskId, tasks: [{...history, status: "completed"}]});
  assert.equal(timers.has(oldTaskTimer), false, "switching between completed tasks still clears the old timer");
  assert.equal(timers.size, 1);
  effects.forEach(effect => effect.cleanup?.());
  effects = [];
  assert.equal(timers.size, 0, "unmounting cancels auto-close");
  advance(1000);
  assert.equal(closed, 1);

  refs = [];
  renderEffect({taskId: "never-seen", tasks: []});
  assert.equal(closed, 1, "initial connected state without the target is not proof of deletion");
  renderEffect({taskId: "never-seen", tasks: [], connectionState: "connecting"});
  assert.equal(closed, 1);
  renderEffect({tasks: [target]});
  renderEffect({tasks: [], connectionState: "disconnected"});
  assert.equal(closed, 1, "a temporary disconnect must not be treated as deletion");
  renderEffect({tasks: [], connectionState: "connected"});
  assert.equal(closed, 2, "a previously seen task disappearing from a connected snapshot closes its panel");
  renderEffect({tasks: [target]});
  renderEffect({taskId: "next-task-not-loaded", tasks: []});
  assert.equal(closed, 2, "changing task ID resets deletion tracking");
  renderEffect({taskId: "next-task-not-loaded", tasks: [task("next-task-not-loaded", "next.zip")]});
  renderEffect({taskId: "next-task-not-loaded", tasks: []});
  assert.equal(closed, 3);
} finally {
  effects.forEach(effect => effect.cleanup?.());
  if (previousWindow === undefined) delete globalThis.window;
  else globalThis.window = previousWindow;
}
console.log("progress auto-dismiss passed: completion, connected external deletion, initial/loading/disconnection safety, task switch and cleanup");

// Test the actual bridge's command result and flash behavior without mounting
// its polling/subscription effects. State setters remain functional.
const commandBundle = await build({
  entryPoints: ["src/popup/hooks/usePopupBridge.ts"], bundle: true,
  platform: "node", format: "cjs", packages: "external", target: "node22", write: false,
});
const stateWrites = [];
const simpleReact = {...React,
  useEffect: noop, useCallback: callback => callback, useMemo: callback => callback(),
  useRef: value => ({current: value}),
  useState: initial => {
    let value = typeof initial === "function" ? initial() : initial;
    return [value, next => { value = typeof next === "function" ? next(value) : next; stateWrites.push(value); }];
  },
};
const commandModule = {exports: {}};
new Function("require", "module", "exports", commandBundle.outputFiles[0].text)(
  name => name === "react" ? simpleReact : require(name), commandModule, commandModule.exports,
);
const previousSendMessage = chrome.runtime.sendMessage;
let commandSucceeds = true;
let refreshFails = false;
let pendingReply = null;
let delayCommand = false;
chrome.runtime.sendMessage = (message, reply) => {
  if (message.type === "popup_task_action") {
    const respond = () => reply(commandSucceeds ? {ok: true} : {ok: false, message: "删除失败，请重试"});
    if (delayCommand) pendingReply = respond;
    else respond();
  } else if (message.type === "popup_get_state") {
    if (refreshFails) chrome.runtime.lastError = {message: "snapshot temporarily unavailable"};
    reply({tasks: []});
    delete chrome.runtime.lastError;
  } else reply({});
};
let closeCount = 0;
globalThis.window = {setTimeout: () => 1, clearTimeout: noop, close: () => { closeCount++; }};
try {
  const bridge = commandModule.exports.usePopupBridge("tasks");
  assert.equal(await bridge.performTaskAction(target.taskId, "cancel"), true);
  assert.ok(!stateWrites.includes("任务操作已发送"), "successful actions must not show a green toast");
  commandSucceeds = false;
  assert.equal(await bridge.performTaskAction(target.taskId, "cancel"), false);
  assert.ok(stateWrites.includes("删除失败，请重试"));
  assert.ok(stateWrites.includes("error"), "failed commands keep their error toast");
  commandSucceeds = true;
  refreshFails = true;
  assert.equal(await bridge.performTaskAction(target.taskId, "cancel"), true,
    "a successful backend command remains successful if its follow-up snapshot fails");
  refreshFails = false;

  // Keep App's real callback and the actual bridge command implementation;
  // inject only a stable hook instance to isolate the automatic-panel branch.
  const appBundle = await build({
    entryPoints: ["src/popup/App.tsx"], bundle: true, platform: "node", format: "cjs",
    packages: "external", target: "node22", jsx: "automatic", write: false,
    plugins: [{name: "test-bridge", setup(build) {
      build.onResolve({filter: /^\.\/hooks\/usePopupBridge$/}, () => ({path: "test:popup-bridge", external: true}));
    }}],
  });
  const appRefs = [];
  let appRefIndex = 0;
  const appModule = {exports: {}};
  new Function("require", "module", "exports", appBundle.outputFiles[0].text)(name => {
    if (name === "test:popup-bridge") return {usePopupBridge: () => bridge};
    if (name === "react") return {...simpleReact, useRef: value => appRefs[appRefIndex++] ?? (appRefs[appRefIndex - 1] = {current: value})};
    if (name === "@fluentui/react-components") return {...require(name), makeStyles: () => () => ({root: "test-root"})};
    return require(name);
  }, appModule, appModule.exports);
  const actionFor = id => {
    appRefIndex = 0;
    const tree = appModule.exports.App({themePreference: "system", onThemePreferenceChange: noop, progressTaskId: id});
    return tree.props.children[0].props.children.props.onAction;
  };
  const onAction = actionFor(target.taskId);
  commandSucceeds = false;
  await onAction("cancel");
  assert.equal(closeCount, 0, "backend rejection leaves the panel open");
  commandSucceeds = true;
  await onAction("toggle_pause");
  assert.equal(closeCount, 0, "pause is not a request to dismiss the panel");
  delayCommand = true;
  const deleting = onAction("cancel");
  assert.equal(closeCount, 0, "do not close while deletion is merely pending");
  pendingReply();
  await deleting;
  assert.equal(closeCount, 1);
  const oldTaskDeletion = onAction("cancel");
  actionFor("new-task");
  pendingReply();
  await oldTaskDeletion;
  assert.equal(closeCount, 1, "an old deletion response must not close a newly focused task");
} finally {
  chrome.runtime.sendMessage = previousSendMessage;
  delete chrome.runtime.lastError;
  if (previousWindow === undefined) delete globalThis.window;
  else globalThis.window = previousWindow;
}
console.log("progress delete passed: backend success required, pending/rejected/stale actions stay open, success toast removed and errors retained");

function reset() {
  stored = {};
  calls = [];
  failure = "";
  popupPath = "popup.html";
  openPanel = null;
  chrome.action.openPopup = openPopup;
}
const count = kind => calls.filter(call => call.kind === kind).length;
reset();
const manager = createDownloadProgressPopup();
await manager.show("task-first");
assert.deepEqual(calls, [
  {kind: "set", popup: "popup.html"},
  {kind: "focus", taskId: "task-first"},
  {kind: "set", popup: progressPopupPath("task-first")},
  {kind: "open", popup: progressPopupPath("task-first")},
  {kind: "set", popup: "popup.html"},
]);
assert.equal(openPanel.taskId, "task-first");
assert.equal(popupPath, "popup.html", "manual toolbar clicks must keep the full list after auto-opening");
await manager.show("task-second");
assert.equal(count("open"), 1, "an already open automatic panel is updated in place");
assert.equal(calls.at(-1).kind, "focus");
assert.equal(openPanel.taskId, "task-second");
assert.equal(popupPath, "popup.html");
const previousCalls = calls.length;
await manager.show("task-second");
await manager.show("task-first");
assert.equal(calls.length, previousCalls, "duplicate events cannot refocus or replace current progress");

// A fresh worker resets an interrupted override before any queued popup work.
popupPath = progressPopupPath("interrupted-task");
const resumed = createDownloadProgressPopup();
await resumed.show("task-second");
assert.equal(calls.length, previousCalls + 1, "worker restart resets the URL but must not refocus a duplicate task");
assert.equal(popupPath, "popup.html");
await resumed.show("task-third");
assert.equal(count("open"), 1);
assert.equal(openPanel.taskId, "task-third");
openPanel = null;
const beforeCloseRetry = calls.length;
await createDownloadProgressPopup().show("task-third");
assert.equal(calls.length, beforeCloseRetry + 1, "closing a popup must be respected; only startup reset is allowed");
await resumed.show("task-fourth");
assert.equal(count("open"), 2, "a new task may reopen the toolbar panel after explicit close");
assert.equal(openPanel.taskId, "task-fourth");
assert.equal(popupPath, "popup.html");

reset();
const concurrent = createDownloadProgressPopup();
await Promise.all([concurrent.show("burst-a"), concurrent.show("burst-a"), concurrent.show("burst-b"), concurrent.show("burst-c")]);
assert.equal(count("open"), 1);
assert.equal(count("focus"), 3);
assert.equal(openPanel.taskId, "burst-c");
assert.equal(popupPath, "popup.html");
assert.deepEqual(stored.progressPopupSeenTasks, ["burst-a", "burst-b", "burst-c"]);
console.log("progress manager passed: toolbar-only open/focus, URL restoration, concurrent queue, worker restart, dedupe and close");

// All UI failures are isolated from the already accepted download. The queue
// stays usable, and successful popups still deduplicate without storage.
for (const error of ["set", "open"]) {
  reset();
  const recovering = createDownloadProgressPopup();
  failure = error;
  await assert.doesNotReject(recovering.show("accepted-download"));
  assert.equal(openPanel, null);
  assert.equal(popupPath, "popup.html", "finally restores the full-list URL even after an API failure");
  assert.deepEqual(calls.at(-1), {kind: "set", popup: "popup.html"});
  failure = "";
  await recovering.show("next-accepted-download");
  assert.equal(openPanel.taskId, "next-accepted-download");
  assert.equal(popupPath, "popup.html");
}
reset();
failure = "storage";
const withoutStorage = createDownloadProgressPopup();
await withoutStorage.show("storage-unavailable");
await withoutStorage.show("storage-unavailable");
assert.equal(count("open"), 1);
reset();
delete chrome.action.openPopup;
await assert.doesNotReject(createDownloadProgressPopup().show("unsupported-browser"));
assert.equal(count("open"), 0);
assert.equal(popupPath, "popup.html");
// Updating an existing automatic panel does not require openPopup support.
openPanel = {type: "progress", taskId: "previous-task"};
await createDownloadProgressPopup().show("updated-without-open-api");
assert.equal(openPanel.taskId, "updated-without-open-api");
assert.equal(count("open"), 0);
reset();
openPanel = {type: "manual", taskId: null};
failure = "open"; // Browser refuses another popup while its manual panel is open.
await assert.doesNotReject(createDownloadProgressPopup().show("manual-panel-is-not-progress"));
assert.equal(openPanel.type, "manual");
assert.equal(openPanel.taskId, null);
assert.equal(popupPath, "popup.html");
reset();
await createDownloadProgressPopup().show("   ");
assert.deepEqual(calls, [{kind: "set", popup: "popup.html"}]);
assert.equal(forbiddenCreates, 0, "neither normal operation nor any error path may create tabs/windows");
console.log("progress safety passed: unsupported/failed APIs are best-effort, no standalone fallback, manual panel protected");
