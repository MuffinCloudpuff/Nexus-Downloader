import assert from "node:assert/strict";
import {build} from "esbuild";
import {createRequire} from "node:module";
import React from "react";
import {renderToStaticMarkup} from "react-dom/server";

const bundle = await build({stdin: {contents: `
  export {taskSummaryToDownloadTask} from "./src/adapters/task-adapter";
  export {AppProvider} from "./src/shell/app-store";
  export {TaskCard} from "./src/components/TaskCard";
  export {Sidebar} from "./src/components/Sidebar";
  export {getWindowsFileIcon} from "./src/platform/desktop-shell";
`, resolveDir: process.cwd(), loader: "ts"}, bundle: true, packages: "external",
  platform: "node", format: "cjs", write: false});
const module = {exports: {}};
new Function("module", "exports", "require", bundle.outputFiles[0].text)(module, module.exports, createRequire(import.meta.url));
const {taskSummaryToDownloadTask, AppProvider, TaskCard, Sidebar} = module.exports;
assert.equal(await module.exports.getWindowsFileIcon("D:/file.exe", "exe"), null);
let invoked = null;
globalThis.window = {__TAURI_INTERNALS__: {invoke: async (command, args) => {
  invoked = {command, args};
  return "data:image/png;base64,test";
}}};
assert.equal(await module.exports.getWindowsFileIcon("D:/file.exe", "exe"), "data:image/png;base64,test");
assert.deepEqual(invoked, {command: "get_file_icon", args: {path: "D:/file.exe", ext: "exe"}});
delete globalThis.window;
const base = {task_id: "image", title: "原图.jpg", output_path: "D:/x.com/原图.jpg", output_category: "image",
  status: "completed", artifact_state: "available", file_size: 68, progress: 1};
for (const category of ["image", "other", undefined]) {
  const image = taskSummaryToDownloadTask({...base, output_category: category});
  assert.equal(image.type, "image");
  assert.equal(image.platform, "other");
  const html = renderToStaticMarkup(React.createElement(AppProvider, null, React.createElement(TaskCard, {task: image})));
  assert.match(html, /system-icon\?ext=jpg/);
  assert.match(html, /图片/);
  assert.doesNotMatch(html, /alt="X"/);
}
const sidebar = renderToStaticMarkup(React.createElement(AppProvider, null, React.createElement(Sidebar)));
assert.match(sidebar, /图片/);
const archive = taskSummaryToDownloadTask({...base, task_id: "zip", title: "archive.zip", output_path: "E:/archive.zip", output_category: "archive"});
assert.equal(archive.type, "archive");
const html = renderToStaticMarkup(React.createElement(AppProvider, null, React.createElement(TaskCard, {task: archive})));
assert.match(html, /system-icon\?ext=zip/);
for (const title of ["youtube.zip", "X-tools.zip", "B-archive.7z", "Y-archive.rar"]) {
  const task = taskSummaryToDownloadTask({...base, title, output_path: `D:/youtube/x-tools/${title}`, output_category: "archive"});
  assert.equal(task.platform, "other", title);
}
for (const [pack_name, title, platform] of [
  ["youtube", "视频.mp4", "youtube"], ["bilibili_dash", "视频.mp4", "bilibili"], ["hls", "X-视频.mp4", "twitter"],
]) {
  const task = taskSummaryToDownloadTask({...base, pack_name, title, output_path: `D:/${title}`, output_category: "video"});
  assert.equal(task.platform, platform);
}
const video = taskSummaryToDownloadTask({...base, pack_name: "http", title: "youtube tutorial.mp4", output_path: "D:/x-tools/youtube tutorial.mp4", output_category: "video"});
assert.equal(video.platform, "other");
for (const title of ["B-vacation.mp4", "Y-vacation.mp4", "X-vacation.mp4"]) {
  assert.equal(taskSummaryToDownloadTask({...base, pack_name: "http", source: "browser_download", title, output_path: `D:/${title}`, output_category: "video"}).platform, "other");
}
const extensionless = taskSummaryToDownloadTask({...base, title: "LICENSE", output_path: "D:/LICENSE", output_category: "other"});
const douyin = taskSummaryToDownloadTask({...base, pack_name: "bilibili_dash", media_platform: "douyin",
  title: "抖音视频.mp4", output_path: "D:/抖音视频.mp4", output_category: "video"});
assert.equal(douyin.platform, "douyin");
const douyinHtml = renderToStaticMarkup(React.createElement(AppProvider, null, React.createElement(TaskCard, {task: douyin})));
assert.match(douyinHtml, /src="\/douyin.png" alt="抖音"/);
assert.doesNotMatch(douyinHtml, /system-icon|alt="Bilibili"|dark:invert/);
assert.equal(taskSummaryToDownloadTask({...base, media_platform: "douyin"}).platform, "other");
const noExtHtml = renderToStaticMarkup(React.createElement(AppProvider, null, React.createElement(TaskCard, {task: extensionless})));
assert.match(noExtHtml, /system-icon\?ext=&amp;v=3/);
console.log("desktop image tests passed: image category/sidebar, legacy records, Windows image/archive icons");
