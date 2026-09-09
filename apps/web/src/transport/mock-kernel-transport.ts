import {
  type EventDataByType,
  type KernelEventType,
  type VariableSummary,
} from "../protocol/kernel-v0";
import {
  KERNEL_PROTOCOL_V0,
  KERNEL_PROTOCOL_V1,
  KERNEL_PROTOCOL_V2,
  KERNEL_PROTOCOL_V3,
  MAX_AGGREGATE_DEPTH,
  MAX_AGGREGATE_ELEMENTS,
  MAX_AGGREGATE_NODES,
  MAX_PREVIEW_CODE_UNITS,
  MAX_PREVIEW_ELEMENTS,
  MAX_STRING_ELEMENT_CODE_UNITS,
  SUPPORTED_KERNEL_PROTOCOLS,
  hasValidV2Capabilities,
  validateBootstrapInitializeParams,
  type KernelEvent,
  type KernelFailureResponse,
  type KernelProtocol,
  type KernelRequest,
  type KernelRequestType,
  type KernelSuccessResponse,
  type ResponseFor,
} from "../protocol/kernel-v2";
import { hasValidV1Capabilities } from "../protocol/kernel-v1";
import type {
  KernelConnectionLossListener,
  KernelEventListener,
  KernelTransport,
} from "./kernel-transport";

const mockVariables: readonly VariableSummary[] = [
  {
    name: "x",
    class: "double",
    dimensions: [1, 1],
    complex: false,
    bytes: 8,
  },
  {
    name: "y",
    class: "double",
    dimensions: [1, 1],
    complex: false,
    bytes: 8,
  },
];

interface MockKernelOptions {
  readonly eventDelayMs?: number;
  readonly supportedProtocols?: readonly KernelProtocol[];
}

type SuccessData<T extends KernelRequestType> = Extract<
  KernelSuccessResponse<T>,
  { readonly result: { readonly type: T } }
>["result"]["data"];

