import {build} from "esbuild";
import {mkdtemp, rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import path from "node:path";
import {pathToFileURL} from "node:url";

const root = process.cwd();
const tempDir = await mkdtemp(path.join(tmpdir(), "gd3-overlay-tests-"));
const outfile = path.join(tempDir, "media-state.test.mjs");

try {
  await build({
    entryPoints: [path.join(root, "src/overlay/media-state.test.ts")],
    outfile,
    bundle: true,
    platform: "node",
    format: "esm",
    target: "node22",
    sourcemap: "inline",
  });
  await import(pathToFileURL(outfile).href);
  console.log("overlay state tests passed");
} finally {
  await rm(tempDir, {recursive: true, force: true});
}
