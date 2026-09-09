import type {
  KernelEvent,
  KernelRequest,
  ResponseFor,
} from "../protocol/kernel-v2";

export type KernelEventListener = (event: KernelEvent) => void;
export type KernelConnectionLossListener = (error: Error) => void;
export type KernelTransportKind = "mock" | "websocket";

/**
 * Stable UI boundary for both the in-process demo and the JSON/WebSocket
 * binding. Transports deliver only validated, known events for the negotiated
 * kernel protocol.
 */
export interface KernelTransport {
  readonly kind: KernelTransportKind;
  readonly label: string;
  connect(sessionId: string): Promise<void>;
  disconnect(): Promise<void>;
  request<TRequest extends KernelRequest>(
    request: TRequest,
  ): Promise<ResponseFor<TRequest>>;
  subscribe(listener: KernelEventListener): () => void;
  subscribeConnectionLoss(listener: KernelConnectionLossListener): () => void;
}
