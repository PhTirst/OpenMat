import type * as Monaco from "monaco-editor/esm/vs/editor/editor.api";
import { describe, expect, it, vi } from "vitest";
import { WorkspaceEditorModels } from "./workspace-editor-models";

function harness(normalizeText = (text: string) => text) {
  const models = new Map<string, Monaco.editor.ITextModel>();
  const api = {
    Uri: { parse: (uri: string) => ({ toString: () => uri }) },
    editor: {
      getModel: (uri: Monaco.Uri) => models.get(uri.toString()) ?? null,
      setModelLanguage: vi.fn(),
      createModel: vi.fn((initial: string, _language: string, uri: Monaco.Uri) => {
        let text = normalizeText(initial);
        let disposed = false;
        const listeners = new Set<() => void>();
        const undoValues: string[] = [];
        const model = {
          uri,
          getValue: () => text,
          getLanguageId: () => "openmat",
          isDisposed: () => disposed,
          getFullModelRange: () => ({ startLineNumber: 1, startColumn: 1, endLineNumber: 1, endColumn: text.length + 1 }),
          pushStackElement: vi.fn(),
          pushEditOperations: vi.fn((_selection: unknown, edits: readonly { text: string }[]) => {
            undoValues.push(text);
            text = normalizeText(edits[0]!.text);
            listeners.forEach((listener) => listener());
          }),
          undo: () => {
            const previous = undoValues.pop();
            if (previous !== undefined) {
              text = previous;
              listeners.forEach((listener) => listener());
            }
          },
          setValue: vi.fn((value: string) => {
            text = normalizeText(value);
            listeners.forEach((listener) => listener());
          }),
          onDidChangeContent: (listener: () => void) => {
            listeners.add(listener);
            return { dispose: () => listeners.delete(listener) };
          },
          dispose: vi.fn(() => {
            disposed = true;
            models.delete(uri.toString());
          }),
        } as unknown as Monaco.editor.ITextModel;
        models.set(uri.toString(), model);
        return model;
      }),
    },
  } as unknown as typeof import("monaco-editor/esm/vs/editor/editor.api");
  return { api, models };
}

const main = { id: "main", uri: "file:///main.m", content: "answer = helper(1);", version: 1 };
const helper = { id: "helper", uri: "file:///helper.m", content: "function y = helper(x)\ny = x;\nend", version: 2 };

describe("WorkspaceEditorModels", () => {
  it("keeps inactive models and reports edits against their own document IDs", () => {
    const { api, models } = harness();
    const changed = vi.fn();
    const owner = new WorkspaceEditorModels(api, changed);
    owner.sync([main, helper]);
    const helperModel = models.get(helper.uri)!;
    const renamed = helper.content.replace("helper", "calculate");

    helperModel.setValue(renamed);
    expect(changed).toHaveBeenCalledExactlyOnceWith("helper", renamed);
    owner.sync([main, { ...helper, content: renamed, version: 3 }]);
    expect(api.editor.createModel).toHaveBeenCalledTimes(2);
    expect(helperModel.setValue).toHaveBeenCalledOnce();
    expect(owner.models[1]).toEqual({ model: helperModel, documentVersion: 3 });
    owner.dispose();
  });

  it("does not echo external content replacement as a fresh user edit", () => {
    const { api, models } = harness();
    const changed = vi.fn();
    const owner = new WorkspaceEditorModels(api, changed);
    owner.sync([main]);
    owner.sync([{ ...main, content: "reloaded = 2;", version: 2 }]);
    expect(models.get(main.uri)?.getValue()).toBe("reloaded = 2;");
    expect(changed).not.toHaveBeenCalled();
    owner.dispose();
  });

  it("keeps generated source changes in the model undo history across document synchronization", () => {
    const { api, models } = harness();
    const changed = vi.fn();
    const owner = new WorkspaceEditorModels(api, changed);
    owner.sync([main, helper]);
    const model = models.get(helper.uri)!;
    const generated = `${helper.content}\n% generated callback`;
    expect(owner.editDocument(helper.id, generated)).toBe(true);
    expect(changed).toHaveBeenCalledExactlyOnceWith(helper.id, generated);
    owner.sync([main, { ...helper, content: generated, version: 3 }]);
    expect(model.setValue).not.toHaveBeenCalled();
    (model as unknown as { undo: () => void }).undo();
    expect(model.getValue()).toBe(helper.content);
    expect(changed).toHaveBeenLastCalledWith(helper.id, helper.content);
    expect(owner.editDocument("unopened", "changed")).toBe(false);
    owner.dispose();
  });

  it("preserves a normalized mixed-EOL model when revisiting unchanged source", () => {
    const { api, models } = harness((text) => text.replace(/\r\n?/g, "\n"));
    const changed = vi.fn();
    const owner = new WorkspaceEditorModels(api, changed);
    const mixed = { ...helper, content: "function y = helper(x)\ny = x;\r\nend\r\n" };
    owner.sync([main, mixed]);
    const model = models.get(helper.uri)!;
    owner.sync([mixed, main]);
    owner.sync([main, mixed]);
    expect(model.setValue).not.toHaveBeenCalled();
    expect(changed).not.toHaveBeenCalled();
    model.setValue("function y = helper(x)\ny = x + 1;\nend\n");
    expect(changed).toHaveBeenCalledExactlyOnceWith(helper.id, model.getValue());
    owner.dispose();
  });

  it("releases only the closed document and supports rename to a new URI", async () => {
    const { api, models } = harness();
    const changed = vi.fn();
    const owner = new WorkspaceEditorModels(api, changed);
    owner.sync([main, helper]);
    const oldHelper = models.get(helper.uri)!;
    const mainModel = models.get(main.uri)!;
    owner.sync([main, { ...helper, id: "renamed", uri: "file:///renamed.m" }]);
    await Promise.resolve();
    expect(oldHelper.isDisposed()).toBe(true);
    expect(mainModel.isDisposed()).toBe(false);
    models.get("file:///renamed.m")!.setValue("edited");
    expect(changed).toHaveBeenCalledWith("renamed", "edited");
    owner.dispose();
    await Promise.resolve();
    expect(models.size).toBe(0);
  });

  it("keeps a shared source alive until its last owner closes", async () => {
    const { api, models } = harness();
    const workbench = new WorkspaceEditorModels(api, vi.fn());
    const designer = new WorkspaceEditorModels(api, vi.fn());
    workbench.sync([main]);
    designer.sync([main]);
    const model = models.get(main.uri)!;
    workbench.dispose();
    await Promise.resolve();
    expect(model.isDisposed()).toBe(false);
    designer.dispose();
    // StrictMode effect replay can immediately reacquire the source.
    workbench.sync([main]);
    await Promise.resolve();
    expect(model.isDisposed()).toBe(false);
    workbench.dispose();
    await Promise.resolve();
    expect(model.isDisposed()).toBe(true);
  });
});
