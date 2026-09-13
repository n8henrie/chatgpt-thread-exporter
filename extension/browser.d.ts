type JsonRecord = Record<string, unknown>;

type DomSnapshotNode =
  | { readonly kind: "text"; readonly text: string }
  | {
    readonly kind: "element";
    readonly tag: string;
    readonly attributes: Readonly<Record<string, string>>;
    readonly children: readonly DomSnapshotNode[];
  };

interface DomMessageInput {
  readonly id: string;
  readonly role: string;
  readonly markdown: string;
  readonly content: readonly DomSnapshotNode[];
}

interface DomArtifactInput {
  readonly direction: string;
  readonly message_id: string;
  readonly url: string;
  readonly name: string;
  readonly mime_type: string | null;
  readonly size_bytes: number | null;
  readonly source: string;
}

interface ConversationContext {
  readonly source_url: string;
  readonly origin: string;
  readonly conversation_id: string | null;
  readonly conversation_path: string | null;
}

type ResolutionRequest =
  | { readonly kind: "api"; readonly path: string }
  | { readonly kind: "page"; readonly url: string }
  | { readonly kind: "remote"; readonly url: string }
  | { readonly kind: "unresolved"; readonly reason: string };

interface ArtifactPlan {
  readonly id: string;
  readonly direction: "submitted" | "received";
  readonly message_id: string;
  readonly pointer: string;
  readonly pointer_aliases: readonly string[];
  readonly pointer_kind: string;
  readonly suggested_name: string;
  readonly mime_type: string | null;
  readonly size_bytes: number | null;
  readonly relative_path: string;
  readonly source: string;
  readonly resolution: readonly ResolutionRequest[];
}

interface BranchPlan {
  readonly relative_path: string;
  readonly label: string;
  readonly message_count: number;
  readonly markdown: string;
}

interface ExportPlan {
  readonly schema_version: number;
  readonly generated_at: string;
  readonly title: string;
  readonly source_url: string;
  readonly conversation_id: string | null;
  readonly root_name: string;
  readonly extraction: string;
  readonly markdown: string;
  readonly branches: readonly BranchPlan[];
  readonly artifacts: readonly ArtifactPlan[];
  readonly warnings: readonly string[];
}

interface ResolvedFile {
  readonly url: string;
  readonly name: string | null;
  readonly mime_type: string | null;
  readonly size_bytes: number | null;
}

interface RuntimeInfo {
  readonly implementation: string;
  readonly version: string;
  readonly export_schema_version: number;
}

interface ArtifactResolution {
  readonly artifact_id: string;
  readonly status: "resolved" | "resolved_inline" | "unresolved";
  readonly error?: string | null;
  readonly resolved_url?: string | null;
  readonly resolved_name?: string | null;
  readonly resolved_mime_type?: string | null;
  readonly resolved_size_bytes?: number | null;
  readonly inline_base64?: string | null;
  readonly inline_mime_type?: string | null;
}

type ArchiveJob =
  | {
    readonly path: string;
    readonly kind: "inline";
    readonly base64: string;
    readonly mime_type: string;
  }
  | {
    readonly path: string;
    readonly kind: "remote";
    readonly url: string;
  }
  | {
    readonly path: string;
    readonly kind: "text";
    readonly text: string;
    readonly mime_type: string;
  };

interface ArchivePlan {
  readonly jobs: readonly ArchiveJob[];
  readonly unresolved: number;
}

interface RustCore {
  conversationContext(url: string): ConversationContext;
  selectAccountId(payload: unknown, workspaceId: string | null): string | null;
  parseResolvedFile(payload: unknown): ResolvedFile;
  buildExportPlan(input: unknown): ExportPlan;
  buildArchivePlan(input: unknown): ArchivePlan;
  runtimeInfo(): RuntimeInfo;
  sanitizeError(message: string): string;
}

interface RustCoreLoader {
  load(): Promise<RustCore>;
}

declare const ChatGptThreadExporterCore: RustCoreLoader;

interface BrowserTab {
  readonly id?: number;
  readonly url?: string;
}

interface BrowserMessageSender {
  readonly tab?: BrowserTab;
  readonly url?: string;
}

interface BrowserEvent<TArguments extends readonly unknown[], TResult = void> {
  addListener(listener: (...arguments_: TArguments) => TResult): void;
  removeListener?(listener: (...arguments_: TArguments) => TResult): void;
}

interface BrowserRuntime {
  readonly onMessage: BrowserEvent<
    [message: unknown, sender: BrowserMessageSender],
    unknown
  >;
  getManifest(): { readonly version: string };
  getURL(path: string): string;
  sendMessage(message: unknown): Promise<unknown>;
}

interface BrowserTabs {
  query(queryInfo: Record<string, unknown>): Promise<BrowserTab[]>;
  sendMessage(tabId: number, message: unknown): Promise<unknown>;
}

interface BrowserScripting {
  executeScript(details: {
    readonly target: { readonly tabId: number };
    readonly files: readonly string[];
  }): Promise<unknown>;
}

interface BrowserDownloadDelta {
  readonly id: number;
  readonly state?: { readonly current?: string };
  readonly error?: { readonly current?: string };
}

interface BrowserDownloads {
  readonly onChanged: BrowserEvent<[delta: BrowserDownloadDelta]>;
  download(options: {
    readonly url: string;
    readonly filename: string;
    readonly conflictAction?: "uniquify" | "overwrite" | "prompt";
    readonly saveAs?: boolean;
  }): Promise<number>;
}

interface BrowserAction {
  setBadgeText(details: { readonly text: string }): Promise<void>;
  setBadgeBackgroundColor(details: { readonly color: string }): Promise<void>;
}

declare const browser: {
  readonly action: BrowserAction;
  readonly downloads: BrowserDownloads;
  readonly runtime: BrowserRuntime;
  readonly scripting: BrowserScripting;
  readonly tabs: BrowserTabs;
};
