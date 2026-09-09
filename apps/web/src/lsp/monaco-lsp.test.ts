import type * as Monaco from "monaco-editor/esm/vs/editor/editor.api";
import { describe, expect, it, vi } from "vitest";
import type { LspClient, LspConnectionState } from "./client";
import { OpenMatMonacoLspBridge, OpenMatMonacoLspWorkspace, type OpenMatFileRename } from "./monaco-lsp";
import type { JsonRpcNotification, LspInitializeResult } from "./protocol";

type MonacoApi = typeof import("monaco-editor/esm/vs/editor/editor.api");

class FakeLspClient implements LspClient {
  state: LspConnectionState = "disconnected";
  initializeResult: LspInitializeResult | null = { capabilities: {} };
  readonly notifications: Array<{ method: string; params: unknown }> = [];
  readonly requests: Array<{ method: string; params: unknown }> = [];
  connectCount = 0;
  disconnectCount = 0;
  respond: ((method: string, params: unknown) => unknown) | undefined;
  readonly #notificationListeners = new Set<(notification: JsonRpcNotification) => void>();
  readonly #stateListeners = new Set<(state: LspConnectionState) => void>();

  async connect(): Promise<void> {
    this.connectCount += 1;
    this.setState("connecting");
    this.setState("ready");
  }

  async disconnect(): Promise<void> {
    this.disconnectCount += 1;
    this.setState("disconnected");
  }

  async request<T>(method: string, params: unknown): Promise<T> {
    this.requests.push({ method, params });
    if (this.respond !== undefined) {
      return await this.respond(method, params) as T;
    }
    if (method === "textDocument/completion") {
      return [
        {
          label: "calculate",
          kind: 3,
          detail: "function",
          textEdit: {
            range: {
              start: { line: 3, character: 0 },
              end: { line: 3, character: 3 },
            },
            newText: "calculate",
          },
          data: { uri: "file:///workspace/demo.m", version: 1 },
        },
      ] as T;
    }
    return null as T;
  }

  notify(method: string, params: unknown): boolean {
    if (this.state !== "ready") {
      return false;
    }
    this.notifications.push({ method, params });
    return true;
  }

  onNotification(listener: (notification: JsonRpcNotification) => void): () => void {
    this.#notificationListeners.add(listener);
    return () => this.#notificationListeners.delete(listener);
  }

  onStateChange(listener: (state: LspConnectionState) => void): () => void {
    this.#stateListeners.add(listener);
    return () => this.#stateListeners.delete(listener);
  }

  publish(notification: JsonRpcNotification): void {
    for (const listener of this.#notificationListeners) {
      listener(notification);
    }
  }

  setState(state: LspConnectionState): void {
    this.state = state;
    for (const listener of this.#stateListeners) {
      listener(state);
    }
  }
}

function createMonacoHarness() {
  const providers = new Map<string, unknown>();
  const markerSets: Monaco.editor.IMarkerData[][] = [];
  const markersByModel = new Map<Monaco.editor.ITextModel, Monaco.editor.IMarkerData[]>();
  const registrationCounts = new Map<string, number>();
  const register = (name: string) => (_language: string, provider: unknown) => {
    registrationCounts.set(name, (registrationCounts.get(name) ?? 0) + 1);
    providers.set(name, provider);
    return { dispose: () => providers.delete(name) };
  };
  const monaco = {
    MarkerSeverity: { Hint: 1, Info: 2, Warning: 4, Error: 8 },
    Uri: { parse: (value: string) => ({ toString: () => value }) },
    editor: {
      setModelMarkers: (
        model: Monaco.editor.ITextModel,
        _owner: string,
        markers: Monaco.editor.IMarkerData[],
      ) => {
        markerSets.push(markers);
        markersByModel.set(model, markers);
      },
    },
    languages: {
      CompletionItemKind: {
        Method: 0,
        Function: 1,
        Variable: 4,
        Class: 5,
        Property: 9,
        Keyword: 17,
        Text: 18,
      },
      registerCompletionItemProvider: register("completion"),
      registerHoverProvider: register("hover"),
      registerDefinitionProvider: register("definition"),
      registerReferenceProvider: register("references"),
      registerRenameProvider: register("rename"),
      registerDocumentFormattingEditProvider: register("formatting"),
      registerDocumentSemanticTokensProvider: register("semanticTokens"),
      registerCodeActionProvider: register("codeAction"),
      registerDocumentSymbolProvider: register("documentSymbol"),
    },
  } as unknown as MonacoApi;
  return { monaco, providers, markerSets, markersByModel, registrationCounts };
}

