import type * as Monaco from "monaco-editor/esm/vs/editor/editor.api";
import { OpenMatMonacoLspWorkspace, type OpenMatFileRename } from "./monaco-lsp";
import { WorkspaceEditorModels, type WorkspaceEditorDocument } from "./workspace-editor-models";

type MonacoApi = typeof import("monaco-editor/esm/vs/editor/editor.api");

export interface SharedEditorSessionOptions {
  readonly url?: string | null;
  readonly onChange: (documentId: string, content: string) => void;
  /** Resolve only after documents and their current models have been synchronized. */
  readonly ensureDocuments?: (uris: readonly string[]) => Promise<boolean>;
  readonly stageFileRenames?: (renames: readonly OpenMatFileRename[]) => boolean;
}

/** App-owned model and language session; editor surfaces are only views. */
export class SharedEditorSession {
  #options: SharedEditorSessionOptions = { onChange: () => undefined };
  #documents: readonly WorkspaceEditorDocument[] = [];
  #monaco: MonacoApi | null = null;
  #models: WorkspaceEditorModels | null = null;
  #workspace: OpenMatMonacoLspWorkspace | null = null;

  configure(options: SharedEditorSessionOptions): void {
    const urlChanged = this.#options.url !== options.url;
    this.#options = options;
    if (urlChanged) this.connectWorkspace();
  }

  syncDocuments(documents: readonly WorkspaceEditorDocument[]): void {
    this.#documents = documents;
    this.#models?.sync(documents);
    if (this.#models !== null) this.#workspace?.syncModels(this.#models.models);
  }

  editDocument(documentId: string, content: string): boolean {
    return this.#models?.editDocument(documentId, content) ?? false;
  }

  /** Lazy initialization avoids importing Monaco into the App's initial bundle. */
  attach(monaco: MonacoApi): void {
    if (this.#monaco !== null) {
      if (this.#monaco !== monaco) throw new Error("A shared editor session requires one Monaco runtime");
      return;
    }
    this.#monaco = monaco;
    this.#models = new WorkspaceEditorModels(monaco, (id, content) => {
      this.#options.onChange(id, content);
    });
    this.#models.sync(this.#documents);
    this.connectWorkspace();
  }

  private connectWorkspace(): void {
    this.#workspace?.dispose();
    this.#workspace = null;
    if (this.#monaco === null || this.#models === null || !this.#options.url) return;
    this.#workspace = new OpenMatMonacoLspWorkspace(this.#monaco, {
      url: this.#options.url,
      ensureDocuments: async (uris) => this.#options.ensureDocuments?.(uris) ?? false,
      stageFileRenames: (renames) => this.#options.stageFileRenames?.(renames) ?? false,
    }).start();
    this.#workspace.syncModels(this.#models.models);
  }

  /** The App owns disposal, so switching views retains models and undo history. */
  dispose(): void {
    this.#workspace?.dispose();
    this.#workspace = null;
    this.#models?.dispose();
    this.#models = null;
    this.#monaco = null;
  }
}
