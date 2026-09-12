import type { BlockDefinition, Model, ExecutionOptions } from "./model";
export const SIMULATION_PROTOCOL = "openmat-simulation-v3";
export interface SimulationSnapshot {
    sources: Record<string, string>;
    execution: ExecutionOptions;
}
export interface SolverStats {
    acceptedSteps: number;
    rhsEvaluations: number;
    errorTestFailures: number;
    nonlinearIterations: number;
    reinitializations: number;
}
export interface SimulationDiagnostic {
    code: string;
    message: string;
    block?: string;
    port?: string;
    parameter?: string;
    part?: string;
    sourcePath?: string;
    line?: number;
    column?: number;
}
export class SimulationError extends Error {
    constructor(readonly diagnostic: SimulationDiagnostic) {
        super(diagnostic.message);
        this.name = "SimulationError";
    }
}
export interface ScopeInfo {
    block: string;
    offset: number;
    width: number;
}
export interface SimulationFrame {
    time: number;
    sampleHit: boolean;
    values: number[];
}
export interface RunInfo {
    runId: string;
    revision: string;
    backend: "reference" | "llvm-orc";
    solver?: { type: "rk4" | "cvode" };
    scopes: ScopeInfo[];
    settings: Model["settings"];
    compileSeconds: number;
}
export interface RunEvent {
    runId: string;
    revision: string;
    sequence: number;
    event: "samples" | "finished" | "failed" | "cancelled";
    data: {
        frames?: SimulationFrame[];
        time?: number;
        samples?: number;
        elapsedSeconds?: number;
        solverStats?: SolverStats | null;
        error?: SimulationDiagnostic | null;
    };
}
export interface SlxBlock {
    sid: string;
    name: string;
    blockType: string;
    properties: Record<string, string>;
    source: { part: string };
}
export interface SlxLine {
    properties: Record<string, string>;
    branches: SlxLine[];
}
export interface SlxImport {
    runnable: boolean;
    model?: Model;
    issues: SimulationDiagnostic[];
    document: {
        name: string;
        matlabRelease?: string;
        systems: {
            parentBlock: string | null;
            blocks: SlxBlock[];
            lines: SlxLine[];
        }[];
    };
}
export interface SimulationCatalog {
    execution?: {
        reference: boolean;
        rk4: boolean;
        llvmConfigured: boolean;
        cvodeConfigured: boolean;
    };
    blocks: BlockDefinition[];
    backend: "reference";
    maxSamples: number;
    maxValues: number;
    maxSlxBytes: number;
}
interface Pending {
    resolve(value: unknown): void;
    reject(error: Error): void;
    timer: ReturnType<typeof setTimeout>;
}
export function simulationUrl(kernelUrl: string): string {
    const url = new URL(kernelUrl);
    if (url.protocol !== "ws:" && url.protocol !== "wss:")
        throw new Error("仿真服务需要 WebSocket 地址。");
    url.pathname = url.pathname.replace(/\/kernel\/?$/, "/simulation/v3");
    if (!url.pathname.endsWith("/simulation/v3"))
        url.pathname = "/simulation/v3";
    url.search = "";
    url.hash = "";
    return url.toString();
}
/** One socket owns its jobs. A disconnect never reconnects/replays a run implicitly. */
export class SimulationClient {
    private socket: WebSocket | null = null;
    private connecting: Promise<void> | null = null;
    private closeConnection: ((reason: string) => void) | null = null;
    private pending = new Map<string, Pending>();
    private events = new Set<(event: RunEvent) => void>();
    private statuses = new Set<(connected: boolean, reason?: string) => void>();
    private generation = 0;
    constructor(
        private readonly url: string | undefined,
        private readonly createSocket = (url: string) => new WebSocket(url),
    ) {}
    onEvent(listener: (event: RunEvent) => void): () => void {
        this.events.add(listener);
        return () => this.events.delete(listener);
    }
    onStatus(
        listener: (connected: boolean, reason?: string) => void,
    ): () => void {
        this.statuses.add(listener);
        return () => this.statuses.delete(listener);
    }
    connect(): Promise<void> {
        if (this.socket?.readyState === WebSocket.OPEN)
            return Promise.resolve();
        if (this.connecting) return this.connecting;
        if (!this.url)
            return Promise.reject(
                new Error(
                    "未配置原生仿真服务。请使用 OpenMat 启动脚本连接服务器。",
                ),
            );
        const generation = ++this.generation;
        const socket = this.createSocket(simulationUrl(this.url));
        this.socket = socket;
        const promise = new Promise<void>((resolve, reject) => {
            const timer = setTimeout(() => {
                lost("连接仿真服务超时。");
                socket.close();
            }, 10000);
            const lost = (reason: string) => {
                clearTimeout(timer);
                reject(new Error(reason));
                if (this.socket !== socket) return;
                this.socket = null;
                this.closeConnection = null;
                for (const pending of this.pending.values()) {
                    clearTimeout(pending.timer);
                    pending.reject(new Error(reason));
                }
                this.pending.clear();
                for (const listener of this.statuses) listener(false, reason);
            };
            this.closeConnection = lost;
            socket.onopen = () => {
                clearTimeout(timer);
                if (this.generation !== generation) {
                    socket.close();
                    reject(new Error("连接已取消。"));
                    return;
                }
                for (const listener of this.statuses) listener(true);
                resolve();
            };
            socket.onmessage = (message) => {
                if (this.socket !== socket) return;
                try {
                    this.receive(JSON.parse(String(message.data)));
                } catch {
                    lost("仿真服务返回了无效数据。");
                    socket.close();
                }
            };
            socket.onerror = () => {
                lost("无法连接仿真服务，请确认服务器包含模型编辑器支持。");
                socket.close();
            };
            socket.onclose = () =>
                lost("仿真连接已断开，运行已终止。可以重新连接后再次运行。");
        });
        this.connecting = promise;
        void promise
            .finally(() => {
                if (this.connecting === promise) this.connecting = null;
            })
            .catch(() => {});
        return promise;
    }
    disconnect(): void {
        this.generation++;
        const socket = this.socket;
        this.closeConnection?.("仿真连接已关闭。");
        this.socket = null;
        this.connecting = null;
        this.closeConnection = null;
        for (const pending of this.pending.values()) {
            clearTimeout(pending.timer);
            pending.reject(new Error("仿真连接已关闭。"));
        }
        this.pending.clear();
        socket?.close();
    }
    private receive(raw: unknown): void {
        if (!raw || typeof raw !== "object") throw new Error("envelope");
        const value = raw as Record<string, unknown>;
        if (value.protocol !== SIMULATION_PROTOCOL) throw new Error("protocol");
        if (typeof value.requestId === "string") {
            const pending = this.pending.get(value.requestId);
            if (!pending) return;
            this.pending.delete(value.requestId);
            clearTimeout(pending.timer);
            if (value.ok === true) pending.resolve(value.result);
            else if (
                value.error &&
                typeof value.error === "object" &&
                typeof (value.error as SimulationDiagnostic).message ===
                    "string"
            )
                pending.reject(
                    new SimulationError(value.error as SimulationDiagnostic),
                );
            else pending.reject(new Error("无效的仿真响应。"));
            return;
        }
        if (
            typeof value.runId !== "string" ||
            typeof value.revision !== "string" ||
            !Number.isSafeInteger(value.sequence) ||
            !["samples", "finished", "failed", "cancelled"].includes(
                String(value.event),
            ) ||
            !value.data ||
            typeof value.data !== "object"
        )
            throw new Error("event");
        const event = value as unknown as RunEvent;
        if (event.event === "samples") {
            if (
                !Array.isArray(event.data.frames) ||
                event.data.frames.length > 100000
            )
                throw new Error("frames");
            for (const frame of event.data.frames) {
                if (
                    !Number.isFinite(frame.time) ||
                    !Array.isArray(frame.values) ||
                    frame.values.length > 262144 ||
                    frame.values.some(
                        (v) => typeof v !== "number" || !Number.isFinite(v),
                    )
                )
                    throw new Error("frame");
            }
        }
        for (const listener of this.events) listener(event);
    }
    async request<T>(
        operation: string,
        params: Record<string, unknown> = {},
    ): Promise<T> {
        await this.connect();
        const socket = this.socket;
        if (!socket || socket.readyState !== WebSocket.OPEN)
            throw new Error("仿真服务未连接。");
        const requestId = crypto.randomUUID();
        const body = JSON.stringify({
            protocol: SIMULATION_PROTOCOL,
            requestId,
            operation,
            ...params,
        });
        if (new TextEncoder().encode(body).length > 8 * 1024 * 1024)
            throw new Error("请求超过 8 MiB，请缩小模型或导入文件。");
        return new Promise<T>((resolve, reject) => {
            const timer = setTimeout(() => {
                this.pending.delete(requestId);
                reject(
                    new Error(
                        "仿真请求超时。连接已关闭以取消可能仍在运行的任务。",
                    ),
                );
                this.disconnect();
            }, 30000);
            this.pending.set(requestId, {
                resolve: (value) => resolve(value as T),
                reject,
                timer,
            });
            try {
                socket.send(body);
            } catch (error) {
                clearTimeout(timer);
                this.pending.delete(requestId);
                reject(error);
            }
        });
    }
    catalog(): Promise<SimulationCatalog> {
        return this.request("catalog");
    }
    check(
        model: Model,
        revision: string,
        snapshot?: SimulationSnapshot,
    ): Promise<{
        valid: boolean;
        revision: string;
        plan: Omit<RunInfo, "runId" | "revision">;
    }> {
        return this.request("check", { model, revision, ...snapshot });
    }
    run(
        model: Model,
        revision: string,
        snapshot?: SimulationSnapshot,
    ): Promise<RunInfo> {
        return this.request("run", { model, revision, ...snapshot });
    }
    cancel(runId: string): Promise<unknown> {
        return this.request("cancel", { runId });
    }
    async importSlx(file: Blob, name: string): Promise<SlxImport> {
        if (file.size > 2 * 1024 * 1024)
            throw new Error(
                "交互式 SLX 导入上限为 2 MiB；更大的模型可用命令行检查。",
            );
        return this.request("importSlx", {
            name,
            bytes: Array.from(new Uint8Array(await file.arrayBuffer())),
        });
    }
}
