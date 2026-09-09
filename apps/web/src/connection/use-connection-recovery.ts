import { useCallback, useEffect, useRef, useState } from "react";
import type { KernelStatus } from "../protocol/kernel-v0";

export const DEFAULT_CONNECTION_RETRY_DELAYS = [250, 1000, 3000, 5000] as const;

export type ConnectionRecoveryPhase =
  | "connecting"
  | "connected"
  | "reconnecting"
  | "failed";

export interface ConnectionRecoveryState {
  readonly phase: ConnectionRecoveryPhase;
  readonly generation: number;
  readonly failureCount: number;
  readonly connectedOnce: boolean;
  readonly error: string | null;
  readonly retryDelayMs: number | null;
}

export interface ConnectionRecoveryController extends ConnectionRecoveryState {
  readonly markConnected: (generation: number) => void;
  readonly markFailed: (generation: number, message: string) => void;
  readonly retryNow: () => void;
}

export interface SessionConnectionSummary {
  readonly phase: "connecting" | "ready" | "recovering" | "failed";
  readonly label: string;
  readonly detail: string;
  readonly retryDelayMs: number | null;
}

const INITIAL_CONNECTION_STATE: ConnectionRecoveryState = {
  phase: "connecting",
  generation: 0,
  failureCount: 0,
  connectedOnce: false,
  error: null,
  retryDelayMs: null,
};

function retryDelay(
  delays: readonly number[],
  failureCount: number,
): number | null {
  if (delays.length === 0) {
    return null;
  }
  const candidate = delays[Math.min(failureCount, delays.length - 1)];
  return candidate !== undefined && Number.isFinite(candidate) && candidate >= 0
    ? candidate
    : null;
}

export function useConnectionRecovery(
  retryDelays: readonly number[] = DEFAULT_CONNECTION_RETRY_DELAYS,
): ConnectionRecoveryController {
  const retryDelaysRef = useRef(retryDelays);
  retryDelaysRef.current = retryDelays;
  const [state, setState] = useState<ConnectionRecoveryState>(
    INITIAL_CONNECTION_STATE,
  );

  const markConnected = useCallback((generation: number): void => {
    setState((current) =>
      current.generation !== generation
        ? current
        : {
            ...current,
            phase: "connected",
            failureCount: 0,
            connectedOnce: true,
            error: null,
            retryDelayMs: null,
          },
    );
  }, []);

  const markFailed = useCallback(
    (generation: number, message: string): void => {
      setState((current) => {
        if (
          current.generation !== generation ||
          current.phase === "failed" ||
          (current.phase === "reconnecting" && current.retryDelayMs !== null)
        ) {
          return current;
        }
        const delay = retryDelay(retryDelaysRef.current, current.failureCount);
        return {
          ...current,
          phase: delay === null ? "failed" : "reconnecting",
          failureCount: current.failureCount + 1,
          error: message,
          retryDelayMs: delay,
        };
      });
    },
    [],
  );

  const retryNow = useCallback((): void => {
    setState((current) =>
      current.phase === "connected"
        ? current
        : {
            ...current,
            phase:
              current.connectedOnce || current.failureCount > 0
                ? "reconnecting"
                : "connecting",
            generation: current.generation + 1,
            error: null,
            retryDelayMs: null,
          },
    );
  }, []);

  useEffect(() => {
    if (state.retryDelayMs === null) {
      return;
    }
    const expectedGeneration = state.generation;
    const timer = window.setTimeout(() => {
      setState((current) =>
        current.generation !== expectedGeneration ||
        current.retryDelayMs === null
          ? current
          : {
              ...current,
              phase: "reconnecting",
              generation: current.generation + 1,
              error: null,
              retryDelayMs: null,
            },
      );
    }, state.retryDelayMs);
    return () => window.clearTimeout(timer);
  }, [state.generation, state.retryDelayMs]);

  useEffect(() => {
    const retryWhenOnline = (): void => retryNow();
    window.addEventListener("online", retryWhenOnline);
    return () => window.removeEventListener("online", retryWhenOnline);
  }, [retryNow]);

  return { ...state, markConnected, markFailed, retryNow };
}

function channelLabel(state: ConnectionRecoveryState): string {
  switch (state.phase) {
    case "connecting":
      return "connecting";
    case "connected":
      return "ready";
    case "reconnecting":
      return "recovering";
    case "failed":
      return "offline";
  }
}

export function summarizeSessionConnection(
  kernel: ConnectionRecoveryState,
  workspace: ConnectionRecoveryState,
  kernelStatus: KernelStatus,
): SessionConnectionSummary {
  const detail = `Kernel: ${channelLabel(kernel)} · Files: ${channelLabel(workspace)}`;
  const retryDelays = [kernel.retryDelayMs, workspace.retryDelayMs].filter(
    (delay): delay is number => delay !== null,
  );
  const retryDelayMs = retryDelays.length === 0 ? null : Math.min(...retryDelays);
  if (kernel.phase === "connected" && workspace.phase === "connected") {
    return {
      phase: "ready",
      label: `Ready · ${kernelStatus}`,
      detail,
      retryDelayMs,
    };
  }
  if (kernel.phase === "failed" || workspace.phase === "failed") {
    return { phase: "failed", label: "Connection interrupted", detail, retryDelayMs };
  }
  if (
    !kernel.connectedOnce &&
    !workspace.connectedOnce &&
    kernel.phase === "connecting" &&
    workspace.phase === "connecting"
  ) {
    return { phase: "connecting", label: "Connecting", detail, retryDelayMs };
  }
  const recovering = [
    kernel.phase === "connected" ? null : "kernel",
    workspace.phase === "connected" ? null : "files",
  ].filter((channel): channel is string => channel !== null);
  return {
    phase: "recovering",
    label: `Recovering ${recovering.join(" + ")}`,
    detail,
    retryDelayMs,
  };
}
