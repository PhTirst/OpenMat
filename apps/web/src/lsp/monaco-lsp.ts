import type * as Monaco from "monaco-editor/esm/vs/editor/editor.api";
import { WebSocketLspClient, type LspClient, type LspConnectionState } from "./client";
import {
  OPENMAT_LSP_LANGUAGE_ID,
  OPENMAT_LSP_MARKER_OWNER,
  type JsonRpcNotification,
  type LspCodeAction,
  type LspCompletionItem,
  type LspCompletionList,
  type LspDiagnostic,
  type LspDocumentEdit,
  type LspDocumentSymbol,
  type LspHover,
  type LspLocation,
  type LspPosition,
  type LspPrepareRename,
  type LspRange,
  type LspRenameFile,
  type LspSemanticTokens,
  type LspTextEdit,
  type LspWorkspaceEdit,
  type PublishDiagnosticsParams,
} from "./protocol";

type MonacoApi = typeof import("monaco-editor/esm/vs/editor/editor.api");

const DEFAULT_RECONNECT_DELAYS = [250, 1000, 3000, 5000] as const;
const DEFAULT_SEMANTIC_TOKEN_TYPES = [
  "keyword",
  "function",
  "method",
  "class",
  "property",
  "parameter",
  "variable",
  "number",
  "string",
  "comment",
  "operator",
];

const isCompletionList = (
  value: readonly LspCompletionItem[] | LspCompletionList,
): value is LspCompletionList => !Array.isArray(value);

export interface OpenMatMonacoLspWorkspaceOptions {
  readonly url: string;
  readonly client?: LspClient;
  readonly reconnectDelays?: readonly number[];
  readonly ensureDocuments?: (uris: readonly string[]) => Promise<boolean>;
  readonly stageFileRenames?: (renames: readonly OpenMatFileRename[]) => boolean;
}

export interface OpenMatFileRename {
  readonly oldUri: string;
  readonly newUri: string;
  readonly originalText: string;
  readonly renamedText: string;
}

export interface OpenMatMonacoLspOptions extends OpenMatMonacoLspWorkspaceOptions {
  readonly documentUri: string;
  readonly documentVersion: number;
}

export interface OpenMatMonacoLspDocument {
  readonly model: Monaco.editor.ITextModel;
  readonly documentVersion: number;
}

interface OpenDocument {
  readonly model: Monaco.editor.ITextModel;
  readonly uri: string;
  readonly subscription: Monaco.IDisposable;
  version: number;
  open: boolean;
}

interface RequestContext {
  readonly document: OpenDocument;
  readonly modelVersion: number;
  readonly revision: number;
  readonly generation: number;
}

const toLspPosition = (position: Monaco.Position): LspPosition => ({
  line: position.lineNumber - 1,
  character: position.column - 1,
});

const toMonacoRange = (range: LspRange): Monaco.IRange => ({
  startLineNumber: range.start.line + 1,
  startColumn: range.start.character + 1,
  endLineNumber: range.end.line + 1,
  endColumn: range.end.character + 1,
});

const toLspRange = (range: Monaco.IRange): LspRange => ({
  start: { line: range.startLineNumber - 1, character: range.startColumn - 1 },
  end: { line: range.endLineNumber - 1, character: range.endColumn - 1 },
});

const toMonacoTextEdit = (edit: LspTextEdit): Monaco.languages.TextEdit => ({
  range: toMonacoRange(edit.range),
  text: edit.newText,
});

function applyTextEdits(source: string, edits: readonly LspTextEdit[]): string | null {
  const starts = [0];
  const ends: number[] = [];
  for (const match of source.matchAll(/\r\n|\r|\n/g)) {
    ends.push(match.index);
    starts.push(match.index + match[0].length);
  }
  ends.push(source.length);
  const offset = ({ line, character }: LspPosition): number | null => {
    const start = starts[line];
    const end = ends[line];
    return Number.isInteger(line) && Number.isInteger(character) && start !== undefined &&
      end !== undefined && character >= 0 && character <= end - start ? start + character : null;
  };
  const ranges = [];
  for (const edit of edits) {
    const start = offset(edit.range.start);
    const end = offset(edit.range.end);
    if (start === null || end === null || start > end) return null;
    ranges.push({ start, end, text: edit.newText });
  }
  ranges.sort((left, right) => right.start - left.start || right.end - left.end);
  let previousStart = source.length + 1;
  let result = source;
  for (const edit of ranges) {
    if (edit.end > previousStart) return null;
    result = result.slice(0, edit.start) + edit.text + result.slice(edit.end);
    previousStart = edit.start;
  }
  return result;
}

