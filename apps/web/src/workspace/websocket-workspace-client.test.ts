import { describe, expect, it, vi } from "vitest";
import {
  WebSocketWorkspaceClient,
  downloadUrlFromWorkspaceUrl,
  uploadUrlFromWorkspaceUrl,
  workspaceUrlFromKernelUrl,
} from "./websocket-workspace-client";

class FakeSocket extends EventTarget {
  readyState: number = WebSocket.CONNECTING;
  binaryType: BinaryType = "blob";
  readonly sent: string[] = [];
  closeCode: number | null = null;

  open(): void {
    this.readyState = WebSocket.OPEN;
    this.dispatchEvent(new Event("open"));
  }

  send(frame: string): void {
    this.sent.push(frame);
  }

  close(code = 1000): void {
    this.closeCode = code;
    this.readyState = WebSocket.CLOSED;
    this.dispatchEvent(new CloseEvent("close", { code }));
  }

  respond(value: unknown): void {
    this.dispatchEvent(
      new MessageEvent("message", { data: JSON.stringify(value) }),
    );
  }

  lastRequest(): Record<string, unknown> {
    return JSON.parse(this.sent.at(-1)!) as Record<string, unknown>;
  }
}

async function connectedClient(fetcher?: typeof fetch) {
  const socket = new FakeSocket();
  const client = new WebSocketWorkspaceClient(
    "ws://127.0.0.1:49152/kernel",
    () => socket as unknown as WebSocket,
    fetcher,
  );
  const connection = client.connect();
  socket.open();
  await connection;
  return { client, socket };
}

function reply(
  socket: FakeSocket,
  ok: boolean,
  resultOrError: Record<string, unknown>,
): void {
  const request = socket.lastRequest();
  socket.respond({
    protocol: "openmat-workspace-v3",
    requestId: request.requestId,
    ok,
    ...(ok ? { result: resultOrError } : { error: resultOrError }),
  });
}

const fileEntry = {
  name: "seed.m",
  path: "src/seed.m",
  kind: "file",
  size: 11,
  revision: "fnv1a64-abc-11",
} as const;