export class MockKernelTransport implements KernelTransport {
  readonly kind = "mock" as const;
  readonly label = "Mock demo kernel";
  readonly #listeners = new Set<KernelEventListener>();
  readonly #eventDelayMs: number;
  readonly #supportedProtocols: readonly KernelProtocol[];
  #sessionId: string | null = null;
  #messageSequence = 0;
  #activeExecution: AbortController | null = null;
  #initialized = false;
  #negotiatedProtocol: KernelProtocol | null = null;
  #workspacePopulated = false;
  #workspaceRevision = 0;
  readonly #mockValues = new Map<string, number>([
    ["x", 3],
    ["y", 30],
  ]);

  constructor(options: MockKernelOptions = {}) {
    this.#eventDelayMs = options.eventDelayMs ?? 90;
    this.#supportedProtocols =
      options.supportedProtocols ?? SUPPORTED_KERNEL_PROTOCOLS;
  }

  async connect(sessionId: string): Promise<void> {
    this.#sessionId = sessionId;
    this.#initialized = false;
    this.#negotiatedProtocol = null;
    this.#workspacePopulated = false;
    this.#workspaceRevision = 0;
    this.#mockValues.set("x", 3);
    this.#mockValues.set("y", 30);
    this.#emit("status", { status: "starting" });
  }

  async disconnect(): Promise<void> {
    this.#activeExecution?.abort();
    this.#activeExecution = null;
    this.#sessionId = null;
    this.#initialized = false;
    this.#negotiatedProtocol = null;
    this.#workspacePopulated = false;
    this.#workspaceRevision = 0;
  }

  subscribe(listener: KernelEventListener): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  subscribeConnectionLoss(_listener: KernelConnectionLossListener): () => void {
    return () => undefined;
  }

  async request<TRequest extends KernelRequest>(
    request: TRequest,
  ): Promise<ResponseFor<TRequest>> {
    this.#assertConnected(request);

    switch (request.request.type) {
      case "initialize": {
        if (this.#initialized) {
          return this.#failure(request, {
            category: "kernel.alreadyInitialized",
            message: "The mock kernel is already initialized.",
          }) as ResponseFor<TRequest>;
        }
        const initializeRequest = request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "initialize" } }
        >;
        const capabilitiesError = validateBootstrapInitializeParams(
          initializeRequest.request.params,
        );
        if (capabilitiesError !== null) {
          return this.#failure(request, {
            category: "protocol.invalidCapabilities",
            message: capabilitiesError,
          }) as ResponseFor<TRequest>;
        }
        const negotiatedProtocol =
          initializeRequest.request.params.supportedProtocols.find((protocol) =>
            this.#supportedProtocols.includes(protocol as KernelProtocol),
          );
        if (
          negotiatedProtocol !== KERNEL_PROTOCOL_V0 &&
          negotiatedProtocol !== KERNEL_PROTOCOL_V1 &&
          negotiatedProtocol !== KERNEL_PROTOCOL_V2 &&
          negotiatedProtocol !== KERNEL_PROTOCOL_V3
        ) {
          return this.#failure(request, {
            category: "protocol.noCommonVersion",
            message: "The mock kernel and client have no common protocol.",
          }) as ResponseFor<TRequest>;
        }
        const requested = initializeRequest.request.params.capabilities;
        const baseCapabilities = {
          executionModes: requested.executionModes,
          displayMimeTypes: requested.displayMimeTypes.filter(
            (mime) => mime === "text/plain",
          ),
          maxPreviewElements: Math.min(
            requested.maxPreviewElements,
            MAX_PREVIEW_ELEMENTS,
          ),
          interrupt: requested.interrupt,
          workspaceDelta: requested.workspaceDelta,
        };
        const v1Capabilities = hasValidV1Capabilities(requested)
          ? {
              ...baseCapabilities,
              maxStringElementCodeUnits: Math.min(
                requested.maxStringElementCodeUnits,
                MAX_STRING_ELEMENT_CODE_UNITS,
              ),
              maxPreviewCodeUnits: Math.min(
                requested.maxPreviewCodeUnits,
                MAX_PREVIEW_CODE_UNITS,
              ),
            }
          : baseCapabilities;
        const negotiatedCapabilities =
          (negotiatedProtocol === KERNEL_PROTOCOL_V2 ||
            negotiatedProtocol === KERNEL_PROTOCOL_V3) &&
          hasValidV2Capabilities(requested)
            ? {
                ...v1Capabilities,
                maxAggregateNodes: Math.min(
                  requested.maxAggregateNodes,
                  MAX_AGGREGATE_NODES,
                ),
                maxAggregateElements: Math.min(
                  requested.maxAggregateElements,
                  MAX_AGGREGATE_ELEMENTS,
                ),
                maxAggregateDepth: Math.min(
                  requested.maxAggregateDepth,
                  MAX_AGGREGATE_DEPTH,
                ),
              }
            : negotiatedProtocol === KERNEL_PROTOCOL_V1
              ? v1Capabilities
              : baseCapabilities;
        const response = this.#success(initializeRequest, {
          negotiatedProtocol,
          implementation: { name: "openmat-web-mock", version: "0.1.0" },
          capabilities: negotiatedCapabilities,
        });
        this.#initialized = true;
        this.#negotiatedProtocol = negotiatedProtocol;
        this.#emit("status", { status: "idle" });
        return response as ResponseFor<TRequest>;
      }
      case "execute":
        return this.#execute(
          request as Extract<
            KernelRequest,
            { readonly request: { readonly type: "execute" } }
          >,
        ) as Promise<ResponseFor<TRequest>>;
      case "interrupt": {
        const interruptRequest = request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "interrupt" } }
        >;
        const accepted = this.#activeExecution !== null;
        this.#activeExecution?.abort();
        return this.#success(interruptRequest, { accepted }) as ResponseFor<TRequest>;
      }
      case "listWorkspace": {
        const listRequest = request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "listWorkspace" } }
        >;
        const variables = this.#workspacePopulated ? mockVariables : [];
        return this.#success(
          listRequest,
          this.#negotiatedProtocol === KERNEL_PROTOCOL_V3
            ? { revision: this.#workspaceRevision, variables }
            : { variables },
        ) as ResponseFor<TRequest>;
      }
      case "inspect": {
        const inspectRequest = request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "inspect" } }
        >;
        const name = inspectRequest.request.params.name;
        if (
          !this.#workspacePopulated ||
          !mockVariables.some((variable) => variable.name === name)
        ) {
          return this.#failure(request, {
            category: "workspace.notFound",
            message: `Workspace variable ${name} does not exist.`,
          }) as ResponseFor<TRequest>;
        }
        const preview = {
          class: "double",
          dimensions: [1, 1],
          ...(this.#negotiatedProtocol !== KERNEL_PROTOCOL_V0
            ? { complex: false }
            : {}),
          selectedRange: { start: [1, 1], size: [1, 1] },
          values: [{ kind: "number", value: this.#mockValues.get(name) ?? 0 }],
          truncation: { truncated: false, omittedElements: 0 },
        } as const;
        return this.#success(
          inspectRequest,
          this.#negotiatedProtocol === KERNEL_PROTOCOL_V3
            ? { revision: this.#workspaceRevision, preview }
            : preview,
        ) as ResponseFor<TRequest>;
      }
      case "setVariableElement": {
        const setRequest = request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "setVariableElement" } }
        >;
        if (this.#negotiatedProtocol !== KERNEL_PROTOCOL_V3) {
          return this.#failure(request, {
            category: "protocol.unsupportedRequest",
            message: "Variable mutation requires openmat-kernel-v3.",
          }) as ResponseFor<TRequest>;
        }
        const { name, indices, value, expectedRevision } = setRequest.request.params;
        if (expectedRevision !== this.#workspaceRevision) {
          return this.#failure(request, {
            category: "workspace.revisionConflict",
            message: "The workspace changed after this variable was loaded.",
          }) as ResponseFor<TRequest>;
        }
        if (
          !this.#workspacePopulated ||
          !this.#mockValues.has(name) ||
          indices.length !== 2 ||
          indices.some((index) => index !== 1) ||
          value.imaginary !== "0"
        ) {
          return this.#failure(request, {
            category: "workspace.invalidElement",
            message: "The mock kernel only exposes real scalar x and y values.",
          }) as ResponseFor<TRequest>;
        }
        const nextValue = Number(value.real);
        if (!Number.isFinite(nextValue)) {
          return this.#failure(request, {
            category: "workspace.invalidElement",
            message: "The mock kernel accepts finite numeric values.",
          }) as ResponseFor<TRequest>;
        }
        this.#mockValues.set(name, nextValue);
        this.#workspaceRevision += 1;
        const variable = mockVariables.find((entry) => entry.name === name)!;
        this.#emit("workspaceDelta", {
          added: [],
          changed: [variable],
          removed: [],
        });
        return this.#success(setRequest, {
          revision: this.#workspaceRevision,
          variable,
        }) as ResponseFor<TRequest>;
      }
      case "shutdown": {
        const shutdownRequest = request as Extract<
          KernelRequest,
          { readonly request: { readonly type: "shutdown" } }
        >;
        const response = this.#success(shutdownRequest, {});
        await this.disconnect();
        return response as ResponseFor<TRequest>;
      }
    }
  }

  async #execute(
    request: Extract<
      KernelRequest,
      { readonly request: { readonly type: "execute" } }
    >,
  ): Promise<ResponseFor<typeof request>> {
    if (this.#activeExecution !== null) {
      return this.#failure(request, {
        category: "kernel.busy",
        message: "The mock kernel is already executing code.",
      });
    }

    const controller = new AbortController();
    this.#activeExecution = controller;
    this.#emit("status", { status: "busy" });

    try {
      await this.#pause(controller.signal);
      this.#emit("display", {
        representations: { "text/plain": "30" },
      });

      await this.#pause(controller.signal);
      if (this.#negotiatedProtocol === KERNEL_PROTOCOL_V3) {
        this.#workspaceRevision += 1;
      }
      this.#workspacePopulated = true;
      this.#emit("workspaceDelta", {
        added: mockVariables,
        changed: [],
        removed: [],
      });

      await this.#pause(controller.signal);
      this.#emit("status", { status: "idle" });
      return this.#success(request, { interrupted: false });
    } catch (error: unknown) {
      if (error instanceof DOMException && error.name === "AbortError") {
        this.#emit("status", { status: "interrupted" });
        this.#emit("status", { status: "idle" });
        return this.#success(request, { interrupted: true });
      }
      throw error;
    } finally {
      if (this.#activeExecution === controller) {
        this.#activeExecution = null;
      }
    }
  }

  #assertConnected(request: KernelRequest): void {
    if (this.#sessionId === null) {
      throw new Error("Mock kernel transport is disconnected");
    }
    if (
      request.sessionId !== this.#sessionId ||
      (request.request.type === "initialize"
        ? request.protocol !== KERNEL_PROTOCOL_V0
        : request.protocol !==
          (this.#negotiatedProtocol ?? KERNEL_PROTOCOL_V0))
    ) {
      throw new Error("Request envelope does not match the active session");
    }
  }

  #success<T extends KernelRequestType>(
    request: KernelRequest<T>,
    data: SuccessData<T>,
  ): KernelSuccessResponse<T> {
    return {
      protocol: request.protocol,
      sessionId: request.sessionId,
      messageId: this.#nextMessageId("response"),
      kind: "response",
      replyTo: request.messageId,
      ok: true,
      result: { type: request.request.type, data },
    } as KernelSuccessResponse<T>;
  }

  #failure(
    request: KernelRequest,
    error: KernelFailureResponse["error"],
  ): KernelFailureResponse {
    return {
      protocol: request.protocol,
      sessionId: request.sessionId,
      messageId: this.#nextMessageId("response"),
      kind: "response",
      replyTo: request.messageId,
      ok: false,
      error,
    };
  }

  #emit<T extends KernelEventType>(
    type: T,
    data: EventDataByType[T],
  ): void {
    if (this.#sessionId === null) {
      return;
    }

    const message = {
      protocol: this.#negotiatedProtocol ?? KERNEL_PROTOCOL_V0,
      sessionId: this.#sessionId,
      messageId: this.#nextMessageId("event"),
      kind: "event",
      event: { type, data },
    } as Extract<
      KernelEvent,
      { readonly event: { readonly type: T } }
    >;

    for (const listener of this.#listeners) {
      listener(message);
    }
  }

  #nextMessageId(prefix: string): string {
    this.#messageSequence += 1;
    return `mock-${prefix}-${this.#messageSequence.toString().padStart(4, "0")}`;
  }

  #pause(signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      if (signal.aborted) {
        reject(new DOMException("Interrupted", "AbortError"));
        return;
      }

      const timeout = globalThis.setTimeout(() => {
        signal.removeEventListener("abort", onAbort);
        resolve();
      }, this.#eventDelayMs);

      const onAbort = (): void => {
        globalThis.clearTimeout(timeout);
        reject(new DOMException("Interrupted", "AbortError"));
      };

      signal.addEventListener("abort", onAbort, { once: true });
    });
  }
}
