(() => {
  interface ExportResponse {
    readonly queued?: number;
    readonly unresolved?: number;
  }

  interface ContentPingResponse {
    readonly ready?: boolean;
    readonly core_version?: string;
  }

  const exportButton = requiredElement<HTMLButtonElement>("#export");
  const statusElement = requiredElement<HTMLParagraphElement>("#status");
  const debugLogElement = requiredElement<HTMLPreElement>("#debug-log");
  const includeArtifacts = requiredElement<HTMLInputElement>("#include-artifacts");
  const includeBranches = requiredElement<HTMLInputElement>("#include-branches");
  const includeRaw = requiredElement<HTMLInputElement>("#include-raw");

  let core: RustCore | null = null;
  const debugLines: string[] = [];

  exportButton.addEventListener("click", () => {
    void startExport();
  });

  debugLogElement.textContent = "";
  log("popup script loaded");
  void initialize();

  async function initialize(): Promise<void> {
    setStatus("Loading the Rust/WebAssembly core…", "");
    log("loading Rust core");
    try {
      const loaded = await ChatGptThreadExporterCore.load();
      const information = loaded.runtimeInfo();
      const manifestVersion = browser.runtime.getManifest().version;
      if (information.version !== manifestVersion) {
        throw new Error(
          "CORE_VERSION_MISMATCH: The extension scripts and Rust/WebAssembly core have different versions.",
        );
      }
      core = loaded;
      exportButton.disabled = false;
      setStatus(`Ready. Rust core ${information.version}.`, "success");
      log("Rust core ready");
    } catch (error) {
      setStatus(startupError(error), "error");
      log("startup failed");
    }
  }

  async function startExport(): Promise<void> {
    const loadedCore = core;
    if (loadedCore === null) {
      setStatus(
        "The runtime is unavailable. Rebuild, then load extension/manifest.json.",
        "error",
      );
      return;
    }

    setBusy(true);
    setStatus("Inspecting the active ChatGPT thread…", "");
    log("export clicked");

    try {
      const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
      if (typeof tab?.id !== "number" || typeof tab.url !== "string") {
        throw new Error("The active browser tab is unavailable.");
      }
      loadedCore.conversationContext(tab.url);
      log("active tab validated");

      await ensureContentScript(tab.id, loadedCore.runtimeInfo().version);
      setStatus("Capturing and exporting the thread…", "");
      log("content bridge ready");

      const response = asRecord(await browser.tabs.sendMessage(tab.id, {
        type: "CHATGPT_THREAD_EXPORTER_START_V1",
        options: {
          include_artifacts: includeArtifacts.checked,
          include_alternate_branches: includeBranches.checked,
          include_raw_json: includeRaw.checked,
          max_alternate_branches: 20,
        },
      })) as ExportResponse | null;

      if (response === null) {
        throw new Error("The content bridge returned no result.");
      }
      const queued = integerOr(response.queued, 0);
      const unresolved = integerOr(response.unresolved, 0);
      const suffix = unresolved === 0
        ? ""
        : `; ${unresolved} artifacts could not be resolved`;
      setStatus(`Export queued in Downloads${suffix}.`, "success");
      log(`export complete; queued=${queued}; unresolved=${unresolved}`);
    } catch (error) {
      setStatus(safeError(loadedCore, error), "error");
      log("export failed");
    } finally {
      setBusy(false);
    }
  }

  async function ensureContentScript(tabId: number, expectedVersion: string): Promise<void> {
    const existing = await pingContent(tabId);
    if (existing?.ready === true && existing.core_version === expectedVersion) {
      return;
    }

    log("injecting core bridge");
    await browser.scripting.executeScript({
      target: { tabId },
      files: ["core.js"],
    });
    log("injecting content bridge");
    await browser.scripting.executeScript({
      target: { tabId },
      files: ["content.js"],
    });

    const injected = asRecord(await browser.tabs.sendMessage(tabId, {
      type: "CHATGPT_THREAD_EXPORTER_PING_V1",
    })) as ContentPingResponse | null;
    if (injected?.ready !== true) {
      throw new Error("The injected content bridge did not answer its readiness check.");
    }
    if (injected.core_version !== expectedVersion) {
      throw new Error(
        "The page is using a stale Rust/WebAssembly core. Reload the ChatGPT tab and retry.",
      );
    }
  }

  async function pingContent(tabId: number): Promise<ContentPingResponse | null> {
    try {
      return asRecord(await browser.tabs.sendMessage(tabId, {
        type: "CHATGPT_THREAD_EXPORTER_PING_V1",
      })) as ContentPingResponse | null;
    } catch (_error) {
      return null;
    }
  }

  function log(message: string): void {
    debugLines.push(message);
    if (debugLines.length > 40) {
      debugLines.shift();
    }
    debugLogElement.textContent = debugLines.join("\n");
    console.info("[chatgpt-thread-exporter][popup]", message);
  }

  function setBusy(busy: boolean): void {
    exportButton.disabled = busy || core === null;
    exportButton.textContent = busy ? "Exporting…" : "Export current thread";
  }

  function setStatus(text: string, className: string): void {
    statusElement.textContent = text;
    statusElement.className = className;
  }

  function requiredElement<T extends Element>(selector: string): T {
    const element = document.querySelector<T>(selector);
    if (element === null) {
      throw new Error(`Extension UI is missing ${selector}`);
    }
    return element;
  }

  function startupError(error: unknown): string {
    const record = asRecord(error);
    const message = typeof record?.message === "string" ? record.message : "";
    if (/^CORE_[A-Z_]+:/u.test(message)) {
      return `${message} Run nix develop --command make stage, then load extension/manifest.json.`;
    }
    return "Extension startup failed. Rebuild, then load extension/manifest.json.";
  }

  function safeError(loadedCore: RustCore, error: unknown): string {
    const record = asRecord(error);
    const message = typeof record?.message === "string"
      ? record.message
      : "The export failed.";
    return loadedCore.sanitizeError(message);
  }

  function integerOr(value: unknown, fallback: number): number {
    return typeof value === "number" && Number.isInteger(value) && value >= 0
      ? value
      : fallback;
  }

  function asRecord(value: unknown): Record<string, unknown> | null {
    return typeof value === "object" && value !== null
      ? value as Record<string, unknown>
      : null;
  }
})();