describe("OpenMatMonacoLspBridge", () => {
  it("uses LSP completion, republishes changed diagnostics, and clears markers on close", async () => {
    const configuredUri = "openmat-workspace:///workspace/demo.m";
    const uri = "openmat-workspace:/workspace/demo.m";
    let value = "function y = calculate(x)\ny = x;\nend\ncal";
    let onChange: () => void = () => undefined;
    const model = {
      uri: { toString: () => uri },
      getValue: () => value,
      getVersionId: () => 1,
      getWordUntilPosition: () => ({ word: "cal", startColumn: 1, endColumn: 4 }),
      onDidChangeContent: (listener: () => void) => {
        onChange = listener;
        return { dispose: () => undefined };
      },
    } as unknown as Monaco.editor.ITextModel;
    const client = new FakeLspClient();
    const { monaco, providers, markerSets } = createMonacoHarness();
    const bridge = new OpenMatMonacoLspBridge(monaco, model, {
      url: "ws://127.0.0.1:49152/lsp",
      documentUri: configuredUri,
      documentVersion: 1,
      client,
      reconnectDelays: [],
    }).start();
    await Promise.resolve();

    expect(client.notifications[0]).toMatchObject({
      method: "textDocument/didOpen",
      params: { textDocument: { uri, version: 1, text: value } },
    });
    const completionProvider = providers.get(
      "completion",
    ) as Monaco.languages.CompletionItemProvider;
    const completion = await completionProvider.provideCompletionItems(
      model,
      { lineNumber: 4, column: 4 } as Monaco.Position,
      { triggerKind: 0 },
      { isCancellationRequested: false, onCancellationRequested: () => ({ dispose: () => undefined }) },
    );
    expect(completion?.suggestions).toEqual([
      expect.objectContaining({
        label: "calculate",
        detail: "function",
        insertText: "calculate",
        range: {
          startLineNumber: 4,
          startColumn: 1,
          endLineNumber: 4,
          endColumn: 4,
        },
      }),
    ]);
    expect(client.requests[0]?.method).toBe("textDocument/completion");

    value = "😀";
    onChange();
    expect(client.notifications.at(-1)).toMatchObject({
      method: "textDocument/didChange",
      params: {
        textDocument: { uri, version: 2 },
        contentChanges: [{ text: "😀" }],
      },
    });
    client.publish({
      jsonrpc: "2.0",
      method: "textDocument/publishDiagnostics",
      params: {
        uri,
        version: 2,
        diagnostics: [
          {
            range: {
              start: { line: 0, character: 0 },
              end: { line: 0, character: 2 },
            },
            severity: 1,
            code: "OML0002",
            source: "openmat",
            message: "invalid token",
          },
        ],
      },
    });
    expect(markerSets.at(-1)).toEqual([
      expect.objectContaining({
        code: "OML0002",
        severity: 8,
        startLineNumber: 1,
        endColumn: 3,
      }),
    ]);

    bridge.dispose();
    expect(client.notifications.at(-1)).toMatchObject({
      method: "textDocument/didClose",
      params: { textDocument: { uri } },
    });
    expect(markerSets.at(-1)).toEqual([]);
  });
});


function createModel(uri: string, text = "value = calculate(1);") {
  let value = text;
  let version = 1;
  const listeners = new Set<() => void>();
  const model = {
    uri: { toString: () => uri },
    getValue: () => value,
    getVersionId: () => version,
    getWordUntilPosition: () => ({ word: "calculate", startColumn: 9, endColumn: 18 }),
    onDidChangeContent: (listener: () => void) => {
      listeners.add(listener);
      return { dispose: () => listeners.delete(listener) };
    },
  } as unknown as Monaco.editor.ITextModel;
  return {
    model,
    setValue: (next: string) => {
      value = next;
      version += 1;
      for (const listener of listeners) {
        listener();
      }
    },
  };
}