describe("WebSocketWorkspaceClient", () => {
  it("derives the separate workspace-v3 endpoint from the kernel URL", () => {
    expect(
      workspaceUrlFromKernelUrl("ws://127.0.0.1:49152/kernel?ignored=yes"),
    ).toBe("ws://127.0.0.1:49152/workspace/v3");
    expect(
      downloadUrlFromWorkspaceUrl(
        "wss://openmat.example/workspace/v3",
        "a".repeat(64),
      ),
    ).toBe(`https://openmat.example/workspace/download/${"a".repeat(64)}`);
    expect(
      uploadUrlFromWorkspaceUrl(
        "ws://127.0.0.1:49152/workspace/v3",
        "b".repeat(64),
      ),
    ).toBe(`http://127.0.0.1:49152/workspace/upload/${"b".repeat(64)}`);
  });

  it("changes Current Folder and publishes independent directory events", async () => {
    const { client, socket } = await connectedClient();
    const observed: unknown[] = [];
    const observedChanges: unknown[] = [];
    client.onCurrentDirectoryChanged((directory) => observed.push(directory));
    client.onWorkspaceChanged((change) => observedChanges.push(change));

    const current = client.currentDirectory();
    expect(socket.lastRequest()).toMatchObject({
      request: { type: "currentDirectory", params: {} },
    });
    reply(socket, true, {
      type: "currentDirectory",
      data: { path: "C:\\first", rootName: "first", generation: 1 },
    });
    await expect(current).resolves.toEqual({
      path: "C:\\first",
      rootName: "first",
      generation: 1,
    });

    const browsing = client.browseDirectories("C:\\first");
    expect(socket.lastRequest()).toMatchObject({
      request: { type: "browseDirectories", params: { path: "C:\\first" } },
    });
    reply(socket, true, {
      type: "browseDirectories",
      data: {
        path: "C:\\first",
        parentPath: "C:\\",
        roots: [
          { name: "C:", path: "C:\\" },
          { name: "D:", path: "D:\\" },
        ],
        entries: [{ name: "nested", path: "C:\\first\\nested" }],
      },
    });
    await expect(browsing).resolves.toEqual({
      path: "C:\\first",
      parentPath: "C:\\",
      roots: [
        { name: "C:", path: "C:\\" },
        { name: "D:", path: "D:\\" },
      ],
      entries: [{ name: "nested", path: "C:\\first\\nested" }],
    });

    const changed = client.changeDirectory("D:\\work");
    expect(socket.lastRequest()).toMatchObject({
      request: { type: "changeDirectory", params: { path: "D:\\work" } },
    });
    reply(socket, true, {
      type: "changeDirectory",
      data: { path: "D:\\work", rootName: "work", generation: 2 },
    });
    await changed;
    socket.respond({
      protocol: "openmat-workspace-v3",
      event: {
        type: "currentDirectoryChanged",
        data: { path: "E:\\scripts", rootName: "scripts", generation: 3 },
      },
    });
    expect(observed).toEqual([
      { path: "E:\\scripts", rootName: "scripts", generation: 3 },
    ]);
    socket.respond({
      protocol: "openmat-workspace-v3",
      event: {
        type: "workspaceChanged",
        data: { rootGeneration: 3 },
      },
    });
    expect(observedChanges).toEqual([{ rootGeneration: 3 }]);
  });

  it("lists, reads, downloads, and writes through workspace-v3 frames", async () => {
    const { client, socket } = await connectedClient();
    const listing = client.list("", true);
    expect(socket.lastRequest()).toMatchObject({
      protocol: "openmat-workspace-v3",
      request: { type: "list", params: { path: "", recursive: true } },
    });
    reply(socket, true, {
      type: "list",
      data: {
        rootName: "project",
        rootPath: "C:\\project",
        rootGeneration: 7,
        path: "",
        recursive: true,
        entries: [fileEntry],
      },
    });
    await expect(listing).resolves.toMatchObject({
      rootName: "project",
      rootPath: "C:\\project",
      rootGeneration: 7,
      entries: [fileEntry],
    });

    const reading = client.read("src/seed.m");
    reply(socket, true, {
      type: "read",
      data: {
        path: "src/seed.m",
        content: "seed = 1;\n",
        revision: "fnv1a64-abc-10",
        size: 10,
        rootPath: "C:\\project",
        rootGeneration: 7,
      },
    });
    await expect(reading).resolves.toMatchObject({
      path: "src/seed.m",
      content: "seed = 1;\n",
    });

    const preparing = client.prepareDownload("src/seed.m", 7);
    expect(socket.lastRequest()).toMatchObject({
      request: {
        type: "prepareDownload",
        params: { path: "src/seed.m", rootGeneration: 7 },
      },
    });
    reply(socket, true, {
      type: "prepareDownload",
      data: {
        ticket: "a".repeat(64),
        name: "seed.m",
        size: 11,
        expiresInSeconds: 60,
      },
    });
    await expect(preparing).resolves.toEqual({
      url: `http://127.0.0.1:49152/workspace/download/${"a".repeat(64)}`,
      name: "seed.m",
      size: 11,
      expiresInSeconds: 60,
    });

    const writing = client.write(
      "src/seed.m",
      "seed = 2;\n",
      "fnv1a64-abc-10",
      7,
    );
    expect(socket.lastRequest()).toMatchObject({
      request: {
        type: "write",
        params: {
          path: "src/seed.m",
          content: "seed = 2;\n",
          expectedRevision: "fnv1a64-abc-10",
          rootGeneration: 7,
        },
      },
    });
    reply(socket, true, { type: "write", data: { entry: fileEntry } });
    await expect(writing).resolves.toEqual(fileEntry);
  });

  it("prepares an upload ticket and puts the File body over HTTP", async () => {
    const fetcher = vi.fn(async () =>
      ({
        ok: true,
        status: 201,
        json: async () => ({ entry: { ...fileEntry, name: "data.mat", path: "data.mat" } }),
      }) as Response,
    );
    const { client, socket } = await connectedClient(fetcher);
    const file = new File([new Uint8Array([0, 1, 255])], "data.mat");
    const uploading = client.upload("data.mat", file, 7);
    expect(socket.lastRequest()).toMatchObject({
      request: {
        type: "prepareUpload",
        params: { path: "data.mat", size: 3, rootGeneration: 7, overwrite: false },
      },
    });
    reply(socket, true, {
      type: "prepareUpload",
      data: {
        ticket: "b".repeat(64),
        name: "data.mat",
        size: 3,
        expiresInSeconds: 60,
      },
    });

    await expect(uploading).resolves.toMatchObject({ path: "data.mat", size: 11 });
    expect(fetcher).toHaveBeenCalledWith(
      `http://127.0.0.1:49152/workspace/upload/${"b".repeat(64)}`,
      expect.objectContaining({
        method: "PUT",
        body: file,
        headers: { "Content-Type": "application/octet-stream" },
      }),
    );
  });

  it("manages search paths and publishes path-change events", async () => {
    const { client, socket } = await connectedClient();
    const observed: unknown[] = [];
    client.onSearchPathChanged?.((snapshot) => observed.push(snapshot));

    const adding = client.addSearchPath?.("toolbox", true, "begin");
    expect(socket.lastRequest()).toMatchObject({
      request: {
        type: "addSearchPath",
        params: { path: "toolbox", recursive: true, position: "begin" },
      },
    });
    const snapshot = {
      generation: 1,
      directories: [
        { path: "C:\\project\\toolbox", workspacePath: "toolbox" },
        { path: "C:\\project\\toolbox\\solver", workspacePath: "toolbox/solver" },
      ],
    };
    reply(socket, true, { type: "addSearchPath", data: snapshot });
    await expect(adding).resolves.toEqual(snapshot);

    socket.respond({
      protocol: "openmat-workspace-v3",
      event: { type: "searchPathChanged", data: { ...snapshot, generation: 2 } },
    });
    expect(observed).toEqual([{ ...snapshot, generation: 2 }]);

    const removing = client.removeSearchPath?.("toolbox", true);
    expect(socket.lastRequest()).toMatchObject({
      request: {
        type: "removeSearchPath",
        params: { path: "toolbox", recursive: true },
      },
    });
    reply(socket, true, {
      type: "removeSearchPath",
      data: { generation: 3, directories: [] },
    });
    await expect(removing).resolves.toEqual({ generation: 3, directories: [] });
  });

  it("creates, renames, moves, and sends explicit recursive delete confirmation", async () => {
    const { client, socket } = await connectedClient();
    const creation = client.create("src/new.m", "file");
    reply(socket, true, {
      type: "create",
      data: { entry: { ...fileEntry, name: "new.m", path: "src/new.m" } },
    });
    await creation;

    const rename = client.rename("src/new.m", "renamed.m");
    reply(socket, true, {
      type: "rename",
      data: {
        previousPath: "src/new.m",
        entry: { ...fileEntry, name: "renamed.m", path: "src/renamed.m" },
      },
    });
    await expect(rename).resolves.toMatchObject({
      previousPath: "src/new.m",
      entry: { path: "src/renamed.m" },
    });

    const move = client.move("src/renamed.m", "renamed.m");
    reply(socket, true, {
      type: "move",
      data: {
        previousPath: "src/renamed.m",
        entry: { ...fileEntry, name: "renamed.m", path: "renamed.m" },
      },
    });
    await move;

    const deletion = client.delete("src", true);
    expect(socket.lastRequest()).toMatchObject({
      request: {
        type: "delete",
        params: { path: "src", recursive: true, confirmPath: "src" },
      },
    });
    reply(socket, true, {
      type: "delete",
      data: { path: "src", kind: "directory", recursive: true },
    });
    await expect(deletion).resolves.toEqual({
      path: "src",
      kind: "directory",
      recursive: true,
    });
  });

  it("preserves structured conflict details", async () => {
    const { client, socket } = await connectedClient();
    const writing = client.write("seed.m", "local", "stale", 1);
    reply(socket, false, {
      code: "workspace.revisionConflict",
      message: "seed.m changed outside OpenMat.",
      field: "expectedRevision",
      details: { currentRevision: "current" },
    });
    await expect(writing).rejects.toMatchObject({
      code: "workspace.revisionConflict",
      field: "expectedRevision",
      details: { currentRevision: "current" },
    });
  });

  it("binds a deferred source rename to its original workspace generation", async () => {
    const { client, socket } = await connectedClient();
    const renaming = client.rename("helper.m", "calculate.m", 7);
    expect(socket.lastRequest()).toMatchObject({ request: {
      type: "rename", params: { path: "helper.m", newName: "calculate.m", rootGeneration: 7 },
    } });
    reply(socket, true, { type: "rename", data: {
      previousPath: "helper.m", entry: { ...fileEntry, path: "calculate.m", name: "calculate.m" },
    } });
    await expect(renaming).resolves.toMatchObject({ entry: { path: "calculate.m" } });
  });

  it("reports unexpected post-connect socket closure exactly once", async () => {
    const { client, socket } = await connectedClient();
    const losses: Error[] = [];
    client.subscribeConnectionLoss((error) => losses.push(error));

    socket.close(1006);

    expect(losses).toHaveLength(1);
    expect(losses[0]?.message).toContain("closed (1006)");
    await client.disconnect();
    expect(losses).toHaveLength(1);
  });

  it("reports protocol failure but not an intentional disconnect", async () => {
    const first = await connectedClient();
    const protocolLosses: Error[] = [];
    first.client.subscribeConnectionLoss((error) => protocolLosses.push(error));
    first.socket.respond({ protocol: "unexpected" });
    expect(protocolLosses).toHaveLength(1);
    expect(protocolLosses[0]?.message).toContain("envelope is malformed");

    const second = await connectedClient();
    const intentionalLosses: Error[] = [];
    second.client.subscribeConnectionLoss((error) => intentionalLosses.push(error));
    await second.client.disconnect();
    expect(intentionalLosses).toEqual([]);
  });
});
