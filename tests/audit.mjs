import assert from "node:assert/strict";
import { readFile, stat } from "node:fs/promises";
import { join, resolve } from "node:path";

const root = resolve(process.argv[2] ?? ".");
const extension = resolve(process.argv[3] ?? join(root, "extension"));
const manifest = JSON.parse(
  await readFile(join(extension, "manifest.json"), "utf8"),
);

assert.deepEqual(
  [...manifest.permissions].sort(),
  ["activeTab", "downloads", "scripting"].sort(),
);
assert.equal(Object.hasOwn(manifest, "host_permissions"), false);
assert.equal(manifest.browser_specific_settings.gecko.update_url, undefined);
assert.deepEqual(
  manifest.browser_specific_settings.gecko.data_collection_permissions,
  { required: ["none"] },
);

const cargoToml = await readFile(join(root, "Cargo.toml"), "utf8");
const cargoVersion = cargoToml.match(/^version = "([^"]+)"$/mu)?.[1];
assert.equal(cargoVersion, manifest.version, "Cargo and manifest versions differ");

const packagedFiles = [
  "manifest.json",
  "popup.html",
  "popup.css",
  "background.js",
  "content.js",
  "core.js",
  "popup.js",
  "wasm.js",
  "wasm_bg.wasm",
  "icons/icon.svg",
];

const forbiddenPatterns = [
  ["macOS home directory", /\/Users\/[A-Za-z0-9._-]+\//u],
  ["Linux home directory", /\/home\/[A-Za-z0-9._-]+\//u],
  ["Windows home directory", /[A-Za-z]:\\Users\\[A-Za-z0-9._-]+\\/u],
  ["email address", /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/iu],
  ["OpenAI-style secret", /\bsk-[A-Za-z0-9_-]{20,}\b/u],
  ["GitHub token", /\bgh[opsu]_[A-Za-z0-9]{20,}\b/u],
  [
    "JWT",
    /\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/u,
  ],
  ["source map reference", /sourceMappingURL/u],
];

for (const relativePath of packagedFiles) {
  const path = join(extension, relativePath);
  assert.equal((await stat(path)).isFile(), true, `${relativePath} is missing`);
  const contents = (await readFile(path)).toString("latin1");
  for (const [name, pattern] of forbiddenPatterns) {
    assert.doesNotMatch(contents, pattern, `${relativePath} contains a ${name}`);
  }
}

console.log("Extension manifest, assets, and privacy audit passed.");
