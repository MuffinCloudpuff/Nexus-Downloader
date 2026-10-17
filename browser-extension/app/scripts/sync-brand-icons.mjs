import {mkdir, readFile, writeFile} from "node:fs/promises";
import path from "node:path";
import {fileURLToPath} from "node:url";

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// This is the exact ICO embedded by the Tauri tray, not a historical logo copy.
export const trayIconPath = path.resolve(appRoot, "../../nexus-downloader/src-tauri/icons/icon.ico");
export const brandIconSizes = [24, 32, 48, 64, 128, 256];
const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

export async function readTrayIconFrames() {
  const ico = await readFile(trayIconPath);
  if (ico.length < 6 || ico.readUInt16LE(0) !== 0 || ico.readUInt16LE(2) !== 1) {
    throw new Error("Tray brand icon must be a valid ICO");
  }
  const count = ico.readUInt16LE(4);
  if (!count || ico.length < 6 + count * 16) throw new Error("Truncated tray icon directory");
  const frames = new Map();
  for (let index = 0; index < count; index++) {
    const entry = 6 + index * 16;
    const width = ico[entry] || 256;
    const height = ico[entry + 1] || 256;
    const length = ico.readUInt32LE(entry + 8);
    const offset = ico.readUInt32LE(entry + 12);
    if (offset < 6 + count * 16 || offset + length > ico.length) throw new Error("Invalid tray icon frame bounds");
    const png = ico.subarray(offset, offset + length);
    if (width !== height || png.length < 24 || !png.subarray(0, 8).equals(pngSignature)
      || png.readUInt32BE(16) !== width || png.readUInt32BE(20) !== height) {
      throw new Error(`Tray icon ${width}px must contain a same-sized square PNG`);
    }
    frames.set(width, png);
  }
  for (const size of brandIconSizes) {
    if (!frames.has(size)) throw new Error(`Missing ${size}px tray brand icon`);
  }
  return frames;
}

export async function syncBrandIcons(directory = path.resolve(appRoot, "public")) {
  const frames = await readTrayIconFrames();
  await mkdir(directory, {recursive: true});
  for (const size of brandIconSizes) {
    // Extract existing PNG bytes unchanged: no redrawing, resizing or re-encoding.
    await writeFile(path.resolve(directory, `icon${size}.png`), frames.get(size));
  }
  // Keep old cached icon16 URLs on the same branding. The manifest explicitly
  // points its 16px slot to the existing 24px image; the browser scales it.
  await writeFile(path.resolve(directory, "icon16.png"), frames.get(24));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await syncBrandIcons();
  console.log("Extension brand icons synced from the tray ICO");
}
