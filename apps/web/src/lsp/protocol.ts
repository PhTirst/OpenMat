export const LSP_JSON_RPC_VERSION = "2.0" as const;
export const OPENMAT_LSP_LANGUAGE_ID = "openmat" as const;
export const OPENMAT_LSP_MARKER_OWNER = "openmat-lsp" as const;
export const OPENMAT_LSP_MAX_MESSAGE_BYTES = 1024 * 1024;

export type JsonRpcId = number | string;

export interface LspPosition {
  readonly line: number;
  readonly character: number;
}

export interface LspRange {
  readonly start: LspPosition;
  readonly end: LspPosition;
}

export interface LspTextEdit {
  readonly range: LspRange;
  readonly newText: string;
}

export interface LspLocation {
  readonly uri: string;
  readonly range: LspRange;
}

export interface LspDiagnostic {
  readonly range: LspRange;
  readonly severity?: number;
  readonly code?: string | number;
  readonly source?: string;
  readonly message: string;
}

export interface PublishDiagnosticsParams {
  readonly uri: string;
  readonly version?: number;
  readonly diagnostics: readonly LspDiagnostic[];
}

export interface LspMarkupContent {
  readonly kind: "markdown" | "plaintext";
  readonly value: string;
}

export interface LspHover {
  readonly contents: LspMarkupContent | string | readonly (LspMarkupContent | string)[];
  readonly range?: LspRange;
}

export interface LspCompletionItem {
  readonly label: string;
  readonly kind?: number;
  readonly detail?: string;
  readonly documentation?: LspMarkupContent | string;
  readonly textEdit?: LspTextEdit;
  readonly insertText?: string;
  readonly data?: unknown;
}

export interface LspCompletionList {
  readonly isIncomplete?: boolean;
  readonly items: readonly LspCompletionItem[];
}

export interface LspPrepareRename {
  readonly range: LspRange;
  readonly placeholder: string;
}

export interface LspDocumentEdit {
  readonly textDocument: {
    readonly uri: string;
    readonly version: number | null;
  };
  readonly edits: readonly LspTextEdit[];
}

export interface LspWorkspaceEdit {
  readonly documentChanges?: readonly (LspDocumentEdit | LspRenameFile)[];
  readonly changes?: Readonly<Record<string, readonly LspTextEdit[]>>;
}

export interface LspRenameFile {
  readonly kind: "rename";
  readonly oldUri: string;
  readonly newUri: string;
  readonly options?: {
    readonly overwrite?: boolean;
    readonly ignoreIfExists?: boolean;
  };
}

export interface LspSemanticTokens {
  readonly resultId?: string;
  readonly data: readonly number[];
}

export interface LspDocumentSymbol {
  readonly name: string;
  readonly detail?: string;
  readonly kind: number;
  readonly range: LspRange;
  readonly selectionRange: LspRange;
  readonly children?: readonly LspDocumentSymbol[];
}

export interface LspCodeAction {
  readonly title: string;
  readonly kind?: string;
  readonly diagnostics?: readonly LspDiagnostic[];
  readonly edit?: LspWorkspaceEdit;
  readonly isPreferred?: boolean;
}

export interface LspInitializeResult {
  readonly capabilities: {
    readonly semanticTokensProvider?: {
      readonly legend?: {
        readonly tokenTypes?: readonly string[];
        readonly tokenModifiers?: readonly string[];
      };
    };
  };
  readonly serverInfo?: {
    readonly name: string;
    readonly version?: string;
  };
}

export interface JsonRpcErrorValue {
  readonly code: number;
  readonly message: string;
  readonly data?: unknown;
}

export interface JsonRpcResponse {
  readonly jsonrpc: typeof LSP_JSON_RPC_VERSION;
  readonly id: JsonRpcId | null;
  readonly result?: unknown;
  readonly error?: JsonRpcErrorValue;
}

export interface JsonRpcNotification {
  readonly jsonrpc: typeof LSP_JSON_RPC_VERSION;
  readonly method: string;
  readonly params?: unknown;
}