function completionKind(monaco: MonacoApi, kind: number | undefined): Monaco.languages.CompletionItemKind {
  const kinds = monaco.languages.CompletionItemKind;
  switch (kind) {
    case 2:
      return kinds.Method;
    case 3:
      return kinds.Function;
    case 6:
      return kinds.Variable;
    case 7:
      return kinds.Class;
    case 10:
      return kinds.Property;
    case 14:
      return kinds.Keyword;
    default:
      return kinds.Text;
  }
}

function markerSeverity(monaco: MonacoApi, severity: number | undefined): Monaco.MarkerSeverity {
  switch (severity) {
    case 1:
      return monaco.MarkerSeverity.Error;
    case 2:
      return monaco.MarkerSeverity.Warning;
    case 3:
      return monaco.MarkerSeverity.Info;
    default:
      return monaco.MarkerSeverity.Hint;
  }
}

function diagnosticToMarker(monaco: MonacoApi, diagnostic: LspDiagnostic): Monaco.editor.IMarkerData {
  const range = toMonacoRange(diagnostic.range);
  return {
    ...range,
    severity: markerSeverity(monaco, diagnostic.severity),
    message: diagnostic.message,
    ...(diagnostic.source === undefined ? {} : { source: diagnostic.source }),
    ...(diagnostic.code === undefined ? {} : { code: String(diagnostic.code) }),
  };
}

function markerToDiagnostic(marker: Monaco.editor.IMarkerData): LspDiagnostic {
  const code =
    typeof marker.code === "object" && marker.code !== null ? marker.code.value : marker.code;
  return {
    range: toLspRange(marker),
    severity:
      marker.severity >= 8 ? 1 : marker.severity >= 4 ? 2 : marker.severity >= 2 ? 3 : 4,
    message: marker.message,
    ...(marker.source === undefined ? {} : { source: marker.source }),
    ...(code === undefined ? {} : { code }),
  };
}

function hoverContents(contents: LspHover["contents"]): Monaco.IMarkdownString[] {
  const values = Array.isArray(contents) ? contents : [contents];
  return values.map((content) =>
    typeof content === "string"
      ? { value: content }
      : { value: content.value, isTrusted: false, supportHtml: false },
  );
}

function symbolKind(monaco: MonacoApi, kind: number): Monaco.languages.SymbolKind {
  const normalized = Math.max(1, Math.min(26, kind)) - 1;
  return normalized as Monaco.languages.SymbolKind;
}

function documentSymbol(monaco: MonacoApi, symbol: LspDocumentSymbol): Monaco.languages.DocumentSymbol {
  return {
    name: symbol.name,
    detail: symbol.detail ?? "",
    kind: symbolKind(monaco, symbol.kind),
    tags: [],
    range: toMonacoRange(symbol.range),
    selectionRange: toMonacoRange(symbol.selectionRange),
    ...(symbol.children === undefined
      ? {}
      : { children: symbol.children.map((child) => documentSymbol(monaco, child)) }),
  };
}

export class OpenMatMonacoLspWorkspace {
  readonly #client: LspClient;
  readonly #reconnectDelays: readonly number[];
  readonly #ensureDocuments: OpenMatMonacoLspWorkspaceOptions["ensureDocuments"];
  readonly #stageFileRenames: OpenMatMonacoLspWorkspaceOptions["stageFileRenames"];
  readonly #disposables: Monaco.IDisposable[] = [];
  readonly #documents = new Map<string, OpenDocument>();
  readonly #closedVersions = new Map<string, number>();
  readonly #completionItems = new WeakMap<
    Monaco.languages.CompletionItem,
    { readonly item: LspCompletionItem; readonly context: RequestContext }
  >();
  #started = false;
  #disposed = false;
  #revision = 0;
  #generation = 0;
  #reconnectAttempt = 0;
  #reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  #removeNotificationListener: (() => void) | null = null;
  #removeStateListener: (() => void) | null = null;

