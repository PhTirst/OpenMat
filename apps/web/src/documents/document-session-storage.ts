import {
  parseDocumentSession,
  type PersistedDocumentSession,
} from "./document-session";

const DATABASE_NAME = "openmat-web";
const DATABASE_VERSION = 1;
const SESSION_STORE = "document-sessions";

interface StoredSession {
  readonly key: string;
  readonly session: PersistedDocumentSession;
}

export interface DocumentSessionStore {
  load(key: string): Promise<PersistedDocumentSession | null>;
  save(key: string, session: PersistedDocumentSession): Promise<void>;
  clear(key: string): Promise<void>;
}

export class IndexedDbDocumentSessionStore implements DocumentSessionStore {
  readonly #indexedDb: IDBFactory | undefined;
  #database: Promise<IDBDatabase> | null = null;

  constructor(indexedDb: IDBFactory | undefined = globalThis.indexedDB) {
    this.#indexedDb = indexedDb;
  }

  async load(key: string): Promise<PersistedDocumentSession | null> {
    if (this.#indexedDb === undefined) {
      return null;
    }
    const database = await this.#open();
    const record = await requestResult<StoredSession | undefined>(
      database.transaction(SESSION_STORE, "readonly").objectStore(SESSION_STORE).get(key),
    );
    return parseDocumentSession(record?.session);
  }

  async save(key: string, session: PersistedDocumentSession): Promise<void> {
    if (this.#indexedDb === undefined) {
      return;
    }
    const database = await this.#open();
    const transaction = database.transaction(SESSION_STORE, "readwrite");
    transaction.objectStore(SESSION_STORE).put({ key, session } satisfies StoredSession);
    await transactionCompletion(transaction);
  }

  async clear(key: string): Promise<void> {
    if (this.#indexedDb === undefined) {
      return;
    }
    const database = await this.#open();
    const transaction = database.transaction(SESSION_STORE, "readwrite");
    transaction.objectStore(SESSION_STORE).delete(key);
    await transactionCompletion(transaction);
  }

  #open(): Promise<IDBDatabase> {
    if (this.#database !== null) {
      return this.#database;
    }
    const indexedDb = this.#indexedDb;
    if (indexedDb === undefined) {
      return Promise.reject(new Error("IndexedDB is unavailable."));
    }
    this.#database = new Promise((resolve, reject) => {
      const request = indexedDb.open(DATABASE_NAME, DATABASE_VERSION);
      request.addEventListener("upgradeneeded", () => {
        if (!request.result.objectStoreNames.contains(SESSION_STORE)) {
          request.result.createObjectStore(SESSION_STORE, { keyPath: "key" });
        }
      });
      request.addEventListener("success", () => resolve(request.result));
      request.addEventListener("error", () =>
        reject(request.error ?? new Error("Could not open document recovery storage.")),
      );
      request.addEventListener("blocked", () =>
        reject(new Error("Document recovery storage upgrade is blocked.")),
      );
    });
    return this.#database;
  }
}

export function documentSessionKey(configuredUrl: string | undefined, platform: "web" | "desktop" = "web", workspaceIdentity?: string): string {
  // A desktop application's identity survives edits to its kernel port.
  if (platform === "desktop") return workspaceIdentity === undefined
    ? "workspace:desktop:v1" : `workspace:desktop:v1:${workspaceIdentity}`;
  const raw = configuredUrl?.trim();
  if (raw === undefined || raw.length === 0) {
    return "workspace:mock";
  }
  try {
    const url = new URL(raw);
    url.username = "";
    url.password = "";
    url.search = "";
    url.hash = "";
    return `workspace:${url.toString()}`;
  } catch {
    return `workspace:${raw}`;
  }
}

function requestResult<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.addEventListener("success", () => resolve(request.result));
    request.addEventListener("error", () =>
      reject(request.error ?? new Error("Document recovery storage request failed.")),
    );
  });
}

function transactionCompletion(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.addEventListener("complete", () => resolve());
    transaction.addEventListener("abort", () =>
      reject(transaction.error ?? new Error("Document recovery transaction aborted.")),
    );
    transaction.addEventListener("error", () =>
      reject(transaction.error ?? new Error("Document recovery transaction failed.")),
    );
  });
}
