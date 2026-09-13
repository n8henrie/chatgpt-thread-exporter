import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import vm from "node:vm";

const extensionDirectory = resolve(process.argv[2] ?? "extension");

await testStaticPopup();
await testPopupBridge();
await testPopupStartupFailure();
await testContentBridge();
await testBackgroundBridge();
console.log("Extension bridge tests passed.");

async function testStaticPopup() {
  const source = await readFile(resolve(extensionDirectory, "popup.html"), "utf8");
  assert.match(source, /id="include-raw"[^>]*checked/u);
}

async function testPopupBridge() {
  const elements = new Map();
  for (const selector of [
    "#export",
    "#status",
    "#debug-log",
    "#include-artifacts",
    "#include-branches",
    "#include-raw",
  ]) {
    elements.set(selector, createElement());
  }
  elements.get("#export").disabled = true;
  elements.get("#include-artifacts").checked = true;
  elements.get("#include-branches").checked = true;
  elements.get("#include-raw").checked = true;

  let pingCount = 0;
  const injectedFiles = [];
  const tabMessages = [];
  const core = {
    conversationContext(value) {
      assert.equal(value, "https://chatgpt.com/c/synthetic-thread");
      return {
        source_url: value,
        origin: "https://chatgpt.com",
        conversation_id: "synthetic-thread",
        conversation_path: "/backend-api/conversation/synthetic-thread",
      };
    },
    runtimeInfo() {
      return {
        implementation: "rust-wasm",
        version: "0.3.1",
        export_schema_version: 1,
      };
    },
    sanitizeError(message) {
      return String(message);
    },
  };
  const context = vm.createContext({
    console: quietConsole(),
    ChatGptThreadExporterCore: { async load() { return core; } },
    browser: {
      runtime: { getManifest() { return { version: "0.3.1" }; } },
      scripting: {
        async executeScript(details) {
          injectedFiles.push([...details.files]);
          return [{ frameId: 0 }];
        },
      },
      tabs: {
        async query() {
          return [{ id: 7, url: "https://chatgpt.com/c/synthetic-thread" }];
        },
        async sendMessage(_tabId, message) {
          tabMessages.push(message);
          if (message.type === "CHATGPT_THREAD_EXPORTER_PING_V1") {
            pingCount += 1;
            if (pingCount === 1) {
              throw new Error("no receiver");
            }
            return { ready: true, core_version: "0.3.1" };
          }
          if (message.type === "CHATGPT_THREAD_EXPORTER_START_V1") {
            return { queued: 4, unresolved: 1 };
          }
          throw new Error("unexpected message");
        },
      },
    },
    document: {
      querySelector(selector) { return elements.get(selector) ?? null; },
    },
  });

  vm.runInContext(
    await readFile(resolve(extensionDirectory, "popup.js"), "utf8"),
    context,
    { filename: "popup.js" },
  );
  await eventually(() => elements.get("#export").disabled === false);
  assert.match(elements.get("#status").textContent, /Ready\. Rust core 0\.3\.1/u);

  elements.get("#export").listeners.get("click")();
  await eventually(() => /Export queued in Downloads/u.test(elements.get("#status").textContent));

  assert.deepEqual(injectedFiles, [["core.js"], ["content.js"]]);
  const startMessage = tabMessages.find(
    (message) => message.type === "CHATGPT_THREAD_EXPORTER_START_V1",
  );
  assert.ok(startMessage, "popup did not send an export request");
  assert.equal(startMessage.options.include_raw_json, true);
  assert.match(elements.get("#debug-log").textContent, /export complete/u);
}

