/**
 * Measure solid catalog GLBs into data/furniture_footprints.json for shared Rust.
 * build:wasm regenerates stale footprints; --force always regenerates them.
 * Requires @gltf-transform/core and @gltf-transform/extensions when measuring.
 * Without these tool dependencies, warn and keep existing output.
 */
import { readFile, writeFile, stat } from "fs/promises";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

// Resolve paths from the script's own location (repo root = tools/..) so the
// tool works regardless of the CWD it's launched from.
const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CATALOG_PATH = join(ROOT, "client/public/models/objects/catalog.json");
const MODELS_DIR = join(ROOT, "client/public/models");
const OUT_PATH = join(ROOT, "data/furniture_footprints.json");

function multiply(a, b) {
  const out = new Array(16);
  for (let i = 0; i < 4; i++)
    for (let j = 0; j < 4; j++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += a[k * 4 + j] * b[i * 4 + k];
      out[i * 4 + j] = s;
    }
  return out;
}

function applyTransform(arr, mat) {
  const out = new Float32Array(arr.length);
  for (let i = 0; i < arr.length; i += 3) {
    const x = arr[i],
      y = arr[i + 1],
      z = arr[i + 2];
    out[i] = mat[0] * x + mat[4] * y + mat[8] * z + mat[12];
    out[i + 1] = mat[1] * x + mat[5] * y + mat[9] * z + mat[13];
    out[i + 2] = mat[2] * x + mat[6] * y + mat[10] * z + mat[14];
  }
  return out;
}

function nodeWorld(node) {
  const m = node.getMatrix();
  const parent = node.getParentNode();
  return parent ? multiply(nodeWorld(parent), m) : m;
}

/** World-space XZ bounding box of all mesh vertices in a GLB. */
async function measureXZ(io, path) {
  const doc = await io.read(path);
  const root = doc.getRoot();
  let minX = Infinity,
    maxX = -Infinity,
    minZ = Infinity,
    maxZ = -Infinity;
  for (const node of root.listNodes()) {
    const mesh = node.getMesh();
    if (!mesh) continue;
    const world = nodeWorld(node);
    for (const prim of mesh.listPrimitives()) {
      const pos = prim.getAttribute("POSITION");
      if (!pos) continue;
      const p = applyTransform(pos.getArray(), world);
      for (let i = 0; i < p.length; i += 3) {
        if (p[i] < minX) minX = p[i];
        if (p[i] > maxX) maxX = p[i];
        if (p[i + 2] < minZ) minZ = p[i + 2];
        if (p[i + 2] > maxZ) maxZ = p[i + 2];
      }
    }
  }
  if (!isFinite(minX)) throw new Error(`no mesh vertices in ${path}`);
  return { minX, maxX, minZ, maxZ };
}

const FORCE = process.argv.includes("--force");

const catalog = JSON.parse(await readFile(CATALOG_PATH, "utf8"));
const solidDefs = catalog.filter((d) => d.solid && d.model);

async function mtimeMs(path) {
  try {
    return (await stat(path)).mtimeMs;
  } catch {
    return -Infinity;
  }
}

// Skip the (GLB-reading) measurement when the output is already newer than the
// catalog and every solid GLB, so a build only re-measures when furniture
// actually changed. `--force` regenerates unconditionally. This runs before any
// gltf-transform import, so the common (up-to-date) build path needs no tool deps.
if (!FORCE) {
  const outM = await mtimeMs(OUT_PATH);
  if (outM > -Infinity) {
    const inputs = [
      CATALOG_PATH,
      ...solidDefs.map((d) => `${MODELS_DIR}/${d.model}`),
    ];
    const newestInput = Math.max(...(await Promise.all(inputs.map(mtimeMs))));
    if (outM >= newestInput) {
      console.log(
        "furniture footprints up to date — skipping (use --force to regenerate)",
      );
      process.exit(0);
    }
  }
}

// Keep committed footprints when optional measurement dependencies are missing.
let NodeIO;
let EXTTextureWebP;
try {
  ({ NodeIO } = await import("@gltf-transform/core"));
  ({ EXTTextureWebP } = await import("@gltf-transform/extensions"));
} catch (err) {
  const haveOutput = (await mtimeMs(OUT_PATH)) > -Infinity;
  if (haveOutput) {
    console.warn(
      "furniture footprints may be stale: gltf-transform tool dependencies are unavailable, " +
        "so they could not be re-measured. Keeping the existing " +
        "data/furniture_footprints.json. Install the tool deps and re-run with " +
        "--force to refresh (e.g. after changing a solid furniture GLB).",
    );
    process.exit(0);
  }
  console.error(
    "Cannot generate data/furniture_footprints.json: gltf-transform tool dependencies are " +
      "unavailable and no existing footprint table was found.\n" +
      `  ${err?.message ?? err}`,
  );
  process.exit(1);
}

const io = new NodeIO().registerExtensions([EXTTextureWebP]);
const round = (v) => +v.toFixed(3);

const out = {};
for (const def of solidDefs) {
  const b = await measureXZ(io, `${MODELS_DIR}/${def.model}`);
  out[def.id] = {
    minX: round(b.minX),
    maxX: round(b.maxX),
    minZ: round(b.minZ),
    maxZ: round(b.maxZ),
  };
  const f = out[def.id];
  console.log(`  ${def.id}: x[${f.minX}, ${f.maxX}] z[${f.minZ}, ${f.maxZ}]`);
}

await writeFile(OUT_PATH, JSON.stringify(out, null, 2) + "\n", "utf8");
console.log(`wrote ${Object.keys(out).length} footprints to ${OUT_PATH}`);
