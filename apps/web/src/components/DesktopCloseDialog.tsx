import { useEffect, useRef, useState } from "react";

export interface DesktopCloseActions {
  waitForWrites(): Promise<void>;
  unsavedNames(): readonly string[];
  saveAll(): Promise<void>;
  finish(discard: boolean): Promise<void>;
  resume(): void;
}

export function DesktopCloseDialog({ actions, onClosed }: {
  readonly actions: DesktopCloseActions;
  readonly onClosed: () => void;
}) {
  const [names, setNames] = useState<readonly string[]>([]);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const closed = useRef(onClosed);
  closed.current = onClosed;

  useEffect(() => {
    let disposed = false;
    void actions.waitForWrites().then(async () => {
      if (disposed) return;
      const unsaved = actions.unsavedNames();
      setNames(unsaved);
      if (unsaved.length > 0) { setBusy(false); return; }
      await actions.finish(false);
      if (!disposed) closed.current();
    }).catch((failure: unknown) => {
      if (disposed) return;
      actions.resume();
      setError(failure instanceof Error ? failure.message : String(failure));
      setBusy(false);
    });
    return () => { disposed = true; };
  }, [actions]);

  const cancel = () => {
    if (busy) return;
    actions.resume();
    onClosed();
  };
  const proceed = async (discard: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    try {
      await actions.waitForWrites();
      if (!discard) await actions.saveAll();
      await actions.finish(discard);
      onClosed();
    } catch (failure) {
      actions.resume();
      setNames(actions.unsavedNames());
      setError(failure instanceof Error ? failure.message : String(failure));
      setBusy(false);
    } finally {
      inFlight.current = false;
    }
  };
  return (
    <section className="operation-dialog compact" role="dialog" aria-modal="true" tabIndex={-1}
      aria-labelledby="desktop-close-title" aria-describedby="desktop-close-description"
      onKeyDown={(event) => { if (event.key === "Escape") { event.stopPropagation(); cancel(); } }}>
      <header><div>
        <strong id="desktop-close-title" tabIndex={-1} data-dialog-initial-focus>Close OpenMat</strong>
        <span id="desktop-close-description">Save changes in the editor and App Designer before closing?</span>
      </div></header>
      <div className="operation-dialog-body">
        {names.length > 0 && <ul>{names.map((name) => <li key={name}><code>{name}</code></li>)}</ul>}
        {busy && <p role="status">Waiting for saves and recovery data…</p>}
        {error !== null && <p role="alert">{error} OpenMat will stay open.</p>}
      </div>
      <footer className="operation-dialog-actions spread">
        <button className="button button-danger-text" type="button" disabled={busy} onClick={() => void proceed(true)}>Don’t Save</button>
        <span>
          <button className="button button-secondary" type="button" disabled={busy} onClick={cancel}>Cancel</button>
          <button className="button button-primary" type="button" disabled={busy} onClick={() => void proceed(false)}>Save All</button>
        </span>
      </footer>
    </section>
  );
}