async function testPopupStartupFailure() {
  const elements = new Map();
  for (const selector of [
    "#export",
    "#status",
    "#debug-log",
    "#include-artifacts",
    "#include-branches",
    "#include-raw",
  ]) {
    elements.set(selector, createElement());
  }
  elements.get("#export").disabled = true;

  const context = vm.createContext({
    console: quietConsole(),
    ChatGptThreadExporterCore: {
      async load() {
        throw new Error(
          "CORE_WASM_INITIALIZATION_FAILED: The generated Rust/WebAssembly binary could not be initialized.",
        );
      },
    },
    browser: {
      runtime: { getManifest() { return { version: "0.3.1" }; } },
    },
    document: {
      querySelector(selector) { return elements.get(selector) ?? null; },
    },
  });

  vm.runInContext(
    await readFile(resolve(extensionDirectory, "popup.js"), "utf8"),
    context,
    { filename: "popup.js" },
  );
  await eventually(() => /CORE_WASM_INITIALIZATION_FAILED/u.test(elements.get("#status").textContent));
  assert.equal(elements.get("#export").disabled, true);
  assert.match(elements.get("#status").textContent, /extension\/manifest\.json/u);
}

async function testContentBridge() {
  let listener;
  let sentMessage;
  let buildInput;
  const core = {
    conversationContext() {
      return {
        source_url: "https://chatgpt.com/",
        origin: "https://chatgpt.com",
        conversation_id: null,
        conversation_path: null,
      };
    },
    buildExportPlan(input) {
      buildInput = input;
      return {
        schema_version: 1,
        generated_at: "2026-08-13T00:00:00Z",
        title: "Example",
        source_url: "https://chatgpt.com/",
        conversation_id: null,
        root_name: "example",
        extraction: "dom",
        markdown: "# Example\n",
        branches: [],
        artifacts: [],
        warnings: [],
      };
    },
    buildArchivePlan() {
      throw new Error("not used");
    },
    parseResolvedFile() {
      throw new Error("not used");
    },
    runtimeInfo() {
      return {
        implementation: "rust-wasm",
        version: "0.3.1",
        export_schema_version: 1,
      };
    },
    sanitizeError(message) {
      return String(message);
    },
    selectAccountId() {
      return null;
    },
  };
  const context = vm.createContext({
    console: quietConsole(),
    URL,
    Headers,
    Uint8Array,
    atob,
    btoa,
    Blob,
    Node: { TEXT_NODE: 3, ELEMENT_NODE: 1 },
    HTMLAnchorElement: class {},
    ChatGptThreadExporterCore: { async load() { return core; } },
    browser: {
      runtime: {
        getManifest() { return { version: "0.3.1" }; },
        onMessage: { addListener(value) { listener = value; } },
        async sendMessage(message) {
          sentMessage = message;
          return { queued: 3, unresolved: 0 };
        },
      },
    },
    document: {
      cookie: "",
      title: "Example - ChatGPT",
      querySelector() { return null; },
      querySelectorAll() { return []; },
    },
    fetch: async () => { throw new Error("unexpected fetch"); },
    location: {
      href: "https://chatgpt.com/",
      origin: "https://chatgpt.com",
    },
  });
  vm.runInContext(
    await readFile(resolve(extensionDirectory, "content.js"), "utf8"),
    context,
    { filename: "content.js" },
  );

  assert.equal(typeof listener, "function");
  assert.deepEqual(
    JSON.parse(JSON.stringify(await listener({ type: "CHATGPT_THREAD_EXPORTER_PING_V1" }))),
    { ready: true, core_version: "0.3.1" },
  );
  const result = await listener({
    type: "CHATGPT_THREAD_EXPORTER_START_V1",
    options: { include_artifacts: true, include_raw_json: false },
  });
  assert.deepEqual(JSON.parse(JSON.stringify(result)), { queued: 3, unresolved: 0 });
  assert.equal(buildInput.dom_conversation.messages.length, 0);
  assert.equal(buildInput.options.include_artifacts, true);
  assert.equal(sentMessage.type, "CHATGPT_THREAD_EXPORTER_DOWNLOAD_V1");
  assert.equal(sentMessage.rawConversation, null);
  assert.deepEqual(JSON.parse(JSON.stringify(sentMessage.resolutions)), []);

  await listener({
    type: "CHATGPT_THREAD_EXPORTER_START_V1",
    options: { include_artifacts: false },
  });
  assert.equal(sentMessage.rawConversation.schema, "rendered-dom-fallback-v2");
}

