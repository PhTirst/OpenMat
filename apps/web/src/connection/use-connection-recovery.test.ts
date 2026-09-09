import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  summarizeSessionConnection,
  useConnectionRecovery,
  type ConnectionRecoveryState,
} from "./use-connection-recovery";

afterEach(() => vi.useRealTimers());

describe("useConnectionRecovery", () => {
  it("retries with capped delays and ignores stale attempt completions", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useConnectionRecovery([100, 250]));

    act(() => result.current.markFailed(0, "first failure"));
    expect(result.current).toMatchObject({
      phase: "reconnecting",
      generation: 0,
      failureCount: 1,
      retryDelayMs: 100,
    });
    act(() => vi.advanceTimersByTime(100));
    expect(result.current.generation).toBe(1);

    act(() => result.current.markFailed(1, "second failure"));
    expect(result.current.retryDelayMs).toBe(250);
    act(() => vi.advanceTimersByTime(250));
    expect(result.current.generation).toBe(2);

    act(() => result.current.markFailed(2, "third failure"));
    expect(result.current.retryDelayMs).toBe(250);
    act(() => vi.advanceTimersByTime(250));
    expect(result.current.generation).toBe(3);

    act(() => result.current.markConnected(2));
    expect(result.current.phase).toBe("reconnecting");
    act(() => result.current.markConnected(3));
    expect(result.current).toMatchObject({
      phase: "connected",
      connectedOnce: true,
      failureCount: 0,
      error: null,
    });
  });

  it("supports immediate manual and browser-online retries", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useConnectionRecovery([5000]));
    act(() => result.current.markFailed(0, "offline"));
    act(() => result.current.retryNow());
    expect(result.current).toMatchObject({
      phase: "reconnecting",
      generation: 1,
      retryDelayMs: null,
    });

    act(() => result.current.markFailed(1, "offline again"));
    act(() => window.dispatchEvent(new Event("online")));
    expect(result.current.generation).toBe(2);
  });
});

const channel = (
  phase: ConnectionRecoveryState["phase"],
  connectedOnce = phase === "connected",
): ConnectionRecoveryState => ({
  phase,
  generation: 0,
  failureCount: phase === "connected" ? 0 : 1,
  connectedOnce,
  error: null,
  retryDelayMs: phase === "reconnecting" ? 1000 : null,
});

describe("summarizeSessionConnection", () => {
  it("reports ready, partial recovery, and terminal failure distinctly", () => {
    expect(
      summarizeSessionConnection(channel("connected"), channel("connected"), "idle"),
    ).toMatchObject({ phase: "ready", label: "Ready · idle" });
    expect(
      summarizeSessionConnection(
        channel("connected"),
        channel("reconnecting", true),
        "idle",
      ),
    ).toMatchObject({ phase: "recovering", label: "Recovering files" });
    expect(
      summarizeSessionConnection(channel("failed"), channel("connected"), "dead"),
    ).toMatchObject({ phase: "failed", label: "Connection interrupted" });
  });
});
