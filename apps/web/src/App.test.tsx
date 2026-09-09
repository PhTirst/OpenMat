import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
// @ts-expect-error Monaco exposes this URI runtime without a declaration file.
import { URI } from "monaco-editor/esm/vs/base/common/uri.js";
import { App, createDefaultKernelTransport } from "./App";
import type { DesktopFiles, NativeFileLocation, PlatformServices } from "./platform/platform-services";
import type { CodeEditorProps } from "./components/CodeEditor";
import {
  openDocumentFromWorkspaceFile,
  snapshotDocumentSession,
  type PersistedDocumentSession,
} from "./documents/document-session";
import type { DocumentSessionStore } from "./documents/document-session-storage";
import { SharedEditorSession } from "./lsp/shared-editor-session";
import { SIGNAL_CODE } from "./designer/example";
import { IDE_LAYOUT_STORAGE_KEY } from "./layout/ide-layout";
import {
  KERNEL_PROTOCOL_V3,
  KERNEL_PROTOCOL_V0,
  SUPPORTED_KERNEL_PROTOCOLS,
  type KernelRequest,
  type ResponseFor,
} from "./protocol/kernel-v2";
import { MockKernelTransport } from "./transport/mock-kernel-transport";
import type { KernelConnectionLossListener } from "./transport/kernel-transport";
import { WebSocketKernelTransport } from "./transport/websocket-kernel-transport";
import { THEME_STORAGE_KEY } from "./theme";
import {
  WorkspaceClientError,
  type CreatableWorkspaceEntryKind,
  type CurrentDirectory,
  type DirectoryBrowserSnapshot,
  type WorkspaceClient,
  type WorkspaceConnectionLossListener,
  type WorkspaceChange,
  type WorkspaceEntry,
  workspaceDocumentUri,
  workspaceParentPath,
} from "./workspace/workspace-client";

const mockEditorState = vi.hoisted(() => ({
  props: null as CodeEditorProps | null,
}));

vi.mock("./components/CodeEditor", () => ({
  default: (props: CodeEditorProps) => {
    mockEditorState.props = props;
    const {
      value,
      theme,
      documentPath,
      documentUri,
      documentVersion,
      onChange,
      onSave,
    } = props;
    return (
      <textarea
        aria-label="Mock code editor"
        data-theme={theme}
        data-document-path={documentPath}
        data-document-uri={documentUri}
        data-document-version={documentVersion}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
            event.preventDefault();
            onSave();
          }
        }}
      />
    );
  },
}));

function editorProps(): CodeEditorProps {
  if (mockEditorState.props === null) throw new Error("CodeEditor is not mounted");
  return mockEditorState.props;
}

async function openTreeFile(name: string) {
  fireEvent.click(await screen.findByRole("treeitem", { name }));
  return screen.findByRole("textbox", { name: "Mock code editor" });
}

class MemoryDocumentSessionStore implements DocumentSessionStore {
  readonly sessions = new Map<string, PersistedDocumentSession>();

  async load(key: string): Promise<PersistedDocumentSession | null> {
    return this.sessions.get(key) ?? null;
  }

  async save(key: string, session: PersistedDocumentSession): Promise<void> {
    this.sessions.set(key, session);
  }

  async clear(key: string): Promise<void> {
    this.sessions.delete(key);
  }
}

class RecordingWorkspaceClient implements WorkspaceClient {
  readonly kind = "websocket" as const;
  readonly label = "Recording server workspace";
  readonly createCalls: Array<{
    readonly name: string;
    readonly kind: CreatableWorkspaceEntryKind;
  }> = [];
  readonly writeCalls: Array<{
    readonly path: string;
    readonly content: string;
    readonly expectedRevision: string;
    readonly rootGeneration: number;
  }> = [];
  readonly downloadCalls: Array<{
    readonly path: string;
    readonly rootGeneration: number;
  }> = [];
  readonly uploadCalls: Array<{
    readonly path: string;
    readonly name: string;
    readonly size: number;
    readonly rootGeneration: number;
    readonly overwrite: boolean;
  }> = [];
  readonly moveCalls: Array<{
    readonly path: string;
    readonly targetPath: string;
  }> = [];
  readonly deleteCalls: Array<{
    readonly path: string;
    readonly recursive: boolean;
  }> = [];
  readonly browseCalls: string[] = [];
  connectCount = 0;
  listCount = 0;
  readonly #entries = new Map<string, WorkspaceEntry>([
    [
      "seed.m",
      {
        name: "seed.m",
        path: "seed.m",
        kind: "file",
        size: 10,
        revision: "revision-seed",
      },
    ],
  ]);
  readonly #contents = new Map<string, string>([["seed.m", "seed = 1;\n"]]);
  readonly #failureName: string | null;
  readonly #failWrites: boolean;
  #connected = false;
  #directory = {
    path: "C:\\project-root",
    rootName: "project-root",
    generation: 1,
  };
  readonly #directoryListeners = new Set<(directory: CurrentDirectory) => void>();
  readonly #workspaceListeners = new Set<(change: WorkspaceChange) => void>();
  readonly #connectionLossListeners = new Set<WorkspaceConnectionLossListener>();

  constructor(failureName: string | null = null, failWrites = false) {
    this.#failureName = failureName;
    this.#failWrites = failWrites;
  }

  async connect(): Promise<void> {
    this.connectCount += 1;
    this.#connected = true;
  }

  async disconnect(): Promise<void> {
    this.#connected = false;
  }

  subscribeConnectionLoss(listener: WorkspaceConnectionLossListener) {
    this.#connectionLossListeners.add(listener);
    return () => this.#connectionLossListeners.delete(listener);
  }

  async currentDirectory() {
    return this.#directory;
  }

  async changeDirectory(path: string) {
    this.#directory = {
      path,
      rootName: path.split(/[\\/]/).filter(Boolean).at(-1) ?? "workspace",
      generation: this.#directory.generation + 1,
    };
    for (const listener of this.#directoryListeners) {
      listener(this.#directory);
    }
    return this.#directory;
  }

  async browseDirectories(path: string): Promise<DirectoryBrowserSnapshot> {
    this.browseCalls.push(path);
    return {
      path,
      parentPath:
        path === "C:\\project-root"
          ? "C:\\"
          : path === "C:\\project-root\\nested"
            ? "C:\\project-root"
            : null,
      roots: [
        { name: "C:", path: "C:\\" },
        { name: "D:", path: "D:\\" },
      ],
      entries:
        path === "C:\\project-root"
          ? [{ name: "nested", path: "C:\\project-root\\nested" }]
          : [],
    };
  }

  onCurrentDirectoryChanged(listener: (directory: CurrentDirectory) => void) {
    this.#directoryListeners.add(listener);
    return () => this.#directoryListeners.delete(listener);
  }

  onWorkspaceChanged(listener: (change: WorkspaceChange) => void) {
    this.#workspaceListeners.add(listener);
    return () => this.#workspaceListeners.delete(listener);
  }

  addExternalFile(path: string): void {
    this.#entries.set(path, {
      name: path.slice(path.lastIndexOf("/") + 1),
      path,
      kind: "file",
      size: 8,
      revision: null,
    });
  }

  replaceFileContent(path: string, content: string, revision: string): void {
    const entry = this.#entries.get(path);
    if (entry?.kind !== "file") {
      throw new Error(`${path} is not a file`);
    }
    this.#entries.set(path, { ...entry, size: content.length, revision });
    this.#contents.set(path, content);
  }

  emitWorkspaceChanged(): void {
    const change = { rootGeneration: this.#directory.generation };
    for (const listener of this.#workspaceListeners) {
      listener(change);
    }
  }

  loseConnection(message: string): void {
    this.#connected = false;
    const error = new Error(message);
    for (const listener of this.#connectionLossListeners) {
      listener(error);
    }
  }

  async list(path: string, recursive: boolean) {
    this.listCount += 1;
    if (!this.#connected) {
      throw new Error("workspace disconnected");
    }
    return {
      rootName: this.#directory.rootName,
      rootPath: this.#directory.path,
      rootGeneration: this.#directory.generation,
      path,
      recursive,
      entries: [...this.#entries.values()].filter((entry) =>
        recursive
          ? path.length === 0 || entry.path.startsWith(`${path}/`)
          : workspaceParentPath(entry.path) === path,
      ),
    };
  }

  async read(path: string) {
    const entry = this.#entries.get(path);
    if (entry?.kind !== "file" || entry.revision === null) {
      throw new WorkspaceClientError("workspace.notFound", `${path} missing`);
    }
    const content = this.#contents.get(path) ?? "";
    return {
      path,
      content,
      revision: entry.revision,
      size: content.length,
      rootPath: this.#directory.path,
      rootGeneration: this.#directory.generation,
    };
  }

  async prepareDownload(path: string, rootGeneration: number) {
    const entry = this.#entries.get(path);
    if (entry?.kind !== "file") {
      throw new WorkspaceClientError("workspace.notFound", `${path} missing`);
    }
    this.downloadCalls.push({ path, rootGeneration });
    return {
      url: `http://127.0.0.1:49152/workspace/download/${"a".repeat(64)}`,
      name: entry.name,
      size: entry.size ?? 0,
      expiresInSeconds: 60,
    };
  }

  async upload(
    path: string,
    file: File,
    rootGeneration: number,
    overwrite = false,
  ) {
    this.uploadCalls.push({
      path,
      name: file.name,
      size: file.size,
      rootGeneration,
      overwrite,
    });
    const entry: WorkspaceEntry = {
      name: file.name,
      path,
      kind: "file",
      size: file.size,
      revision: null,
    };
    this.#entries.set(path, entry);
    return entry;
  }

  async write(
    path: string,
    content: string,
    expectedRevision: string,
    rootGeneration: number,
  ) {
    this.writeCalls.push({ path, content, expectedRevision, rootGeneration });
    if (this.#failWrites && path === "seed.m") {
      throw new WorkspaceClientError(
        "workspace.revisionConflict",
        `${path} changed outside OpenMat. The editor content was not overwritten.`,
        "expectedRevision",
        { currentRevision: "external-revision" },
      );
    }
    const entry = this.#entries.get(path);
    if (entry?.revision !== expectedRevision) {
      throw new WorkspaceClientError(
        "workspace.revisionConflict",
        `${path} changed outside OpenMat.`,
      );
    }
    const saved = {
      ...entry,
      size: content.length,
      revision: `${expectedRevision}-saved`,
    };
    this.#entries.set(path, saved);
    this.#contents.set(path, content);
    return saved;
  }

  async create(path: string, kind: CreatableWorkspaceEntryKind) {
    this.createCalls.push({ name: path, kind });
    if (path === this.#failureName) {
      throw new WorkspaceClientError(
        "workspace.reservedWindowsName",
        "'" + path + "' is a reserved Windows device name.",
        "path",
      );
    }
    const entry: WorkspaceEntry = {
      name: path.slice(path.lastIndexOf("/") + 1),
      path,
      kind,
      size: kind === "file" ? 0 : null,
      revision: kind === "file" ? `revision-${path}` : null,
    };
    this.#entries.set(path, entry);
    if (kind === "file") {
      this.#contents.set(path, "");
    }
    return entry;
  }

  async rename(path: string, newName: string) {
    const separator = path.lastIndexOf("/");
    const targetPath = separator < 0 ? newName : `${path.slice(0, separator)}/${newName}`;
    return this.move(path, targetPath);
  }

  async move(path: string, targetPath: string) {
    this.moveCalls.push({ path, targetPath });
    const entry = this.#entries.get(path);
    if (entry === undefined) {
      throw new WorkspaceClientError("workspace.notFound", `${path} missing`);
    }
    this.#entries.delete(path);
    const moved = {
      ...entry,
      name: targetPath.slice(targetPath.lastIndexOf("/") + 1),
      path: targetPath,
    };
    this.#entries.set(targetPath, moved);
    const content = this.#contents.get(path);
    if (content !== undefined) {
      this.#contents.delete(path);
      this.#contents.set(targetPath, content);
    }
    return { previousPath: path, entry: moved };
  }

  async delete(path: string, recursive: boolean) {
    const entry = this.#entries.get(path);
    if (entry === undefined) {
      throw new WorkspaceClientError("workspace.notFound", `${path} missing`);
    }
    this.deleteCalls.push({ path, recursive });
    this.#entries.delete(path);
    this.#contents.delete(path);
    return { path, kind: entry.kind, recursive };
  }
}

