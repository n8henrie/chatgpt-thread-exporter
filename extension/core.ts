(() => {
  type WasmModule = typeof import("./wasm.js");

  const runtimeGlobal = globalThis as typeof globalThis & {
    ChatGptThreadExporterCore?: RustCoreLoader;
  };

  let pendingCore: Promise<RustCore> | null = null;

  runtimeGlobal.ChatGptThreadExporterCore = {
    load(): Promise<RustCore> {
      pendingCore ??= loadCore();
      return pendingCore;
    },
  };

  async function loadCore(): Promise<RustCore> {
    console.info("[chatgpt-thread-exporter][core] loading module");
    const wasmUrl = browser.runtime.getURL("wasm_bg.wasm");

    let imported: unknown;
    try {
      imported = await import("./wasm.js");
    } catch (_error) {
      throw new Error(
        "CORE_MODULE_IMPORT_FAILED: The generated Rust/WebAssembly JavaScript module could not be loaded.",
      );
    }

    const record = asRecord(imported);
    const initialize = record?.default;
    const requiredFunctions = [
      "build_export_plan",
      "conversation_context",
      "select_account_id",
      "parse_resolved_file",
      "build_archive_plan",
      "runtime_info",
      "sanitize_error",
    ] as const satisfies readonly (keyof WasmModule)[];

    if (typeof initialize !== "function"
      || requiredFunctions.some((name) => typeof record?.[name] !== "function")) {
      throw new TypeError(
        "CORE_MODULE_INCOMPLETE: The generated Rust/WebAssembly module is incomplete.",
      );
    }

    try {
      await (initialize as WasmModule["default"])(wasmUrl);
    } catch (_error) {
      throw new Error(
        "CORE_WASM_INITIALIZATION_FAILED: The generated Rust/WebAssembly binary could not be initialized.",
      );
    }

    const module = record as unknown as WasmModule;
    const core: RustCore = {
      conversationContext(url: string): ConversationContext {
        return parseJson(module.conversation_context(url));
      },
      selectAccountId(payload: unknown, workspaceId: string | null): string | null {
        return parseJson(module.select_account_id(
          JSON.stringify(payload),
          workspaceId ?? "",
        ));
      },
      parseResolvedFile(payload: unknown): ResolvedFile {
        return parseJson(module.parse_resolved_file(JSON.stringify(payload)));
      },
      buildExportPlan(input: unknown): ExportPlan {
        return parseJson(module.build_export_plan(JSON.stringify(input)));
      },
      buildArchivePlan(input: unknown): ArchivePlan {
        return parseJson(module.build_archive_plan(JSON.stringify(input)));
      },
      runtimeInfo(): RuntimeInfo {
        return parseJson(module.runtime_info());
      },
      sanitizeError(message: string): string {
        return module.sanitize_error(message);
      },
    };

    const information = core.runtimeInfo();
    if (information.implementation !== "rust-wasm" || information.version.trim() === "") {
      throw new Error(
        "CORE_RUNTIME_INFO_INVALID: The Rust/WebAssembly core returned invalid build information.",
      );
    }
    console.info(
      "[chatgpt-thread-exporter][core] ready",
      `version=${information.version}`,
    );
    return core;
  }

  function parseJson<T>(value: string): T {
    return JSON.parse(value) as T;
  }

  function asRecord(value: unknown): JsonRecord | null {
    return typeof value === "object" && value !== null
      ? value as JsonRecord
      : null;
  }
})();
