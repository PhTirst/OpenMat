import type { CodeEditorProps } from "../components/CodeEditor";
import type { WorkspaceFile } from "../workspace/workspace-client";
import type { OpenDocument } from "./document-session";

export interface DesignerSourceSeed {
  readonly path: string;
  readonly content: string;
  readonly savedContent: string;
  readonly file: WorkspaceFile | null;
}

/** The workbench owns source drafts; the Designer owns the visual document. */
export interface DesignerSourceWorkspace {
  readonly documents: readonly OpenDocument[];
  readonly editorSession: NonNullable<CodeEditorProps["editorSession"]>;
  readonly lspUrl: string | null;
  readonly getSource: (path: string) => OpenDocument | undefined;
  /** Existing editor drafts win over a disk read or restored Designer draft. */
  readonly ensureSource: (source: DesignerSourceSeed) => OpenDocument;
  readonly updateSource: (id: string, content: string) => void;
  /** Only advances the saved baseline; edits made during a write survive. */
  readonly acceptSaved: (file: WorkspaceFile) => void;
  readonly openDocument: NonNullable<CodeEditorProps["onOpenDocument"]>;
}