  constructor(
    private readonly monaco: MonacoApi,
    options: OpenMatMonacoLspWorkspaceOptions,
  ) {
    this.#client = options.client ?? new WebSocketLspClient(options.url);
    this.#reconnectDelays = options.reconnectDelays ?? DEFAULT_RECONNECT_DELAYS;
    this.#ensureDocuments = options.ensureDocuments;
    this.#stageFileRenames = options.stageFileRenames;
  }

  start(): this {
    if (this.#started || this.#disposed) {
      return this;
    }
    this.#started = true;
    this.#removeNotificationListener = this.#client.onNotification((notification) => {
      this.handleNotification(notification);
    });
    this.#removeStateListener = this.#client.onStateChange((state) => {
      this.handleStateChange(state);
    });
    this.#disposables.push(...this.registerProviders());
    if (this.#client.state === "ready") {
      this.handleStateChange("ready");
    }
    void this.#client.connect().catch(() => {
      // State transitions schedule a bounded reconnect. Editing remains independent.
    });
    return this;
  }

  syncModels(documents: readonly OpenMatMonacoLspDocument[]): void {
    if (this.#disposed) {
      return;
    }
    const requested = new Map(documents.map((document) => [document.model.uri.toString(), document]));
    for (const [uri, document] of this.#documents) {
      if (requested.get(uri)?.model !== document.model) {
        this.closeDocument(document);
        this.#documents.delete(uri);
        this.#revision += 1;
      }
    }
    for (const [uri, { model, documentVersion }] of requested) {
      const existing = this.#documents.get(uri);
      if (existing !== undefined) {
        if (documentVersion > existing.version) {
          existing.version = documentVersion;
          this.#revision += 1;
          this.clearMarkers(existing);
          this.sendDocumentChange(existing);
        }
        continue;
      }
      // Monaco canonicalizes custom-scheme URIs. The model URI is the identity
      // for both providers and the entire LSP document lifecycle.
      const document: OpenDocument = {
        model,
        uri,
        version: Math.max(documentVersion, (this.#closedVersions.get(uri) ?? -1) + 1),
        open: false,
        subscription: model.onDidChangeContent(() => this.handleDocumentChange(document)),
      };
      this.#documents.set(uri, document);
      this.#revision += 1;
      this.openDocument(document);
    }
  }

  dispose(): void {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    if (this.#reconnectTimer !== null) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = null;
    }
    this.#generation += 1;
    for (const document of this.#documents.values()) {
      this.closeDocument(document);
    }
    this.#documents.clear();
    this.#removeNotificationListener?.();
    this.#removeStateListener?.();
    for (const disposable of this.#disposables.splice(0)) {
      disposable.dispose();
    }
    void this.#client.disconnect();
  }

  private handleStateChange(state: LspConnectionState): void {
    if (this.#disposed) {
      return;
    }
    if (state === "ready") {
      this.#reconnectAttempt = 0;
      if (this.#reconnectTimer !== null) {
        clearTimeout(this.#reconnectTimer);
        this.#reconnectTimer = null;
      }
      for (const document of this.#documents.values()) {
        this.openDocument(document);
      }
      return;
    }
    this.#generation += 1;
    for (const document of this.#documents.values()) {
      document.open = false;
      this.clearMarkers(document);
    }
    if (state === "disconnected") {
      this.scheduleReconnect();
    }
  }

  private scheduleReconnect(): void {
    if (this.#disposed || this.#reconnectTimer !== null || this.#reconnectDelays.length === 0) {
      return;
    }
    const delay = this.#reconnectDelays[
      Math.min(this.#reconnectAttempt, this.#reconnectDelays.length - 1)
    ];
    this.#reconnectAttempt += 1;
    this.#reconnectTimer = setTimeout(() => {
      this.#reconnectTimer = null;
      void this.#client.connect().catch(() => {
        // A later disconnected transition schedules the next retry.
      });
    }, delay);
  }

  private openDocument(document: OpenDocument): void {
    if (this.#disposed || !this.#started || document.open || this.#client.state !== "ready") {
      return;
    }
    document.open = this.#client.notify("textDocument/didOpen", {
      textDocument: {
        uri: document.uri,
        languageId: OPENMAT_LSP_LANGUAGE_ID,
        version: document.version,
        text: document.model.getValue(),
      },
    });
  }

  private closeDocument(document: OpenDocument): void {
    if (document.open && this.#client.state === "ready") {
      this.#client.notify("textDocument/didClose", { textDocument: { uri: document.uri } });
    }
    document.open = false;
    document.subscription.dispose();
    this.#closedVersions.set(document.uri, document.version);
    this.clearMarkers(document);
  }

  private clearMarkers(document: OpenDocument): void {
    this.monaco.editor.setModelMarkers(document.model, OPENMAT_LSP_MARKER_OWNER, []);
  }

  private handleDocumentChange(document: OpenDocument): void {
    if (this.#disposed || this.#documents.get(document.uri) !== document) {
      return;
    }
    document.version += 1;
    this.#revision += 1;
    this.clearMarkers(document);
    this.sendDocumentChange(document);
  }

  private sendDocumentChange(document: OpenDocument): void {
    if (!document.open || this.#client.state !== "ready") {
      return;
    }
    this.#client.notify("textDocument/didChange", {
      textDocument: { uri: document.uri, version: document.version },
      contentChanges: [{ text: document.model.getValue() }],
    });
  }

  private handleNotification(notification: JsonRpcNotification): void {
    if (
      this.#disposed ||
      this.#client.state !== "ready" ||
      notification.method !== "textDocument/publishDiagnostics" ||
      typeof notification.params !== "object" ||
      notification.params === null
    ) {
      return;
    }
    const params = notification.params as Partial<PublishDiagnosticsParams>;
    const document = typeof params.uri === "string" ? this.#documents.get(params.uri) : undefined;
    if (document === undefined || !document.open || !Array.isArray(params.diagnostics)) {
      return;
    }
    if (typeof params.version === "number" && params.version !== document.version) {
      return;
    }
    this.monaco.editor.setModelMarkers(
      document.model,
      OPENMAT_LSP_MARKER_OWNER,
      params.diagnostics.map((diagnostic) => diagnosticToMarker(this.monaco, diagnostic)),
    );
  }

  private registerProviders(): Monaco.IDisposable[] {
    const languages = this.monaco.languages;
    return [
      languages.registerCompletionItemProvider(OPENMAT_LSP_LANGUAGE_ID, {
        provideCompletionItems: async (model, position, _context, token) => {
          const context = this.requestContext(model);
          if (context === null || token.isCancellationRequested) {
            return { suggestions: [] };
          }
          const result = await this.safeRequest<readonly LspCompletionItem[] | LspCompletionList>(
            model,
            "textDocument/completion",
            this.positionParams(model, position),
            context,
          );
          if (result === null || token.isCancellationRequested) {
            return { suggestions: [] };
          }
          const items: readonly LspCompletionItem[] = isCompletionList(result)
            ? result.items
            : result;
          const suggestions = items.map((item) => {
            const edit = item.textEdit;
            const suggestion: Monaco.languages.CompletionItem = {
              label: item.label,
              kind: completionKind(this.monaco, item.kind),
              insertText: edit?.newText ?? item.insertText ?? item.label,
              range: (() => {
                if (edit !== undefined) {
                  return toMonacoRange(edit.range);
                }
                const word = model.getWordUntilPosition(position);
                return {
                  startLineNumber: position.lineNumber,
                  startColumn: word.startColumn,
                  endLineNumber: position.lineNumber,
                  endColumn: word.endColumn,
                };
              })(),
              ...(item.detail === undefined ? {} : { detail: item.detail }),
              ...(item.documentation === undefined
                ? {}
                : { documentation: this.markup(item.documentation) }),
            };
            this.#completionItems.set(suggestion, { item, context });
            return suggestion;
          });
          return {
            suggestions,
            ...(isCompletionList(result) && result.isIncomplete === true
              ? { incomplete: true }
              : {}),
          };
        },
        resolveCompletionItem: async (item, token) => {
          const original = this.#completionItems.get(item);
          if (original === undefined || token.isCancellationRequested) {
            return item;
          }
          const resolved = await this.safeRequest<LspCompletionItem>(
            original.context.document.model,
            "completionItem/resolve",
            original.item,
            original.context,
          );
          if (resolved === null || token.isCancellationRequested) {
            return item;
          }
          const suggestion = {
            ...item,
            ...(resolved.detail === undefined ? {} : { detail: resolved.detail }),
            ...(resolved.documentation === undefined
              ? {}
              : { documentation: this.markup(resolved.documentation) }),
          };
          this.#completionItems.set(suggestion, { item: resolved, context: original.context });
          return suggestion;
        },
      }),
      languages.registerHoverProvider(OPENMAT_LSP_LANGUAGE_ID, {
        provideHover: async (model, position, token) => {
          if (!this.matches(model)) {
            return null;
          }
          const hover = await this.safeRequest<LspHover>(
            model,
            "textDocument/hover",
            this.positionParams(model, position),
          );
          if (hover === null || token.isCancellationRequested) {
            return null;
          }
          return {
            contents: hoverContents(hover.contents),
            ...(hover.range === undefined ? {} : { range: toMonacoRange(hover.range) }),
          };
        },
      }),
      languages.registerDefinitionProvider(OPENMAT_LSP_LANGUAGE_ID, {
        provideDefinition: async (model, position, token) => {
          if (!this.matches(model)) {
            return null;
          }
          const result = await this.prepareLocationModels(
            model,
            "textDocument/definition",
            this.positionParams(model, position),
            token,
          );
          return result.length === 0 ? null : result.map((location) => this.location(location));
        },
      }),
      languages.registerReferenceProvider(OPENMAT_LSP_LANGUAGE_ID, {
        provideReferences: async (model, position, context, token) => {
          if (!this.matches(model)) {
            return [];
          }
          const result = await this.prepareLocationModels(model, "textDocument/references", {
            ...this.positionParams(model, position),
            context: { includeDeclaration: context.includeDeclaration },
          }, token);
          return result.map((location) => this.location(location));
        },
      }),
      languages.registerRenameProvider(OPENMAT_LSP_LANGUAGE_ID, {
        resolveRenameLocation: async (model, position, token) => {
          if (!this.matches(model)) {
            return { range: this.emptyRange(position), text: "", rejectReason: "Document is not open" };
          }
          const prepared = await this.safeRequest<LspPrepareRename>(
            model,
            "textDocument/prepareRename",
            this.positionParams(model, position),
          );
          if (prepared === null || token.isCancellationRequested) {
            return { range: this.emptyRange(position), text: "", rejectReason: "Symbol cannot be renamed" };
          }
          return { range: toMonacoRange(prepared.range), text: prepared.placeholder };
        },
        provideRenameEdits: async (model, position, newName, token) => {
          return this.prepareWorkspaceRename(model, position, newName, token);
        },
      }),
      languages.registerDocumentFormattingEditProvider(OPENMAT_LSP_LANGUAGE_ID, {
        displayName: "OpenMat",
        provideDocumentFormattingEdits: async (model, options, token) => {
          if (!this.matches(model)) {
            return [];
          }
          const edits = await this.safeRequest<readonly LspTextEdit[]>(model, "textDocument/formatting", {
            textDocument: { uri: model.uri.toString() },
            options,
          });
          if (edits === null || token.isCancellationRequested) {
            return [];
          }
          return edits.map(toMonacoTextEdit);
        },
      }),
      languages.registerDocumentSemanticTokensProvider(OPENMAT_LSP_LANGUAGE_ID, {
        getLegend: () => ({
          tokenTypes: [...DEFAULT_SEMANTIC_TOKEN_TYPES],
          tokenModifiers: [],
        }),
        provideDocumentSemanticTokens: async (model, _lastResultId, token) => {
          if (!this.matches(model)) {
            return { data: new Uint32Array() };
          }
          const semantic = await this.safeRequest<LspSemanticTokens>(
            model,
            "textDocument/semanticTokens/full",
            { textDocument: { uri: model.uri.toString() } },
          );
          if (semantic === null || token.isCancellationRequested) {
            return { data: new Uint32Array() };
          }
          return {
            data: Uint32Array.from(semantic.data),
            ...(semantic.resultId === undefined ? {} : { resultId: semantic.resultId }),
          };
        },
        releaseDocumentSemanticTokens: () => undefined,
      }),
      languages.registerCodeActionProvider(
        OPENMAT_LSP_LANGUAGE_ID,
        {
          provideCodeActions: async (model, range, context, token) => {
            if (!this.matches(model)) {
              return { actions: [], dispose: () => undefined };
            }
            const actions = await this.safeRequest<readonly LspCodeAction[]>(model, "textDocument/codeAction", {
              textDocument: { uri: model.uri.toString() },
              range: toLspRange(range),
              context: {
                diagnostics: context.markers.map(markerToDiagnostic),
                ...(context.only === undefined ? {} : { only: [context.only] }),
              },
            });
            if (actions === null || token.isCancellationRequested) {
              return { actions: [], dispose: () => undefined };
            }
            return {
              actions: actions.flatMap((action) => {
                const edit = action.edit === undefined ? undefined : this.workspaceEdit(action.edit);
                if (edit === null) {
                  return [];
                }
                return [{
                  title: action.title,
                  ...(action.kind === undefined ? {} : { kind: action.kind }),
                  ...(action.isPreferred === undefined ? {} : { isPreferred: action.isPreferred }),
                  ...(action.diagnostics === undefined
                    ? {}
                    : {
                        diagnostics: action.diagnostics.map((diagnostic) =>
                          diagnosticToMarker(this.monaco, diagnostic),
                        ),
                      }),
                  ...(edit === undefined ? {} : { edit }),
                }];
              }),
              dispose: () => undefined,
            };
          },
        },
        { providedCodeActionKinds: ["quickfix"] },
      ),
      languages.registerDocumentSymbolProvider(OPENMAT_LSP_LANGUAGE_ID, {
        displayName: "OpenMat",
        provideDocumentSymbols: async (model, token) => {
          if (!this.matches(model)) {
            return [];
          }
          const symbols = await this.safeRequest<readonly LspDocumentSymbol[]>(
            model,
            "textDocument/documentSymbol",
            { textDocument: { uri: model.uri.toString() } },
          );
          if (symbols === null || token.isCancellationRequested) {
            return [];
          }
          return symbols.map((symbol) => documentSymbol(this.monaco, symbol));
        },
      }),
    ];
  }

  private matches(model: Monaco.editor.ITextModel): boolean {
    return !this.#disposed && this.#documents.get(model.uri.toString())?.model === model;
  }

  private positionParams(model: Monaco.editor.ITextModel, position: Monaco.Position): object {
    return {
      textDocument: { uri: model.uri.toString() },
      position: toLspPosition(position),
    };
  }

  private emptyRange(position: Monaco.Position): Monaco.IRange {
    return {
      startLineNumber: position.lineNumber,
      startColumn: position.column,
      endLineNumber: position.lineNumber,
      endColumn: position.column,
    };
  }

  private markup(value: LspCompletionItem["documentation"]): string | Monaco.IMarkdownString {
    if (value === undefined || typeof value === "string") {
      return value ?? "";
    }
    return { value: value.value, isTrusted: false, supportHtml: false };
  }

  private location(location: LspLocation): Monaco.languages.Location {
    return {
      uri: this.monaco.Uri.parse(location.uri),
      range: toMonacoRange(location.range),
    };
  }

  private async prepareWorkspaceRename(
    model: Monaco.editor.ITextModel,
    position: Monaco.Position,
    newName: string,
    token: Monaco.CancellationToken,
  ): Promise<Monaco.languages.WorkspaceEdit & Monaco.languages.Rejection> {
    const reject = (rejectReason: string) => ({ edits: [], rejectReason });
    const source = this.requestContext(model);
    if (source === null) return reject("Document is not open");
    const requestedUris = new Set<string>();
    // Opening a disk target changes the index. Recompute after didOpen rather
    // than applying offsets calculated against an unversioned disk snapshot.
    for (let attempt = 0; attempt < 3; attempt += 1) {
      if (token.isCancellationRequested || !this.isCurrentSource(source)) {
        return reject("Document changed before rename completed");
      }
      const edit = await this.safeRequest<LspWorkspaceEdit>(model, "textDocument/rename", {
        ...this.positionParams(model, position), newName,
      });
      if (edit === null || token.isCancellationRequested) return reject("Rename was not available");
      const changes = (edit.documentChanges ?? []).filter((change): change is LspDocumentEdit => "textDocument" in change);
      const renames = (edit.documentChanges ?? []).filter((change): change is LspRenameFile => !("textDocument" in change));
      if (renames.some((rename) => rename.kind !== "rename" || rename.options?.overwrite || rename.options?.ignoreIfExists)) {
        return reject("Unsupported file operation in rename");
      }
      const uris = [...new Set([
        ...changes.map((change) => this.monaco.Uri.parse(change.textDocument.uri).toString()),
        ...renames.map((rename) => this.monaco.Uri.parse(rename.oldUri).toString()),
        ...Object.keys(edit.changes ?? {}).map((uri) => this.monaco.Uri.parse(uri).toString()),
      ])];
      for (const uri of uris) requestedUris.add(uri);
      if (requestedUris.size > 256) return reject("Rename affects too many files; narrow the workspace first");
      const missing = uris.filter((uri) => !this.#documents.get(uri)?.open);
      const versioned = Object.keys(edit.changes ?? {}).length === 0 &&
        changes.every((change) => typeof change.textDocument.version === "number");
      if (missing.length === 0 && versioned) {
        const converted = this.workspaceEdit({ documentChanges: changes });
        if (converted === null) return reject("Document changed before rename completed");
        if (renames.length > 0 && !this.stageFileRenames(renames, changes)) {
          return reject("Could not prepare the file names required for rename");
        }
        return converted;
      }
      if (attempt === 2) return reject("Could not obtain current versions for every rename target");
      if (missing.length === 0) continue;
      if (this.#ensureDocuments === undefined) return reject("Open all affected files before renaming");
      const existing = [...this.#documents.values()].map((document) => ({
        document, version: document.model.getVersionId(),
      }));
      let ensured = false;
      try {
        ensured = await this.#ensureDocuments(missing);
      } catch {
        // Fail the whole refactor if even one target could not be loaded.
      }
      if (token.isCancellationRequested || !this.isCurrentSource(source) || existing.some(
        ({ document, version }) => this.#documents.get(document.uri) !== document ||
          document.model.getVersionId() !== version,
      )) return reject("Document changed before rename completed");
      if (!ensured || missing.some((uri) => !this.#documents.get(uri)?.open)) {
        return reject("Could not open all files required for rename");
      }
    }
    return reject("Rename was not available");
  }

  private async prepareLocationModels(
    model: Monaco.editor.ITextModel,
    method: "textDocument/definition" | "textDocument/references",
    params: unknown,
    token: Monaco.CancellationToken,
  ): Promise<readonly LspLocation[]> {
    const source = this.requestContext(model);
    if (source === null) return [];
    const requestedUris = new Set<string>();
    for (let attempt = 0; attempt < 3; attempt += 1) {
      if (token.isCancellationRequested || !this.isCurrentSource(source)) return [];
      const result = await this.safeRequest<LspLocation | readonly LspLocation[]>(model, method, params);
      if (result === null || token.isCancellationRequested) return [];
      const locations: readonly LspLocation[] = Array.isArray(result) ? result : [result as LspLocation];
      const uris = [...new Set(locations.map((location) => this.monaco.Uri.parse(location.uri).toString()))];
      for (const uri of uris) requestedUris.add(uri);
      if (requestedUris.size > 256) return [];
      const missing = uris.filter((uri) => !this.#documents.get(uri)?.open);
      if (missing.length === 0) return locations;
      if (attempt === 2 || this.#ensureDocuments === undefined) return [];
      const existing = [...this.#documents.values()].map((document) => ({
        document, version: document.model.getVersionId(),
      }));
      let ensured = false;
      try {
        ensured = await this.#ensureDocuments(missing);
      } catch {
        return [];
      }
      if (!ensured || token.isCancellationRequested || !this.isCurrentSource(source) || existing.some(
        ({ document, version }) => this.#documents.get(document.uri) !== document ||
          document.model.getVersionId() !== version,
      ) || missing.some((uri) => !this.#documents.get(uri)?.open)) return [];
      // Monaco's standalone peek UI cannot preview resources without models.
      // Requery after didOpen so previews and ranges describe these exact drafts.
    }
    return [];
  }

  private stageFileRenames(renames: readonly LspRenameFile[], changes: readonly LspDocumentEdit[]): boolean {
    if (this.#stageFileRenames === undefined) return false;
    const plans: OpenMatFileRename[] = [];
    const oldUris = new Set<string>();
    const newUris = new Set<string>();
    for (const rename of renames) {
      const oldUri = this.monaco.Uri.parse(rename.oldUri).toString();
      const newUri = this.monaco.Uri.parse(rename.newUri).toString();
      if (oldUri === newUri || oldUris.has(oldUri) || newUris.has(newUri)) return false;
      oldUris.add(oldUri);
      newUris.add(newUri);
      const document = this.#documents.get(oldUri);
      const ownerEdits = changes.filter((change) => this.monaco.Uri.parse(change.textDocument.uri).toString() === oldUri);
      if (!document?.open || ownerEdits.length === 0) return false;
      const originalText = document.model.getValue();
      const renamedText = applyTextEdits(originalText, ownerEdits.flatMap((change) => change.edits));
      if (renamedText === null || renamedText === originalText) return false;
      plans.push({ oldUri, newUri, originalText, renamedText });
    }
    try {
      return this.#stageFileRenames(plans);
    } catch {
      return false;
    }
  }

  private workspaceEdit(edit: LspWorkspaceEdit): Monaco.languages.WorkspaceEdit | null {
    const edits: Monaco.languages.IWorkspaceTextEdit[] = [];
    for (const documentChange of edit.documentChanges ?? []) {
      if (!("textDocument" in documentChange)) return null;
      const resource = this.monaco.Uri.parse(documentChange.textDocument.uri);
      const document = this.#documents.get(resource.toString());
      const version = documentChange.textDocument.version;
      if (!document?.open || (typeof version === "number" && document.version !== version)) {
        return null;
      }
      for (const textEdit of documentChange.edits) {
        edits.push({
          resource,
          textEdit: toMonacoTextEdit(textEdit),
          versionId: document?.model.getVersionId(),
        });
      }
    }
    for (const [uri, changes] of Object.entries(edit.changes ?? {})) {
      const resource = this.monaco.Uri.parse(uri);
      const document = this.#documents.get(resource.toString());
      if (!document?.open) return null;
      for (const textEdit of changes) {
        edits.push({
          resource,
          textEdit: toMonacoTextEdit(textEdit),
          versionId: document?.model.getVersionId(),
        });
      }
    }
    return { edits };
  }

  private requestContext(model: Monaco.editor.ITextModel): RequestContext | null {
    const document = this.#documents.get(model.uri.toString());
    if (this.#disposed || this.#client.state !== "ready" || !document?.open || document.model !== model) {
      return null;
    }
    return {
      document,
      modelVersion: model.getVersionId(),
      revision: this.#revision,
      generation: this.#generation,
    };
  }

  private isCurrentRequest(context: RequestContext): boolean {
    return this.isCurrentSource(context) && this.#revision === context.revision;
  }

  private isCurrentSource(context: RequestContext): boolean {
    return (
      !this.#disposed &&
      this.#client.state === "ready" &&
      context.document.open &&
      this.#documents.get(context.document.uri) === context.document &&
      context.document.model.getVersionId() === context.modelVersion &&
      this.#generation === context.generation
    );
  }

  private async safeRequest<T>(
    model: Monaco.editor.ITextModel,
    method: string,
    params: unknown,
    context = this.requestContext(model),
  ): Promise<T | null> {
    if (context === null || !this.isCurrentRequest(context)) {
      return null;
    }
    try {
      const result = await this.#client.request<T>(method, params);
      // Definitions and edits can depend on inactive documents as well. Any
      // workspace change invalidates results calculated against the old index.
      return this.isCurrentRequest(context) ? result : null;
    } catch {
      return null;
    }
  }
}

/** Compatibility wrapper for consumers that own just one Monaco model. */
export class OpenMatMonacoLspBridge extends OpenMatMonacoLspWorkspace {
  constructor(monaco: MonacoApi, model: Monaco.editor.ITextModel, options: OpenMatMonacoLspOptions) {
    super(monaco, options);
    this.syncModels([{ model, documentVersion: options.documentVersion }]);
  }
}

export function attachOpenMatLsp(
  monaco: MonacoApi,
  model: Monaco.editor.ITextModel,
  options: OpenMatMonacoLspOptions,
): Monaco.IDisposable {
  return new OpenMatMonacoLspBridge(monaco, model, options).start();
}
