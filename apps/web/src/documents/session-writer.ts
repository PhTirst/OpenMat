import type { PersistedDocumentSession } from "./document-session";
import type { DocumentSessionStore } from "./document-session-storage";

/** An older, slower autosave must not overwrite the final exit snapshot. */
export class SessionWriter {
  #pending: Promise<void> | null = null;

  constructor(private readonly store: DocumentSessionStore, private readonly key: string) {}

  save(session: PersistedDocumentSession): Promise<void> {
    const write = () => this.store.save(this.key, session);
    const pending = this.#pending === null ? write() : this.#pending.then(write, write);
    this.#pending = pending;
    const finished = () => { if (this.#pending === pending) this.#pending = null; };
    void pending.then(finished, finished);
    return pending;
  }
}
