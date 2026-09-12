import {
    DEFAULT_EXECUTION,
    parseExecution,
    type ExecutionOptions,
} from "./model";
import type { SimulationCatalog } from "./client";
export function SolverInspector({
    value = DEFAULT_EXECUTION,
    capabilities,
    onChange,
    onError,
}: {
    value?: ExecutionOptions | undefined;
    capabilities: SimulationCatalog["execution"];
    onChange(value: ExecutionOptions): void;
    onError(message: string): void;
}) {
    const update = (next: ExecutionOptions) => {
        try {
            onChange(parseExecution(next));
        } catch (error) {
            onError(String(error));
        }
    };
    return (
        <>
            <label>
                计算后端
                <select
                    aria-label="计算后端"
                    value={value.backend}
                    onChange={(e) =>
                        update({
                            ...value,
                            backend: e.target
                                .value as ExecutionOptions["backend"],
                        })
                    }
                >
                    <option value="reference">Rust reference</option>
                    <option
                        value="llvm"
                        disabled={!capabilities?.llvmConfigured}
                    >
                        LLVM
                        {capabilities?.llvmConfigured ? "" : "（服务器未配置）"}
                    </option>
                </select>
            </label>
            <label>
                求解器
                <select
                    aria-label="求解器"
                    value={
                        value.solver.type === "cvode"
                            ? value.solver.method
                            : "rk4"
                    }
                    onChange={(e) =>
                        update({
                            ...value,
                            solver:
                                e.target.value === "rk4"
                                    ? { type: "rk4" }
                                    : {
                                          type: "cvode",
                                          method: e.target.value as
                                              | "adams"
                                              | "bdf",
                                          relativeTolerance: 1e-6,
                                          absoluteTolerance: 1e-9,
                                      },
                        })
                    }
                >
                    <option value="rk4">RK4 · 固定步长</option>
                    <option
                        value="adams"
                        disabled={!capabilities?.cvodeConfigured}
                    >
                        CVODE Adams · 非刚性
                    </option>
                    <option
                        value="bdf"
                        disabled={!capabilities?.cvodeConfigured}
                    >
                        CVODE BDF · 刚性
                    </option>
                </select>
            </label>
            {value.solver.type === "cvode" && (
                <>
                    {(
                        [
                            ["relativeTolerance", "相对误差容限"],
                            ["absoluteTolerance", "绝对误差容限"],
                        ] as const
                    ).map(([key, title]) => (
                        <label key={key}>
                            {title}
                            <input
                                key={`${key}:${value.solver.type === "cvode" ? value.solver[key] : ""}`}
                                aria-label={title}
                                defaultValue={
                                    value.solver.type === "cvode"
                                        ? value.solver[key]
                                        : ""
                                }
                                onBlur={(e) => {
                                    if (value.solver.type !== "cvode") return;
                                    const next = Number(e.target.value);
                                    try {
                                        const execution = parseExecution({
                                            ...value,
                                            solver: {
                                                ...value.solver,
                                                [key]: next,
                                            },
                                        });
                                        onChange(execution);
                                    } catch (error) {
                                        onError(String(error));
                                        e.target.value = String(
                                            value.solver[key],
                                        );
                                    }
                                }}
                            />
                        </label>
                    ))}
                    <p className="sim-help">
                        CVODE
                        自适应控制步长；在离散采样时刻停止并更新状态。第一版使用稠密线性求解器，最多
                        2048 个连续状态。
                    </p>
                </>
            )}
        </>
    );
}
