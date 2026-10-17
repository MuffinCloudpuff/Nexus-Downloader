// Exercise an installed backend with locally generated HLS media. Only tasks
// created by this script are deleted. No existing user task is retried/changed.
import assert from "node:assert/strict";
import {spawnSync} from "node:child_process";
import {createServer} from "node:http";
import {mkdir, mkdtemp, readFile, rm, writeFile, access} from "node:fs/promises";
import path from "node:path";

assert.ok(process.argv[2], "Usage: node tests/installed_workspace_smoke.mjs <portable-app-folder>");
const app = path.resolve(process.argv[2]);
const token = (await readFile(path.join(app, "config/pairing_token.txt"), "utf8")).trim();
const nonce = Date.now().toString();
const taskIds = [];
const workspaces = new Map();
const observed = new Set();
const assets = await mkdtemp(path.join(app, "workspace-smoke-assets-"));
const exists = async (file) => access(file).then(() => true, () => false);
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function api(route, options = {}) {
  const response = await fetch(`http://127.0.0.1:14371/api/${route}`, {
    ...options, headers: {Authorization: `Bearer ${token}`, "Content-Type": "application/json"},
  });
  const text = await response.text();
  assert.ok(response.ok, `Local API ${route} returned ${response.status}`);
  const body = JSON.parse(text);
  return body;
}
async function waitFor(check, label) {
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    const result = await check();
    if (result) return result;
    await delay(50);
  }
  throw new Error(`Timed out: ${label}`);
}
function mediaTool(tool, args) {
  const result = spawnSync(tool, args, {cwd: assets, windowsHide: true, encoding: "utf8"});
  assert.equal(result.status, 0, `${tool}: ${result.stderr}`);
  return result.stdout;
}
const before = new Set((await api("tasks")).tasks.map((task) => task.task_id));
const server = createServer(async (request, response) => {
  try {
    const [mode, filename] = new URL(request.url, "http://localhost").pathname.split("/").filter(Boolean);
    if (!["success", "delete"].includes(mode) || !filename || path.basename(filename) !== filename) {
      response.writeHead(404).end(); return;
    }
    if (filename.endsWith(".m4s")) {
      await delay(1200); // Leave enough time to observe/delete the active workspace.
      if (workspaces.has(mode) && await exists(workspaces.get(mode))) observed.add(mode);
    }
    const body = await readFile(path.join(assets, filename));
    response.writeHead(200, {"Content-Length": body.length}).end(body);
  } catch {
    if (!response.headersSent) response.writeHead(404);
    response.end();
  }
});

try {
  mediaTool("ffmpeg", ["-y", "-nostdin", "-hide_banner", "-loglevel", "error",
    "-f", "lavfi", "-i", "color=c=blue:s=64x64:r=10:d=1",
    "-f", "lavfi", "-i", "sine=frequency=880:duration=1",
    "-c:v", "libx264", "-g", "10", "-c:a", "aac", "-f", "hls",
    "-hls_time", "1", "-hls_playlist_type", "vod", "-hls_segment_type", "fmp4",
    path.join(assets, "index.m3u8")]);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  for (const mode of ["success", "delete"]) {
    const id = `workspace_smoke_${nonce}_${mode}`;
    taskIds.push(id);
    const legacy = path.join(app, "temp", id);
    await mkdir(legacy, {recursive: true});
    await writeFile(path.join(legacy, "legacy-test-marker"), "test-owned legacy partial");
    const created = await api("tasks", {method: "POST", body: JSON.stringify({
      request_id: id, source: "page_media", title: `Nexus workspace smoke ${nonce} ${mode}`,
      payload: {url: `http://127.0.0.1:${server.address().port}/${mode}/index.m3u8`, mime: "application/vnd.apple.mpegurl"},
    })});
    assert.equal(created.task_id, id);
    const getTask = async () => (await api("tasks")).tasks.find((task) => task.task_id === id);
    const task = await getTask();
    const output = path.resolve(app, task.output_path);
    assert.ok(path.basename(output).includes(nonce), "Refusing to touch an unrelated output");
    const workspace = path.join(path.dirname(output), `.${id}.parts`);
    workspaces.set(mode, workspace);
    await waitFor(() => exists(workspace), "adjacent workspace creation");
    if (mode === "success") {
      const completed = await waitFor(async () => {
        const task = await getTask();
        assert.notEqual(task?.status, "failed", "Installed download failed");
        return task?.status === "completed" ? task : null;
      }, "download completion");
      assert.equal(completed.status, "completed");
      assert.ok(observed.has(mode), "Media was not observed in the target-adjacent workspace");
      assert.equal(await exists(workspace), false);
      assert.equal(await exists(legacy), false);
      for (const suffix of ["manifest.json", "manifest.json.tmp"]) {
        assert.equal(await exists(path.join(app, "data/resumes", `${id}.${suffix}`)), false);
      }
      const probe = JSON.parse(mediaTool("ffprobe", ["-v", "error", "-show_entries", "stream=codec_type", "-of", "json", output]));
      assert.deepEqual(probe.streams.map((stream) => stream.codec_type), ["video", "audio"]);
    }
    const deleted = await api(`tasks/${id}/delete`, {method: "POST", body: "{}"});
    assert.equal(deleted.command_accepted, true);
    assert.equal(await getTask(), undefined);
    assert.equal(await exists(workspace), false);
    assert.equal(await exists(legacy), false);
    assert.equal(await exists(output), false);
  }
  assert.deepEqual(new Set((await api("tasks")).tasks.map((task) => task.task_id)), before);
  console.log("Installed smoke test passed: adjacent workspace, AV output, success cleanup, deletion during download, legacy cleanup, existing tasks unchanged");
} finally {
  for (const id of taskIds) {
    if ((await api("tasks")).tasks.some((task) => task.task_id === id)) {
      await api(`tasks/${id}/delete`, {method: "POST", body: "{}"});
    }
  }
  server.closeAllConnections();
  server.close();
  assert.ok(assets.startsWith(app + path.sep) && path.basename(assets).startsWith("workspace-smoke-assets-"));
  await rm(assets, {recursive: true, force: true});
}
