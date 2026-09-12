import { useEffect, useRef } from "react";
import type { SlxAsset } from "./slx-authoring";

export default function SlxParameters({
    asset,
    busy,
    onChange,
    onApply,
    onError,
}: {
    asset: SlxAsset;
    busy: boolean;
    onChange(text: string): void;
    onApply(): void;
    onError(message: string): void;
}) {
    const file = useRef<HTMLInputElement>(null);
    const current = useRef<SlxAsset | null>(asset);
    const mounted = useRef(false);
    current.current = asset;
    useEffect(() => {
        mounted.current = true;
        return () => {
            mounted.current = false;
        };
    }, []);
    const pending = asset.parameters !== asset.appliedParameters;
    return (
        <section className="sim-slx-parameters">
            <div className="sim-pane-title">
                SLX 参数{" "}
                <span>
                    {pending ? "待应用" : asset.runnable ? "已应用" : "需检查"}
                </span>
            </div>
            <p className="sim-help">
                在这里定义 K、Ts、A
                等数值参数。支持常量表达式、矩阵和简单数学函数。
            </p>
            <textarea
                aria-label="SLX 参数代码"
                spellCheck={false}
                disabled={busy}
                value={asset.parameters}
                placeholder={"K = 2;\nTs = 0.05;\nA = [-1 1; 0 -2];"}
                onChange={(event) => onChange(event.target.value)}
            />
            <div className="sim-slx-parameter-actions">
                <button disabled={busy} onClick={() => file.current?.click()}>
                    读取 .m 参数文件
                </button>
                <button disabled={busy} onClick={onApply}>
                    {busy ? "正在检查…" : "应用参数并检查"}
                </button>
            </div>
            <input
                type="file"
                accept=".m"
                hidden
                ref={file}
                onChange={async (event) => {
                    const input = event.target.files?.[0];
                    event.target.value = "";
                    if (!input) return;
                    const snapshot = current.current;
                    try {
                        if (input.size > 65536)
                            throw new Error("参数文件上限为 64 KiB。");
                        const text = await input.text();
                        if (mounted.current && current.current === snapshot)
                            onChange(text);
                    } catch (error) {
                        if (mounted.current && current.current === snapshot)
                            onError(String(error));
                    }
                }}
            />
            <p className="sim-help">
                这里只读取参数赋值，不执行初始化回调。应用参数会重新生成数值模型。
            </p>
            {asset.snapshotEdited && (
                <p className="sim-help sim-stale">
                    数值模型已有独立修改；重新应用参数会替换这些修改。
                </p>
            )}
        </section>
    );
}