async function testBackgroundBridge() {
  let listener;
  const downloads = [];
  const blobs = [];
  let nextDownloadId = 1;
  const archive = {
    unresolved: 1,
    jobs: [
      {
        path: "ChatGPT Exports/example--20260813T000000Z/conversation.md",
        kind: "text",
        text: "# Example\n",
        mime_type: "text/markdown;charset=utf-8",
      },
      {
        path: "ChatGPT Exports/example--20260813T000000Z/received/001-bytes.bin",
        kind: "inline",
        base64: "AAEC/w==",
        mime_type: "application/octet-stream",
      },
      {
        path: "ChatGPT Exports/example--20260813T000000Z/received/002-report.pdf",
        kind: "remote",
        url: "https://files.oaiusercontent.com/report.pdf?sig=synthetic",
      },
    ],
  };
  const context = vm.createContext({
    console: quietConsole(),
    URL: class extends URL {
      static createObjectURL(blob) {
        blobs.push(blob);
        return `blob:synthetic-${blobs.length}`;
      }
      static revokeObjectURL() {}
    },
    Uint8Array,
    atob,
    Blob,
    ChatGptThreadExporterCore: {
      async load() {
        return {
          conversationContext(value) {
            if (!String(value).startsWith("https://chatgpt.com/")) {
              throw new Error("unsupported origin");
            }
            return {
              source_url: value,
              origin: "https://chatgpt.com",
              conversation_id: null,
              conversation_path: null,
            };
          },
          buildArchivePlan(input) {
            assert.equal(input.extension_version, "0.3.1");
            return archive;
          },
          runtimeInfo() {
            return {
              implementation: "rust-wasm",
              version: "0.3.1",
              export_schema_version: 1,
            };
          },
        };
      },
    },
    browser: {
      action: {
        async setBadgeBackgroundColor() {},
        async setBadgeText() {},
      },
      downloads: {
        onChanged: { addListener() {} },
        async download(options) {
          downloads.push(options);
          return nextDownloadId++;
        },
      },
      runtime: {
        getManifest() { return { version: "0.3.1" }; },
        onMessage: { addListener(value) { listener = value; } },
      },
    },
  });
  vm.runInContext(
    await readFile(resolve(extensionDirectory, "background.js"), "utf8"),
    context,
    { filename: "background.js" },
  );

  assert.equal(typeof listener, "function");
  await assert.rejects(
    listener({ type: "CHATGPT_THREAD_EXPORTER_DOWNLOAD_V1" }, { url: "https://attacker.example/" }),
    /unsupported origin/u,
  );
  const result = await listener(
    { type: "CHATGPT_THREAD_EXPORTER_DOWNLOAD_V1", plan: {}, resolutions: [] },
    { url: "https://chatgpt.com/c/example" },
  );
  assert.deepEqual(JSON.parse(JSON.stringify(result)), {
    queued: 3,
    unresolved: 1,
  });
  assert.equal(downloads.length, 3);
  assert.equal(downloads[0].filename.endsWith("conversation.md"), true);
  assert.equal(downloads[1].filename.endsWith("001-bytes.bin"), true);
  assert.equal(downloads[2].url.includes("sig=synthetic"), true);
  assert.equal(blobs.length, 2);
  assert.equal(blobs[1].type, "application/octet-stream");
  assert.deepEqual(
    [...new Uint8Array(await blobs[1].arrayBuffer())],
    [0, 1, 2, 255],
  );
}

function createElement() {
  return {
    checked: false,
    className: "",
    disabled: false,
    listeners: new Map(),
    textContent: "",
    addEventListener(type, listener) {
      this.listeners.set(type, listener);
    },
  };
}

function quietConsole() {
  return { info() {} };
}

async function eventually(predicate) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    if (predicate()) {
      return;
    }
    await new Promise((resolvePromise) => setImmediate(resolvePromise));
  }
  assert.fail("condition was not reached");
}