const cancellationToken: Monaco.CancellationToken = {
  isCancellationRequested: false,
  onCancellationRequested: () => ({ dispose: () => undefined }),
};
const requestPosition = { lineNumber: 1, column: 12 } as Monaco.Position;
const lspRange = { start: { line: 0, character: 8 }, end: { line: 0, character: 17 } };
const monacoRange = { startLineNumber: 1, startColumn: 9, endLineNumber: 1, endColumn: 18 };
const textEdit = { range: lspRange, newText: "renamed" };

function workspaceHarness(
  reconnectDelays: readonly number[] = [],
  ensureDocuments?: (uris: readonly string[]) => Promise<boolean>,
  stageFileRenames?: (renames: readonly OpenMatFileRename[]) => boolean,
) {
  const harness = createMonacoHarness();
  const client = new FakeLspClient();
  const first = createModel("openmat-workspace:/first.m");
  const second = createModel("openmat-workspace:/second.m", "function y = calculate(x)\ny = x;\nend");
  const documents = [
    { model: first.model, documentVersion: 3 },
    { model: second.model, documentVersion: 7 },
  ];
  const workspace = new OpenMatMonacoLspWorkspace(harness.monaco, {
    url: "ws://127.0.0.1:49152/lsp", client, reconnectDelays,
    ...(ensureDocuments === undefined ? {} : { ensureDocuments }),
    ...(stageFileRenames === undefined ? {} : { stageFileRenames }),
  });
  workspace.syncModels(documents);
  workspace.start();
  return { ...harness, client, first, second, documents, workspace };
}

function publishDiagnostic(client: FakeLspClient, model: Monaco.editor.ITextModel, version: number) {
  client.publish({
    jsonrpc: "2.0",
    method: "textDocument/publishDiagnostics",
    params: {
      uri: model.uri.toString(), version,
      diagnostics: [{ range: lspRange, severity: 1, message: "Unclosed expression" }],
    },
  });
}

function deferred() {
  let resolve: (value: unknown) => void = () => undefined;
  const promise = new Promise<unknown>((complete) => { resolve = complete; });
  return { promise, resolve };
}

