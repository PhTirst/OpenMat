import { useEffect, useState } from "react";
import {
    numericLiteral,
    validateFunctionKind,
    type FunctionKind,
} from "./model";

export function FunctionInspector({
    value,
    onChange,
    onOpen,
    onError,
}: {
    value: FunctionKind;
    onChange(value: FunctionKind): void;
    onOpen(): void;
    onError(message: string): void;
}) {
    const [draft, setDraft] = useState(value);
    const [inputs, setInputs] = useState("");
    const [parameters, setParameters] = useState("");
    useEffect(() => {
        setDraft(value);
        setInputs(value.inputs.map((p) => `${p.name}: ${p.width}`).join("\n"));
        setParameters(
            value.parameters
                .map((p) => `${p.name} = [${p.value.join("; ")}]`)
                .join("\n"),
        );
    }, [value]);
    const apply = () => {
        try {
            const next = validateFunctionKind({
                ...draft,
                inputs: inputs
                    .split("\n")
                    .filter((line) => line.trim())
                    .map((line) => {
                        const [name, width, extra] = line.split(":");
                        if (extra !== undefined)
                            throw new Error("输入格式：每行 名称: 宽度。");
                        return { name: name!.trim(), width: Number(width) };
                    }),
                parameters: parameters
                    .split("\n")
                    .filter((line) => line.trim())
                    .map((line) => {
                        const [name, value, extra] = line.split("=");
                        if (value === undefined || extra !== undefined)
                            throw new Error(
                                "参数格式：每行 名称 = 数值或列向量。",
                            );
                        return {
                            name: name!.trim(),
                            value: numericLiteral(value),
                        };
                    }),
            });
            onChange(next);
        } catch (error) {
            onError(error instanceof Error ? error.message : String(error));
        }
    };
    return (
        <div className="sim-function-inspector">
            <label>
                源码文件
                <input
                    aria-label="函数源码文件"
                    value={draft.source}
                    onChange={(e) =>
                        setDraft({ ...draft, source: e.target.value })
                    }
                />
            </label>
            <label>
                函数名
                <input
                    aria-label="函数名"
                    value={draft.entry}
                    onChange={(e) =>
                        setDraft({ ...draft, entry: e.target.value })
                    }
                />
            </label>
            <label>
                输入端口（每行 名称: 宽度）
                <textarea
                    aria-label="函数输入端口"
                    rows={3}
                    value={inputs}
                    onChange={(e) => setInputs(e.target.value)}
                />
            </label>
            <label>
                参数（每行 名称 = 数值）
                <textarea
                    aria-label="函数参数值"
                    rows={3}
                    value={parameters}
                    onChange={(e) => setParameters(e.target.value)}
                />
            </label>
            <label>
                输出宽度
                <input
                    aria-label="函数输出宽度"
                    type="number"
                    min={1}
                    max={4096}
                    value={draft.outputWidth}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            outputWidth: Number(e.target.value),
                        })
                    }
                />
            </label>
            <button onClick={apply}>应用函数接口</button>
            <button onClick={onOpen}>编辑 m 函数</button>
            <p className="sim-help">
                函数参数顺序为输入端口、参数。输出为实数列向量；支持局部赋值、算术、固定索引和
                sin、cos、exp、sqrt、abs、tanh。
            </p>
        </div>
    );
}
