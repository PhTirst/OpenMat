import { CommitField } from "./CommitField";
import { numericLiteral, type BlockKind } from "./model";
import { parseHybridKind, type ControlOperation } from "./hybrid";

export function HybridInspector({
    kind,
    disabled,
    onChange,
    onError,
}: {
    kind: BlockKind;
    disabled: boolean;
    onChange(value: BlockKind): void;
    onError(message: string): void;
}) {
    const commit = (candidate: unknown) => {
        try {
            onChange(parseHybridKind(candidate));
            return true;
        } catch (error) {
            onError(error instanceof Error ? error.message : String(error));
            return false;
        }
    };
    const field = (
        name: string,
        values: number[],
        change: (value: number[]) => unknown,
    ) => (
        <label>
            {name}
            <CommitField
                name={name}
                value={values.join(", ")}
                onCommit={(text) => {
                    try {
                        return commit(change(numericLiteral(text)));
                    } catch (error) {
                        onError(
                            error instanceof Error
                                ? error.message
                                : String(error),
                        );
                        return false;
                    }
                }}
            />
        </label>
    );
    const scalar = (
        name: string,
        value: number,
        change: (value: number) => unknown,
    ) =>
        field(name, [value], (values) => {
            if (values.length !== 1)
                throw new Error(`${name}需要一个有限实数。`);
            return change(values[0]!);
        });
    if (kind.type === "integrator" || kind.type === "discreteIntegrator")
        return (
            <button
                disabled={disabled}
                onClick={() =>
                    commit({
                        type: "resetIntegrator",
                        initial: kind.initial,
                        gain:
                            kind.type === "discreteIntegrator" ? kind.gain : 1,
                        discrete: kind.type === "discreteIntegrator",
                        reset: "rising",
                    })
                }
            >
                启用外部复位端口
            </button>
        );
    if (kind.type === "resetIntegrator")
        return (
            <fieldset className="sim-hybrid-fields" disabled={disabled}>
                <legend>积分与复位</legend>
                {field("初始状态", kind.initial, (initial) => ({
                    ...kind,
                    initial,
                }))}
                {scalar("积分增益", kind.gain, (gain) => ({ ...kind, gain }))}
                <label>
                    积分方式
                    <select
                        aria-label="积分方式"
                        value={kind.discrete ? "discrete" : "continuous"}
                        onChange={(e) =>
                            commit({
                                ...kind,
                                discrete: e.target.value === "discrete",
                            })
                        }
                    >
                        <option value="continuous">连续积分</option>
                        <option value="discrete">离散 Forward Euler</option>
                    </select>
                </label>
                <label>
                    复位边沿
                    <select
                        aria-label="复位边沿"
                        value={kind.reset}
                        onChange={(e) =>
                            commit({ ...kind, reset: e.target.value })
                        }
                    >
                        <option value="rising">上升沿</option>
                        <option value="falling">下降沿</option>
                        <option value="either">任意边沿</option>
                    </select>
                </label>
                <p className="sim-help">
                    in 为被积信号；reset
                    为标量复位信号。触发后恢复初始状态。离散积分器在自身采样时刻检测复位。
                </p>
            </fieldset>
        );
    if (kind.type !== "control") return null;
    const op = kind.operation;
    const operation = (value: ControlOperation) => ({
        ...kind,
        operation: value,
    });
    return (
        <fieldset className="sim-hybrid-fields" disabled={disabled}>
            <legend>控制参数</legend>
            {op.type === "saturation" && (
                <>
                    {field("下限", op.lower, (lower) =>
                        operation({ ...op, lower }),
                    )}
                    {field("上限", op.upper, (upper) =>
                        operation({ ...op, upper }),
                    )}
                </>
            )}
            {op.type === "switch" && (
                <>
                    <label>
                        选择条件
                        <select
                            aria-label="选择条件"
                            value={op.criterion}
                            onChange={(e) =>
                                commit(
                                    operation({
                                        ...op,
                                        criterion: e.target
                                            .value as typeof op.criterion,
                                    }),
                                )
                            }
                        >
                            <option value="greaterEqual">u2 ≥ 阈值</option>
                            <option value="greater">u2 &gt; 阈值</option>
                            <option value="nonzero">u2 ≠ 0</option>
                        </select>
                    </label>
                    {op.criterion !== "nonzero" &&
                        scalar("切换阈值", op.threshold, (threshold) =>
                            operation({ ...op, threshold }),
                        )}
                    <p className="sim-help">
                        in0 / u1：条件成立时的数据；in1 / u2：控制信号；in2 /
                        u3：条件不成立时的数据。
                    </p>
                </>
            )}
            {op.type === "relational" && (
                <label>
                    比较运算
                    <select
                        aria-label="比较运算"
                        value={op.operator}
                        onChange={(e) =>
                            commit(
                                operation({
                                    ...op,
                                    operator: e.target
                                        .value as typeof op.operator,
                                }),
                            )
                        }
                    >
                        {(
                            [
                                ["equal", "="],
                                ["notEqual", "≠"],
                                ["less", "<"],
                                ["lessEqual", "≤"],
                                ["greater", ">"],
                                ["greaterEqual", "≥"],
                            ] as const
                        ).map(([value, label]) => (
                            <option key={value} value={value}>
                                {label}
                            </option>
                        ))}
                    </select>
                </label>
            )}
            {op.type === "logical" && (
                <label>
                    逻辑运算
                    <select
                        aria-label="逻辑运算"
                        value={op.operator}
                        onChange={(e) => {
                            const operator = e.target
                                .value as typeof op.operator;
                            commit(
                                operation({
                                    ...op,
                                    operator,
                                    inputs:
                                        operator === "not"
                                            ? 1
                                            : Math.max(2, op.inputs),
                                }),
                            );
                        }}
                    >
                        {["and", "or", "xor", "nand", "nor", "not", "nxor"].map(
                            (value) => (
                                <option key={value} value={value}>
                                    {value.toUpperCase()}
                                </option>
                            ),
                        )}
                    </select>
                </label>
            )}
            {op.type === "minMax" && (
                <label>
                    极值运算
                    <select
                        aria-label="极值运算"
                        value={op.minimum ? "min" : "max"}
                        onChange={(e) =>
                            commit(
                                operation({
                                    ...op,
                                    minimum: e.target.value === "min",
                                }),
                            )
                        }
                    >
                        <option value="min">最小值</option>
                        <option value="max">最大值</option>
                    </select>
                </label>
            )}
            {(op.type === "minMax" ||
                (op.type === "logical" && op.operator !== "not")) &&
                scalar("输入端口数量", op.inputs, (inputs) =>
                    operation({ ...op, inputs }),
                )}
            {(op.type === "relational" || op.type === "logical") && (
                <p className="sim-help">
                    输出为 logical，数值结果显示为 0 或 1。
                </p>
            )}
            {op.type !== "logical" && (
                <label>
                    <input
                        type="checkbox"
                        checked={kind.zeroCrossing}
                        onChange={(e) =>
                            commit({ ...kind, zeroCrossing: e.target.checked })
                        }
                    />
                    零交叉检测
                </label>
            )}
            {kind.zeroCrossing && (
                <p className="sim-help">
                    RK4 在主步记录穿越；CVODE 可定位连续信号的零交叉时刻。
                </p>
            )}
        </fieldset>
    );
}
