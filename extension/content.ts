(() => {
  interface ExportOptionsInput {
    readonly include_artifacts?: boolean;
    readonly include_alternate_branches?: boolean;
    readonly include_raw_json?: boolean;
    readonly max_alternate_branches?: number;
  }

  interface CapturedConversation {
    readonly title: string;
    readonly messages: readonly DomMessageInput[];
    readonly messageElements: readonly Element[];
    readonly warnings: readonly string[];
  }

  interface ChatGptApiClient {
    fetchConversation(path: string): Promise<unknown>;
    fetchResolver(path: string): Promise<unknown>;
  }

  interface CaptureState {
    count: number;
    truncated: boolean;
  }

  interface InlineBudget {
    remainingBytes: number;
  }

  const runtimeGlobal = globalThis as typeof globalThis & {
    __CHATGPT_THREAD_EXPORTER_INSTALLED__?: string;
  };

  const CONTENT_BRIDGE_VERSION = browser.runtime.getManifest().version;
  if (runtimeGlobal.__CHATGPT_THREAD_EXPORTER_INSTALLED__ === CONTENT_BRIDGE_VERSION) {
    return;
  }
  runtimeGlobal.__CHATGPT_THREAD_EXPORTER_INSTALLED__ = CONTENT_BRIDGE_VERSION;

  const MESSAGE_SELECTORS = [
    'article[data-testid^="conversation-turn"]',
    'article[data-testid*="conversation-turn"]',
    '[data-message-id][data-message-author-role]',
    'div[data-message-author-role]',
  ];
  const CONTENT_SELECTORS = [
    '[data-message-author-role]',
    '.markdown.prose',
    '[class*="markdown"]',
    '[class*="prose"]',
  ];
  const ARTIFACT_CARD_SELECTOR = [
    '[data-testid*="artifact"]',
    '[data-testid*="canvas-preview"]',
    '[data-testid*="generated-file"]',
    '[data-testid*="download-card"]',
    '[data-testid*="attachment"]',
    '[data-testid*="file-card"]',
  ].join(",");
  const SKIPPED_TAGS = new Set([
    "script",
    "style",
    "svg",
    "button",
    "nav",
    "textarea",
  ]);
  const CAPTURED_ATTRIBUTES = [
    "alt",
    "aria-hidden",
    "class",
    "data-language",
    "data-math",
    "encoding",
    "href",
    "start",
  ] as const;
  const MAX_DOM_DEPTH = 96;
  const MAX_DOM_NODES = 100_000;
  const MAX_INLINE_ARTIFACT_BYTES = 16 * 1024 * 1024;
  const MAX_INLINE_EXPORT_BYTES = 32 * 1024 * 1024;

  browser.runtime.onMessage.addListener((message) => {
    const record = asRecord(message);
    if (record?.type === "CHATGPT_THREAD_EXPORTER_PING_V1") {
      return initializeContentRuntime();
    }
    if (record?.type !== "CHATGPT_THREAD_EXPORTER_START_V1") {
      return undefined;
    }
    return runExport(asRecord(record.options) ?? {});
  });

  debug("content bridge loaded");

  async function initializeContentRuntime(): Promise<{
    readonly ready: true;
    readonly core_version: string;
  }> {
    const core = await loadVerifiedCore();
    return { ready: true, core_version: core.runtimeInfo().version };
  }

  async function loadVerifiedCore(): Promise<RustCore> {
    debug("loading Rust core");
    const core = await ChatGptThreadExporterCore.load();
    const information = core.runtimeInfo();
    if (information.version !== CONTENT_BRIDGE_VERSION) {
      throw new Error(
        "The content bridge and Rust/WebAssembly core have different versions.",
      );
    }
    debug("Rust core ready");
    return core;
  }

  async function runExport(options: ExportOptionsInput): Promise<unknown> {
    const core = await loadVerifiedCore();
    try {
      const context = core.conversationContext(location.href);
      debug("capturing rendered thread");
      const captured = captureConversation();
      const domArtifacts = captureDomArtifacts(captured.messageElements);
      debug(`rendered capture complete; messages=${captured.messages.length}; artifacts=${domArtifacts.length}`);
      const extractionWarnings = [...captured.warnings];
      let structuredConversation: unknown = null;
      let apiClient: ChatGptApiClient | null = null;

      if (context.conversation_id === null || context.conversation_path === null) {
        extractionWarnings.push(
          "No conversation identifier was present in the URL; the exporter used the rendered page.",
        );
      } else {
        try {
          debug("retrieving structured thread");
          apiClient = await createChatGptApiClient(core, context.origin);
          structuredConversation = await apiClient.fetchConversation(context.conversation_path);
          debug("structured thread retrieved");
        } catch (error) {
          debug("structured retrieval failed; using rendered fallback");
          extractionWarnings.push(
            `Structured conversation retrieval failed; the exporter used the rendered page instead: ${safeError(core, error)}`,
          );
        }
      }

      const includeArtifacts = options.include_artifacts !== false;
      const includeRawJson = options.include_raw_json !== false;
      debug("building export plan in Rust");
      const plan = core.buildExportPlan({
        source_url: context.source_url,
        exported_at: new Date().toISOString(),
        structured_conversation: structuredConversation,
        dom_conversation: {
          title: captured.title,
          messages: captured.messages,
        },
        dom_artifacts: domArtifacts,
        options: {
          include_artifacts: includeArtifacts,
          include_alternate_branches: options.include_alternate_branches !== false,
          max_alternate_branches: integerOr(options.max_alternate_branches, 20),
        },
        warnings: extractionWarnings,
      });
      debug(`export plan ready; branches=${plan.branches.length}; artifacts=${plan.artifacts.length}`);

      debug("resolving artifacts");
      const resolutions = includeArtifacts
        ? await resolveArtifacts(core, plan.artifacts, apiClient)
        : [];
      debug(`artifact resolution complete; results=${resolutions.length}`);
      const rawConversation = includeRawJson
        ? structuredConversation ?? {
          schema: "rendered-dom-fallback-v2",
          source_url: context.source_url,
          title: captured.title,
          messages: captured.messages,
        }
        : null;

      return browser.runtime.sendMessage({
        type: "CHATGPT_THREAD_EXPORTER_DOWNLOAD_V1",
        plan,
        resolutions,
        rawConversation,
      });
    } catch (error) {
      throw new Error(safeError(core, error));
    }
  }

  async function createChatGptApiClient(
    core: RustCore,
    origin: string,
  ): Promise<ChatGptApiClient> {
    const sessionResponse = await fetch(new URL("/api/auth/session", origin), {
      credentials: "include",
      cache: "no-store",
      headers: { Accept: "application/json" },
    });
    if (!sessionResponse.ok) {
      throw new Error(`Session lookup returned HTTP ${sessionResponse.status}`);
    }

    const session = asRecord(await sessionResponse.json());
    const accessToken = firstString(session?.accessToken, session?.access_token);
    if (accessToken === null || accessToken.length < 20) {
      throw new Error("The ChatGPT web session did not expose an access token");
    }
    const accountId = await discoverAccountId(core, origin, accessToken);

    async function fetchApiJson(path: string, operation: string): Promise<unknown> {
      const url = new URL(path, origin);
      if (url.origin !== origin) {
        throw new Error("Refusing to send session credentials outside the ChatGPT origin");
      }

      const headers = new Headers({
        Accept: "application/json",
        Authorization: `Bearer ${accessToken}`,
        "X-Authorization": `Bearer ${accessToken}`,
      });
      if (accountId !== null) {
        headers.set("Chatgpt-Account-Id", accountId);
      }
      const response = await fetch(url, {
        credentials: "include",
        cache: "no-store",
        headers,
      });
      if (!response.ok) {
        throw new Error(`${operation} returned HTTP ${response.status}`);
      }
      return response.json();
    }

    return {
      fetchConversation(path: string): Promise<unknown> {
        return fetchApiJson(path, "Conversation retrieval");
      },
      fetchResolver(path: string): Promise<unknown> {
        return fetchApiJson(path, "Artifact resolver");
      },
    };
  }

  async function discoverAccountId(
    core: RustCore,
    origin: string,
    accessToken: string,
  ): Promise<string | null> {
    const workspaceId = readCookie("_account");
    try {
      const response = await fetch(
        new URL("/backend-api/accounts/check/v4-2023-04-27", origin),
        {
          credentials: "include",
          cache: "no-store",
          headers: {
            Accept: "application/json",
            Authorization: `Bearer ${accessToken}`,
            "X-Authorization": `Bearer ${accessToken}`,
          },
        },
      );
      if (response.ok) {
        return core.selectAccountId(await response.json(), workspaceId) ?? workspaceId;
      }
    } catch (_error) {
      // The workspace cookie is the best-effort fallback and is never persisted.
    }
    return workspaceId;
  }

  function readCookie(name: string): string | null {
    const prefix = `${name}=`;
    const item = document.cookie
      .split(";")
      .map((part) => part.trim())
      .find((part) => part.startsWith(prefix));
    if (item === undefined) {
      return null;
    }
    try {
      const value = decodeURIComponent(item.slice(prefix.length))
        .replace(/^"|"$/gu, "")
        .trim();
      return value || null;
    } catch (_error) {
      return null;
    }
  }

  async function resolveArtifacts(
    core: RustCore,
    artifacts: readonly ArtifactPlan[],
    apiClient: ChatGptApiClient | null,
  ): Promise<ArtifactResolution[]> {
    const budget = { remainingBytes: MAX_INLINE_EXPORT_BYTES };
    const output: ArtifactResolution[] = [];
    for (const artifact of artifacts) {
      output.push(await resolveArtifact(core, artifact, apiClient, budget));
    }
    return output;
  }

  async function resolveArtifact(
    core: RustCore,
    artifact: ArtifactPlan,
    apiClient: ChatGptApiClient | null,
    budget: InlineBudget,
  ): Promise<ArtifactResolution> {
    let lastError: string | null = null;
    for (const request of artifact.resolution) {
      if (request.kind === "unresolved") {
        lastError = request.reason;
        continue;
      }
      try {
        if (request.kind === "remote") {
          return {
            artifact_id: artifact.id,
            status: "resolved",
            resolved_url: request.url,
          };
        }
        if (request.kind === "api") {
          if (apiClient === null) {
            throw new Error("Artifact resolution requires a structured ChatGPT session");
          }
          const file = core.parseResolvedFile(await apiClient.fetchResolver(request.path));
          return {
            artifact_id: artifact.id,
            status: "resolved",
            resolved_url: file.url,
            resolved_name: file.name,
            resolved_mime_type: file.mime_type,
            resolved_size_bytes: file.size_bytes,
          };
        }
        return await resolvePageArtifact(artifact.id, request.url, budget);
      } catch (error) {
        lastError = safeError(core, error);
      }
    }

    return {
      artifact_id: artifact.id,
      status: "unresolved",
      error: safeError(core, lastError ?? "No downloadable artifact representation was found"),
    };
  }

  async function resolvePageArtifact(
    artifactId: string,
    value: string,
    budget: InlineBudget,
  ): Promise<ArtifactResolution> {
    const url = new URL(value, location.href);
    if (url.protocol !== "blob:" && url.protocol !== "data:") {
      throw new Error("The Rust core requested an unsupported in-page artifact URL");
    }
    const response = await fetch(url.href);
    if (!response.ok) {
      throw new Error(`In-page artifact fetch returned HTTP ${response.status}`);
    }
    const blob = await response.blob();
    const maximum = Math.min(MAX_INLINE_ARTIFACT_BYTES, budget.remainingBytes);
    if (blob.size > maximum) {
      throw new Error("The in-page artifact exceeded the inline transfer limit");
    }
    const bytes = new Uint8Array(await blob.arrayBuffer());
    budget.remainingBytes -= bytes.byteLength;
    return {
      artifact_id: artifactId,
      status: "resolved_inline",
      inline_base64: bytesToBase64(bytes),
      inline_mime_type: blob.type || "application/octet-stream",
      resolved_size_bytes: bytes.byteLength,
    };
  }

  function captureConversation(): CapturedConversation {
    const messageElements = findMessageElements();
    const state: CaptureState = { count: 0, truncated: false };
    const messages = messageElements.map((element, index) => ({
      id: findMessageId(element, index),
      role: findMessageRole(element),
      markdown: "",
      content: captureChildren(findContentRoot(element), 0, state),
    }));
    return {
      title: discoverConversationTitle(),
      messages,
      messageElements,
      warnings: state.truncated
        ? ["The rendered-page fallback reached its DOM capture safety limit and may be incomplete."]
        : [],
    };
  }

  function findMessageElements(): Element[] {
    for (const selector of MESSAGE_SELECTORS) {
      const candidates = [...document.querySelectorAll(selector)];
      const unique = candidates.filter((element, index, all) =>
        !all.some((other, otherIndex) => otherIndex < index && other.contains(element)),
      );
      if (unique.length > 0) {
        return unique;
      }
    }
    return [];
  }

  function findContentRoot(messageElement: Element): Element {
    if (messageElement.matches("[data-message-author-role]")) {
      return messageElement;
    }
    for (const selector of CONTENT_SELECTORS) {
      const candidate = messageElement.querySelector(selector);
      if (candidate !== null) {
        return candidate;
      }
    }
    return messageElement;
  }

  function findMessageRole(element: Element): string {
    const roleElement = element.matches("[data-message-author-role]")
      ? element
      : element.querySelector("[data-message-author-role]");
    const explicit = roleElement?.getAttribute("data-message-author-role")?.toLowerCase();
    if (explicit !== undefined && ["user", "assistant", "system", "tool"].includes(explicit)) {
      return explicit;
    }
    return element.getAttribute("data-testid")?.toLowerCase().includes("user") === true
      ? "user"
      : "assistant";
  }

  function findMessageId(element: Element, index: number): string {
    return firstString(
      element.getAttribute("data-message-id"),
      element.querySelector("[data-message-id]")?.getAttribute("data-message-id"),
      element.id,
    ) ?? `dom-message-${String(index + 1).padStart(4, "0")}`;
  }

  function captureChildren(
    element: Element,
    depth: number,
    state: CaptureState,
  ): DomSnapshotNode[] {
    const output: DomSnapshotNode[] = [];
    for (const child of element.childNodes) {
      const captured = captureNode(child, depth, state);
      if (captured !== null) {
        output.push(captured);
      }
      if (state.truncated) {
        break;
      }
    }
    return output;
  }

  function captureNode(
    node: Node,
    depth: number,
    state: CaptureState,
  ): DomSnapshotNode | null {
    if (state.count >= MAX_DOM_NODES || depth >= MAX_DOM_DEPTH) {
      state.truncated = true;
      return null;
    }
    if (node.nodeType === Node.TEXT_NODE) {
      state.count += 1;
      return { kind: "text", text: node.nodeValue ?? "" };
    }
    if (node.nodeType !== Node.ELEMENT_NODE) {
      return null;
    }

    const element = node as Element;
    const tag = element.tagName.toLowerCase();
    if (SKIPPED_TAGS.has(tag) || element.getAttribute("aria-hidden") === "true") {
      return null;
    }
    state.count += 1;
    const attributes: Record<string, string> = {};
    for (const name of CAPTURED_ATTRIBUTES) {
      const value = name === "href" && element instanceof HTMLAnchorElement
        ? element.href
        : element.getAttribute(name);
      if (value !== null && value !== "") {
        attributes[name] = value;
      }
    }
    return {
      kind: "element",
      tag,
      attributes,
      children: captureChildren(element, depth + 1, state),
    };
  }

  function captureDomArtifacts(messageElements: readonly Element[]): DomArtifactInput[] {
    const output: DomArtifactInput[] = [];
    const seen = new Set<string>();
    for (const [index, messageElement] of messageElements.entries()) {
      const direction = findMessageRole(messageElement) === "user" ? "submitted" : "received";
      const messageId = findMessageId(messageElement, index);

      for (const anchor of messageElement.querySelectorAll<HTMLAnchorElement>("a[href]")) {
        const source = anchor.hasAttribute("download")
          ? "dom-download"
          : anchor.closest(ARTIFACT_CARD_SELECTOR) === null
            ? "dom-anchor"
            : "dom-card";
        addDomArtifact(output, seen, {
          direction,
          message_id: messageId,
          url: anchor.href,
          name: artifactName(anchor),
          mime_type: null,
          size_bytes: null,
          source,
        });
      }

      for (const image of messageElement.querySelectorAll<HTMLImageElement>("img[src]")) {
        addDomArtifact(output, seen, {
          direction,
          message_id: messageId,
          url: image.src,
          name: image.alt.trim(),
          mime_type: null,
          size_bytes: null,
          source: "dom-image",
        });
      }

      for (const card of messageElement.querySelectorAll(ARTIFACT_CARD_SELECTOR)) {
        if (card.querySelector("a[href]") !== null || card.matches("a[href]")) {
          continue;
        }
        addDomArtifact(output, seen, {
          direction,
          message_id: messageId,
          url: "",
          name: artifactName(card),
          mime_type: null,
          size_bytes: null,
          source: "dom-card",
        });
      }
    }
    return output;
  }

  function addDomArtifact(
    output: DomArtifactInput[],
    seen: Set<string>,
    artifact: DomArtifactInput,
  ): void {
    const key = [
      artifact.direction,
      artifact.message_id,
      artifact.url,
      artifact.name,
      artifact.source,
    ].join("\u0000");
    if (!seen.has(key)) {
      seen.add(key);
      output.push(artifact);
    }
  }

  function artifactName(element: Element): string {
    const download = element.getAttribute("download")?.trim();
    if (download) {
      return download;
    }
    const named = element.querySelector(
      '[data-testid*="filename"], [class*="filename"], [class*="file-name"]',
    )?.textContent?.trim();
    if (named && named.length <= 180) {
      return named;
    }
    const text = element.textContent?.trim() ?? "";
    return text.length <= 180 ? text : "";
  }

  function discoverConversationTitle(): string {
    const heading = document.querySelector('main h1, [data-testid="conversation-title"]');
    return firstString(
      heading?.textContent?.trim(),
      document.title.replace(/\s*[-|]\s*ChatGPT.*$/u, "").trim(),
    ) ?? "ChatGPT conversation";
  }

  function bytesToBase64(bytes: Uint8Array): string {
    const chunkSize = 0x8000;
    let binary = "";
    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
    }
    return btoa(binary);
  }

  function integerOr(value: unknown, fallback: number): number {
    return typeof value === "number" && Number.isInteger(value) ? value : fallback;
  }

  function firstString(...values: readonly unknown[]): string | null {
    for (const value of values) {
      if (typeof value === "string" && value.trim() !== "") {
        return value;
      }
    }
    return null;
  }

  function debug(message: string): void {
    console.info("[chatgpt-thread-exporter][content]", message);
  }

  function safeError(core: RustCore, error: unknown): string {
    const record = asRecord(error);
    const message = typeof record?.message === "string"
      ? record.message
      : String(error ?? "Unknown error");
    return core.sanitizeError(message);
  }

  function asRecord(value: unknown): JsonRecord | null {
    return typeof value === "object" && value !== null
      ? value as JsonRecord
      : null;
  }
})();
