/** Workbench exit hooks for the mounted designer, including when its pane is hidden. */
export interface DesignerSession {
  readonly path: string;
  readonly dirty: boolean;
  readonly hasSavedDesign: boolean;
  readonly busy: boolean;
  readonly error: string | null;
  save(): Promise<boolean>;
  flush(discard: boolean): void;
  resume(): void;
}
