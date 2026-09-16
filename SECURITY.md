# Security policy and threat model

## Security goals

The extension is designed to minimize browser privilege and prevent accidental disclosure while exporting data that is inherently sensitive.

The expected invariants are:

1. The ChatGPT bearer token exists only in the active content-script closure.
2. Credentialed requests are restricted by the TypeScript fetch bridge to the active HTTPS ChatGPT origin.
3. The token, cookies, signed resolver URLs, and raw conversation are never written to extension storage; the extension does not request the `storage` permission.
   Raw conversation JSON is written only to the user-requested download archive and is enabled by default in the popup.
4. The background page accepts export requests only from HTTPS ChatGPT tabs.
5. Persisted artifact records omit raw pointers and resolved signed URLs.
6. Runtime code is packaged with the extension.
   No JavaScript, CSS, or WASM is loaded from a CDN.
7. The extension has no persistent `host_permissions` and is injected only after a toolbar action grants `activeTab` access.
8. Rust owns URL policy, resolver paths, path sanitization, artifact resolution strategies, error redaction, manifests, and the final download-job plan.
9. Resolver-returned and direct artifact URLs are accepted only when Rust's conservative trusted-host validation succeeds.
10. `blob:` and `data:` payloads are subject to per-artifact and per-export size limits in both the TypeScript transfer bridge and Rust archive planner.
11. CI verifies the manifest-declared runtime assets before publishing the XPI.
12. Errors written to the archive redact URL query strings, bearer tokens, signed-URL parameters, and JWT-shaped values.
13. Runtime diagnostics contain only fixed stage names, package versions, and numeric counts.
    They do not serialize URLs, titles, filenames, identifiers, credentials, artifact pointers, or conversation payloads.

## Trust boundaries

Trusted components are:

- Firefox and its WebExtension implementation;
- the installed extension package and its update channel;
- the active ChatGPT origin;
- ChatGPT's same-origin session, conversation, file, and interpreter APIs;
- the local download destination and operating system.

A compromise of Firefox, the operating system, the ChatGPT origin, or the extension update channel can expose conversation content and session credentials.
This extension cannot defend against those platform-level compromises.

The TypeScript layer is a browser bridge, not an archive policy engine.
It can observe the active page because that is necessary to capture the conversation, and it performs authenticated fetches because browser credentials cannot be meaningfully handled by a pure Rust transformation layer.
It passes response data, never the bearer token, into Rust/WASM.

Rust/WASM is trusted to transform that data and enforce archive policy.
The background TypeScript layer receives Rust-generated jobs and invokes Firefox's Downloads API.
It retains a minimal structural path check as defense in depth.

## Requested permissions

The extension requests:

- `activeTab`: temporary access to the user-selected ChatGPT tab;
- `scripting`: injection of the WASM loader and content bridge after a toolbar action;
- `downloads`: writing Markdown, metadata, and artifacts to the download directory.

It does not request persistent host access, browsing history, cookies, storage, clipboard, native messaging, or arbitrary network permissions.

## Credential handling

The content bridge obtains the active ChatGPT web-session token only when an export is requested.
The token remains in a function closure and is used only to make same-origin requests.
Before each credentialed request, TypeScript resolves the Rust-generated path against the validated ChatGPT origin and rejects any origin mismatch.

The token is never:

- passed to Rust/WASM;
- sent through extension messaging;
- sent to the background page;
- written to extension storage;
- logged;
- included in Markdown, JSON, manifests, artifact indexes, or filenames.

Console and popup diagnostics contain fixed stage names, package versions, and numeric metrics rather than request or response bodies.
They do not use a cross-context correlation identifier and are not persisted.

Account-selection logic runs in Rust over the account-check response.
Only the selected non-secret account identifier returns to the request bridge.

## Artifact URL and path policy

Rust classifies discovered artifact pointers and emits an ordered list of resolution requests.
It normalizes file-service identifiers and interpreter sandbox paths before constructing same-origin API paths.
Sandbox paths must stay beneath `/mnt/data/` and cannot contain empty, dot, traversal, backslash, or control-character segments.

Direct and resolver-returned HTTPS URLs are conservatively parsed and restricted to expected ChatGPT/OpenAI storage hosts.
Ambiguous authorities, user info, ports, percent-encoded authority text, control characters, and backslashes are rejected.
The accepted remote URL exists only in the transient archive job and is not copied into persisted metadata.

