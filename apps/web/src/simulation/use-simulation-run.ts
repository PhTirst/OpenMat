import { parseEventPlan, type EventPlan } from "./hybrid";
import { parseSamplingPlan, type SamplingPlan } from "./sampling";
import { useCallback, useEffect, useRef, useState } from "react";
import {
    SimulationClient,
    SimulationError,
    type RunInfo,
    type SimulationDiagnostic,
    type SimulationFrame,
    type SimulationSnapshot,
    type SimulationCatalog,
    type SolverStats,
} from "./client";
import { DEFINITIONS, type Model } from "./model";
export type RunStatus =
    | "idle"
    | "checking"
    | "compiling"
    | "running"
    | "cancelling"
    | "finished"
    | "cancelled"
    | "failed";
export function numericalSource(
    model: Model,
    snapshot?: SimulationSnapshot,
): string {
    return JSON.stringify({
        ...model,
        blocks: model.blocks.map(({ position: _position, ...block }) => block),
        ...snapshot,
    });
}
export function useSimulationRun(url: string | undefined) {
    const client = useRef<SimulationClient | null>(null);
    const [connected, setConnected] = useState(false);
    const [connectionError, setConnectionError] = useState<string | null>(null);
    const [status, setStatus] = useState<RunStatus>("idle");
    const [checkedSampling, setCheckedSampling] = useState<{
        source: string;
        plan: SamplingPlan;
        eventPlan?: EventPlan;
    } | null>(null);
    const [info, setInfo] = useState<RunInfo | null>(null);
    const [diagnostics, setDiagnostics] = useState<SimulationDiagnostic[]>([]);
    const [version, setVersion] = useState(0);
    const [elapsed, setElapsed] = useState(0);
    const [capabilities, setCapabilities] =
        useState<SimulationCatalog["execution"]>();
    const [solverStats, setSolverStats] = useState<SolverStats | null>(null);
    const frames = useRef<SimulationFrame[]>([]);
    const active = useRef<RunInfo | null>(null);
    const source = useRef("");
    const statusRef = useRef(status);
    statusRef.current = status;
    const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
    const refresh = useCallback(() => {
        if (timer.current === null)
            timer.current = setTimeout(() => {
                timer.current = null;
                setVersion((value) => value + 1);
            }, 33);
    }, []);
    const reconnect = useCallback(async () => {
        const connection = client.current;
        if (!connection) return;
        try {
            const catalog = await connection.catalog();
            if (client.current !== connection) return;
            if (
                catalog?.backend !== "reference" ||
                !Array.isArray(catalog.blocks) ||
                !DEFINITIONS.every((def) =>
                    catalog.blocks.some(
                        (remote) =>
                            remote.type === def.type &&
                            JSON.stringify(remote.inputs) ===
                                JSON.stringify(def.inputs) &&
                            JSON.stringify(remote.outputs) ===
                                JSON.stringify(def.outputs) &&
                            remote.parameter === def.parameter,
                    ),
                )
            )
                throw new Error("仿真服务版本不兼容。");
            setConnected(true);
            setCapabilities(catalog.execution);
            setConnectionError(null);
        } catch (error) {
            if (client.current === connection) {
                setConnected(false);
                setConnectionError(
                    error instanceof Error ? error.message : String(error),
                );
            }
        }
    }, []);
    useEffect(() => {
        const connection = new SimulationClient(url);
        client.current = connection;
        let sequence = 0,
            values = 0,
            eventRecords = 0;
        const offStatus = connection.onStatus((online, reason) => {
            if (!online) {
                setConnected(false);
                setConnectionError(reason ?? "连接已断开。");
                if (
                    ["compiling", "running", "cancelling"].includes(
                        statusRef.current,
                    )
                )
                    setStatus("failed");
                active.current = null;
            }
        });
        let lastRunId = "";
        const offEvent = connection.onEvent((event) => {
            const run = active.current;
            if (
                !run ||
                event.runId !== run.runId ||
                event.revision !== run.revision
            )
                return;
            if (lastRunId !== run.runId) {
                sequence = 0;
                values = 0;
                eventRecords = 0;
                lastRunId = run.runId;
            }
            if (event.sequence <= sequence) return;
            sequence = event.sequence;
            if (event.event === "samples") {
                const batch = event.data.frames ?? [];
                const width = run.scopes.reduce(
                    (max, scope) => Math.max(max, scope.offset + scope.width),
                    0,
                );
                let last = frames.current.at(-1)?.time ?? -Infinity;
                for (const frame of batch) {
                    if (frame.time <= last || frame.values.length !== width)
                        throw new Error("Invalid sample order or signal width");
                    last = frame.time;
                    values += frame.values.length;
                    eventRecords += frame.events?.length ?? 0;
                }
                if (
                    frames.current.length + batch.length > 100000 ||
                    values > 2000000 ||
                    eventRecords > 100000
                )
                    throw new Error("Result bounds exceeded");
                frames.current.push(...batch);
                refresh();
            } else {
                setStatus(event.event);
                setElapsed(event.data.elapsedSeconds ?? 0);
                setSolverStats(event.data.solverStats ?? null);
                if (event.data.error && event.event === "failed")
                    setDiagnostics([event.data.error]);
                active.current = null;
                refresh();
            }
        });
        void reconnect();
        return () => {
            offEvent();
            offStatus();
            if (client.current === connection) client.current = null;
            active.current = null;
            connection.disconnect();
            if (timer.current) clearTimeout(timer.current);
            timer.current = null;
        };
    }, [url, reconnect, refresh]);
    const fail = useCallback((error: unknown) => {
        statusRef.current = "failed";
        setStatus("failed");
        setDiagnostics([
            error instanceof SimulationError
                ? error.diagnostic
                : {
                      code: "simulation",
                      message:
                          error instanceof Error
                              ? error.message
                              : String(error),
                  },
        ]);
    }, []);
    const run = useCallback(
        async (model: Model, snapshot?: SimulationSnapshot) => {
            const connection = client.current;
            if (
                !connection ||
                ["compiling", "running", "cancelling", "checking"].includes(
                    statusRef.current,
                )
            )
                return;
            statusRef.current = "compiling";
            setStatus("compiling");
            setDiagnostics([]);
            setInfo(null);
            setElapsed(0);
            setSolverStats(null);
            frames.current = [];
            setVersion((value) => value + 1);
            source.current = numericalSource(model, snapshot);
            try {
                const result = await connection.run(
                    structuredClone(model),
                    crypto.randomUUID(),
                    snapshot,
                );
                if (client.current !== connection) return;
                if (
                    !Array.isArray(result.scopes) ||
                    !["reference", "llvm-orc"].includes(result.backend) ||
                    typeof result.runId !== "string" ||
                    typeof result.revision !== "string" ||
                    result.scopes.some(
                        (scope) =>
                            typeof scope.block !== "string" ||
                            !Number.isSafeInteger(scope.offset) ||
                            scope.offset < 0 ||
                            !Number.isSafeInteger(scope.width) ||
                            scope.width < 1 ||
                            scope.offset + scope.width > 262144,
                    )
                ) {
                    connection.disconnect();
                    throw new Error("运行响应无效。");
                }
                if (result.eventPlan)
                    result.eventPlan = parseEventPlan(result.eventPlan);
                if (result.sampling)
                    result.sampling = parseSamplingPlan(result.sampling);
                setCheckedSampling(null);
                active.current = result;
                setInfo(result);
                setStatus("running");
                statusRef.current = "running";
            } catch (error) {
                if (client.current === connection) fail(error);
            }
        },
        [fail],
    );
    const check = useCallback(
        async (model: Model, snapshot?: SimulationSnapshot) => {
            const connection = client.current;
            if (
                !connection ||
                ["compiling", "running", "cancelling", "checking"].includes(
                    statusRef.current,
                )
            )
                return;
            setCheckedSampling(null);
            setStatus("checking");
            statusRef.current = "checking";
            setDiagnostics([]);
            try {
                const result = await connection.check(
                    structuredClone(model),
                    crypto.randomUUID(),
                    snapshot,
                );
                if (client.current !== connection) return;
                if (result.plan.sampling)
                    setCheckedSampling({
                        source: numericalSource(model, snapshot),
                        plan: parseSamplingPlan(result.plan.sampling),
                        ...(result.plan.eventPlan
                            ? {
                                  eventPlan: parseEventPlan(
                                      result.plan.eventPlan,
                                  ),
                              }
                            : {}),
                    });
                statusRef.current = "idle";
                setStatus("idle");
                setDiagnostics([
                    {
                        code: "valid",
                        message: "模型检查通过：连接、端口尺寸与执行顺序有效。",
                    },
                ]);
            } catch (error) {
                if (client.current === connection) fail(error);
            }
        },
        [fail],
    );
    const cancel = useCallback(async () => {
        const run = active.current;
        if (!run || !client.current) return;
        statusRef.current = "cancelling";
        setStatus("cancelling");
        try {
            await client.current.cancel(run.runId);
        } catch (error) {
            if (
                active.current === run &&
                !(
                    error instanceof SimulationError &&
                    error.diagnostic.code === "run_not_found"
                )
            )
                fail(error);
        }
    }, [fail]);
    const reset = useCallback(() => {
        if (active.current) return;
        frames.current = [];
        source.current = "";
        setInfo(null);
        setCheckedSampling(null);
        setSolverStats(null);
        setDiagnostics([]);
        setElapsed(0);
        setStatus("idle");
        statusRef.current = "idle";
        refresh();
    }, [refresh]);
    return {
        client,
        connected,
        connectionError,
        reconnect,
        status,
        info,
        checkedSampling,
        diagnostics,
        setDiagnostics,
        frames,
        version,
        elapsed,
        capabilities,
        solverStats,
        source,
        run,
        check,
        cancel,
        reset,
        busy: ["checking", "compiling", "running", "cancelling"].includes(
            status,
        ),
    };
}
