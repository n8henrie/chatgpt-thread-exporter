import assert from "node:assert/strict";
import { readdir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

const root = resolve(process.argv[2] ?? ".");
const extensionDirectory = resolve(process.argv[3] ?? join(root, "extension"));
const manifest = JSON.parse(
  await readFile(join(extensionDirectory, "manifest.json"), "utf8"),
);

assert.deepEqual(
  [...manifest.permissions].sort(),
  ["activeTab", "downloads", "scripting"].sort(),
);
assert.equal(Object.hasOwn(manifest, "host_permissions"), false);
assert.equal(manifest.permissions.includes("storage"), false);
assert.match(
  manifest.content_security_policy.extension_pages,
  /script-src 'self' 'wasm-unsafe-eval'/u,
);
assert.equal(manifest.background.service_worker, undefined);
assert.deepEqual(manifest.background.scripts, ["core.js", "background.js"]);
assert.equal(manifest.action.default_popup, "popup.html");
assert.deepEqual(
  manifest.browser_specific_settings.gecko.data_collection_permissions,
  { required: ["none"] },
);
assert.equal(manifest.browser_specific_settings.gecko.strict_min_version, "142.0");
const updateUrl = manifest.browser_specific_settings.gecko.update_url;
if (updateUrl !== undefined) {
  assert.match(updateUrl, /^https:\/\//u);
}

const cargoToml = await readFile(join(root, "Cargo.toml"), "utf8");
const cargoVersion = cargoToml.match(/^version = "([^"]+)"$/mu)?.[1];
assert.equal(cargoVersion, manifest.version, "Cargo and manifest versions differ");
assert.doesNotMatch(cargoToml, /^authors\s*=/mu, "Cargo metadata contains authors");

const workflow = await readFile(join(root, ".github/workflows/ci.yml"), "utf8");
const buildStep = workflow.indexOf("      - name: Build\n");
const testStep = workflow.indexOf("      - name: Test\n");
const lintStep = workflow.indexOf("      - name: Lint\n");
assert.ok(buildStep >= 0 && buildStep < testStep && testStep < lintStep);
assert.match(workflow, /DeterminateSystems\/determinate-nix-action@v3/u);
assert.match(workflow, /NIX_INSTALLER_DIAGNOSTIC_ENDPOINT:\s*""/u);
assert.match(workflow, /nix develop --command make build/u);
assert.match(workflow, /nix develop --command make test/u);
assert.match(workflow, /nix develop --command make lint/u);
assert.match(workflow, /dist\/chatgpt-thread-exporter-firefox\.xpi/u);
assert.match(
  workflow,
  /releases\/latest\/download\/updates\.json/u,
  "release builds must inject a stable update manifest URL",
);
assert.match(workflow, /web-ext sign/u);
assert.match(workflow, /--channel unlisted/u);
assert.match(workflow, /--upload-source-code/u);
assert.match(workflow, /AMO_JWT_ISSUER/u);
assert.match(workflow, /AMO_JWT_SECRET/u);
assert.match(workflow, /WEB_EXT_API_KEY/u);
assert.match(workflow, /WEB_EXT_API_SECRET/u);
assert.doesNotMatch(workflow, /--api-key|--api-secret/u);
assert.match(workflow, /release\/updates\.json/u);

const requiredAssets = [
  "manifest.json",
  "background.js",
  "content.js",
  "core.js",
  "popup.html",
  "popup.js",
  "popup.css",
  "icons/icon.svg",
  "wasm.js",
  "wasm.d.ts",
  "wasm_bg.wasm",
];
for (const relativePath of requiredAssets) {
  const metadata = await stat(join(extensionDirectory, relativePath));
  assert.equal(metadata.isFile(), true, `${relativePath} is missing`);
}

const popupHtml = await readFile(join(extensionDirectory, "popup.html"), "utf8");
const coreScript = popupHtml.indexOf('src="core.js"');
const popupScript = popupHtml.indexOf('src="popup.js"');
assert.ok(coreScript >= 0, "popup.html does not load core.js");
assert.ok(popupScript > coreScript, "popup.html must load core.js before popup.js");
assert.match(popupHtml, /id="export"[^>]*disabled/u);
assert.match(popupHtml, /id="include-raw"[^>]*checked/u);
assert.match(popupHtml, /id="debug-log"/u);
assert.match(popupHtml, /extension\/manifest\.json/u);

for (const file of ["background.js", "content.js", "core.js", "popup.js"]) {
  const source = await readFile(join(extensionDirectory, file), "utf8");
  assert.doesNotMatch(source, /browser\.storage/u, `${file} uses extension storage`);
  assert.doesNotMatch(source, /\beval\s*\(/u, `${file} uses eval`);
  assert.doesNotMatch(source, /\bnew\s+Function\b/u, `${file} uses new Function`);
  assert.doesNotMatch(source, /console\.(?:log|debug)\s*\(/u, `${file} uses noisy logging`);
  for (const match of source.matchAll(/console\.(?:info|error)\s*\(([\s\S]*?)\);/gu)) {
    assert.match(match[1], /\[chatgpt-thread-exporter\]/u, `${file} has an unscoped log`);
    assert.doesNotMatch(
      match[1],
      /\b(?:accessToken|conversationId|filename|pointer|rawConversation|resolved_url|source_url|tab\.url)\b/u,
      `${file} may log sensitive runtime data`,
    );
  }
  assert.doesNotMatch(
    source,
    /(?:import|<script[^>]+)\s*(?:\(|[^\n])*https?:\/\//iu,
    `${file} loads remote code`,
  );
  assert.doesNotMatch(source, /sourceMappingURL/u, `${file} includes a source map`);
  assert.doesNotMatch(
    source,
    /instanceof Error/u,
    `${file} relies on cross-realm Error identity`,
  );
}

const coreSource = await readFile(join(root, "extension/core.ts"), "utf8");
assert.match(coreSource, /typeof import\("\.\/wasm\.js"\)/u);
assert.match(coreSource, /await import\("\.\/wasm\.js"\)/u);
assert.doesNotMatch(coreSource, /getURL\("wasm\.js"\)/u);
const contentSource = await readFile(join(root, "extension/content.ts"), "utf8");
const backgroundSource = await readFile(join(root, "extension/background.ts"), "utf8");
assert.doesNotMatch(contentSource, /function\s+normalizeSandboxPath/u);
assert.doesNotMatch(contentSource, /function\s+selectAccountId/u);
assert.doesNotMatch(contentSource, /function\s+buildSandboxDownloadPath/u);
assert.doesNotMatch(contentSource, /oaiusercontent\.com/u);
assert.doesNotMatch(backgroundSource, /function\s+buildManifest/u);
assert.doesNotMatch(backgroundSource, /function\s+sanitizePathSegment/u);
assert.doesNotMatch(backgroundSource, /oaiusercontent\.com/u);
assert.doesNotMatch(backgroundSource, /chatgpt\.com/u);

const rustPolicy = await Promise.all(
  [
    "src/archive.rs",
    "src/artifacts.rs",
    "src/bridge.rs",
    "src/dom.rs",
    "src/lib.rs",
    "src/security.rs",
  ].map((file) => readFile(join(root, file), "utf8")),
);
const rustPolicyText = rustPolicy.join("\n");
for (const requiredSymbol of [
  "build_archive",
  "build_sandbox_download_path",
  "render_dom",
  "resolved_file_from_payload",
  "runtime_info",
  "sanitize_error",
  "trusted_https_url",
]) {
  assert.match(rustPolicyText, new RegExp(`\\b${requiredSymbol}\\b`, "u"));
}

const makefile = await readFile(join(root, "Makefile"), "utf8");
for (const target of [
  "build",
  "stage",
  "package",
  "test",
  "lint",
  "format",
  "ci",
  "release-check",
  "clean",
]) {
  assert.match(makefile, new RegExp(`^${target}:`, "mu"), `Makefile is missing ${target}`);
}
assert.match(makefile, /^build: stage package$/mu);
assert.match(makefile, /^package:$/mu);
assert.match(makefile, /^test: stage$/mu);
assert.match(makefile, /^lint: stage$/mu);
assert.match(makefile, /^ci: build test lint$/mu);
assert.match(makefile, /wasm-pack build/u);
assert.match(makefile, /--target web/u);
assert.match(makefile, /--no-pack/u);
assert.match(makefile, /wasm-opt -Oz/u);
assert.match(makefile, /web-ext lint/u);
assert.match(makefile, /--self-hosted/u);
assert.match(makefile, /--warnings-as-errors/u);
assert.doesNotMatch(makefile, /--no-typescript|--no-opt/u);
assert.doesNotMatch(
  makefile,
  /^(?:SHELL\s*:?=|\.SHELLFLAGS:|\.ONESHELL:|\.NOTPARALLEL:)/mu,
);
assert.doesNotMatch(makefile, /\b(?:BUILD_DIR|DIST_DIR|EXTENSION_DIR|ROOT)\b/u);
assert.doesNotMatch(
  makefile,
  /\bprintf\b|scripts\/|CARGO_ENCODED_RUSTFLAGS|RUSTFLAGS/u,
);
assert.doesNotMatch(makefile, /^\s*wasm-bindgen\b/mu);
assert.equal(await pathExists(join(root, "scripts")), false);
assert.equal(await pathExists(join(root, "extension-src")), false);
assert.equal(await pathExists(join(root, "extension", "pkg")), false);

const flakeSource = await readFile(join(root, "flake.nix"), "utf8");
assert.match(flakeSource, /unpacked = pkgs\.callPackage \.\/package\.nix/u);
assert.match(
  flakeSource,
  /archive = pkgs\.callPackage \.\/archive\.nix \{ extension = unpacked; \};/u,
);
assert.match(flakeSource, /default = archive;/u);
assert.match(flakeSource, /debug = unpacked;/u);
assert.match(flakeSource, /checks\.default = archive;/u);
assert.match(flakeSource, /pkgs\.web-ext/u);

const packageSource = await readFile(join(root, "package.nix"), "utf8");
assert.match(packageSource, /\bbinaryen\b/u);
assert.match(packageSource, /\bwasm-pack\b/u);
assert.doesNotMatch(packageSource, /extension-src/u);
for (const redundantSetting of [
  /auditable\s*=/u,
  /CARGO_INCREMENTAL/u,
  /strictDeps\s*=/u,
  /doCheck\s*=/u,
  /platforms\s*=/u,
]) {
  assert.doesNotMatch(packageSource, redundantSetting);
}

const archiveSource = await readFile(join(root, "archive.nix"), "utf8");
assert.match(archiveSource, /\bextension\b/u);
assert.match(archiveSource, /-firefox\.xpi/u);
assert.match(archiveSource, /make -f .*Makefile.* package/u);
assert.doesNotMatch(archiveSource, /zip -r -X/u);
assert.doesNotMatch(archiveSource, /\b(?:cargo|tsc|wasm-pack)\b/u);

const privacyPatterns = [
  { name: "macOS home directory", pattern: /\/Users\/[A-Za-z0-9._-]+\//u },
  { name: "Linux home directory", pattern: /\/home\/[A-Za-z0-9._-]+\//u },
  {
    name: "Windows home directory",
    pattern: /[A-Za-z]:\\Users\\[A-Za-z0-9._-]+\\/u,
  },
  {
    name: "email address",
    pattern: /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/iu,
  },
  { name: "OpenAI-style secret", pattern: /\bsk-[A-Za-z0-9_-]{20,}\b/u },
  { name: "GitHub token", pattern: /\bgh[opsu]_[A-Za-z0-9]{20,}\b/u },
  {
    name: "JWT",
    pattern: /\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/u,
  },
];

const scanRoots = [
  ".github",
  "extension",
  "src",
  "tests",
  "Makefile",
  "archive.nix",
  "package.nix",
  "Cargo.toml",
  "Cargo.lock",
  "README.md",
  "SECURITY.md",
  "flake.nix",
  "flake.lock",
  "rust-toolchain.toml",
  "tsconfig.json",
  ".gitignore",
  "LICENSE",
];
for (const entry of scanRoots) {
  const path = resolve(root, entry);
  if (!(await pathExists(path))) {
    continue;
  }
  const metadata = await stat(path);
  const files = metadata.isDirectory() ? await listFiles(path) : [path];
  for (const file of files) {
    const source = (await readFile(file)).toString("latin1");
    for (const { name, pattern } of privacyPatterns) {
      assert.doesNotMatch(
        source,
        pattern,
        `${relative(root, file)} contains a ${name}`,
      );
    }
  }
}

console.log("Extension packaging, Rust-boundary, and privacy audit passed.");

async function listFiles(directory) {
  const output = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      output.push(...(await listFiles(path)));
    } else if (entry.isFile()) {
      output.push(path);
    }
  }
  return output.sort();
}

async function pathExists(path) {
  try {
    await stat(path);
    return true;
  } catch (error) {
    if (error && typeof error === "object" && error.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}