Rust sanitizes every archive path segment, handles Windows reserved names, and rebuilds the complete path from sanitized segments.
The background bridge also rejects absolute, empty, dot, and traversal-containing paths before invoking Firefox Downloads.

## DOM fallback

When structured retrieval fails, TypeScript walks the rendered message DOM and captures only:

- text nodes;
- element tag names;
- a small allowlist of formatting attributes;
- raw anchor, image, and artifact-card observations.

It skips scripts, styles, SVG, controls, navigation, hidden nodes, and textareas.
Depth and node-count limits prevent an unexpectedly large page tree from exhausting memory.
Rust performs DOM-to-Markdown conversion and decides which raw artifact observations are plausible downloads.

DOM fallback cannot guarantee completeness because ChatGPT can virtualize messages, hide alternate branches, or represent artifacts outside the rendered thread.

## Inline artifact handling

Remote HTTPS resources are normally handed directly to Firefox Downloads.
`blob:` and `data:` resources must be read while the page is alive, so the content bridge converts their bytes to base64 for transfer to the background page.
The limits are:

```text
16 MiB per inline artifact
32 MiB aggregate inline data per export
```

Rust validates the base64 shape and calculated decoded size before generating an inline download job.
Object URLs created by the background page are revoked when the corresponding Firefox download completes or is interrupted.

## Download behavior

The archive is a directory of independent Firefox downloads rather than a ZIP assembled in memory.
This avoids retaining the entire conversation and all artifacts in one large buffer.
It also means browser settings can produce multiple download prompts and individual downloads can fail independently.

The persisted manifest describes resolution results known before queueing.
A Firefox queue or network failure is returned to the popup count but cannot be retroactively written into an already queued manifest.

Firefox may retain directly downloaded signed URLs in its own download history.
The extension does not include those URLs in archive text files.

## Build and supply-chain controls

The release package contains local static assets, TypeScript compiled by `tsc`, and Rust/WebAssembly generated by `wasm-pack`.
Runtime CDN imports, remote scripts, `eval`, and `new Function` are prohibited by the audit.

`make stage` writes generated JavaScript and WebAssembly at the extension root.
`wasm-pack` uses `target/` only for intermediate output.
`make package` archives only the known runtime files.
The Nix `debug` package installs the same unpacked tree, and the default package invokes that same `package` target against the immutable debug output without recompiling it.

`wasm-pack` uses `wasm-bindgen` internally.
Nix supplies the matching CLI and Binaryen.
Cargo metadata disables `wasm-pack`'s private optimizer download; `make stage` invokes the Nix-provided `wasm-opt` explicitly.
Cargo's release profile also applies size optimization, LTO, panic abort, and symbol stripping.

The default Nix output is unsigned.
Tag builds submit the extension to Mozilla's unlisted signing channel before GitHub publishes it.
Mozilla API credentials are read from GitHub Actions secrets and are never written into the source archive, extension, update manifest, or release assets.

Release builds derive `update_url` from GitHub's repository context.
The URL points to the stable `updates.json` asset on the latest GitHub Release, while the update entry points to that version's signed XPI.
The tracked and debug manifests omit `update_url`, so local temporary installations do not contact the release channel and the repository contains no hard-coded owner identity.

CI runs build, test, and lint in that order.
Lint includes Rust formatting, Clippy with warnings and pedantic lints denied, Nix formatting, Actionlint, Mozilla's self-hosted extension validator with warnings denied, package-boundary audits, and privacy-pattern scans.

## Local data and fixtures

Real exported conversations, downloaded artifacts, browser profiles, and local build output should not be committed.
Test fixtures use synthetic names, identifiers, URLs, and content.

The lint audit scans source files and generated extension files for common personal-path and credential patterns.
This is a backstop, not proof that arbitrary prose contains no identifying information; review new fixtures before committing them.

## Endpoint instability

The structured conversation, file, and interpreter endpoints are undocumented.
A server-side schema change can cause incomplete output or failed artifact resolution.
Interpreter-backed files can also expire independently of the conversation.
A successful download queue is not proof of archival completeness.

For important exports, verify the first and last messages, selected branch, message count, citations, and expected artifacts while offline.

## Reporting

Use the repository's private security-advisory mechanism for suspected vulnerabilities.
Do not attach real conversations, bearer tokens, cookies, signed URLs, personal paths, or protected artifacts to a public issue.
