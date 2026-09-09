/** Tracks complete save workflows, including the state updates after a write. */
export class PendingOperations {
  readonly #pending = new Set<Promise<unknown>>();

  run<T>(operation: () => Promise<T>): Promise<T> {
    const pending = operation();
    this.#pending.add(pending);
    const finished = () => { this.#pending.delete(pending); };
    void pending.then(finished, finished);
    return pending;
  }

  async wait(): Promise<void> {
    while (this.#pending.size > 0) await Promise.allSettled([...this.#pending]);
  }
}
