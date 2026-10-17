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
`, resolveDir: process.cwd(), loader: "ts"}, bundle: true, packages: "external",
  platform: "node", format: "cjs", write: false});
const module = {exports: {}};
new Function("module", "exports", "require", bundle.outputFiles[0].text)(module, module.exports, createRequire(import.meta.url));
const {taskSummaryToDownloadTask, AppProvider, TaskCard, Sidebar} = module.exports;
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
console.log("desktop image tests passed: image category/sidebar, legacy records, Windows image/archive icons");
