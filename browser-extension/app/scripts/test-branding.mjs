import assert from "node:assert/strict";
import {createHash} from "node:crypto";
import {readFile, readdir} from "node:fs/promises";
import path from "node:path";

const brand = "Nexus Downloader";
const repoRoot = path.resolve("../..");
const desktop = path.join(repoRoot, "nexus-downloader");
const native = path.join(repoRoot, "src/native_messaging_setup.rs");
const manifest = JSON.parse(await readFile("public/manifest.json", "utf8"));
const tauri = JSON.parse(await readFile(path.join(desktop, "src-tauri/tauri.conf.json"), "utf8"));
assert.equal(manifest.name, brand);
assert.equal(manifest.action.default_title, brand);
assert.ok(manifest.description.includes(brand));
assert.equal(tauri.productName, brand);
assert.equal(tauri.app.windows[0].title, brand);
assert.equal(tauri.identifier, "com.gd3.nexusdownloader");
const extensionId = createHash("sha256").update(Buffer.from(manifest.key, "base64"))
  .digest("hex").slice(0, 32).replace(/[0-9a-f]/g, (hex) => String.fromCharCode(97 + parseInt(hex, 16)));
assert.equal(extensionId, "kacpjjdipobohmfbpmnmlmfghoidnngh", "renaming must not change the installed extension identity");
const nativeSource = await readFile(native, "utf8");
assert.ok(nativeSource.includes(`chrome-extension://${extensionId}/`));
assert.ok(nativeSource.includes(`"description": "${brand} Native Messaging Host"`));

async function verifyDisplaySources(file) {
  const text = await readFile(file, "utf8");
  // Camel-case legacy bridge/storage identifiers are deliberately compatible.
  assert.doesNotMatch(text, /Ghost[ -]Downloader(?:-3)?|\bEXUS\b|NEXUS - Download Manager/i, file);
}
async function verifyDirectory(directory) {
  for (const entry of await readdir(directory, {withFileTypes: true})) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) await verifyDirectory(file);
    else if (/\.(?:ts|tsx|js|html)$/.test(entry.name)) await verifyDisplaySources(file);
  }
}
for (const directory of ["src", "public/catch-script", path.join(desktop, "src")]) {
  await verifyDirectory(directory);
}
for (const file of ["popup.html", path.join(desktop, "index.html"), native]) await verifyDisplaySources(file);
for (const file of ["popup.html", path.join(desktop, "index.html")]) {
  assert.ok((await readFile(file, "utf8")).includes(`<title>${brand}</title>`));
}
assert.equal((await readFile(path.join(desktop, "src/i18n.ts"), "utf8")).match(/title: "Nexus Downloader"/g)?.length, 2);
const logo = await readFile(path.join(desktop, "src/components/Logo.tsx"), "utf8");
assert.ok(logo.includes(`aria-label="${brand}"`));
const tray = await readFile(path.join(desktop, "src-tauri/src/lib.rs"), "utf8");
assert.ok(tray.includes(`.tooltip("${brand}")`));
console.log("branding tests passed: tray, both UI languages, popup, extension identity and native pairing compatibility");
