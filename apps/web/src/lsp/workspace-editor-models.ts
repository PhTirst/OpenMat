import type * as Monaco from "monaco-editor/esm/vs/editor/editor.api";

type MonacoApi = typeof import("monaco-editor/esm/vs/editor/editor.api");

export interface WorkspaceEditorDocument {
  readonly id: string;
  readonly uri: string;
  readonly content: string;
  readonly version: number;
}

interface RetainedDocument {
  document: WorkspaceEditorDocument;
  readonly model: Monaco.editor.ITextModel;
  readonly subscription: Monaco.IDisposable;
}

// Multiple editor mounts may hold the same URI. A model survives until every
// editor releases it; Monaco's React wrapper must keepCurrentModel.
const owners = new WeakMap<Monaco.editor.ITextModel, { count: number }>();

function retain(model: Monaco.editor.ITextModel): void {
  const owner = owners.get(model) ?? { count: 0 };
  owner.count += 1;
  owners.set(model, owner);
}

function release(model: Monaco.editor.ITextModel): void {
  const owner = owners.get(model);
  if (owner === undefined) return;
  owner.count -= 1;
  // Let the editor detach and the LSP send didClose first. This also permits
  // React StrictMode to reacquire the same model during its effect replay.
  queueMicrotask(() => {
    if (owner.count === 0 && !model.isDisposed()) {
      owners.delete(model);
      model.dispose();
    }
  });
}

function sameModelText(modelText: string, sourceText: string): boolean {
  // Monaco normalizes mixed line endings on load. Merely revisiting that model
  // must not reset its undo stack or turn the original file into a dirty draft.
  return modelText === sourceText ||
    modelText.replace(/\r\n?/g, "\n") === sourceText.replace(/\r\n?/g, "\n");
}

/** Owns every open document model, including edits to inactive rename targets. */
export class WorkspaceEditorModels {
  readonly #documents = new Map<string, RetainedDocument>();
  #syncing = false;

  constructor(
    private readonly monaco: MonacoApi,
    private readonly onChange: (id: string, content: string) => void,
  ) {}

  sync(documents: readonly WorkspaceEditorDocument[]): void {
    this.#syncing = true;
    try {
      const desired = new Set<string>();
      for (const document of documents) {
        const uri = this.monaco.Uri.parse(document.uri);
        const key = uri.toString();
        desired.add(key);
        let retained = this.#documents.get(key);
        if (retained === undefined) {
          const model = this.monaco.editor.getModel(uri) ??
            this.monaco.editor.createModel(document.content, "openmat", uri);
          if (model.getLanguageId() !== "openmat") {
            this.monaco.editor.setModelLanguage(model, "openmat");
          }
          retain(model);
          retained = {
            document,
            model,
            subscription: model.onDidChangeContent(() => {
              const current = this.#documents.get(key);
              if (!this.#syncing && current !== undefined) {
                this.onChange(current.document.id, model.getValue());
              }
            }),
          };
          this.#documents.set(key, retained);
        }
        retained.document = document;
        // Ordinary typing and multi-file refactors already updated the model.
        // Only external reload/recovery should replace its contents here.
        if (!sameModelText(retained.model.getValue(), document.content)) {
          retained.model.setValue(document.content);
        }
      }
      for (const [key, retained] of this.#documents) {
        if (!desired.has(key)) {
          retained.subscription.dispose();
          this.#documents.delete(key);
          release(retained.model);
        }
      }
    } finally {
      this.#syncing = false;
    }
  }

  get models(): readonly {
    readonly model: Monaco.editor.ITextModel;
    readonly documentVersion: number;
  }[] {
    return [...this.#documents.values()].map(({ model, document }) => ({
      model,
      documentVersion: document.version,
    }));
  }

  /** Generated source changes participate in the same undo stack as typing. */
  editDocument(documentId: string, content: string): boolean {
    const retained = [...this.#documents.values()].find(({ document }) => document.id === documentId);
    if (retained === undefined) return false;
    const model = retained.model;
    if (sameModelText(model.getValue(), content)) return true;
    model.pushStackElement();
    model.pushEditOperations(null, [{ range: model.getFullModelRange(), text: content }], () => null);
    model.pushStackElement();
    return true;
  }

  dispose(): void {
    for (const retained of this.#documents.values()) {
      retained.subscription.dispose();
      release(retained.model);
    }
    this.#documents.clear();
  }
}
