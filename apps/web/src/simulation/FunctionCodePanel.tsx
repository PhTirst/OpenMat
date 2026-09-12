import { lazy, Suspense } from "react";
import type { DesignerSourceWorkspace } from "../documents/designer-source-workspace";
import type { IdeTheme } from "../theme";
const CodeEditor = lazy(() => import("../components/CodeEditor"));
export function FunctionCodePanel({
    path,
    shared,
    theme,
    reveal,
    onRun,
    onSave,
}: {
    path: string | null;
    shared?: DesignerSourceWorkspace | undefined;
    theme: IdeTheme;
    reveal?:
        | { lineNumber: number; column: number; requestId: number }
        | undefined;
    onRun(): void;
    onSave(): void;
}) {
    const doc = path ? shared?.getSource(path) : undefined;
    if (!doc || !shared)
        return (
            <p className="sim-help">
                双击 M Function 方块，或在自定义组件的检查器中选择一个 m 回调。
            </p>
        );
    return (
        <div className="sim-function-code">
            <div className="sim-source-title">
                <span>
                    {doc.path}
                    {doc.content !== doc.savedContent || !doc.revision
                        ? " ●"
                        : ""}
                </span>
                <button onClick={onSave}>保存模型与源码</button>
            </div>
            <Suspense fallback={<p>加载代码编辑器…</p>}>
                <CodeEditor
                    value={doc.content}
                    theme={theme}
                    documentId={doc.id}
                    documentPath={doc.path}
                    documentUri={doc.uri}
                    documentVersion={doc.version}
                    viewState={doc.viewState}
                    onViewStateChange={() => {}}
                    onChange={(content) => shared.updateSource(doc.id, content)}
                    onRun={onRun}
                    onSave={onSave}
                    lspUrl={shared.lspUrl}
                    workspaceDocuments={shared.documents}
                    editorSession={shared.editorSession}
                    onWorkspaceDocumentChange={shared.updateSource}
                    onOpenDocument={shared.openDocument}
                    {...(reveal ? { reveal } : {})}
                />
            </Suspense>
        </div>
    );
}
