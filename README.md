# ChatGPT Thread Exporter

A Firefox WebExtension that exports the active ChatGPT conversation as Markdown and makes a best-effort attempt to download its artifacts.
User-submitted files and assistant/tool-generated files are stored separately.

The extension has one archive engine: Rust compiled to WebAssembly.
TypeScript is limited to browser-boundary work: loading WASM, reading raw DOM data, performing authenticated requests, transferring in-page bytes, and invoking Firefox download APIs.
Rust owns conversation graph selection, DOM-to-Markdown conversion, artifact discovery and resolution policy, URL and path validation, archive construction, manifests, and error redaction.

## LLM Policy

Caveat emptor: This project was almost entirely vibe-coded with ChatGPT 5.6.
In spite of this, I generally do not care for LLM-generated or LLM-assisted contributions.
Please divulge LLM involvement in any communication or code, and note that issues, PR, or other contributions may (or may not) be closed on this basis alone, with or without additional feedback from me.

## Archive layout

```text
ChatGPT Exports/
└── conversation-title--conversation-id--20260804T170000123Z/
    ├── conversation.md
    ├── artifacts.md
    ├── manifest.json
    ├── branches/
    │   ├── alternate-01.md
    │   └── alternate-02.md
    ├── submitted/
    │   └── 001-input.pdf
    ├── received/
    │   ├── 001-generated-report.pdf
    │   └── 002-image.png
    └── raw/
        └── conversation.json       # enabled by default
```

`conversation.md` follows ChatGPT's selected `current_node`.
Alternate leaves are exported under `branches/` when enabled.
Artifact links use relative paths.
Unresolved artifacts remain listed in `artifacts.md` and `manifest.json`.


## Build, test, and lint

The flake exposes two packages on each supported system:

- `packages.default`: an unsigned Firefox XPI for Mozilla signing or AMO submission;
- `packages.debug`: the unpacked extension directory.

Build the XPI:

```bash
nix build
```

The result is:

```text
result/chatgpt-thread-exporter-firefox.xpi
```

Build the unpacked extension:

```bash
nix build .#debug --out-link result-debug
```

Load `result-debug/manifest.json` through Firefox's **Load Temporary Add-on** flow.

For development:

```bash
nix develop
make stage
make package
make test
make lint
```

`make stage` compiles the TypeScript and Rust/WebAssembly output directly beside the static files in `extension/`.
There is no separate source, package, or staging directory.
`make package` creates `dist/chatgpt-thread-exporter-firefox.xpi` from the runtime files in that directory.
`make clean` removes generated JavaScript, TypeScript declarations, WebAssembly, Cargo output, and the XPI.

`wasm-pack` invokes `wasm-bindgen` and Binaryen's `wasm-opt` internally.
Nix supplies both tools, and `--mode no-install` prevents `wasm-pack` from downloading them.
The generated TypeScript declaration describes the low-level Rust/WASM exports used by `extension/core.ts`.
`--no-pack` suppresses only npm package metadata, README, and license copies that are not part of a Firefox extension.
The explicit `wasm-opt` feature flags in `Cargo.toml` work around a known `wasm-pack` incompatibility with WebAssembly features emitted by recent Rust versions.

Run the complete local sequence with:

```bash
nix develop --command make ci
```

One Nix derivation produces both outputs from the same extension tree: the unsigned XPI and the unpacked `debug` tree.
`nix flake check` builds and tests the same derivation.

Formatting is separate and mutating:

```bash
make format
```

`Cargo.lock` and `flake.lock` should be committed before release.

### Why there is no `build.rs`

Cargo build scripts are for crate build prerequisites and generated files under Cargo's `OUT_DIR`.
Compiling TypeScript, testing the repository, and packaging a Firefox extension are repository-level tasks, so they remain in the Makefile.

## GitHub Actions and releases

`.github/workflows/ci.yml` installs Determinate Nix and runs build, test, and lint in that order.
Successful runs upload the unsigned XPI.
A `v*` tag must match the manifest version before the workflow publishes the XPI as a GitHub release asset.

## Temporary Firefox installation

Firefox 140 or newer is required.

Using Nix:

1. Run `nix build .#debug --out-link result-debug`.
2. Open `about:debugging` and select **This Firefox**.
3. Select **Load Temporary Add-on**.
4. Choose `result-debug/manifest.json`.

For an editable build, run `nix develop --command make stage` and load `extension/manifest.json` instead.

The `.xpi` suffix is the Firefox extension package format.
The default `nix build` output is unsigned and must pass through Mozilla signing before ordinary release or beta Firefox installations can install it persistently.

## Diagnostics

The popup initializes the Rust/WebAssembly core before enabling the export button.
It also verifies that the manifest, TypeScript bridges, and Rust core report the same version.
A failed or incomplete build therefore leaves the button disabled and displays an actionable startup error instead of an inert popup.

During an export, the popup shows its startup, injection, and completion stages.
The content and background bridges log their own stages to their respective consoles with the prefix `[chatgpt-thread-exporter]`.
Diagnostic messages contain only fixed stage names, build versions, and numeric counts.
They intentionally exclude conversation text, titles, filenames, URLs, conversation or account identifiers, request credentials, and artifact pointers.