describe("OpenMatMonacoLspWorkspace", () => {
  it("shares one connection and provider set while syncing and closing individual documents", async () => {
    const { workspace, client, first, second, documents, providers, registrationCounts } = workspaceHarness();
    expect(client.notifications).toEqual(documents.map(({ model, documentVersion }) => ({
      method: "textDocument/didOpen",
      params: { textDocument: { uri: model.uri.toString(), languageId: "openmat", version: documentVersion, text: model.getValue() } },
    })));

    workspace.start();
    workspace.syncModels([...documents].reverse());
    expect(client.connectCount).toBe(1);
    expect(client.notifications).toHaveLength(2);
    expect([...registrationCounts.values()]).toEqual(Array(9).fill(1));

    second.setValue("function y = calculate(x)\ny = x + 1;\nend");
    expect(client.notifications.at(-1)).toEqual({
      method: "textDocument/didChange",
      params: { textDocument: { uri: second.model.uri.toString(), version: 8 }, contentChanges: [{ text: second.model.getValue() }] },
    });
    workspace.syncModels([{ model: second.model, documentVersion: 8 }]);
    expect(client.notifications.at(-1)).toEqual({ method: "textDocument/didClose", params: { textDocument: { uri: first.model.uri.toString() } } });
    const notificationsAfterClose = client.notifications.length;
    first.setValue("closed model change");
    expect(client.notifications).toHaveLength(notificationsAfterClose);
    expect(client.disconnectCount).toBe(0);
    const completion = providers.get("completion") as Monaco.languages.CompletionItemProvider;
    expect(await completion.provideCompletionItems(first.model, requestPosition, { triggerKind: 0 }, cancellationToken)).toEqual({ suggestions: [] });
    await completion.provideCompletionItems(second.model, requestPosition, { triggerKind: 0 }, cancellationToken);
    expect(client.requests.at(-1)).toMatchObject({ params: { textDocument: { uri: second.model.uri.toString() } } });

    workspace.dispose();
    expect(client.notifications.at(-1)).toEqual({ method: "textDocument/didClose", params: { textDocument: { uri: second.model.uri.toString() } } });
    expect(client.disconnectCount).toBe(1);
    expect(providers.size).toBe(0);
  });

  it("reopens every latest document after reconnect and cancels retries on disposal", async () => {
    vi.useFakeTimers();
    try {
      const { workspace, client, first, second, markersByModel } = workspaceHarness([5]);
      publishDiagnostic(client, first.model, 3);
      publishDiagnostic(client, second.model, 7);
      client.setState("disconnected");
      expect(markersByModel.get(first.model)).toEqual([]);
      expect(markersByModel.get(second.model)).toEqual([]);
      first.setValue("offline first");
      second.setValue("offline second");
      expect(client.notifications).toHaveLength(2);
      await vi.advanceTimersByTimeAsync(5);
      expect(client.connectCount).toBe(2);
      expect(client.notifications.slice(-2)).toEqual([
        { method: "textDocument/didOpen", params: { textDocument: { uri: first.model.uri.toString(), languageId: "openmat", version: 4, text: "offline first" } } },
        { method: "textDocument/didOpen", params: { textDocument: { uri: second.model.uri.toString(), languageId: "openmat", version: 8, text: "offline second" } } },
      ]);
      client.setState("disconnected");
      workspace.dispose();
      await vi.advanceTimersByTimeAsync(10);
      expect(client.connectCount).toBe(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("routes diagnostics by model, discards obsolete versions, and rejects diagnostics for closed models", () => {
    const { workspace, client, first, second, documents, markersByModel } = workspaceHarness();
    publishDiagnostic(client, first.model, 3);
    publishDiagnostic(client, second.model, 7);
    expect(markersByModel.get(first.model)).toHaveLength(1);
    expect(markersByModel.get(second.model)).toHaveLength(1);
    first.setValue("updated");
    expect(markersByModel.get(first.model)).toEqual([]);
    publishDiagnostic(client, first.model, 3);
    publishDiagnostic(client, first.model, 5);
    expect(markersByModel.get(first.model)).toEqual([]);
    expect(markersByModel.get(second.model)).toHaveLength(1);
    workspace.syncModels([documents[1]!]);
    publishDiagnostic(client, first.model, 4);
    expect(markersByModel.get(first.model)).toEqual([]);
    workspace.syncModels(documents);
    publishDiagnostic(client, first.model, 4);
    expect(markersByModel.get(first.model)).toEqual([]);
    publishDiagnostic(client, first.model, 5);
    expect(markersByModel.get(first.model)).toHaveLength(1);
    client.setState("disconnected");
    publishDiagnostic(client, first.model, 5);
    expect(markersByModel.get(first.model)).toEqual([]);
    workspace.dispose();
  });

  it("resolves each completion using its original item and stops resolving an obsolete model", async () => {
    const { workspace, client, first, second, documents, providers } = workspaceHarness();
    client.respond = (method, params) => {
      if (method === "textDocument/completion") {
        const { textDocument } = params as { textDocument: { uri: string } };
        return [{ label: "calculate", data: { uri: textDocument.uri } }];
      }
      return { label: "calculate", detail: "Resolved function" };
    };
    const provider = providers.get("completion") as Monaco.languages.CompletionItemProvider;
    const firstList = await provider.provideCompletionItems(first.model, requestPosition, { triggerKind: 0 }, cancellationToken);
    const secondList = await provider.provideCompletionItems(second.model, requestPosition, { triggerKind: 0 }, cancellationToken);
    const firstItem = firstList!.suggestions[0]!;
    const secondItem = secondList!.suggestions[0]!;
    expect(await provider.resolveCompletionItem!(firstItem, cancellationToken)).toMatchObject({ detail: "Resolved function" });
    expect(client.requests.at(-1)).toEqual({ method: "completionItem/resolve", params: { label: "calculate", data: { uri: first.model.uri.toString() } } });
    expect(await provider.resolveCompletionItem!(secondItem, cancellationToken)).toMatchObject({ detail: "Resolved function" });
    expect(client.requests.at(-1)).toMatchObject({ params: { data: { uri: second.model.uri.toString() } } });
    workspace.syncModels([documents[1]!]);
    const requestCount = client.requests.length;
    expect(await provider.resolveCompletionItem!(firstItem, cancellationToken)).toBe(firstItem);
    expect(client.requests).toHaveLength(requestCount);
    workspace.dispose();
  });

  it.each(["source change", "inactive change", "source replacement", "reconnect", "dispose"])("ignores pending completion after %s", async (change) => {
    const { workspace, client, first, second, documents, providers } = workspaceHarness();
    const pending = deferred();
    client.respond = () => pending.promise;
    const provider = providers.get("completion") as Monaco.languages.CompletionItemProvider;
    const result = provider.provideCompletionItems(first.model, requestPosition, { triggerKind: 0 }, cancellationToken);
    if (change === "source change") first.setValue("new source");
    if (change === "inactive change") second.setValue("new dependency");
    if (change === "source replacement") workspace.syncModels([{ model: createModel(first.model.uri.toString()).model, documentVersion: 3 }, documents[1]!]);
    if (change === "reconnect") {
      client.setState("disconnected");
      await client.connect();
    }
    if (change === "dispose") workspace.dispose();
    pending.resolve([{ label: "obsolete" }]);
    expect(await result).toEqual({ suggestions: [] });
    workspace.dispose();
  });

  it("attaches both Monaco versions to multi-document rename edits and rejects mismatched LSP versions", async () => {
    const { workspace, client, first, second, providers } = workspaceHarness();
    second.setValue("function y = calculate(x)\ny = x + 1;\nend");
    let targetVersion = 8;
    client.respond = () => ({ documentChanges: [
      { textDocument: { uri: first.model.uri.toString(), version: 3 }, edits: [textEdit] },
      { textDocument: { uri: second.model.uri.toString(), version: targetVersion }, edits: [textEdit] },
    ] });
    const provider = providers.get("rename") as Monaco.languages.RenameProvider;
    const result = await provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken);
    expect(result?.edits).toEqual([
      { resource: expect.anything(), textEdit: { range: monacoRange, text: "renamed" }, versionId: 1 },
      { resource: expect.anything(), textEdit: { range: monacoRange, text: "renamed" }, versionId: 2 },
    ]);
    expect(result?.edits.map((edit) => (edit as Monaco.languages.IWorkspaceTextEdit).resource.toString())).toEqual([first.model.uri.toString(), second.model.uri.toString()]);
    targetVersion = 7;
    expect(await provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken)).toMatchObject({ edits: [], rejectReason: expect.any(String) });
    workspace.dispose();
  });

  it("opens disk rename targets and recomputes their edits after didOpen with current versions", async () => {
    const third = createModel("openmat-workspace:/unopened.m", "calculate(1)");
    const ensure = vi.fn(async (_uris: readonly string[]) => {
      workspace.syncModels([...documents, { model: third.model, documentVersion: 11 }]);
      return true;
    });
    const { workspace, client, first, documents, providers } = workspaceHarness([], ensure);
    let requests = 0;
    client.respond = () => {
      requests += 1;
      if (requests === 2) expect(client.notifications.at(-1)).toMatchObject({
        method: "textDocument/didOpen", params: { textDocument: { uri: third.model.uri.toString(), version: 11 } },
      });
      return { documentChanges: [
        { textDocument: { uri: first.model.uri.toString(), version: 3 }, edits: [textEdit] },
        { textDocument: { uri: third.model.uri.toString(), version: requests === 1 ? null : 11 },
          edits: [{ ...textEdit, newText: requests === 1 ? "obsolete disk edit" : "fresh draft edit" }] },
      ] };
    };
    const provider = providers.get("rename") as Monaco.languages.RenameProvider;
    const result = await provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken);
    expect(ensure).toHaveBeenCalledExactlyOnceWith([third.model.uri.toString()]);
    expect(requests).toBe(2);
    expect(result?.edits).toHaveLength(2);
    expect(result?.edits[1]).toMatchObject({ textEdit: { text: "fresh draft edit" }, versionId: 1 });
    workspace.dispose();
  });

  it.each(["source edited", "inactive edited", "reconnect", "inaccessible", "cancelled"])(
    "rejects the whole rename when loading a disk target encounters %s", async (failure) => {
      const third = createModel("openmat-workspace:/unopened.m");
      const loading = deferred();
      const ensure = vi.fn(async () => {
        await loading.promise;
        workspace.syncModels([...documents, { model: third.model, documentVersion: 1 }]);
        return failure !== "inaccessible";
      });
      const { workspace, client, first, second, documents, providers } = workspaceHarness([], ensure);
      const token = { ...cancellationToken, isCancellationRequested: false };
      client.respond = () => ({ documentChanges: [
        { textDocument: { uri: third.model.uri.toString(), version: null }, edits: [textEdit] },
      ] });
      const provider = providers.get("rename") as Monaco.languages.RenameProvider;
      const result = provider.provideRenameEdits(first.model, requestPosition, "renamed", token);
      await vi.waitFor(() => expect(ensure).toHaveBeenCalledOnce());
      if (failure === "source edited") first.setValue("new source");
      if (failure === "inactive edited") second.setValue("new dependency");
      if (failure === "cancelled") token.isCancellationRequested = true;
      if (failure === "reconnect") {
        client.setState("disconnected");
        await client.connect();
      }
      loading.resolve(true);
      expect(await result).toMatchObject({ edits: [], rejectReason: expect.any(String) });
      expect(client.requests).toHaveLength(1);
      workspace.dispose();
    },
  );

  it("rejects changes to a newly loaded target while the second rename request is pending", async () => {
    const third = createModel("openmat-workspace:/unopened.m");
    const ensure = async () => {
      workspace.syncModels([...documents, { model: third.model, documentVersion: 1 }]);
      return true;
    };
    const { workspace, client, first, documents, providers } = workspaceHarness([], ensure);
    const pending = deferred();
    client.respond = () => client.requests.length === 1 ? ({ documentChanges: [
      { textDocument: { uri: third.model.uri.toString(), version: null }, edits: [textEdit] },
    ] }) : pending.promise;
    const provider = providers.get("rename") as Monaco.languages.RenameProvider;
    const result = provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken);
    await vi.waitFor(() => expect(client.requests).toHaveLength(2));
    third.setValue("new unsaved text");
    pending.resolve({ documentChanges: [
      { textDocument: { uri: third.model.uri.toString(), version: 1 }, edits: [textEdit] },
    ] });
    expect(await result).toMatchObject({ edits: [], rejectReason: expect.any(String) });
    workspace.dispose();
  });

  it("bounds rename retries instead of accepting an unversioned edit to an open file", async () => {
    const { workspace, client, first, providers } = workspaceHarness();
    client.respond = () => ({ changes: { [first.model.uri.toString()]: [textEdit] } });
    const provider = providers.get("rename") as Monaco.languages.RenameProvider;
    expect(await provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken))
      .toMatchObject({ edits: [], rejectReason: expect.any(String) });
    expect(client.requests).toHaveLength(3);
    workspace.dispose();
  });

  it("bounds automatic rename target loading", async () => {
    const ensure = vi.fn(async () => true);
    const { workspace, client, first, providers } = workspaceHarness([], ensure);
    client.respond = () => ({ documentChanges: Array.from({ length: 257 }, (_, index) => ({
      textDocument: { uri: `openmat-workspace:/target${index}.m`, version: null }, edits: [textEdit],
    })) });
    const provider = providers.get("rename") as Monaco.languages.RenameProvider;
    expect(await provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken))
      .toMatchObject({ edits: [], rejectReason: expect.any(String) });
    expect(ensure).not.toHaveBeenCalled();
    workspace.dispose();
  });

  it("stages a standard RenameFile using the unchanged old URI and exact UTF-16 text edits", async () => {
    const originalText = '% 😀\r\nfunction y = calculate(x)\r\nprefix = "😀"; y = calculate(x);\r\nend\r\n';
    const owner = createModel("openmat-workspace:/calculate.m", originalText);
    const newUri = "openmat-workspace:/renamed.m";
    const ownerEdits = originalText.split(/\r\n/).flatMap((line, index) => {
      const character = line.indexOf("calculate");
      return character < 0 ? [] : [{
        range: { start: { line: index, character }, end: { line: index, character: character + 9 } },
        newText: "renamed",
      }];
    });
    const ensure = vi.fn(async () => {
      workspace.syncModels([...documents, { model: owner.model, documentVersion: 1 }]);
      return true;
    });
    const stage = vi.fn(() => true);
    const { workspace, client, first, documents, providers } = workspaceHarness([], ensure, stage);
    client.respond = () => ({ documentChanges: [
      { textDocument: { uri: first.model.uri.toString(), version: 3 }, edits: [textEdit] },
      { textDocument: { uri: owner.model.uri.toString(), version: client.requests.length === 1 ? null : 1 }, edits: ownerEdits },
      { kind: "rename", oldUri: owner.model.uri.toString(), newUri, options: { overwrite: false } },
    ] });
    const provider = providers.get("rename") as Monaco.languages.RenameProvider;
    const result = await provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken);
    expect(ensure).toHaveBeenCalledExactlyOnceWith([owner.model.uri.toString()]);
    expect(stage).toHaveBeenCalledExactlyOnceWith([{
      oldUri: owner.model.uri.toString(), newUri, originalText,
      renamedText: originalText.replaceAll("calculate", "renamed"),
    }]);
    expect(result?.edits).toHaveLength(3);
    expect(result?.edits.every((edit) => "textEdit" in edit)).toBe(true);
    expect(owner.model.getValue()).toBe(originalText);
    expect(client.requests).toHaveLength(2);
    workspace.dispose();
  });

  it.each(["unavailable", "rejected", "stale", "overlapping"])(
    "rejects both text and file rename operations when staging is %s", async (failure) => {
      const stage = failure === "unavailable" ? undefined : vi.fn(() => failure !== "rejected");
      const { workspace, client, first, providers } = workspaceHarness([], undefined, stage);
      const originalText = first.model.getValue();
      client.respond = () => ({ documentChanges: [
        { textDocument: { uri: first.model.uri.toString(), version: failure === "stale" ? 2 : 3 },
          edits: failure === "overlapping" ? [textEdit, textEdit] : [textEdit] },
        { kind: "rename", oldUri: first.model.uri.toString(), newUri: "openmat-workspace:/renamed.m", options: { overwrite: false } },
      ] });
      const provider = providers.get("rename") as Monaco.languages.RenameProvider;
      expect(await provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken))
        .toMatchObject({ edits: [], rejectReason: expect.any(String) });
      expect(first.model.getValue()).toBe(originalText);
      if (failure === "stale" || failure === "overlapping") expect(stage).not.toHaveBeenCalled();
      workspace.dispose();
    },
  );

  it.each(["definition", "references"])("opens preview models before delivering %s locations", async (kind) => {
    const third = createModel("openmat-workspace:/unopened.m", "line one\nline two");
    const ensure = vi.fn(async () => {
      workspace.syncModels([...documents, { model: third.model, documentVersion: 1 }]);
      return true;
    });
    const { workspace, client, first, documents, providers } = workspaceHarness([], ensure);
    client.respond = () => [{
      uri: third.model.uri.toString(),
      range: client.requests.length === 1 ? lspRange : { start: { line: 1, character: 0 }, end: { line: 1, character: 4 } },
    }];
    const result = kind === "definition"
      ? await (providers.get(kind) as Monaco.languages.DefinitionProvider).provideDefinition(first.model, requestPosition, cancellationToken)
      : await (providers.get(kind) as Monaco.languages.ReferenceProvider).provideReferences(first.model, requestPosition, { includeDeclaration: true }, cancellationToken);
    expect(ensure).toHaveBeenCalledExactlyOnceWith([third.model.uri.toString()]);
    expect(client.requests).toHaveLength(2);
    expect(result).toEqual([{ uri: expect.anything(), range: {
      startLineNumber: 2, startColumn: 1, endLineNumber: 2, endColumn: 5,
    } }]);
    workspace.dispose();
  });

  it.each(["cancelled", "inaccessible", "source edited"])("does not deliver reference previews when loading is %s", async (failure) => {
    const token = { ...cancellationToken, isCancellationRequested: false };
    const third = createModel("openmat-workspace:/unopened.m");
    const ensure = async () => {
      workspace.syncModels([...documents, { model: third.model, documentVersion: 1 }]);
      if (failure === "cancelled") token.isCancellationRequested = true;
      if (failure === "source edited") first.setValue("new source");
      if (failure === "inaccessible") throw new Error("Cannot read file");
      return true;
    };
    const { workspace, client, first, documents, providers } = workspaceHarness([], ensure);
    client.respond = () => [{ uri: third.model.uri.toString(), range: lspRange }];
    const provider = providers.get("references") as Monaco.languages.ReferenceProvider;
    expect(await provider.provideReferences(first.model, requestPosition, { includeDeclaration: true }, token)).toEqual([]);
    expect(client.requests).toHaveLength(1);
    workspace.dispose();
  });

  it("rejects a pending rename when an inactive target was edited", async () => {
    const { workspace, client, first, second, providers } = workspaceHarness();
    const pending = deferred();
    client.respond = () => pending.promise;
    const provider = providers.get("rename") as Monaco.languages.RenameProvider;
    const result = provider.provideRenameEdits(first.model, requestPosition, "renamed", cancellationToken);
    second.setValue("a newer unsaved target");
    pending.resolve({ changes: { [second.model.uri.toString()]: [textEdit] } });
    expect(await result).toMatchObject({ edits: [], rejectReason: expect.any(String) });
    workspace.dispose();
  });

  it("invalidates every provider result when the workspace changes while requests are pending", async () => {
    const { workspace, client, first, second, providers } = workspaceHarness();
    const pending = new Map<string, ReturnType<typeof deferred>>();
    client.respond = (method) => {
      const result = deferred();
      pending.set(method, result);
      return result.promise;
    };
    const hover = (providers.get("hover") as Monaco.languages.HoverProvider).provideHover(first.model, requestPosition, cancellationToken);
    const definition = (providers.get("definition") as Monaco.languages.DefinitionProvider).provideDefinition(first.model, requestPosition, cancellationToken);
    const references = (providers.get("references") as Monaco.languages.ReferenceProvider).provideReferences(first.model, requestPosition, { includeDeclaration: true }, cancellationToken);
    const rename = (providers.get("rename") as Monaco.languages.RenameProvider).resolveRenameLocation!(first.model, requestPosition, cancellationToken);
    const formatting = (providers.get("formatting") as Monaco.languages.DocumentFormattingEditProvider).provideDocumentFormattingEdits(first.model, { tabSize: 2, insertSpaces: true }, cancellationToken);
    const semantic = (providers.get("semanticTokens") as Monaco.languages.DocumentSemanticTokensProvider).provideDocumentSemanticTokens(first.model, null, cancellationToken);
    const codeAction = (providers.get("codeAction") as Monaco.languages.CodeActionProvider).provideCodeActions(first.model, monacoRange as Monaco.Range, { markers: [], trigger: 1 }, cancellationToken);
    const symbols = (providers.get("documentSymbol") as Monaco.languages.DocumentSymbolProvider).provideDocumentSymbols(first.model, cancellationToken);
    second.setValue("new dependency");
    pending.get("textDocument/hover")!.resolve({ contents: "obsolete" });
    pending.get("textDocument/definition")!.resolve({ uri: first.model.uri.toString(), range: lspRange });
    pending.get("textDocument/references")!.resolve([{ uri: first.model.uri.toString(), range: lspRange }]);
    pending.get("textDocument/prepareRename")!.resolve({ range: lspRange, placeholder: "calculate" });
    pending.get("textDocument/formatting")!.resolve([textEdit]);
    pending.get("textDocument/semanticTokens/full")!.resolve({ data: [0, 0, 1, 1, 0] });
    pending.get("textDocument/codeAction")!.resolve([{ title: "Fix", edit: { changes: { [first.model.uri.toString()]: [textEdit] } } }]);
    pending.get("textDocument/documentSymbol")!.resolve([{ name: "calculate", kind: 12, range: lspRange, selectionRange: lspRange }]);
    expect(await hover).toBeNull();
    expect(await definition).toBeNull();
    expect(await references).toEqual([]);
    expect(await rename).toMatchObject({ rejectReason: expect.any(String) });
    expect(await formatting).toEqual([]);
    expect(await semantic).toEqual({ data: new Uint32Array() });
    expect(await codeAction).toMatchObject({ actions: [] });
    expect(await symbols).toEqual([]);
    workspace.dispose();
  });
});
