(() => {
  interface DownloadResult {
    readonly queued: number;
    readonly unresolved: number;
  }

  const objectUrls = new Map<number, string>();
  const BACKGROUND_BRIDGE_VERSION = browser.runtime.getManifest().version;

  browser.downloads.onChanged.addListener((delta) => {
    const state = delta.state?.current;
    if (state !== "complete" && state !== "interrupted") {
      return;
    }
    const objectUrl = objectUrls.get(delta.id);
    if (objectUrl !== undefined) {
      URL.revokeObjectURL(objectUrl);
      objectUrls.delete(delta.id);
    }
  });

  browser.runtime.onMessage.addListener((message, sender) => {
    const record = asRecord(message);
    if (record?.type !== "CHATGPT_THREAD_EXPORTER_DOWNLOAD_V1") {
      return undefined;
    }
    debug("download request received");
    return handleDownload(record, sender).catch(async (error: unknown) => {
      debug("download request failed");
      await updateBadge("error");
      throw error;
    });
  });

  debug("background bridge loaded");

  async function handleDownload(
    message: JsonRecord,
    sender: BrowserMessageSender,
  ): Promise<DownloadResult> {
    const core = await ChatGptThreadExporterCore.load();
    if (core.runtimeInfo().version !== BACKGROUND_BRIDGE_VERSION) {
      throw new Error(
        "The background bridge and Rust/WebAssembly core have different versions.",
      );
    }
    const senderUrl = sender.url ?? sender.tab?.url;
    if (typeof senderUrl !== "string") {
      throw new Error("The export request did not originate from a browser tab");
    }
    core.conversationContext(senderUrl);
    return downloadExport(message, core);
  }

  async function downloadExport(
    message: JsonRecord,
    core: RustCore,
  ): Promise<DownloadResult> {
    debug("building archive plan in Rust");
    const archive = core.buildArchivePlan({
      plan: message.plan,
      resolutions: message.resolutions,
      raw_conversation: message.rawConversation ?? null,
      extension_version: BACKGROUND_BRIDGE_VERSION,
    });
    debug(`archive plan ready; jobs=${archive.jobs.length}; unresolved=${archive.unresolved}`);

    await updateBadge("downloading");
    let queued = 0;
    let queueFailures = 0;
    for (const job of archive.jobs) {
      try {
        await queueJob(job);
        queued += 1;
      } catch (_error) {
        queueFailures += 1;
        debug(`download job failed; index=${queued + queueFailures}`);
      }
    }
    await updateBadge(queueFailures === 0 ? "complete" : "error");
    debug(`download queue complete; queued=${queued}; failed=${queueFailures}`);
    return {
      queued,
      unresolved: archive.unresolved + queueFailures,
    };
  }

  async function queueJob(job: ArchiveJob): Promise<number> {
    assertBridgePath(job.path);
    if (job.kind === "remote") {
      return browser.downloads.download({
        url: job.url,
        filename: job.path,
        conflictAction: "uniquify",
        saveAs: false,
      });
    }
    if (job.kind === "inline") {
      const buffer = base64ToArrayBuffer(job.base64);
      return queueBlob(job.path, new Blob([buffer], { type: job.mime_type }));
    }
    return queueBlob(job.path, new Blob([job.text], { type: job.mime_type }));
  }

  async function queueBlob(path: string, blob: Blob): Promise<number> {
    const objectUrl = URL.createObjectURL(blob);
    try {
      const downloadId = await browser.downloads.download({
        url: objectUrl,
        filename: path,
        conflictAction: "uniquify",
        saveAs: false,
      });
      objectUrls.set(downloadId, objectUrl);
      return downloadId;
    } catch (error) {
      URL.revokeObjectURL(objectUrl);
      throw error;
    }
  }

  function base64ToArrayBuffer(value: string): ArrayBuffer {
    const binary = atob(value);
    const buffer = new ArrayBuffer(binary.length);
    const bytes = new Uint8Array(buffer);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return buffer;
  }

  function assertBridgePath(value: string): void {
    const segments = value.split("/");
    if (value === ""
      || value.startsWith("/")
      || segments.some((segment) => segment === "" || segment === "." || segment === "..")) {
      throw new Error("The Rust core returned an invalid archive path");
    }
  }

  async function updateBadge(state: "complete" | "downloading" | "error"): Promise<void> {
    try {
      await browser.action.setBadgeText({
        text: state === "complete" ? "✓" : state === "error" ? "!" : "…",
      });
      await browser.action.setBadgeBackgroundColor({
        color: state === "error" ? "#9f1239" : "#166534",
      });
    } catch (_error) {
      // Badge support is nonessential to the download bridge.
    }
  }

  function debug(message: string): void {
    console.info("[chatgpt-thread-exporter][background]", message);
  }

  function asRecord(value: unknown): JsonRecord | null {
    return typeof value === "object" && value !== null
      ? value as JsonRecord
      : null;
  }
})();