class LazyRecordingWorkspaceClient extends RecordingWorkspaceClient {
  readonly listCalls: Array<{ readonly path: string; readonly recursive: boolean }> = [];

  override async list(path: string, recursive: boolean) {
    this.listCalls.push({ path, recursive });
    const directory: WorkspaceEntry = {
      name: "src",
      path: "src",
      kind: "directory",
      size: null,
      revision: null,
    };
    const file: WorkspaceEntry = {
      name: "lazy.m",
      path: "src/lazy.m",
      kind: "file",
      size: 10,
      revision: "revision-lazy",
    };
    return {
      rootName: "large-project",
      rootPath: "C:\\large-project",
      rootGeneration: 1,
      path,
      recursive,
      entries: path === "" ? [directory] : path === "src" ? [file] : [],
    };
  }
}

class DeferredBrowseWorkspaceClient extends RecordingWorkspaceClient {
  #resolveBrowse: ((snapshot: DirectoryBrowserSnapshot) => void) | null = null;

  override browseDirectories(path: string): Promise<DirectoryBrowserSnapshot> {
    this.browseCalls.push(path);
    return new Promise((resolve) => {
      this.#resolveBrowse = resolve;
    });
  }

  resolveBrowse(): void {
    this.#resolveBrowse?.({
      path: "C:\\project-root",
      parentPath: "C:\\",
      roots: [{ name: "C:", path: "C:\\" }],
      entries: [],
    });
    this.#resolveBrowse = null;
  }
}

class DisconnectableMockTransport extends MockKernelTransport {
  readonly #connectionLossListeners = new Set<KernelConnectionLossListener>();
  connectCount = 0;

  override async connect(sessionId: string): Promise<void> {
    this.connectCount += 1;
    await super.connect(sessionId);
  }

  override subscribeConnectionLoss(
    listener: KernelConnectionLossListener,
  ): () => void {
    this.#connectionLossListeners.add(listener);
    return () => this.#connectionLossListeners.delete(listener);
  }

  loseConnection(message: string): void {
    const error = new Error(message);
    for (const listener of this.#connectionLossListeners) {
      listener(error);
    }
  }
}

class RecordingMockTransport extends MockKernelTransport {
  readonly inspectRequests: Extract<
    KernelRequest,
    { readonly request: { readonly type: "inspect" } }
  >[] = [];
  readonly executeRequests: Extract<
    KernelRequest,
    { readonly request: { readonly type: "execute" } }
  >[] = [];
  readonly listWorkspaceRequests: Extract<
    KernelRequest,
    { readonly request: { readonly type: "listWorkspace" } }
  >[] = [];
  readonly setVariableElementRequests: Extract<
    KernelRequest,
    { readonly request: { readonly type: "setVariableElement" } }
  >[] = [];

  override async request<TRequest extends KernelRequest>(
    request: TRequest,
  ): Promise<ResponseFor<TRequest>> {
    if (request.request.type === "inspect") {
      this.inspectRequests.push(
        request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "inspect" } }
        >,
      );
    }
    if (request.request.type === "execute") {
      this.executeRequests.push(
        request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "execute" } }
        >,
      );
    }
    if (request.request.type === "listWorkspace") {
      this.listWorkspaceRequests.push(
        request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "listWorkspace" } }
        >,
      );
    }
    if (request.request.type === "setVariableElement") {
      this.setVariableElementRequests.push(
        request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "setVariableElement" } }
        >,
      );
    }
    return super.request(request);
  }
}

const aggregatePreview = {
  class: "cell",
  dimensions: [1, 2],
  complex: false,
  selectedRange: { start: [1, 1], size: [1, 2] },
  kind: "cell",
  items: [
    {
      class: "struct",
      size: [1, 1],
      ndims: 2,
      numel: 1,
      complex: false,
      kind: "struct",
      fields: ["zeta", "alpha", "glyph"],
      records: [
        {
          zeta: {
            class: "uint64",
            size: [1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            kind: "integer",
            integer: [
              { real: "18446744073709551615", imaginary: "0" },
            ],
          },
          alpha: {
            class: "string",
            size: [1, 2],
            ndims: 2,
            numel: 2,
            complex: false,
            kind: "string",
            string_code_units: [[], []],
            missing: [true, false],
          },
          glyph: {
            class: "char",
            size: [1, 1],
            ndims: 2,
            numel: 1,
            complex: false,
            kind: "char",
            code_units: [55_357],
          },
        },
      ],
    },
    {
      class: "struct",
      size: [0, 3],
      ndims: 2,
      numel: 0,
      complex: false,
      kind: "struct",
      fields: ["beta", "alpha"],
      records: [],
    },
  ],
  truncation: { truncated: false, omittedElements: 0 },
  usage: { nodes: 6, elements: 7, codeUnits: 24, depth: 2 },
} as const;

class V3AggregateMockTransport extends MockKernelTransport {
  readonly requests: KernelRequest[] = [];
  readonly #deferInspect: boolean;
  readonly #pendingInspect: Array<() => void> = [];
  #responseSequence = 0;

  constructor(deferInspect = false) {
    super({ eventDelayMs: 0 });
    this.#deferInspect = deferInspect;
  }

  override async request<TRequest extends KernelRequest>(
    request: TRequest,
  ): Promise<ResponseFor<TRequest>> {
    this.requests.push(request);
    if (request.request.type === "listWorkspace") {
      return this.#success(request, {
        revision: 0,
        variables: [
          {
            name: "mixed",
            class: "cell",
            dimensions: [1, 2],
            complex: false,
            bytes: 256,
          },
        ],
      });
    }
    if (request.request.type === "inspect") {
      const response = this.#success(request, {
        revision: 0,
        preview: aggregatePreview,
      });
      if (this.#deferInspect) {
        return new Promise((resolve) => {
          this.#pendingInspect.push(() => resolve(response));
        });
      }
      return response;
    }
    return super.request(request);
  }

  resolveNextInspect(): void {
    this.#pendingInspect.shift()?.();
  }

  #success<TRequest extends KernelRequest>(
    request: TRequest,
    data: unknown,
  ): ResponseFor<TRequest> {
    this.#responseSequence += 1;
    return {
      protocol: request.protocol,
      sessionId: request.sessionId,
      messageId: `aggregate-response-${this.#responseSequence.toString()}`,
      kind: "response",
      replyTo: request.messageId,
      ok: true,
      result: { type: request.request.type, data },
    } as unknown as ResponseFor<TRequest>;
  }
}

