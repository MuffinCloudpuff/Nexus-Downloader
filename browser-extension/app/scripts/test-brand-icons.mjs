import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import path from "node:path";
import {brandIconSizes, readTrayIconFrames} from "./sync-brand-icons.mjs";

const frames = await readTrayIconFrames();
const manifest = JSON.parse(await readFile("public/manifest.json", "utf8"));
assert.deepEqual(manifest.icons, manifest.action.default_icon);
for (const size of brandIconSizes) {
  const actual = await readFile(`public/icon${size}.png`);
  assert.deepEqual(actual, frames.get(size), `${size}px must exactly match the active tray ICO`);
}
for (const [slot, filename] of Object.entries(manifest.icons)) {
  const actual = await readFile(path.join("public", filename));
  const size = actual.readUInt32BE(16);
  assert.ok(size >= Number(slot), `${slot}px icon must not be upscaled`);
  assert.deepEqual(actual, frames.get(size), `${filename} must use the tray branding`);
}
assert.deepEqual(await readFile("public/icon16.png"), frames.get(24));
assert.ok(manifest.web_accessible_resources.some((entry) => entry.resources.includes("icon48.png")),
  "page recorder panels must still be able to show the shared brand icon");
const html = await readFile("popup.html", "utf8");
assert.match(html, /rel="icon"[^>]+href="\/icon32\.png"/);
console.log("brand icon tests passed: tray-identical PNGs, toolbar/manager sizes, favicon and page panels");
