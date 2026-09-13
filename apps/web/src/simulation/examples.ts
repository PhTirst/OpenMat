import saturatedPi from "../../../../simulation/examples/saturated-pi.omsim.json";
import switchedControl from "../../../../simulation/examples/switched-control.omsim.json";
import periodicReset from "../../../../simulation/examples/periodic-reset.omsim.json";
import multirateModel from "../../../../simulation/examples/multirate-control.omsim.json";
import {
    parseDocument,
    emptyDocument,
    fromModel,
    parseModel,
    type ModelDocument,
} from "./model";
import pendulumModel from "../../../../simulation/examples/pendulum.omsim.json";
import pendulumSource from "../../../../simulation/examples/pendulum.m?raw";
import delayModel from "../../../../simulation/examples/custom-delay.omsim.json";
import plantModel from "../../../../simulation/examples/mass-spring-damper.omsim.json";
import piModel from "../../../../simulation/examples/pi-control.omsim.json";
import { copyComponentSources } from "./component-library";
import { modelSources } from "./components";
const componentSources = import.meta.glob<string>(
    "../../../../simulation/examples/components/**/*.m",
    { eager: true, query: "?raw", import: "default" },
);
export function initializeComponentExample(
    doc: ModelDocument,
    suffix: string,
): Record<string, string> {
    const sources = Object.fromEntries(
        modelSources(doc.model).map((reference) => [
            reference,
            componentSources[`../../../../simulation/examples/${reference}`] ??
                "",
        ]),
    );
    const copied: Record<string, string> = {};
    doc.model.components =
        doc.model.components?.map((d, i) => {
            const result = copyComponentSources(d, sources, `${suffix}_${i}`);
            Object.assign(copied, result.sources);
            return result.definition;
        }) ?? [];
    return copied;
}
export const PENDULUM_SOURCE = pendulumSource;
export type Example =
    | "feedback"
    | "vector"
    | "counter"
    | "blank"
    | "pendulum"
    | "customDelay"
    | "massSpring"
    | "piControl"
    | "multirate"
    | "saturatedPi"
    | "switchedControl"
    | "periodicReset";
export function example(name: Example): ModelDocument {
    if (name === "saturatedPi")
        return parseDocument(JSON.stringify(saturatedPi));
    if (name === "switchedControl")
        return parseDocument(JSON.stringify(switchedControl));
    if (name === "periodicReset")
        return parseDocument(JSON.stringify(periodicReset));
    if (name === "pendulum") return fromModel(parseModel(pendulumModel));
    if (name === "customDelay") return fromModel(parseModel(delayModel));
    if (name === "massSpring") return fromModel(parseModel(plantModel));
    if (name === "multirate") return fromModel(parseModel(multirateModel));
    if (name === "piControl") return fromModel(parseModel(piModel));
    const doc = emptyDocument(
        name === "blank"
            ? "Untitled"
            : name === "feedback"
              ? "First order feedback"
              : name === "vector"
                ? "Two time constants"
                : "Discrete counter",
    );
    if (name === "blank") return doc;
    doc.model.blocks = [
        {
            id: "input",
            kind: { type: "constant", value: name === "vector" ? [1, 2] : [1] },
            position: { x: 40, y: 140 },
        },
        {
            id: "sum",
            kind: { type: "sum", signs: name === "counter" ? [1, 1] : [1, -1] },
            position: { x: 260, y: 140 },
        },
        {
            id: "state",
            kind:
                name === "counter"
                    ? { type: "unitDelay", initial: [0] }
                    : {
                          type: "integrator",
                          initial: name === "vector" ? [0, 0] : [0],
                      },
            position: { x: 480, y: 140 },
        },
        { id: "scope", kind: { type: "scope" }, position: { x: 730, y: 140 } },
    ];
    doc.editor.labels = {
        input: "Input",
        sum: "Error",
        state: name === "counter" ? "Counter" : "State",
        scope: "Response",
    };
    const connection = (from: string, to: string, port: string) => ({
        from: { block: from, port: "out" },
        to: { block: to, port },
    });
    doc.model.connections = [
        connection("input", "sum", "in0"),
        connection("sum", "state", "in"),
        connection("state", "scope", "in"),
    ];
    if (name === "vector") {
        doc.model.blocks.push({
            id: "gain",
            kind: { type: "gain", gain: [1, 2] },
            position: { x: 480, y: 330 },
        });
        doc.model.connections.push(
            connection("state", "gain", "in"),
            connection("gain", "sum", "in1"),
        );
        doc.editor.labels.gain = "Decay rates";
    } else doc.model.connections.push(connection("state", "sum", "in1"));
    if (name === "counter") {
        doc.model.settings.maxStep = 0.1;
        doc.model.settings.stopTime = 2;
    }
    return doc;
}
export function stressExample(count = 300): ModelDocument {
    const doc = emptyDocument("Graph interaction benchmark");
    doc.model.settings.stopTime = 1;
    for (let i = 0; i < count; i++) {
        const id = `block_${i}`;
        doc.model.blocks.push({
            id,
            kind:
                i === 0
                    ? { type: "constant", value: [1] }
                    : i === count - 1
                      ? { type: "scope" }
                      : { type: "gain", gain: [1] },
            position: { x: (i % 15) * 200, y: Math.floor(i / 15) * 145 },
        });
        if (i)
            doc.model.connections.push({
                from: { block: `block_${i - 1}`, port: "out" },
                to: { block: id, port: "in" },
            });
    }
    return doc;
}
