import { afterEach, describe, expect, it, vi } from "vitest";
import { SIMULATION_PROTOCOL, SimulationClient, simulationUrl } from "./client";
import { example } from "./examples";

class Socket {
    readyState: number = WebSocket.CONNECTING;
    onopen: (() => void) | null = null;
    onclose: (() => void) | null = null;
    onerror: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    sent: Record<string, unknown>[] = [];
    open() {
        this.readyState = WebSocket.OPEN;
        this.onopen?.();
    }
    close() {
        this.readyState = WebSocket.CLOSED;
        this.onclose?.();
    }
    send(text: string) {
        this.sent.push(JSON.parse(text));
    }
    receive(value: unknown) {
        this.onmessage?.({ data: JSON.stringify(value) });
    }
    reply(ok: boolean, result: unknown) {
        this.receive({
            protocol: SIMULATION_PROTOCOL,
            requestId: this.sent.at(-1)!.requestId,
            ok,
            ...(ok ? { result } : { error: result }),
        });
    }
}
const clients: SimulationClient[] = [];
async function create() {
    const socket = new Socket();
    const client = new SimulationClient(
        "ws://localhost:42000/kernel",
        () => socket as unknown as WebSocket,
    );
    clients.push(client);
    const connect = client.connect();
    socket.open();
    await connect;
    return { client, socket };
}
afterEach(() => {
    clients.splice(0).forEach((client) => client.disconnect());
    vi.useRealTimers();
});

describe("simulation websocket", () => {
    it("uses the existing kernel host and port including reverse-proxy prefix", () => {
        expect(
            simulationUrl("wss://example.org/openmat/kernel?ignored=1"),
        ).toBe("wss://example.org/openmat/simulation/v9");
        expect(simulationUrl("ws://localhost:42000/kernel")).toBe(
            "ws://localhost:42000/simulation/v9",
        );
    });
    it("correlates native replies and preserves block/port diagnostics", async () => {
        const { client, socket } = await create();
        const result = client.check(example("feedback").model, "snapshot");
        await Promise.resolve();
        expect(socket.sent[0]).toMatchObject({
            operation: "check",
            revision: "snapshot",
        });
        socket.reply(false, {
            code: "input",
            message: "Missing input",
            block: "sum",
            port: "in0",
        });
        await expect(result).rejects.toMatchObject({
            diagnostic: { block: "sum", port: "in0" },
        });
    });
    it("delivers sequenced sample batches and disconnects on malformed data", async () => {
        const { client, socket } = await create();
        const receive = vi.fn(),
            status = vi.fn();
        client.onEvent(receive);
        client.onStatus(status);
        const event = {
            protocol: SIMULATION_PROTOCOL,
            event: "samples",
            runId: "r",
            revision: "m",
            sequence: 1,
            data: { frames: [{ time: 0, values: [1], sampleHit: false }] },
        };
        socket.receive(event);
        expect(receive).toHaveBeenCalledOnce();
        socket.receive({ ...event, sequence: Number.MAX_SAFE_INTEGER + 1 });
        expect(status).toHaveBeenCalledWith(
            false,
            expect.stringContaining("无效数据"),
        );
        expect(socket.readyState).toBe(WebSocket.CLOSED);
    });
    it("disconnects pending work without replaying it", async () => {
        const { client, socket } = await create();
        const pending = client.catalog();
        await Promise.resolve();
        socket.close();
        await expect(pending).rejects.toThrow(/断开/);
        expect(socket.sent).toHaveLength(1);
    });
    it("cancels a pending connection immediately on unmount", async () => {
        vi.useFakeTimers();
        const socket = new Socket();
        const client = new SimulationClient(
            "ws://localhost:42000/kernel",
            () => socket as unknown as WebSocket,
        );
        clients.push(client);
        const pending = client.connect();
        client.disconnect();
        await expect(pending).rejects.toThrow(/关闭/);
        expect(vi.getTimerCount()).toBe(0);
    });
    it("closes the transport after a request timeout so an orphan run cannot continue", async () => {
        vi.useFakeTimers();
        const { client, socket } = await create();
        const pending = client.run(example("feedback").model, "snapshot");
        const rejected = expect(pending).rejects.toThrow(/超时/);
        await vi.advanceTimersByTimeAsync(30001);
        await rejected;
        expect(socket.readyState).toBe(WebSocket.CLOSED);
    });
});