The page Web Console shows content-script events only.
To inspect popup and background events, open `about:debugging`, select **This Firefox**, and select **Inspect** for the temporary extension.

## Extraction model

The content bridge first attempts structured retrieval from ChatGPT's same-origin web backend:

1. Ask Rust to validate and canonicalize the active conversation URL.
2. Read the active web session in TypeScript.
3. Keep the bearer token inside the content-script closure.
4. Retrieve the active conversation graph.
5. Pass conversation data, but never the token, to Rust/WASM.
6. Let Rust select the active branch, discover alternates, render Markdown, and create artifact-resolution strategies.
7. Execute only those network or in-page reads through the TypeScript bridge.
8. Let Rust validate the results and construct the complete archive download plan before Firefox receives any download job.

If structured retrieval is unavailable, TypeScript captures a small, sanitized DOM snapshot.
Rust converts that snapshot to Markdown.
DOM fallback is useful for shared conversations and endpoint changes, but it can omit virtualized messages, hidden branches, and non-rendered metadata.

ChatGPT's web endpoints and DOM are not public stable APIs.
Both extraction paths are defensive and report warnings rather than silently claiming completeness.

## Artifact handling

The Rust core recursively searches structured content, metadata, and raw DOM artifact observations for common representations such as:

- `sediment://` and file-service identifiers;
- `sandbox:/mnt/data/...` links;
- same-origin file resolver paths;
- generated image, audio, video, and file URLs;
- `blob:` and `data:` resources visible in the page.

Interpreter sandbox links are normalized and converted by Rust into a same-origin resolver request using the conversation identifier, originating message identifier, and `/mnt/data/...` path.
Alternate representations are retained as ordered fallbacks, so a rendered artifact URL can still be tried if a structured resolver fails.

The owning message determines the destination:

- user message: `submitted/`;
- assistant or tool message: `received/`.

Trusted HTTPS artifacts are queued directly through Firefox's Downloads API.
Only `blob:` and `data:` resources, which cannot be handed off as durable remote URLs, are copied through memory.
An individual in-page artifact is limited to 16 MiB and the aggregate inline transfer budget is 32 MiB per export.
Rust validates those limits again while constructing the archive.

Signed download URLs are transient download-job data and are omitted from `manifest.json`, `artifacts.md`, and other persisted text.
Firefox can retain a directly downloaded URL in its own download history.

Artifact discovery and resolution are intentionally best effort.
Files exposed only through unsupported tools, connectors, Canvas state, or expired URLs may remain unresolved.

## Privacy and security

The manifest requests only:

```text
activeTab
scripting
downloads
```

There are no persistent host permissions, extension storage, telemetry, analytics, runtime CDN dependencies, or remotely loaded scripts.
The extension runs only after a toolbar action on a supported ChatGPT origin.

Additional controls include:

- the bearer token stays in the content-script closure and is never sent to Rust, the background page, storage, logs, or archive files;
- Rust creates all artifact API paths, validates resolver responses, applies the trusted-host policy, sanitizes archive paths, and builds final download jobs;
- the TypeScript network bridge rejects credentialed requests that escape the active ChatGPT origin;
- the background bridge accepts messages only from HTTPS ChatGPT tabs and refuses empty, absolute, or traversal-containing paths even after Rust validation;
- signed URL query strings and common credential forms are redacted from errors;
- TypeScript source maps are disabled;
- CI scans source and built files for home-directory paths, email addresses, and common secret formats;
- diagnostic logs are limited to fixed stage names, build versions, and numeric counts, and never serialize request or conversation payloads;
- all test conversations and identifiers are synthetic.

Raw JSON export is enabled by default and contains the complete retrieved conversation object.
It can be disabled in the popup and should be treated as sensitive.
Exported artifacts are untrusted files and are not opened or executed by the extension.

See [SECURITY.md](SECURITY.md) for the threat model.

## Source layout

```text
src/archive.rs           final archive jobs, manifest, artifact index
src/artifacts.rs         discovery, de-duplication, resolution strategies
src/bridge.rs            account and resolver-response parsing
src/core.rs              conversation graph and branch selection
src/dom.rs               captured DOM to Markdown
src/markdown.rs          structured message Markdown
src/security.rs          URL, path, size, and redaction policy
extension/core.ts       WASM loader bridge
extension/content.ts    DOM, authentication, fetch, and byte-transfer bridge
extension/background.ts Firefox Downloads bridge
extension/popup.ts      toolbar UI bridge
extension/              TypeScript, static assets, and generated runtime files
tests/                  Rust fixtures and Node boundary/packaging audits
Makefile                local and CI task interface
package.nix             callPackage derivation with XPI and unpacked outputs
.github/workflows/      CI and release workflow
flake.nix               systems, packages, dev shell, and formatter
```

## Current limitations

- Structured endpoints and response schemas can change without notice.
- DOM fallback captures only content currently represented in the page.
- Alternate branch discovery is bounded to 20 branches by default.
- Expired interpreter files and some connector resources cannot be resolved.
- Firefox settings that prompt for every download can produce multiple prompts, because the archive is written as a directory of files.
- The release XPI is unsigned.
- This project is not affiliated with or endorsed by OpenAI.

## License

MIT
