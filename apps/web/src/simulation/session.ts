import type { DesignerSession } from "../designer/designer-session";
export interface ModelEditorSession extends DesignerSession {
    /** Resolve the editor's save/discard dialog before changing its workspace root. */
    prepareSwitch(): Promise<boolean>;
}
