# 条件执行子系统 v1

当前源码支持 Enabled Subsystem 和 Triggered Subsystem。Web 与 Desktop 共用编辑器，
计算仍在 Rust 服务中执行；可使用 reference / LLVM 数值后端及 RK4 / CVODE 求解器。
数值模型版本为 8。当前编辑器使用 `/simulation/v9`；参数表达式或内部控制位置
使用编辑文档版本 9，旧文档仍可读取。v1–v7 路由拒绝数值版本 8，避免忽略执行条件。

## 直接体验

打开模型编辑器，在「示例模型」中选择：

- **启停控制 · 离散 PI + 连续对象**：50 ms 离散 PI 在 0.5 秒启用，3 秒停用，
  4.5 秒恢复。停用时输出归零，内部积分状态保持；连续对象放在外层。
- **触发计数 · m 组件与私有状态**：50 ms 采样的正弦控制信号，在 0.1、1.1、2.1、
  3.1、4.1 秒触发计数。内部 m 组件每次调用加一，内部 Scope 只记录这 5 次调用。
  外部 Scope 同时显示控制信号及保持的计数输出。改为任意边沿后有 10 次调用。

按 F5 运行；Scope 提供执行事件标记和事件列表。原生模型的内嵌 m 回调可在底部
「m 函数」面板编辑，并随 `.omsim` 保存。SLX 生成的回调仍通过原始参数重新生成。
复制、粘贴、撤销、保存及重开都会保留执行配置和子系统边界。

## 编辑与语义

方块库的「条件执行」分组包含两种子系统以及 Enable / Trigger。拖入子系统后进入内部，添加
Inport / Outport 和计算方块。数据端口使用 `in1` / `out1` 等编号；`enable` / `trigger`
是上方独立的标量控制端口。选择内部 Enable / Trigger 修改状态策略、周期及触发
边沿；选择内部 Outport 修改各输出初值和停用策略，点击「应用参数」提交。
外部子系统保留执行模式与导航。详见[参数编辑指南](block-authoring-compatibility.md)。

| 设置 | 行为 |
| --- | --- |
| Enabled 控制大于 0 | 在声明的采样时刻执行内部方块 |
| Enabled 控制小于或等于 0 | 跳过内部计算与状态更新 |
| 重新启用时状态保持 | 从停用前最后提交的离散状态继续 |
| 重新启用时状态复位 | 先恢复各内部状态初值，再计算本次输出与更新 |
| 输出保持 | 停用期间保持最近一次输出 |
| 输出恢复初值 | 停用期间使用该 Outport 的输出初值 |
| Triggered | 按上升沿、下降沿或任意边沿调用一次，调用之间保持输出 |

内部状态初值与 Outport 初值独立。例如 Unit Delay 初值 3、Outport 初值 -7，
初始未启用时输出 -7，首次启用输出 3。每个输出可单独选择保持/复位，初值允许
标量广播或与输出宽度相同的向量；逻辑输出只接受 0、1。

Forward Euler 离散积分器在停用时保留最后已经输出的积分值，丢弃尚未输出的下一步
增量。Unit Delay 和显式 m `update` 则保留下一次调用的状态；两者不能用同一个
停用规则处理。

触发器在初始时刻建立符号基准，不产生触发。负数到零、零到正数属于上升变化；
穿过零的连续两个采样不会被计为同一次方向的两次触发。经过更长的零平台后再离开
零可以形成新事件。这些边界行为经过自行编写的 R2022b 模型对照。

子系统内每个有状态 m 组件拥有独立的 `q`。Triggered 内使用 `sampleTime: -1`，
`outputsFunction` 读取调用前的状态，`update` 生成供下次调用使用的状态。禁用期间
不会求值内部数值表达式或输出/更新回调；初始化回调用于建立各实例初值。
同一仿真时刻发生求解器重试、外部复位或重复求值，也只提交一次有效状态更新。

组件库文件版本 2 支持纯离散组件的 `sampleTime: -1`；正周期组件仍保存为版本 1。
版本 1 保持原来的校验规则。普通 m 语言运行方式没有改变。

## SLX 导入

编辑器新导入使用 `conditional-v1`，原来保存的模型仍使用其记录的导入配置。
新配置支持本页限定范围内的 EnablePort / TriggerPort，保留子系统的执行边界、
控制线分支、Outport 策略及原始 SLX 结构。

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- import-slx model.slx --slx-profile conditional-v1 --output model.omsim
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run model.omsim
```

第一版的边界：

- 控制信号为单个实数或逻辑值，按一个明确的离散周期采样。SLX 控制周期必须能从
  信号源推断；常量控制可采用唯一的内部周期，未指定时采用模型基础步长。
- 固定宽度信号、每个 Enabled 域一个周期。允许普通子系统嵌套，暂不允许条件域嵌套。
- 内部不支持连续状态、带外部复位的积分器和 Rate Transition。
  Triggered 内另外禁止显式正周期方块、周期积分器和 Zero-Order Hold。
- SLX 要求 Simplified 初始化，Outport **显式填写有限的 InitialOutput**。
  默认的 `[]` 继承初值暂不支持，会报告诊断，不会猜成 0。
- 不支持 Enable/Trigger 的输出端口、组合使能触发、函数调用子系统、异步调度、
  Stateflow、Simscape 或 SLX 回写。

连续对象可以连接在条件域外侧，由现有 RK4、CVODE Adams 或 BDF 求解。
不支持的配置会阻止运行并尽量给出源方块位置。

## 验证

从仓库根目录运行通用验证：

```powershell
cargo fmt --manifest-path simulation/Cargo.toml --all --check
cargo clippy --manifest-path simulation/Cargo.toml --locked --workspace --all-targets -- -D warnings
cargo test --manifest-path simulation/Cargo.toml --locked --workspace
cargo clippy --locked -p openmat-server --all-targets -- -D warnings
cargo test --locked -p openmat-server --lib
cargo test --locked -p openmat-server --test simulation_transport
pnpm --dir apps/web typecheck
pnpm --dir apps/web exec vitest run --maxWorkers=4
pnpm --dir apps/web exec vite build
node tools/release/check-public-source.mjs --staged
```

通用测试覆盖状态/输出策略分离、触发启动和零平台、复制实例的私有状态、禁用期间
跳过无效 m 表达式、失败不发布部分状态、外部复位同时发生、端口重编号、文件版本、
组件库、保存重开及搬移独立模型文件。LLVM 使用真正的条件分支跳过内部指令。
可选原生测试比较两个示例的 reference、LLVM 和 CVODE 轨迹及调用事件。

R2022b 对照模型通过 OpenMat 自行编写的公开 API 调用生成，覆盖 Unit Delay 和
离散积分器的启停状态/输出策略、初始控制符号及三种触发边沿。运行方式：

```powershell
pwsh -NoProfile -File simulation/tools/Invoke-SlxOracle.ps1 -Profile conditional -OutputDirectory simulation/.openmat/conditional-oracle
$env:OPENMAT_SLX_CONDITIONAL_ORACLE_DIR = (Resolve-Path simulation/.openmat/conditional-oracle).Path
cargo test --manifest-path simulation/Cargo.toml --locked -p openmat-sim-slx --test conditional_oracle conditional_reference -- --ignored
# 按现有指南配置 LLVM / SUNDIALS 后：
cargo test --manifest-path simulation/Cargo.toml --locked --workspace conditional -- --ignored
```

生成的 SLX、观察值和原生依赖仅保存在忽略目录，不进入公开源码。