describe("App vertical slice", () => {
  beforeEach(() => {
    localStorage.clear();
    mockEditorState.props = null;
  });

  it("negotiates v3 and carries an aggregate inspect through to the read-only editor", async () => {
    const transport = new V3AggregateMockTransport();
    render(<App transport={transport} />);

    const workspace = await screen.findByRole("grid", {
      name: "Kernel workspace summaries",
    });
    const mixedRow = await within(workspace).findByRole("row", {
      name: /mixed cell 1 × 2/,
    });
    expect(screen.getByText("kernel-v3")).toBeVisible();
    const initialize = transport.requests.find(
      (request) => request.request.type === "initialize",
    );
    expect(initialize).toMatchObject({
      protocol: KERNEL_PROTOCOL_V0,
      request: {
        type: "initialize",
        params: {
          supportedProtocols: SUPPORTED_KERNEL_PROTOCOLS,
          capabilities: {
            displayMimeTypes: expect.arrayContaining([
              "application/vnd.openmat.figure+json",
              "application/vnd.openmat.plot+json",
              "application/vnd.openmat.command-window-clear+json",
            ]),
          },
        },
      },
    });

    fireEvent.doubleClick(mixedRow);
    const editor = await screen.findByRole("dialog", {
      name: "Variable Editor: mixed",
    });
    expect(
      await within(editor).findByText("Field order: zeta, alpha, glyph"),
    ).toBeVisible();
    expect(within(editor).getByText("18446744073709551615")).toBeVisible();
    expect(within(editor).getByText("<missing string>")).toBeVisible();
    expect(
      within(editor).getByText("Shaped empty struct 0 × 3 · schema retained"),
    ).toBeVisible();
    expect(editor.textContent).not.toContain("[object Object]");
    const inspect = transport.requests.find(
      (request) => request.request.type === "inspect",
    );
    expect(inspect).toMatchObject({
      protocol: KERNEL_PROTOCOL_V3,
      request: {
        params: {
          name: "mixed",
          range: { start: [1, 1], size: [1, 2] },
          maxElements: 256,
        },
      },
    });
  });

  it("ignores a late aggregate response after the editor is closed", async () => {
    const transport = new V3AggregateMockTransport(true);
    render(<App transport={transport} />);
    const workspace = await screen.findByRole("grid", {
      name: "Kernel workspace summaries",
    });
    fireEvent.doubleClick(
      await within(workspace).findByRole("row", { name: /mixed cell 1 × 2/ }),
    );
    const editor = await screen.findByRole("dialog", {
      name: "Variable Editor: mixed",
    });
    fireEvent.click(
      within(editor).getByRole("button", {
        name: "Close Variable Editor for mixed",
      }),
    );

    act(() => transport.resolveNextInspect());
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Variable Editor: mixed" }),
      ).toBeNull(),
    );
  });

  it("selects workspace summaries and opens inspect results in a Variable Editor", async () => {
    const transport = new RecordingMockTransport({ eventDelayMs: 0 });
    render(<App transport={transport} />);

    expect(await screen.findByText("In-memory demo workspace")).toBeVisible();
    const editor = await openTreeFile("welcome.m");
    expect((editor as HTMLTextAreaElement).value).toContain("while x < 3");

    const runButton = await screen.findByRole("button", { name: /^run$/i });
    await waitFor(() => expect(runButton).toBeEnabled());
    fireEvent.click(runButton);

    expect(await screen.findByText("30")).toBeVisible();
    await waitFor(() => expect(runButton).toBeEnabled());
    expect(transport.listWorkspaceRequests).toHaveLength(1);

    const workspace = await screen.findByRole("grid", {
      name: "Kernel workspace summaries",
    });
    const xRow = within(workspace).getByRole("row", { name: /x double 1 × 1/ });
    fireEvent.click(xRow);
    expect(xRow).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByRole("dialog", { name: /Variable Editor/ })).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: "Open x in Variable Editor" }),
    );
    const xEditor = await screen.findByRole("dialog", {
      name: "Variable Editor: x",
    });
    expect(
      await within(xEditor).findByRole("cell", { name: "3" }),
    ).toBeVisible();
    expect(
      within(xEditor).getByText("Loaded 1 of 1 elements · complete"),
    ).toBeVisible();
    expect(transport.inspectRequests[0]?.request.params).toEqual({
      name: "x",
      range: { start: [1, 1], size: [1, 1] },
      maxElements: 256,
    });

    const yRow = within(workspace).getByRole("row", { name: /y double 1 × 1/ });
    fireEvent.doubleClick(yRow);
    const yEditor = await screen.findByRole("dialog", {
      name: "Variable Editor: y",
    });
    expect(
      await within(yEditor).findByRole("cell", { name: "30" }),
    ).toBeVisible();
    expect(xEditor).toBeVisible();

    fireEvent.doubleClick(within(yEditor).getByRole("cell", { name: "30" }));
    const cellInput = within(yEditor).getByRole("textbox", {
      name: "Edit y(1, 1)",
    });
    fireEvent.change(cellInput, { target: { value: "41" } });
    fireEvent.keyDown(cellInput, { key: "Enter" });
    expect(
      await within(yEditor).findByRole("cell", { name: "41" }),
    ).toBeVisible();
    expect(transport.setVariableElementRequests).toHaveLength(1);
    expect(
      transport.setVariableElementRequests[0]?.request.params,
    ).toMatchObject({
      name: "y",
      indices: [1, 1],
      value: { real: "41", imaginary: "0" },
      expectedRevision: 1,
    });

    fireEvent.click(
      within(xEditor).getByRole("button", {
        name: "Close Variable Editor for x",
      }),
    );
    await waitFor(() => expect(xEditor).not.toBeInTheDocument());
    expect(yEditor).toBeVisible();
  });

  it("executes REPL commands and records command history", async () => {
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} />);

    const input = await screen.findByRole("textbox", {
      name: "Command Window input",
    });
    await waitFor(() => expect(input).toBeEnabled());
    fireEvent.change(input, { target: { value: "answer = 6 * 7" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(await screen.findByText(">> answer = 6 * 7")).toBeVisible();
    expect(
      await screen.findByRole("button", { name: "answer = 6 * 7" }),
    ).toBeVisible();
    expect(await screen.findByText("30")).toBeVisible();
  });

  it("rejects a kernel that cannot negotiate the required v3 protocol", async () => {
    render(
      <App
        transport={
          new MockKernelTransport({
            eventDelayMs: 0,
            supportedProtocols: [KERNEL_PROTOCOL_V0],
          })
        }
      />,
    );

    await openTreeFile("welcome.m");
    const runButton = await screen.findByRole("button", { name: /^run$/i });
    expect(runButton).toBeDisabled();
    expect(await screen.findByText(/no common protocol/i)).toBeVisible();
  });

  it("selects WebSocket only when a URL is configured", () => {
    expect(createDefaultKernelTransport(undefined)).toBeInstanceOf(
      MockKernelTransport,
    );
    expect(
      createDefaultKernelTransport("ws://127.0.0.1:8765/kernel"),
    ).toBeInstanceOf(WebSocketKernelTransport);
  });

  it("creates real workspace-client files and folders, refreshes, and selects each new item", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );

    const newFile = screen.getByRole("button", { name: "New File" });
    await waitFor(() => expect(newFile).toBeEnabled());
    expect(newFile).toHaveAttribute("title", "New File");
    expect(newFile.textContent).toBe("");
    fireEvent.click(newFile);
    const fileName = screen.getByRole("textbox", { name: "Name" });
    expect(fileName).toHaveFocus();
    fireEvent.change(fileName, { target: { value: "analysis.m" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    const createdFile = await screen.findByRole("treeitem", {
      name: "analysis.m",
    });
    expect(createdFile).toHaveAttribute("aria-selected", "true");
    expect(workspaceClient.createCalls[0]).toEqual({
      name: "analysis.m",
      kind: "file",
    });

    const newFolder = screen.getByRole("button", { name: "New Folder" });
    expect(newFolder).toHaveAttribute("title", "New Folder");
    expect(newFolder.textContent).toBe("");
    fireEvent.click(newFolder);
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "results" },
    });
    fireEvent.submit(screen.getByRole("textbox", { name: "Name" }).closest("form")!);

    const createdFolder = await screen.findByRole("treeitem", { name: "results" });
    expect(createdFolder).toHaveAttribute("aria-selected", "true");
    expect(workspaceClient.createCalls[1]).toEqual({
      name: "results",
      kind: "directory",
    });
    expect(screen.getByText("3 items")).toBeVisible();
  });

  it("refreshes from an icon action and from debounced server filesystem events", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );

    const refresh = screen.getByRole("button", { name: "Refresh Current Folder" });
    await waitFor(() => expect(refresh).toBeEnabled());
    expect(refresh).toHaveAttribute("title", "Refresh Current Folder");
    expect(refresh.textContent).toBe("");

    workspaceClient.addExternalFile("manual.mat");
    fireEvent.click(refresh);
    expect(await screen.findByRole("treeitem", { name: "manual.mat" })).toBeVisible();

    const listsBeforeEvent = workspaceClient.listCount;
    workspaceClient.addExternalFile("save1.mat");
    act(() => workspaceClient.emitWorkspaceChanged());
    expect(await screen.findByRole("treeitem", { name: "save1.mat" })).toBeVisible();
    expect(workspaceClient.listCount).toBeGreaterThan(listsBeforeEvent);
  });

  it("loads the root and expanded directories separately for large workspaces", async () => {
    const workspaceClient = new LazyRecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );

    const directory = await screen.findByRole("treeitem", { name: "src" });
    expect(screen.queryByRole("treeitem", { name: "lazy.m" })).toBeNull();
    expect(workspaceClient.listCalls).toEqual([{ path: "", recursive: false }]);

    fireEvent.click(directory);
    expect(await screen.findByRole("treeitem", { name: "lazy.m" })).toBeVisible();
    expect(workspaceClient.listCalls).toEqual([
      { path: "", recursive: false },
      { path: "src", recursive: false },
    ]);
  });

  it("prepares a ticket and starts an HTTP download from the file context menu", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    const started: Array<{ readonly url: string; readonly name: string }> = [];
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(function (this: HTMLAnchorElement) {
        started.push({ url: this.href, name: this.download });
      });
    try {
      render(
        <App
          transport={new MockKernelTransport({ eventDelayMs: 0 })}
          workspaceClient={workspaceClient}
        />,
      );
      const file = await screen.findByRole("treeitem", { name: "seed.m" });
      fireEvent.contextMenu(file);
      fireEvent.click(screen.getByRole("menuitem", { name: "Download" }));

      await waitFor(() => expect(workspaceClient.downloadCalls).toEqual([
        { path: "seed.m", rootGeneration: 1 },
      ]));
      expect(started).toEqual([
        {
          url: `http://127.0.0.1:49152/workspace/download/${"a".repeat(64)}`,
          name: "seed.m",
        },
      ]);
    } finally {
      click.mockRestore();
    }
  });

  it("uploads selected files into Current Folder and refreshes the tree", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    const { container } = render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    await screen.findByRole("treeitem", { name: "seed.m" });
    const file = new File([new Uint8Array([0, 1, 2, 255])], "uploaded.mat");
    fireEvent.change(container.querySelector('input[type="file"]')!, {
      target: { files: [file] },
    });

    await waitFor(() =>
      expect(workspaceClient.uploadCalls).toEqual([
        {
          path: "uploaded.mat",
          name: "uploaded.mat",
          size: 4,
          rootGeneration: 1,
          overwrite: false,
        },
      ]),
    );
    expect(await screen.findByRole("treeitem", { name: "uploaded.mat" })).toBeVisible();
  });

  it("copies a file path relative to Current Folder", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    try {
      render(
        <App
          transport={new MockKernelTransport({ eventDelayMs: 0 })}
          workspaceClient={workspaceClient}
        />,
      );
      const file = await screen.findByRole("treeitem", { name: "seed.m" });
      fireEvent.contextMenu(file);
      fireEvent.click(screen.getByRole("menuitem", { name: "Copy Relative Path" }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith("seed.m"));
    } finally {
      if (previousClipboard === undefined) {
        Reflect.deleteProperty(navigator, "clipboard");
      } else {
        Object.defineProperty(navigator, "clipboard", previousClipboard);
      }
    }
  });

  it("renames with immediate validation in a managed dialog", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const file = await screen.findByRole("treeitem", { name: "seed.m" });
    fireEvent.contextMenu(file);
    fireEvent.click(screen.getByRole("menuitem", { name: "Rename…" }));
    const dialog = await screen.findByRole("dialog", { name: "Rename file" });
    const input = within(dialog).getByRole("textbox", { name: "New name" });
    fireEvent.change(input, { target: { value: "CON" } });
    expect(within(dialog).getByRole("button", { name: "Rename" })).toBeDisabled();
    expect(within(dialog).getByRole("alert")).toHaveTextContent("reserved");
    fireEvent.change(input, { target: { value: "renamed.m" } });
    fireEvent.submit(input.closest("form")!);
    expect(await screen.findByRole("treeitem", { name: "renamed.m" })).toBeVisible();
    expect(workspaceClient.moveCalls.at(-1)).toEqual({
      path: "seed.m",
      targetPath: "renamed.m",
    });
    expect(screen.queryByRole("dialog", { name: "Rename file" })).toBeNull();
  });

  it("preflights and permanently deletes through a managed danger dialog", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const file = await screen.findByRole("treeitem", { name: "seed.m" });
    fireEvent.contextMenu(file);
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Permanently delete file?",
    });
    expect(within(dialog).getByText("seed.m")).toBeVisible();
    expect(await within(dialog).findByText("1")).toBeVisible();
    fireEvent.click(
      within(dialog).getByRole("button", { name: "Permanently Delete" }),
    );
    await waitFor(() => expect(workspaceClient.deleteCalls).toEqual([
      { path: "seed.m", recursive: false },
    ]));
    expect(screen.queryByRole("treeitem", { name: "seed.m" })).toBeNull();
  });

  it("confirms clearing every kernel variable in the managed modal layer", async () => {
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={new RecordingWorkspaceClient()}
      />,
    );
    const command = await screen.findByRole("textbox", {
      name: "Command Window input",
    });
    await waitFor(() => expect(command).toBeEnabled());
    fireEvent.change(command, { target: { value: "x = 1" } });
    fireEvent.keyDown(command, { key: "Enter" });
    const clearAll = screen.getByRole("button", { name: "Clear all" });
    await waitFor(() => expect(clearAll).toBeEnabled());
    fireEvent.click(clearAll);
    const dialog = await screen.findByRole("dialog", {
      name: "Clear the entire workspace?",
    });
    expect(dialog).toHaveTextContent("2 variables will be removed");
    fireEvent.click(within(dialog).getByRole("button", { name: "Clear Workspace" }));
    await waitFor(() => expect(clearAll).toBeDisabled());
  });

  it("opens, edits, saves, and runs the active workspace document with its relative sourceName", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    const transport = new RecordingMockTransport({ eventDelayMs: 0 });
    render(<App transport={transport} workspaceClient={workspaceClient} />);

    const editor = await openTreeFile("seed.m");
    expect(editor).toHaveAttribute("data-document-path", "seed.m");
    expect(editor).toHaveAttribute(
      "data-document-uri",
      workspaceDocumentUri("seed.m", 1, "C:\\project-root"),
    );
    fireEvent.change(editor, { target: { value: "answer = 6 * 7;\n" } });
    expect(screen.getByLabelText("Unsaved changes")).toBeVisible();
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    await waitFor(() => expect(workspaceClient.writeCalls).toHaveLength(1));
    expect(workspaceClient.writeCalls[0]).toMatchObject({
      path: "seed.m",
      content: "answer = 6 * 7;\n",
      expectedRevision: "revision-seed",
      rootGeneration: 1,
    });
    await waitFor(() =>
      expect(screen.queryByLabelText("Unsaved changes")).toBeNull(),
    );

    fireEvent.change(editor, { target: { value: "current_value = 9;\n" } });
    const runButton = screen.getByRole("button", { name: /^run$/i });
    await waitFor(() => expect(runButton).toBeEnabled());
    fireEvent.click(runButton);
    await waitFor(() => expect(transport.executeRequests.length).toBeGreaterThan(0));
    expect(transport.executeRequests.at(-1)?.request.params).toMatchObject({
      code: "current_value = 9;\n",
      sourceName: "seed.m",
      mode: "cell",
    });
  });

  it("browses folders in-app, cancels without side effects, and changes only on confirmation", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const address = await screen.findByRole("textbox", {
      name: "Current Folder path",
    });
    await waitFor(() => expect(address).toHaveValue("C:\\project-root"));

    fireEvent.click(screen.getByRole("button", { name: "Choose Current Folder" }));
    expect(await screen.findByRole("dialog", { name: "Choose Current Folder" })).toBeVisible();
    expect(await screen.findByRole("button", { name: "Open folder nested" })).toBeVisible();
    expect(address).toHaveValue("C:\\project-root");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Choose Current Folder" })).toBeNull(),
    );
    expect(address).toHaveValue("C:\\project-root");

    fireEvent.click(screen.getByRole("button", { name: "Choose Current Folder" }));
    fireEvent.click(await screen.findByRole("button", { name: "Open folder nested" }));
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Folder chooser path" })).toHaveValue(
        "C:\\project-root\\nested",
      ),
    );
    expect(address).toHaveValue("C:\\project-root");
    fireEvent.click(screen.getByRole("button", { name: "Select Folder" }));
    await waitFor(() => expect(address).toHaveValue("C:\\project-root\\nested"));
    expect(screen.queryByRole("dialog", { name: "Choose Current Folder" })).toBeNull();
    expect(workspaceClient.browseCalls).toEqual([
      "C:\\project-root",
      "C:\\project-root",
      "C:\\project-root\\nested",
    ]);
  });

  it("can cancel immediately while a directory browse request is still pending", async () => {
    const workspaceClient = new DeferredBrowseWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    await screen.findByRole("textbox", { name: "Current Folder path" });
    fireEvent.click(screen.getByRole("button", { name: "Choose Current Folder" }));
    expect(await screen.findByText("Loading…")).toHaveAttribute("role", "status");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Choose Current Folder" })).toBeNull();

    await act(async () => workspaceClient.resolveBrowse());
    expect(screen.queryByRole("dialog", { name: "Choose Current Folder" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Current Folder path" })).toHaveValue(
      "C:\\project-root",
    );
  });

  it("switches Current Folder without closing an open document and saves to its original root", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    const transport = new RecordingMockTransport({ eventDelayMs: 0 });
    render(
      <App
        transport={transport}
        workspaceClient={workspaceClient}
      />,
    );
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "kept = 42;\n" } });
    const address = screen.getByRole("textbox", { name: "Current Folder path" });
    fireEvent.change(address, { target: { value: "D:\\other-project" } });
    fireEvent.submit(address.closest("form")!);
    await waitFor(() => expect(address).toHaveValue("D:\\other-project"));
    expect(editor).toHaveValue("kept = 42;\n");

    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));
    await waitFor(() => expect(transport.executeRequests).toHaveLength(1));
    expect(transport.executeRequests.at(0)?.request.params.sourceName).toBe(
      "C:\\project-root\\seed.m",
    );

    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    await waitFor(() => expect(workspaceClient.writeCalls).toHaveLength(1));
    expect(workspaceClient.writeCalls[0]).toMatchObject({
      path: "seed.m",
      content: "kept = 42;\n",
      rootGeneration: 1,
    });
  });

  it("updates the Current Folder address when the shared session changes externally", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const address = await screen.findByRole("textbox", {
      name: "Current Folder path",
    });
    await waitFor(() => expect(address).toHaveValue("C:\\project-root"));
    await act(async () => {
      await workspaceClient.changeDirectory("E:\\changed-by-cd");
    });
    await waitFor(() => expect(address).toHaveValue("E:\\changed-by-cd"));
  });

  it("treats equal relative paths in different Current Folders as different documents", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const editor = await openTreeFile("seed.m");
    expect(editor).toHaveAttribute(
      "data-document-uri",
      workspaceDocumentUri("seed.m", 1, "C:\\project-root"),
    );
    const address = screen.getByRole("textbox", { name: "Current Folder path" });
    fireEvent.change(address, { target: { value: "D:\\second-root" } });
    fireEvent.submit(address.closest("form")!);
    await waitFor(() => expect(address).toHaveValue("D:\\second-root"));
    fireEvent.click(await screen.findByRole("treeitem", { name: "seed.m" }));
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Mock code editor" })).toHaveAttribute(
        "data-document-uri",
        workspaceDocumentUri("seed.m", 2, "D:\\second-root"),
      ),
    );
  });

  it("keeps all open documents in the workspace LSP across tab switches", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
        wsUrl="ws://127.0.0.1:9080/kernel"
      />,
    );
    await openTreeFile("seed.m");
    expect(editorProps().lspUrl).toBe("ws://127.0.0.1:9080/lsp");
    fireEvent.click(screen.getByRole("treeitem", { name: "helper.m" }));
    await waitFor(() => expect(editorProps().documentPath).toBe("helper.m"));
    const documentIds = editorProps().workspaceDocuments!.map((document) => document.id);
    expect(editorProps().workspaceDocuments).toEqual([
      expect.objectContaining({ uri: workspaceDocumentUri("seed.m", 1, "C:\\project-root"), content: "seed = 1;\n" }),
      expect.objectContaining({ uri: workspaceDocumentUri("helper.m", 1, "C:\\project-root"), content: "" }),
    ]);
    fireEvent.click(screen.getByRole("tab", { name: /seed\.m/ }));
    expect(editorProps().documentPath).toBe("seed.m");
    expect(editorProps().workspaceDocuments!.map((document) => document.id)).toEqual(documentIds);
    fireEvent.click(screen.getByRole("button", { name: "Close helper.m" }));
    expect(editorProps().workspaceDocuments).toHaveLength(1);
  });

  it("routes workspace edits to the inactive document and saves its own file", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    await openTreeFile("seed.m");
    const seedId = editorProps().documentId;
    fireEvent.click(screen.getByRole("treeitem", { name: "helper.m" }));
    await waitFor(() => expect(editorProps().documentPath).toBe("helper.m"));
    const helperId = editorProps().documentId;
    act(() => {
      editorProps().onWorkspaceDocumentChange!(seedId, "renamed = 1;\n");
      editorProps().onWorkspaceDocumentChange!(helperId, "renamed + 1;\n");
      editorProps().onWorkspaceDocumentChange!(seedId, "renamed = 1;\n");
    });
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("renamed + 1;\n");
    expect(editorProps().workspaceDocuments).toEqual([
      expect.objectContaining({ id: seedId, content: "renamed = 1;\n", version: 2 }),
      expect.objectContaining({ id: helperId, content: "renamed + 1;\n", version: 2 }),
    ]);
    fireEvent.click(screen.getByRole("tab", { name: /seed\.m/ }));
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("renamed = 1;\n");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(workspaceClient.writeCalls).toEqual([{
      path: "seed.m",
      content: "renamed = 1;\n",
      expectedRevision: "revision-seed",
      rootGeneration: 1,
    }]));
    expect(editorProps().workspaceDocuments![1]).toEqual(
      expect.objectContaining({ content: "renamed + 1;\n", savedContent: "" }),
    );
  });

  it("navigates to an open URI and selection without rereading its unsaved draft", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper 中文.m", "file");
    const read = vi.spyOn(workspaceClient, "read");
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const editor = await openTreeFile("helper 中文.m");
    fireEvent.change(editor, { target: { value: "function answer = helper()\nanswer = 42;\nend\n" } });
    fireEvent.click(screen.getByRole("treeitem", { name: "seed.m" }));
    await waitFor(() => expect(editorProps().documentPath).toBe("seed.m"));
    read.mockClear();
    act(() => expect(editorProps().onOpenDocument!(
      URI.parse(workspaceDocumentUri("helper 中文.m", 1, "C:\\project-root")).toString(),
      { startLineNumber: 2, startColumn: 1, endLineNumber: 2, endColumn: 7 },
    )).toBe(true));
    expect(editorProps().documentPath).toBe("helper 中文.m");
    expect(editorProps().value).toContain("answer = 42;");
    expect(editorProps().reveal).toEqual({
      documentUri: editorProps().documentUri,
      lineNumber: 2,
      column: 1,
      endLineNumber: 2,
      endColumn: 7,
      requestId: 1,
    });
    expect(screen.getByRole("tab", { name: /helper 中文\.m/ })).toHaveAttribute("aria-selected", "true");
    act(() => expect(editorProps().onOpenDocument!(workspaceDocumentUri("missing.m", 1, "D:\\other-root"))).toBe(false));
    expect(read).not.toHaveBeenCalled();
    expect(editorProps().documentPath).toBe("helper 中文.m");
    fireEvent.click(screen.getByRole("tab", { name: /seed\.m/ }));
    expect(editorProps().reveal).toBeNull();
  });

  it("opens an unopened language target and reveals its definition without replacing the caller draft", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper 中文.m", "file");
    workspaceClient.replaceFileContent("helper 中文.m", "function y = helper(x)\ny = x;\nend\n", "helper-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "answer = helper(2);\n" } });
    const read = vi.spyOn(workspaceClient, "read");
    await act(async () => {
      expect(await editorProps().onOpenDocument!(
        URI.parse(workspaceDocumentUri("helper 中文.m", 1, "C:\\project-root")).toString(),
        { startLineNumber: 1, startColumn: 14, endLineNumber: 1, endColumn: 20 },
      )).toBe(true);
    });
    expect(read).toHaveBeenCalledExactlyOnceWith("helper 中文.m");
    expect(editorProps().documentPath).toBe("helper 中文.m");
    expect(editorProps().reveal).toMatchObject({ lineNumber: 1, column: 14, endColumn: 20 });
    expect(editorProps().workspaceDocuments![0]).toMatchObject({ content: "answer = helper(2);\n", savedContent: "seed = 1;\n" });
    expect(workspaceClient.writeCalls).toHaveLength(0);
  });

  it("shares one source draft and language session between Designer and the workbench", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("SignalApp.m", "file");
    workspaceClient.replaceFileContent("SignalApp.m", SIGNAL_CODE, "signal-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    const editor = await openTreeFile("SignalApp.m");
    const session = editorProps().editorSession;
    const uri = editorProps().documentUri;
    fireEvent.change(editor, { target: { value: `${SIGNAL_CODE}\n% workbench edit\n` } });
    fireEvent.click(screen.getByRole("button", { name: "App Designer" }));
    const designer = await screen.findByRole("region", { name: "App Designer" });
    fireEvent.click(within(designer).getByRole("button", { name: "代码" }));
    const designerEditor = await within(designer).findByRole("textbox", { name: "Mock code editor" });
    expect(designerEditor).toHaveValue(`${SIGNAL_CODE}\n% workbench edit\n`);
    expect(editorProps().editorSession).toBe(session);
    expect(designerEditor).toHaveAttribute("data-document-uri", uri);
    const edited = `${SIGNAL_CODE}\n% Designer edit\n`;
    fireEvent.change(designerEditor, { target: { value: edited } });
    fireEvent.click(within(designer).getByRole("button", { name: "返回工作台" }));
    expect(screen.getByRole("textbox", { name: "Mock code editor" })).toHaveValue(edited);
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Mock code editor" }), { key: "s", ctrlKey: true });
    await waitFor(() => expect(workspaceClient.writeCalls.at(-1)).toMatchObject({
      path: "SignalApp.m", content: edited, expectedRevision: "signal-revision",
    }));
    fireEvent.click(screen.getByRole("button", { name: "App Designer" }));
    expect(await within(designer).findByRole("textbox", { name: "Mock code editor" })).toHaveValue(edited);
  });

  it("creates and saves a new Designer source from its shared workbench tab", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("seed.m");
    fireEvent.click(screen.getByRole("button", { name: "App Designer" }));
    const designer = await screen.findByRole("region", { name: "App Designer" });
    await waitFor(() => expect(screen.getByRole("tab", { name: /SignalApp\.m/, hidden: true })).toBeInTheDocument());
    fireEvent.click(within(designer).getByRole("button", { name: "返回工作台" }));
    fireEvent.click(screen.getByRole("tab", { name: /SignalApp\.m/ }));
    const editor = screen.getByRole("textbox", { name: "Mock code editor" });
    expect(editor).toHaveValue(SIGNAL_CODE);
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    await waitFor(() => expect(workspaceClient.writeCalls.at(-1)).toMatchObject({ path: "SignalApp.m", content: SIGNAL_CODE }));
    expect(workspaceClient.createCalls.filter((call) => call.name === "SignalApp.m")).toHaveLength(1);
    expect(screen.getByRole("tab", { name: "SignalApp.m" })).not.toHaveTextContent("●");
  });

  it("does not reopen a closed source from a hidden Designer's stale fallback", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("SignalApp.m", "file");
    const content = `${SIGNAL_CODE}\n% actual file\n`;
    workspaceClient.replaceFileContent("SignalApp.m", content, "signal-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("SignalApp.m");
    fireEvent.click(screen.getByRole("button", { name: "App Designer" }));
    const designer = await screen.findByRole("region", { name: "App Designer" });
    fireEvent.click(within(designer).getByRole("button", { name: "代码" }));
    expect(await within(designer).findByRole("textbox", { name: "Mock code editor" })).toHaveValue(content);
    fireEvent.click(within(designer).getByRole("button", { name: "返回工作台" }));
    fireEvent.click(screen.getByRole("button", { name: "Close SignalApp.m" }));
    await openTreeFile("seed.m");
    expect(screen.queryByRole("tab", { name: /SignalApp\.m/ })).toBeNull();
    const updated = `${SIGNAL_CODE}\n% changed while closed\n`;
    workspaceClient.replaceFileContent("SignalApp.m", updated, "signal-updated");
    fireEvent.click(screen.getByRole("button", { name: "App Designer" }));
    await waitFor(() => expect(within(designer).getByRole("textbox", { name: "Mock code editor" })).toHaveValue(updated));
  });

  it("loads all rename targets together and synchronizes them before completing preparation", async () => {
    const configured = vi.spyOn(SharedEditorSession.prototype, "configure");
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    await workspaceClient.create("consumer.m", "file");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("seed.m");
    const session = editorProps().editorSession!;
    const sync = vi.spyOn(session, "syncDocuments");
    const ensureDocuments = configured.mock.calls.at(-1)![0].ensureDocuments!;
    await act(async () => {
      expect(await ensureDocuments(["helper.m", "consumer.m"].map(
        (path) => workspaceDocumentUri(path, 1, "C:\\project-root"),
      ))).toBe(true);
      expect(sync).toHaveBeenCalledWith(expect.arrayContaining([
        expect.objectContaining({ uri: workspaceDocumentUri("helper.m", 1, "C:\\project-root") }),
        expect.objectContaining({ uri: workspaceDocumentUri("consumer.m", 1, "C:\\project-root") }),
      ]));
    });
    expect(editorProps().workspaceDocuments).toHaveLength(3);
    expect(editorProps().documentPath).toBe("seed.m");
    expect(workspaceClient.writeCalls).toHaveLength(0);
    configured.mockRestore();
  });

  it("stages a public function file rename only after its text edit and performs it on Save", async () => {
    const configured = vi.spyOn(SharedEditorSession.prototype, "configure");
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    const originalText = "function y = helper(x)\ny = x + 1;\nend\n";
    const renamedText = originalText.replace("helper", "calculate");
    workspaceClient.replaceFileContent("helper.m", originalText, "helper-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("helper.m");
    const id = editorProps().documentId;
    const stage = configured.mock.calls.at(-1)![0].stageFileRenames!;
    expect(stage([{ oldUri: editorProps().documentUri,
      newUri: workspaceDocumentUri("calculate.m", 1, "C:\\project-root"), originalText, renamedText,
    }])).toBe(true);
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    expect(workspaceClient.moveCalls).toHaveLength(0);
    act(() => editorProps().onWorkspaceDocumentChange!(id, renamedText));
    expect(editorProps().workspaceDocuments![0]).toMatchObject({ pendingPath: "calculate.m" });
    expect(screen.getByText(/Saving will rename/)).toHaveTextContent("calculate.m");
    // Undo before saving cancels the corresponding staged filesystem operation.
    act(() => editorProps().onWorkspaceDocumentChange!(id, originalText));
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    act(() => editorProps().onWorkspaceDocumentChange!(id, renamedText));
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Mock code editor" }), { key: "s", ctrlKey: true });
    await waitFor(() => expect(editorProps().documentPath).toBe("calculate.m"));
    expect(workspaceClient.writeCalls.at(-1)).toMatchObject({ path: "helper.m", content: renamedText, expectedRevision: "helper-revision" });
    expect(workspaceClient.moveCalls).toEqual([{ path: "helper.m", targetPath: "calculate.m" }]);
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    expect(await workspaceClient.read("calculate.m")).toMatchObject({ content: renamedText });
    configured.mockRestore();
  });

  it("keeps a pending rename unsaved without writing when its destination already exists", async () => {
    const configured = vi.spyOn(SharedEditorSession.prototype, "configure");
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    await workspaceClient.create("calculate.m", "file");
    const originalText = "function y = helper(x)\ny = x;\nend\n";
    const renamedText = originalText.replace("helper", "calculate");
    workspaceClient.replaceFileContent("helper.m", originalText, "helper-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("helper.m");
    expect(configured.mock.calls.at(-1)![0].stageFileRenames!([{ oldUri: editorProps().documentUri,
      newUri: workspaceDocumentUri("calculate.m", 1, "C:\\project-root"), originalText, renamedText,
    }])).toBe(true);
    act(() => editorProps().onWorkspaceDocumentChange!(editorProps().documentId, renamedText));
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Mock code editor" }), { key: "s", ctrlKey: true });
    await waitFor(() => expect(screen.getAllByText(/Cannot rename the source/).length).toBeGreaterThan(0));
    expect(workspaceClient.writeCalls).toHaveLength(0);
    expect(workspaceClient.moveCalls).toHaveLength(0);
    expect(editorProps().workspaceDocuments![0]).toMatchObject({ pendingPath: "calculate.m", content: renamedText });
    configured.mockRestore();
  });

  it("clears a pending and armed source rename when a save conflict reloads from disk", async () => {
    const configured = vi.spyOn(SharedEditorSession.prototype, "configure");
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    const originalText = "function y = helper(x)\ny = x;\nend\n";
    const renamedText = originalText.replace("helper", "calculate");
    workspaceClient.replaceFileContent("helper.m", originalText, "helper-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    const editor = await openTreeFile("helper.m");
    const id = editorProps().documentId;
    expect(configured.mock.calls.at(-1)![0].stageFileRenames!([{
      oldUri: editorProps().documentUri,
      newUri: workspaceDocumentUri("calculate.m", 1, "C:\\project-root"),
      originalText, renamedText,
    }])).toBe(true);
    act(() => editorProps().onWorkspaceDocumentChange!(id, renamedText));
    const externalText = `${originalText}% external change\n`;
    workspaceClient.replaceFileContent("helper.m", externalText, "external-revision");
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    const conflict = await screen.findByRole("dialog", { name: "The file changed on disk" });
    fireEvent.click(within(conflict).getByRole("button", { name: "Reload from Disk" }));
    await waitFor(() => expect(editor).toHaveValue(externalText));
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    expect(screen.queryByText(/Saving will rename/)).toBeNull();
    // Returning to the old refactor text must not revive the discarded plan.
    act(() => editorProps().onWorkspaceDocumentChange!(id, renamedText));
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    await waitFor(() => expect(editorProps().workspaceDocuments![0]).toMatchObject({
      savedContent: renamedText, revision: "external-revision-saved",
    }));
    expect(workspaceClient.moveCalls).toHaveLength(0);
    expect(editorProps().documentPath).toBe("helper.m");
    configured.mockRestore();
  });

  it("moves a pending source rename with its document and keeps undo and save in the new folder", async () => {
    const configured = vi.spyOn(SharedEditorSession.prototype, "configure");
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    await workspaceClient.create("archive", "directory");
    const originalText = "function y = helper(x)\ny = x;\nend\n";
    const renamedText = originalText.replace("helper", "calculate");
    workspaceClient.replaceFileContent("helper.m", originalText, "helper-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("helper.m");
    expect(configured.mock.calls.at(-1)![0].stageFileRenames!([{
      oldUri: editorProps().documentUri,
      newUri: workspaceDocumentUri("calculate.m", 1, "C:\\project-root"),
      originalText, renamedText,
    }])).toBe(true);
    act(() => editorProps().onWorkspaceDocumentChange!(editorProps().documentId, renamedText));
    fireEvent.contextMenu(screen.getByRole("treeitem", { name: "helper.m" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Move…" }));
    const moveDialog = await screen.findByRole("dialog", { name: "Move file" });
    fireEvent.click(await within(moveDialog).findByRole("button", { name: "archive" }));
    fireEvent.click(within(moveDialog).getByRole("button", { name: "Move" }));
    await waitFor(() => expect(editorProps().documentPath).toBe("archive/helper.m"));
    expect(editorProps().workspaceDocuments![0]).toMatchObject({ pendingPath: "archive/calculate.m" });
    const movedId = editorProps().documentId;
    act(() => editorProps().onWorkspaceDocumentChange!(movedId, originalText));
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    act(() => editorProps().onWorkspaceDocumentChange!(movedId, renamedText));
    expect(editorProps().workspaceDocuments![0]).toMatchObject({ pendingPath: "archive/calculate.m" });
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Mock code editor" }), { key: "s", ctrlKey: true });
    await waitFor(() => expect(editorProps().documentPath).toBe("archive/calculate.m"));
    expect(workspaceClient.moveCalls).toEqual([
      { path: "helper.m", targetPath: "archive/helper.m" },
      { path: "archive/helper.m", targetPath: "archive/calculate.m" },
    ]);
    expect(await workspaceClient.read("archive/calculate.m")).toMatchObject({ content: renamedText });
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    configured.mockRestore();
  });

  it("clears the staged operation when a manual rename reaches its pending source filename", async () => {
    const configured = vi.spyOn(SharedEditorSession.prototype, "configure");
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    const originalText = "function y = helper(x)\ny = x;\nend\n";
    const renamedText = originalText.replace("helper", "calculate");
    workspaceClient.replaceFileContent("helper.m", originalText, "helper-revision");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("helper.m");
    expect(configured.mock.calls.at(-1)![0].stageFileRenames!([{
      oldUri: editorProps().documentUri,
      newUri: workspaceDocumentUri("calculate.m", 1, "C:\\project-root"),
      originalText, renamedText,
    }])).toBe(true);
    act(() => editorProps().onWorkspaceDocumentChange!(editorProps().documentId, renamedText));
    fireEvent.contextMenu(screen.getByRole("treeitem", { name: "helper.m" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Rename…" }));
    const dialog = await screen.findByRole("dialog", { name: "Rename file" });
    const input = within(dialog).getByRole("textbox", { name: "New name" });
    fireEvent.change(input, { target: { value: "calculate.m" } });
    fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(editorProps().documentPath).toBe("calculate.m"));
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    act(() => editorProps().onWorkspaceDocumentChange!(editorProps().documentId, originalText));
    act(() => editorProps().onWorkspaceDocumentChange!(editorProps().documentId, renamedText));
    expect(editorProps().workspaceDocuments![0]).not.toHaveProperty("pendingPath");
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Mock code editor" }), { key: "s", ctrlKey: true });
    await waitFor(() => expect(editorProps().workspaceDocuments![0]).toMatchObject({ savedContent: renamedText }));
    expect(workspaceClient.moveCalls).toEqual([{ path: "helper.m", targetPath: "calculate.m" }]);
    expect(await workspaceClient.read("calculate.m")).toMatchObject({ content: renamedText });
    configured.mockRestore();
  });

  it("leaves editor state intact if any unopened rename target cannot be read", async () => {
    const configured = vi.spyOn(SharedEditorSession.prototype, "configure");
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("seed.m");
    const ensureDocuments = configured.mock.calls.at(-1)![0].ensureDocuments!;
    await act(async () => {
      await expect(ensureDocuments(["helper.m", "missing.m"].map(
        (path) => workspaceDocumentUri(path, 1, "C:\\project-root"),
      ))).rejects.toThrow("missing");
    });
    expect(editorProps().workspaceDocuments).toHaveLength(1);
    expect(editorProps().documentPath).toBe("seed.m");
    configured.mockRestore();
  });

  it("rejects a stale unopened navigation target if Current Folder changes during the read", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} />);
    await openTreeFile("seed.m");
    const helper = await workspaceClient.read("helper.m");
    let completeRead!: (file: typeof helper) => void;
    vi.spyOn(workspaceClient, "read").mockReturnValueOnce(new Promise((resolve) => { completeRead = resolve; }));
    let result!: boolean | Promise<boolean>;
    act(() => { result = editorProps().onOpenDocument!(workspaceDocumentUri("helper.m", 1, "C:\\project-root")); });
    await act(async () => { await workspaceClient.changeDirectory("D:\\other-root"); });
    await act(async () => {
      completeRead(helper);
      expect(await result).toBe(false);
    });
    expect(editorProps().workspaceDocuments).toHaveLength(1);
    expect(editorProps().documentPath).toBe("seed.m");
  });

  it("targets the original root for navigation and edits when open relative paths match", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    const read = vi.spyOn(workspaceClient, "read");
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    await openTreeFile("seed.m");
    const firstId = editorProps().documentId;
    const address = screen.getByRole("textbox", { name: "Current Folder path" });
    fireEvent.change(address, { target: { value: "D:\\second-root" } });
    fireEvent.submit(address.closest("form")!);
    await waitFor(() => expect(address).toHaveValue("D:\\second-root"));
    fireEvent.click(await screen.findByRole("treeitem", { name: "seed.m" }));
    await waitFor(() => expect(editorProps().documentUri).toBe(workspaceDocumentUri("seed.m", 2, "D:\\second-root")));
    read.mockClear();
    act(() => {
      editorProps().onWorkspaceDocumentChange!(firstId, "first_root = 2;\n");
      expect(editorProps().onOpenDocument!(workspaceDocumentUri("seed.m", 1, "C:\\project-root"))).toBe(true);
    });
    expect(editorProps().value).toBe("first_root = 2;\n");
    expect(editorProps().documentUri).toBe(workspaceDocumentUri("seed.m", 1, "C:\\project-root"));
    expect(address).toHaveValue("D:\\second-root");
    expect(read).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(workspaceClient.writeCalls.at(-1)).toEqual({
      path: "seed.m", content: "first_root = 2;\n", expectedRevision: "revision-seed", rootGeneration: 1,
    }));
  });

  it("keeps the current editor mounted while another workspace file is loading", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    await workspaceClient.create("helper.m", "file");
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const editor = await openTreeFile("seed.m");
    const helperFile = await workspaceClient.read("helper.m");
    let completeRead!: (file: typeof helperFile) => void;
    vi.spyOn(workspaceClient, "read").mockReturnValueOnce(
      new Promise((resolve) => { completeRead = resolve; }),
    );
    fireEvent.click(screen.getByRole("treeitem", { name: "helper.m" }));
    expect(screen.getByLabelText("Mock code editor")).toBe(editor);
    expect(editorProps().documentPath).toBe("seed.m");
    fireEvent.change(editor, { target: { value: "still_editing = 1;\n" } });
    await act(async () => completeRead(helperFile));
    expect(screen.getByLabelText("Mock code editor")).toBe(editor);
    expect(editorProps().documentPath).toBe("helper.m");
    expect(editorProps().workspaceDocuments![0]).toEqual(
      expect.objectContaining({ content: "still_editing = 1;\n" }),
    );
  });

  it("keeps the dirty document intact when a revision-conflict save fails", async () => {
    const workspaceClient = new RecordingWorkspaceClient(null, true);
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "local_work = 99;\n" } });
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });

    const conflict = await screen.findByRole("dialog", {
      name: "The file changed on disk",
    });
    expect(conflict).toHaveTextContent(
      "changed outside OpenMat",
    );
    fireEvent.click(within(conflict).getByRole("button", { name: "Keep Editing" }));
    expect((screen.getByLabelText("Mock code editor") as HTMLTextAreaElement).value).toBe(
      "local_work = 99;\n",
    );
    expect(screen.getByLabelText("Unsaved changes")).toBeVisible();
  });

  it("reloads the latest disk contents from the save-conflict dialog", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const editor = await openTreeFile("seed.m");
    workspaceClient.replaceFileContent(
      "seed.m",
      "external_change = 9;\n",
      "external-revision",
    );
    fireEvent.change(editor, { target: { value: "local_work = 99;\n" } });
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    const conflict = await screen.findByRole("dialog", {
      name: "The file changed on disk",
    });
    fireEvent.click(
      within(conflict).getByRole("button", { name: "Reload from Disk" }),
    );
    await waitFor(() => expect(editor).toHaveValue("external_change = 9;\n"));
    expect(screen.queryByLabelText("Unsaved changes")).toBeNull();
  });

  it("saves conflicted editor contents as a new copy without overwriting the disk file", async () => {
    const workspaceClient = new RecordingWorkspaceClient(null, true);
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "local_copy = 42;\n" } });
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    const conflict = await screen.findByRole("dialog", {
      name: "The file changed on disk",
    });
    fireEvent.click(
      within(conflict).getByRole("button", { name: "Save a Copy…" }),
    );
    const copyDialog = await screen.findByRole("dialog", {
      name: "Save an edited copy",
    });
    expect(within(copyDialog).getByRole("textbox", { name: "Copy name" })).toHaveValue(
      "seed copy.m",
    );
    fireEvent.click(within(copyDialog).getByRole("button", { name: "Save Copy" }));
    await waitFor(() =>
      expect(editor).toHaveAttribute("data-document-path", "seed copy.m"),
    );
    expect(editor).toHaveValue("local_copy = 42;\n");
    expect(screen.queryByLabelText("Unsaved changes")).toBeNull();
    expect(workspaceClient.writeCalls.at(-1)).toMatchObject({
      path: "seed copy.m",
      content: "local_copy = 42;\n",
    });
  });

  it("keeps dirty tabs while switching, confirms close, and updates identity after a move", async () => {
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "New File" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "other.m" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    const editor = await screen.findByLabelText("Mock code editor");
    expect(editor).toHaveAttribute("data-document-path", "other.m");
    fireEvent.change(editor, { target: { value: "unsaved = 1;\n" } });

    fireEvent.click(screen.getByRole("button", { name: "New Folder" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "archive" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await screen.findByRole("treeitem", { name: "archive" });

    fireEvent.click(screen.getByRole("treeitem", { name: "seed.m" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Mock code editor")).toHaveAttribute(
        "data-document-path",
        "seed.m",
      ),
    );
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    const workspaceExplorer = screen.getByRole("complementary", {
      name: "Workspace explorer",
    });
    const tabList = within(workspaceExplorer).getByRole("tablist");
    expect(tabList).toHaveAttribute(
      "aria-orientation",
      "vertical",
    );
    expect(tabList.closest(".open-editors-section")?.nextElementSibling).toHaveClass(
      "current-folder-header",
    );
    expect(
      within(screen.getByRole("tabpanel", { name: "Editor for seed.m" })).queryByRole(
        "tablist",
      ),
    ).toBeNull();
    const otherTab = screen.getByRole("tab", { name: /other\.m/ });
    fireEvent.click(otherTab);
    expect(screen.getByLabelText("Mock code editor")).toHaveValue(
      "unsaved = 1;\n",
    );
    fireEvent.keyDown(otherTab, { key: "ArrowDown" });
    await waitFor(() =>
      expect(screen.getByLabelText("Mock code editor")).toHaveAttribute(
        "data-document-path",
        "seed.m",
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "Close other.m" }));
    expect(screen.getByLabelText("Mock code editor")).toBeVisible();
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    const closeDialog = screen.getByRole("dialog", {
      name: "Save changes before closing?",
    });
    fireEvent.click(within(closeDialog).getByRole("button", { name: "Cancel" }));
    expect(screen.getAllByRole("tab")).toHaveLength(2);

    fireEvent.click(screen.getByRole("button", { name: "Close other.m" }));
    fireEvent.click(
      within(
        screen.getByRole("dialog", { name: "Save changes before closing?" }),
      ).getByRole("button", { name: "Don’t Save" }),
    );
    await waitFor(() => expect(screen.getAllByRole("tab")).toHaveLength(1));
    fireEvent.contextMenu(screen.getByRole("treeitem", { name: "seed.m" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Move…" }));
    const moveDialog = await screen.findByRole("dialog", { name: "Move file" });
    fireEvent.click(
      await within(moveDialog).findByRole("button", { name: "archive" }),
    );
    fireEvent.click(within(moveDialog).getByRole("button", { name: "Move" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Mock code editor")).toHaveAttribute(
        "data-document-path",
        "archive/seed.m",
      ),
    );
    expect(workspaceClient.moveCalls.at(-1)).toEqual({
      path: "seed.m",
      targetPath: "archive/seed.m",
    });
    fireEvent.click(screen.getByRole("treeitem", { name: "archive" }));
    const movedFile = await screen.findByRole("treeitem", { name: "seed.m" });
    fireEvent.click(movedFile);
    await waitFor(() => expect(movedFile).toHaveAttribute("aria-selected", "true"));
  });

  it("persists dirty tabs on page exit and restores them after a remount", async () => {
    const documentStore = new MemoryDocumentSessionStore();
    const firstWorkspace = new RecordingWorkspaceClient();
    const first = render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={firstWorkspace}
        documentSessionStore={documentStore}
      />,
    );
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "recovered = 42;\n" } });

    act(() => window.dispatchEvent(new Event("pagehide")));
    await waitFor(() =>
      expect(documentStore.sessions.get("workspace:mock")?.documents[0]?.content).toBe(
        "recovered = 42;\n",
      ),
    );
    first.unmount();

    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={new RecordingWorkspaceClient()}
        documentSessionStore={documentStore}
      />,
    );
    expect(await screen.findByLabelText("Mock code editor")).toHaveValue(
      "recovered = 42;\n",
    );
    expect(screen.getByRole("tab", { name: /seed\.m/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByLabelText("Unsaved changes")).toBeVisible();
  });

  it("keeps a recovered old-root draft separate when the current root reuses its generation", async () => {
    const documentStore = new MemoryDocumentSessionStore();
    const oldDraft = {
      ...openDocumentFromWorkspaceFile({
        path: "seed.m",
        rootPath: "D:\\previous-root",
        rootGeneration: 1,
        content: "old_disk = 1;\n",
        revision: "revision-old-root",
        size: 14,
      }),
      content: "old_unsaved = 7;\n",
    };
    await documentStore.save(
      "workspace:mock", snapshotDocumentSession([oldDraft], oldDraft.id),
    );
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
        documentSessionStore={documentStore}
      />,
    );
    expect(await screen.findByLabelText("Mock code editor")).toHaveValue("old_unsaved = 7;\n");
    expect(editorProps().documentUri).toBe(oldDraft.uri);
    fireEvent.click(screen.getByRole("treeitem", { name: "seed.m" }));
    const currentUri = workspaceDocumentUri("seed.m", 1, "C:\\project-root");
    await waitFor(() => expect(editorProps().documentUri).toBe(currentUri));
    expect(currentUri).not.toBe(oldDraft.uri);
    expect(new Set(editorProps().workspaceDocuments!.map((document) => document.uri)).size).toBe(2);
    const currentId = editorProps().documentId;
    act(() => editorProps().onWorkspaceDocumentChange!(currentId, "current_root = 2;\n"));
    expect(editorProps().workspaceDocuments!.find((document) => document.id === oldDraft.id)?.content)
      .toBe("old_unsaved = 7;\n");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(workspaceClient.writeCalls.at(-1)).toEqual({
      path: "seed.m", content: "current_root = 2;\n", expectedRevision: "revision-seed", rootGeneration: 1,
    }));
    act(() => expect(editorProps().onOpenDocument!(URI.parse(oldDraft.uri).toString())).toBe(true));
    expect(editorProps().value).toBe("old_unsaved = 7;\n");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(editorProps().workspaceDocuments!.find((document) => document.id === currentId)?.content)
      .toBe("current_root = 2;\n");
  });

  it("requires an explicit choice before a recovered draft can overwrite changed disk data", async () => {
    const documentStore = new MemoryDocumentSessionStore();
    const baseFile = {
      path: "seed.m",
      rootPath: "C:\\project-root",
      rootGeneration: 1,
      content: "seed = 1;\n",
      revision: "revision-seed",
      size: 10,
    };
    const draft = {
      ...openDocumentFromWorkspaceFile(baseFile),
      content: "local_draft = 7;\n",
    };
    await documentStore.save(
      "workspace:mock",
      snapshotDocumentSession([draft], draft.id),
    );
    const workspaceClient = new RecordingWorkspaceClient();
    workspaceClient.replaceFileContent(
      "seed.m",
      "external_change = 9;\n",
      "revision-external",
    );

    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
        documentSessionStore={documentStore}
      />,
    );

    expect(await screen.findByLabelText("Mock code editor")).toHaveValue(
      "local_draft = 7;\n",
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "older disk revision",
    );
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Resolve Conflict…" }));
    const conflict = await screen.findByRole("dialog", {
      name: "The file changed on disk",
    });
    fireEvent.click(within(conflict).getByRole("button", { name: "Keep Editing" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(workspaceClient.writeCalls).toHaveLength(1));
    expect(workspaceClient.writeCalls[0]).toMatchObject({
      content: "local_draft = 7;\n",
      expectedRevision: "revision-external",
    });
  });

  it("keeps the creation flow open and shows structured workspace errors", async () => {
    const workspaceClient = new RecordingWorkspaceClient("CON");
    render(
      <App
        transport={new MockKernelTransport({ eventDelayMs: 0 })}
        workspaceClient={workspaceClient}
      />,
    );
    const newFile = screen.getByRole("button", { name: "New File" });
    await waitFor(() => expect(newFile).toBeEnabled());
    fireEvent.click(newFile);
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "CON" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("reserved Windows device name");
    expect(alert).toHaveTextContent("workspace.reservedWindowsName");
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveAttribute(
      "aria-invalid",
      "true",
    );
  });

  it("uses a fixed light default and persists and restores Modern Dark with Monaco", async () => {
    const first = render(
      <App transport={new MockKernelTransport({ eventDelayMs: 0 })} />,
    );
    await openTreeFile("welcome.m");
    const shell = screen.getByLabelText("OpenMat editor").closest(".app-shell");
    expect(shell).toHaveAttribute("data-theme", "modern-light");
    expect(screen.getByLabelText("Mock code editor")).toHaveAttribute(
      "data-theme",
      "modern-light",
    );

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    const themeSelect = screen.getByRole("combobox", { name: "Theme" });
    expect(themeSelect).toHaveFocus();
    fireEvent.change(themeSelect, { target: { value: "modern-dark" } });
    expect(shell).toHaveAttribute("data-theme", "modern-dark");
    expect(screen.getByLabelText("Mock code editor")).toHaveAttribute(
      "data-theme",
      "modern-dark",
    );
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("modern-dark");

    first.unmount();
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} />);
    expect(screen.getByLabelText("OpenMat editor").closest(".app-shell")).toHaveAttribute(
      "data-theme",
      "modern-dark",
    );
  });

  it("integrates accessible persisted splitters without overwriting the theme key", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "modern-dark");
    render(<App transport={new MockKernelTransport({ eventDelayMs: 0 })} />);

    const leftSplitter = screen.getByRole("separator", {
      name: "Resize Current Folder and Editor",
    });
    expect(screen.getAllByRole("separator")).toHaveLength(3);
    expect(leftSplitter).toHaveAttribute("aria-orientation", "vertical");
    fireEvent.keyDown(leftSplitter, { key: "ArrowRight" });

    expect(localStorage.getItem(IDE_LAYOUT_STORAGE_KEY)).not.toBeNull();
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("modern-dark");
  });

  it("disables execution and offers recovery after transport loss", async () => {
    const transport = new DisconnectableMockTransport({ eventDelayMs: 0 });
    render(<App transport={transport} reconnectDelays={[]} />);
    await openTreeFile("welcome.m");
    const runButton = await screen.findByRole("button", { name: /^run$/i });
    await waitFor(() => expect(runButton).toBeEnabled());

    act(() => transport.loseConnection("Kernel WebSocket closed (1006)"));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Kernel WebSocket closed (1006)",
    );
    expect(runButton).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Retry connection" }),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Retry connection" }));
    await waitFor(() => expect(runButton).toBeEnabled());
    expect(screen.queryByText("Kernel WebSocket closed (1006)")).toBeNull();
  });

  it("automatically reconnects only the kernel and preserves unsaved editor text", async () => {
    const transport = new DisconnectableMockTransport({ eventDelayMs: 0 });
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={transport}
        workspaceClient={workspaceClient}
        reconnectDelays={[100]}
      />,
    );
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "seed = 99;\n" } });
    const runButton = await screen.findByRole("button", { name: /^run$/i });
    await waitFor(() => expect(runButton).toBeEnabled());
    expect(transport.connectCount).toBe(1);
    expect(workspaceClient.connectCount).toBe(1);

    act(() => transport.loseConnection("Kernel WebSocket closed (1006)"));

    expect(runButton).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("Recovering kernel");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Retrying automatically",
    );
    await waitFor(() => expect(transport.connectCount).toBe(2));
    await waitFor(() => expect(runButton).toBeEnabled());
    expect(workspaceClient.connectCount).toBe(1);
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("seed = 99;\n");
  });

  it("automatically reconnects and resynchronizes only the file service", async () => {
    const transport = new DisconnectableMockTransport({ eventDelayMs: 0 });
    const workspaceClient = new RecordingWorkspaceClient();
    render(
      <App
        transport={transport}
        workspaceClient={workspaceClient}
        reconnectDelays={[100]}
      />,
    );
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "seed = 42;\n" } });
    await waitFor(() => expect(workspaceClient.listCount).toBe(1));
    expect(transport.connectCount).toBe(1);

    act(() => workspaceClient.loseConnection("Workspace WebSocket closed (1006)"));

    expect(screen.getByRole("status")).toHaveTextContent("Recovering files");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Workspace WebSocket closed (1006)",
    );
    await waitFor(() => expect(workspaceClient.connectCount).toBe(2));
    await waitFor(() => expect(workspaceClient.listCount).toBe(2));
    expect(transport.connectCount).toBe(1);
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("seed = 42;\n");
    expect(screen.getByRole("status")).toHaveTextContent("Ready · idle");
  });
});


