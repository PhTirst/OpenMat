import { CommitField } from "./CommitField";
import { componentFor } from "./components";
import type { Block, BlockKind, Model } from "./model";
import { sampleTimeText, type SampleTime } from "./sampling";

export function SamplingInspector({
    model,
    block,
    resolved,
    onChange,
    onKindChange,
    onError,
}: {
    model: Model;
    block: Block;
    resolved?: SampleTime | undefined;
    onChange(rate: SampleTime | undefined): void;
    onKindChange(kind: BlockKind): void;
    onError(message: string): void;
}) {
    const configured =
        model.sampleTimes && Object.hasOwn(model.sampleTimes, block.id)
            ? model.sampleTimes[block.id]
            : undefined;
    const definition = componentFor(model, block);
    const fixed =
        block.kind.type === "integrator" ||
        (block.kind.type === "resetIntegrator" && !block.kind.discrete) ||
        block.kind.type === "scope" ||
        (definition &&
            (definition.continuousStates > 0 ||
                (definition.sampleTime ?? -1) > 0));
    const positive = (text: string, allowZero = false): number | undefined => {
        const value = Number(text);
        if (
            !text.trim() ||
            !Number.isFinite(value) ||
            (!allowZero && value <= 0)
        ) {
            onError("请输入有限正数。");
            return undefined;
        }
        return value;
    };
    return (
        <>
            <h3>采样时间</h3>
            <p className="sim-help" aria-label="实际采样时间">
                实际：{sampleTimeText(resolved)}
            </p>
            {fixed ? (
                <p className="sim-help">
                    {block.kind.type === "scope"
                        ? "跟随输入信号的采样时刻记录。"
                        : definition?.sampleTime && definition.sampleTime > 0
                          ? `组件状态更新周期：${definition.sampleTime} s`
                          : "连续状态输出；使用 Zero-Order Hold 对信号采样。"}
                </p>
            ) : (
                <>
                    <label>
                        配置
                        <select
                            aria-label="采样时间类型"
                            value={configured?.kind ?? "default"}
                            onChange={(e) => {
                                const kind = e.target.value;
                                onChange(
                                    kind === "default"
                                        ? undefined
                                        : kind === "discrete"
                                          ? {
                                                kind,
                                                period:
                                                    model.settings.sampleTime ??
                                                    model.settings.maxStep,
                                            }
                                          : {
                                                kind: kind as
                                                    | "inherited"
                                                    | "continuous"
                                                    | "constant",
                                            },
                                );
                            }}
                        >
                            <option value="default">方块默认</option>
                            <option value="inherited">继承</option>
                            <option value="discrete">离散周期</option>
                            <option value="continuous">连续</option>
                            {block.kind.type === "constant" && (
                                <option value="constant">常量</option>
                            )}
                        </select>
                    </label>
                    {configured?.kind === "discrete" && (
                        <label>
                            周期（秒）
                            <CommitField
                                name="方块采样周期"
                                value={String(configured.period)}
                                onCommit={(text) => {
                                    const period = positive(text);
                                    if (period === undefined) return false;
                                    onChange({ kind: "discrete", period });
                                }}
                            />
                        </label>
                    )}
                    <p className="sim-help">
                        周期需为模型最大步长的整数倍。点击“检查模型”更新实际采样时间。
                    </p>
                </>
            )}
            {block.kind.type === "discreteIntegrator" && (
                <label>
                    积分增益（Forward Euler）
                    <CommitField
                        name="离散积分增益"
                        value={String(block.kind.gain)}
                        onCommit={(text) => {
                            const gain = positive(text, true);
                            if (gain === undefined) return false;
                            if (block.kind.type === "discreteIntegrator")
                                onKindChange({ ...block.kind, gain });
                        }}
                    />
                </label>
            )}
            {block.kind.type === "rateTransition" && (
                <>
                    <label>
                        <input
                            type="checkbox"
                            aria-label="确定性速率转换"
                            checked={block.kind.deterministic}
                            onChange={(e) => {
                                if (block.kind.type === "rateTransition")
                                    onKindChange({
                                        ...block.kind,
                                        deterministic: e.target.checked,
                                    });
                            }}
                        />
                        确定性传输与数据完整性
                    </label>
                    <p className="sim-help">
                        慢转快启用时延迟一个慢采样周期；关闭时读取最近的源样本。快转慢在目标采样时刻读取源信号。当前支持单任务、零偏移、整数倍速率。
                    </p>
                </>
            )}
        </>
    );
}