function desktopServices(files: Partial<DesktopFiles> = {}): PlatformServices {
  return {
    kind: "desktop",
    files: {
      pickDirectory: vi.fn(async () => null),
      pickFile: vi.fn(async () => null),
      revealPath: vi.fn(async () => undefined),
      saveTextFile: vi.fn(async () => null),
      ...files,
    },
    saveExport: vi.fn(async () => true),
  };
}

describe("Desktop file integration", () => {
  beforeEach(() => {
    localStorage.clear();
    mockEditorState.props = null;
  });

  function renderDesktop(workspaceClient: RecordingWorkspaceClient, platform: PlatformServices) {
    return render(<App platform={platform} transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={workspaceClient} documentSessionStore={new MemoryDocumentSessionStore()} />);
  }

  function closeableDesktop() {
    let request: (() => void) | undefined;
    const close = vi.fn().mockResolvedValue(undefined);
    const remove = vi.fn();
    const platform: PlatformServices = { ...desktopServices(), lifecycle: {
      onCloseRequested: async (handler) => { request = handler; return remove; }, close,
    } };
    return { platform, close, remove, request: () => { if (!request) throw new Error("Close listener missing"); act(() => request!()); } };
  }

  it("cancels native close without losing edits and avoids duplicate exit prompts", async () => {
    const desktop = closeableDesktop();
    const mounted = renderDesktop(new RecordingWorkspaceClient(), desktop.platform);
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "keep_editing = 42;" } });
    desktop.request();
    desktop.request();
    const dialog = await screen.findByRole("dialog", { name: "Close OpenMat" });
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeEnabled());
    expect(document.querySelector(".openmat-dialog-layer")).toHaveAttribute("data-dialog-queue-length", "1");
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(editor).toHaveValue("keep_editing = 42;");
    expect(desktop.close).not.toHaveBeenCalled();
    mounted.unmount();
    expect(desktop.remove).toHaveBeenCalledOnce();
  });

  it("saves the script and a hidden App Designer before closing", async () => {
    const desktop = closeableDesktop();
    const client = new RecordingWorkspaceClient();
    renderDesktop(client, desktop.platform);
    fireEvent.change(await openTreeFile("seed.m"), { target: { value: "saved_before_exit = 42;" } });
    fireEvent.click(screen.getByRole("button", { name: "App Designer" }));
    const designer = await screen.findByRole("region", { name: "App Designer" });
    await waitFor(() => expect(within(designer).getByRole("button", { name: "保存" })).toBeEnabled());
    fireEvent.click(within(designer).getByRole("button", { name: "返回工作台" }));
    desktop.request();
    const dialog = await screen.findByRole("dialog", { name: "Close OpenMat" });
    await waitFor(() => expect(within(dialog).getByText("App Designer: SignalApp.omui")).toBeVisible());
    fireEvent.click(within(dialog).getByRole("button", { name: "Save All" }));
    await waitFor(() => expect(desktop.close, within(dialog).queryByRole("alert")?.textContent ?? "Closing after save").toHaveBeenCalledOnce());
    expect((await client.read("seed.m")).content).toBe("saved_before_exit = 42;");
    expect((await client.read("SignalApp.m")).content).toContain("classdef SignalApp");
    expect((await client.read("SignalApp.omui")).content).toContain("SignalApp");
  });

  it("keeps the window and dirty script when Save All meets a revision conflict", async () => {
    const desktop = closeableDesktop();
    renderDesktop(new RecordingWorkspaceClient(null, true), desktop.platform);
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "keep_conflicted_draft = 42;" } });
    desktop.request();
    const dialog = await screen.findByRole("dialog", { name: "Close OpenMat" });
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "Save All" })).toBeEnabled());
    fireEvent.click(within(dialog).getByRole("button", { name: "Save All" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Could not save seed.m");
    expect(desktop.close).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(editor).toHaveValue("keep_conflicted_draft = 42;");
  });

  it("discards the recovery draft without writing the disk file or resurrecting it on pagehide", async () => {
    const desktop = closeableDesktop();
    const client = new RecordingWorkspaceClient();
    const store = new MemoryDocumentSessionStore();
    render(<App platform={desktop.platform} transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={client} documentSessionStore={store} />);
    fireEvent.change(await openTreeFile("seed.m"), { target: { value: "discard_this = 42;" } });
    desktop.request();
    const dialog = await screen.findByRole("dialog", { name: "Close OpenMat" });
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "Don’t Save" })).toBeEnabled());
    fireEvent.click(within(dialog).getByRole("button", { name: "Don’t Save" }));
    await waitFor(() => expect(desktop.close).toHaveBeenCalledOnce());
    act(() => window.dispatchEvent(new Event("pagehide")));
    expect(client.writeCalls).toHaveLength(0);
    expect(store.sessions.get("workspace:desktop:v1")?.documents[0]?.content).toBe("seed = 1;\n");
  });

  it("waits for a file save already in flight and for the final recovery transaction", async () => {
    const desktop = closeableDesktop();
    const client = new RecordingWorkspaceClient();
    const store = new MemoryDocumentSessionStore();
    const originalWrite = client.write.bind(client);
    let finishFile!: () => void;
    vi.spyOn(client, "write").mockImplementation(async (...args) => {
      await new Promise<void>((resolve) => { finishFile = resolve; });
      return originalWrite(...args);
    });
    render(<App platform={desktop.platform} transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={client} documentSessionStore={store} />);
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "slow_save = 42;" } });
    fireEvent.keyDown(editor, { key: "s", ctrlKey: true });
    let finishRecovery!: () => void;
    const originalSave = store.save.bind(store);
    vi.spyOn(store, "save").mockImplementation(async (...args) => {
      await new Promise<void>((resolve) => { finishRecovery = resolve; });
      await originalSave(...args);
    });
    desktop.request();
    expect(await screen.findByRole("dialog", { name: "Close OpenMat" })).toBeVisible();
    expect(desktop.close).not.toHaveBeenCalled();
    await act(async () => finishFile());
    await waitFor(() => expect(finishRecovery).toBeTypeOf("function"));
    expect(desktop.close).not.toHaveBeenCalled();
    await act(async () => finishRecovery());
    await waitFor(() => expect(desktop.close).toHaveBeenCalledOnce());
    expect(store.sessions.get("workspace:desktop:v1")?.documents[0]?.savedContent).toBe("slow_save = 42;");
  });

  it("preserves the window and edits if recovery storage fails during exit", async () => {
    const desktop = closeableDesktop();
    const store = new MemoryDocumentSessionStore();
    render(<App platform={desktop.platform} transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={new RecordingWorkspaceClient()} documentSessionStore={store} />);
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "recovery_failure = 42;" } });
    vi.spyOn(store, "save").mockRejectedValue(new Error("Recovery disk is full"));
    desktop.request();
    const dialog = await screen.findByRole("dialog", { name: "Close OpenMat" });
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "Don’t Save" })).toBeEnabled());
    fireEvent.click(within(dialog).getByRole("button", { name: "Don’t Save" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Recovery disk is full");
    expect(desktop.close).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(editor).toHaveValue("recovery_failure = 42;");
  });

  it("restores the last Desktop folder, active file, cursor and scroll before editing", async () => {
    const client = new RecordingWorkspaceClient();
    const store = new MemoryDocumentSessionStore();
    const savedRoot = "D:\\研究 work";
    const viewState = { lineNumber: 3, column: 2, scrollTop: 120, scrollLeft: 8 };
    const file = { ...openDocumentFromWorkspaceFile(await client.read("seed.m")), rootPath: savedRoot,
      id: JSON.stringify([savedRoot, "seed.m"]), viewState };
    store.sessions.set("workspace:desktop:v1", snapshotDocumentSession([file], file.id,
      { rootPath: savedRoot, designerMounted: false, designerVisible: false }));
    render(<App platform={desktopServices()} transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={client} documentSessionStore={store} />);
    expect(await screen.findByLabelText("Mock code editor")).toHaveValue("seed = 1;\n");
    expect((await client.currentDirectory()).path).toBe(savedRoot);
    expect(editorProps().viewState).toEqual(viewState);
  });

  it("keeps invalid Designer source on a failed exit save and discards it only on explicit request", async () => {
    const desktop = closeableDesktop();
    const store = new MemoryDocumentSessionStore();
    render(<App platform={desktop.platform} transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={new RecordingWorkspaceClient()} documentSessionStore={store} />);
    await openTreeFile("seed.m");
    fireEvent.click(screen.getByRole("button", { name: "App Designer" }));
    const designer = await screen.findByRole("region", { name: "App Designer" });
    await waitFor(() => expect(within(designer).getByRole("button", { name: "保存" })).toBeEnabled());
    fireEvent.click(within(designer).getByRole("button", { name: "XML" }));
    fireEvent.change(within(designer).getByLabelText("界面 XML"), { target: { value: "<unfinished-design" } });
    desktop.request();
    const dialog = await screen.findByRole("dialog", { name: "Close OpenMat" });
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "Save All" })).toBeEnabled());
    fireEvent.click(within(dialog).getByRole("button", { name: "Save All" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("OpenMat will stay open");
    expect(desktop.close).not.toHaveBeenCalled();
    expect(within(designer).getByLabelText("界面 XML")).toHaveValue("<unfinished-design");
    fireEvent.click(within(dialog).getByRole("button", { name: "Don’t Save" }));
    await waitFor(() => expect(desktop.close).toHaveBeenCalledOnce());
    expect(localStorage.getItem("openmat.designer.draft.v1:C:\\project-root")).toBeNull();
    expect(store.sessions.get("workspace:desktop:v1")?.desktopWorkspace?.designerMounted).toBe(false);
    expect(store.sessions.get("workspace:desktop:v1")?.documents.map((document) => document.path)).not.toContain("SignalApp.m");
  });

  it("retains a missing-folder recovery draft without writing the same relative path in the fallback folder", async () => {
    const desktop = closeableDesktop();
    const client = new RecordingWorkspaceClient();
    const store = new MemoryDocumentSessionStore();
    const unavailable = "D:\\removed folder";
    const file = { ...openDocumentFromWorkspaceFile(await client.read("seed.m")), rootPath: unavailable,
      id: JSON.stringify([unavailable, "seed.m"]), content: "missing_folder_draft = 42;" };
    store.sessions.set("workspace:desktop:v1", snapshotDocumentSession([file], file.id,
      { rootPath: unavailable, designerMounted: false, designerVisible: false }));
    vi.spyOn(client, "changeDirectory").mockRejectedValue(new Error("Folder no longer exists"));
    render(<App platform={desktop.platform} transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={client} documentSessionStore={store} />);
    expect(await screen.findByLabelText("Mock code editor")).toHaveValue("missing_folder_draft = 42;");
    desktop.request();
    const dialog = await screen.findByRole("dialog", { name: "Close OpenMat" });
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "Save All" })).toBeEnabled());
    fireEvent.click(within(dialog).getByRole("button", { name: "Save All" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Could not save seed.m");
    expect(client.writeCalls).toHaveLength(0);
    expect(desktop.close).not.toHaveBeenCalled();
  });

  it("restores a desktop draft when the next launch uses a different kernel port", async () => {
    const store = new MemoryDocumentSessionStore();
    const platform = desktopServices();
    const first = render(<App platform={platform} wsUrl="ws://127.0.0.1:42000/kernel" transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={new RecordingWorkspaceClient()} documentSessionStore={store} />);
    fireEvent.change(await openTreeFile("seed.m"), { target: { value: "restore_after_port_change = 42;" } });
    act(() => window.dispatchEvent(new Event("pagehide")));
    await waitFor(() => expect(store.sessions.get("workspace:desktop:v1")?.documents[0]?.content).toBe("restore_after_port_change = 42;"));
    first.unmount();
    render(<App platform={platform} wsUrl="ws://127.0.0.1:42123/kernel" transport={new MockKernelTransport({ eventDelayMs: 0 })} workspaceClient={new RecordingWorkspaceClient()} documentSessionStore={store} />);
    expect(await screen.findByLabelText("Mock code editor")).toHaveValue("restore_after_port_change = 42;");
    expect(screen.getByLabelText("Unsaved changes")).toBeVisible();
  });

  it("replaces web transfers with native Explorer actions and refuses file drops", async () => {
    const client = new RecordingWorkspaceClient();
    const platform = desktopServices();
    const { container } = renderDesktop(client, platform);
    const file = await screen.findByRole("treeitem", { name: "seed.m" });
    expect(screen.queryByRole("button", { name: "Upload Files" })).toBeNull();
    expect(container.querySelector('input[type="file"]')).toBeNull();
    fireEvent.contextMenu(file);
    expect(screen.queryByRole("menuitem", { name: "Download" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Upload files here…" })).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: "Show in File Explorer" }));
    await waitFor(() => expect(platform.files!.revealPath).toHaveBeenCalledWith("C:\\project-root", "seed.m"));
    fireEvent.click(screen.getByRole("button", { name: "Open Current Folder in File Explorer" }));
    await waitFor(() => expect(platform.files!.revealPath).toHaveBeenCalledWith("C:\\project-root", ""));
    fireEvent.drop(file, { dataTransfer: { types: ["Files"], files: [new File(["x=1"], "drop.m")] } });
    expect(client.uploadCalls).toEqual([]);
    expect(client.downloadCalls).toEqual([]);
  });

  it("uses native folder selection, with cancellation preserving Current Folder", async () => {
    const client = new RecordingWorkspaceClient();
    const change = vi.spyOn(client, "changeDirectory");
    const pickDirectory = vi.fn<DesktopFiles["pickDirectory"]>().mockResolvedValueOnce(null).mockResolvedValueOnce("D:\\my work");
    renderDesktop(client, desktopServices({ pickDirectory }));
    await screen.findByRole("treeitem", { name: "seed.m" });
    const choose = screen.getByRole("button", { name: "Choose Current Folder" });
    fireEvent.click(choose);
    await waitFor(() => expect(choose).toBeEnabled());
    expect(change).not.toHaveBeenCalled();
    expect(client.browseCalls).toEqual([]);
    expect(screen.queryByRole("dialog", { name: "Choose Current Folder" })).toBeNull();
    fireEvent.click(choose);
    await waitFor(() => expect(change).toHaveBeenCalledWith("D:\\my work"));
  });

  it("opens a selected native file from another folder and preserves the original draft", async () => {
    const client = new RecordingWorkspaceClient();
    const platform = desktopServices({ pickFile: vi.fn(async () => ({ path: "D:\\my work\\seed.m", directory: "D:\\my work", name: "seed.m" })) });
    renderDesktop(client, platform);
    fireEvent.change(await openTreeFile("seed.m"), { target: { value: "original draft = 42;" } });
    fireEvent.click(screen.getByRole("button", { name: "Open File…" }));
    await waitFor(() => expect(screen.getByLabelText("Mock code editor")).toHaveValue("seed = 1;\n"));
    expect(screen.getByText("2 open files")).toBeVisible();
    expect(client.uploadCalls).toEqual([]);
    expect(screen.getAllByRole("tab").some(tab => tab.textContent?.includes("●"))).toBe(true);
  });

  it("opens a native .omui selection in App Designer", async () => {
    const client = new RecordingWorkspaceClient();
    const platform = desktopServices({ pickFile: vi.fn(async () => ({ path: "C:\\project-root\\missing.omui", directory: "C:\\project-root", name: "missing.omui" })) });
    renderDesktop(client, platform);
    await screen.findByRole("treeitem", { name: "seed.m" });
    fireEvent.click(screen.getByRole("button", { name: "Open File…" }));
    expect(await screen.findByRole("button", { name: "返回工作台" })).toBeVisible();
    expect(screen.queryByLabelText("Mock code editor")).toBeNull();
  });

  it("reuses a nested file's open draft after the native picker changes its root", async () => {
    const client = new RecordingWorkspaceClient();
    await client.create("src", "directory");
    client.addExternalFile("src/nested.m");
    client.replaceFileContent("src/nested.m", "disk content", "nested-base");
    const platform = desktopServices({ pickFile: vi.fn(async () => ({ path: "C:\\project-root\\src\\nested.m", directory: "C:\\project-root\\src", name: "nested.m" })) });
    renderDesktop(client, platform);
    fireEvent.click(await screen.findByRole("treeitem", { name: "src" }));
    fireEvent.change(await openTreeFile("nested.m"), { target: { value: "nested draft" } });
    fireEvent.click(screen.getByRole("button", { name: "Open File…" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Open File…" })).toBeEnabled());
    expect(screen.getByText("1 open files")).toBeVisible();
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("nested draft");
  });

  it("cancels native Save As without changing the editor or writing a file", async () => {
    const client = new RecordingWorkspaceClient();
    const change = vi.spyOn(client, "changeDirectory");
    const platform = desktopServices();
    renderDesktop(client, platform);
    fireEvent.change(await openTreeFile("seed.m"), { target: { value: "unsaved = 2;" } });
    fireEvent.click(screen.getByRole("button", { name: "Save As…" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Save As…" })).toBeEnabled());
    expect(platform.files!.saveTextFile).toHaveBeenCalledWith("seed.m", "C:\\project-root", "unsaved = 2;");
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("unsaved = 2;");
    expect(change).not.toHaveBeenCalled();
    expect(client.writeCalls).toEqual([]);
  });

  it("keeps edits made during Save As dirty and writes later saves to the new file", async () => {
    const client = new RecordingWorkspaceClient();
    let complete!: (location: NativeFileLocation) => void;
    const saveTextFile = vi.fn<DesktopFiles["saveTextFile"]>(() => new Promise(resolve => { complete = resolve; }));
    renderDesktop(client, desktopServices({ saveTextFile }));
    const editor = await openTreeFile("seed.m");
    fireEvent.change(editor, { target: { value: "captured = 2;" } });
    fireEvent.click(screen.getByRole("button", { name: "Save As…" }));
    expect(screen.getByRole("button", { name: "Save As…" })).toBeDisabled();
    fireEvent.change(editor, { target: { value: "newer = 3;" } });
    client.addExternalFile("copy.m");
    client.replaceFileContent("copy.m", "captured = 2;", "native-revision");
    await act(async () => complete({ path: "D:\\my work\\copy.m", directory: "D:\\my work", name: "copy.m" }));
    await waitFor(() => expect(screen.getByLabelText("Mock code editor")).toHaveAttribute("data-document-path", "copy.m"));
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("newer = 3;");
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeEnabled();
    fireEvent.click(save);
    await waitFor(() => expect(client.writeCalls).toContainEqual({ path: "copy.m", content: "newer = 3;", expectedRevision: "native-revision", rootGeneration: 2 }));
    expect(saveTextFile).toHaveBeenCalledTimes(1);
  });

  it("preserves an existing destination draft after saving a native copy", async () => {
    const client = new RecordingWorkspaceClient();
    client.addExternalFile("copy.m");
    client.replaceFileContent("copy.m", "disk copy", "copy-base");
    const saveTextFile = vi.fn<DesktopFiles["saveTextFile"]>(async (_name, _directory, content) => {
      client.replaceFileContent("copy.m", content, "native-revision");
      return { path: "C:\\project-root\\copy.m", directory: "C:\\project-root", name: "copy.m" };
    });
    renderDesktop(client, desktopServices({ saveTextFile }));
    fireEvent.change(await openTreeFile("copy.m"), { target: { value: "destination draft" } });
    fireEvent.change(await openTreeFile("seed.m"), { target: { value: "source draft" } });
    fireEvent.click(screen.getByRole("button", { name: "Save As…" }));
    await waitFor(() => expect(screen.getByText(/both editors were preserved/)).toBeVisible());
    fireEvent.click(await screen.findByRole("treeitem", { name: "copy.m" }));
    expect(screen.getByLabelText("Mock code editor")).toHaveValue("destination draft");
  });
});
