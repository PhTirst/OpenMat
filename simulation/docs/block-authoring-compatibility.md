# 方块外观与参数编辑 v1

这一阶段对齐 R2022b 常用方块的数学表示、参数名称和参数归属。React 前端与
Rust 仿真服务继续分离，沿用 OpenMat 配色；并不表示已经完整兼容 Simulink。

## 直接体验

打开「模型编辑器」，选择 **模型参数 · A − Kx 反馈**，按 F5。模型为
`x' = A - K*x`，初始参数 `A=1; K=2; x0=0;`，响应趋近 0.5。

1. 选择三角形 Gain，右侧「增益」显示 `K` 和当前解析值 `[2]`。
2. 改为 `2*K`，点击「应用参数」，再运行，响应趋近 0.25。
3. 工具栏「模型参数」中修改 `K`，点击「应用模型参数」。所有绑定一起重新求值。
4. 双击 Gain、Integrator 等常用方块，在弹窗中编辑同一套参数。双击子系统仍然
   进入内部，Scope 仍然打开结果区域。
5. 保存为 `.omsim` 后重开，模型变量定义、参数原文和最后解析值一并保留。

参数草稿需要应用才生效；未应用或求值失败时不能运行，也不能通过保存写入旧参数。
「还原编辑」恢复已应用值。切换方块或关闭参数窗口会放弃当前未应用的表单草稿。
复制方块会复制它的参数绑定，变量仍引用同一份模型参数；撤销恢复绑定和数值。

## 方块与检查器

常用块采用独立绘制的数学符号：Gain 三角形、Sum 求和节点、积分器 `1/s`、
Unit Delay `1/z`、传递函数分式、状态空间方程、Mux/Demux 汇线等。名称在方块
下方，数值或表达式显示在图形内。条件子系统控制输入位于上方，数据端口在两侧。

共同参数由 `simulation/blocks/common.json` 描述。原生检查器、双击弹窗和 SLX
原始属性面板使用同一目录中的参数名称与分组，避免维护两套相互矛盾的字段。

| 方块 | 常用参数 |
| --- | --- |
| Constant / Gain | Value / Gain、SampleTime；Gain 明确标注逐元素乘法 |
| Sum / Product | Inputs、SampleTime；Product 当前至少两个输入 |
| Integrator / Discrete Integrator | InitialCondition、ExternalReset；离散积分为 Forward Euler |
| Unit Delay / Zero-Order Hold | 初值（适用时）、SampleTime |
| Transfer Fcn | Numerator、Denominator |
| State-Space | A、B、C、D、InitialCondition；可一起修改维数 |
| Mux / Demux | 输入数量 / 输出数量 |
| Inport / Outport | 端口号；条件域 Outport 有初值和停用策略 |
| Enable / Trigger | 状态策略、触发边沿（适用时）、OpenMat 控制周期 |

未实现的矩阵 Gain、积分器外部初值/限幅/其他积分方式等选项明确禁用。
信号属性说明当前支持范围；不会显示尚未生效的复数、定点或矩阵信号配置。
自定义 m 组件和非共同目录方块继续使用已有编辑入口。

## Enable、Trigger 与 Outport 的归属

进入条件子系统后，可以选择内部的 **Enable** 或 **Trigger** 控制方块，修改
状态保持/复位、触发边沿及控制周期。选择内部 **Outport** 修改该端口的
`InitialOutput` 和 `OutputWhenDisabled`。这些参数不再集中放在外部子系统面板。
外部子系统保留模式设置与内部导航。

方块库可以向普通子系统添加 Enable/Trigger；一个子系统只有一种控制。
移动控制块的位置会保存；删除控制块会移除父级执行配置和控制连线，内部计算
块仍保留。撤销可以恢复；复制整个子系统保留内部控制位置。不能单独复制控制块。

控制块是父级执行配置的编辑投影，数值模型仍只有一份权威执行策略，避免控制块
和父级字段不一致。Outport 策略按端口号映射到这一配置。
原有[条件执行限制](conditional-subsystems.md)继续适用：控制周期必须明确，
条件域内没有连续状态，也不支持嵌套条件域。导入时 `InitialOutput=[]` 的
继承初值仍不支持，不能将参考默认值误认为已经实现的行为。

## 参数语言与持久化

模型参数使用已有 SLX 参数解析器的受限 m 表达式：赋值、标量/向量/矩阵字面量、
已定义变量、算术以及 `pi`、`zeros`、`ones`、`eye`、`linspace` 和常用初等函数。
具体边界见 [SLX 导入指南](slx-import.md)。禁止任意脚本、循环、文件访问、
`eval` 和交互工作区读写；这里没有普通 m 语言 JIT。

参数工作区属于模型，独立于命令窗口。服务器原子求值和验证全部绑定，成功后
提交新的数值快照。未定义变量、维数错误或不支持选项会拒绝提交，保留旧模型及
当前表单输入。断开服务时仍可编辑数值字面量；一般表达式需要服务连接。
采样时间也接受 `[Ts 0]` 和 `[inf 0]`；非零偏移暂不支持。

`.omsim` 继续使用 JSON。编辑文档版本 **9** 新增：

```json
{
  "schemaVersion": 9,
  "parameters": {
    "source": "K = 2;",
    "bindings": { "gain": { "Gain": "2*K" } }
  }
}
```

以上是新增字段片段，并非完整文件。`model` 内保存解析后的数值；其版本仍然为
1–8，取决于模型功能。`editor.controlPositions` 保存内部控制块位置。
服务器和 CLI 检查/运行时重新计算 `parameters`，不会信任旧数值缓存。
旧版编辑器不能打开文档版本 9；当前版本仍能读取旧文档。

WebSocket **`/simulation/v9`** 与内核共用既有服务器端口，新增
`resolveParameters` 和 check/run 的可选 `parameters`；旧路由拒绝这些字段。
前端可以求值尚未连完的模型，完整连通性与类型检查仍由模型编译器负责。

SLX 原始视图将属性分组，并区分「原文件显式值」与「R2022b 参考默认值」。
后者仅用于理解参数，不替代导入器的默认值继承与兼容性检查。转换到可编辑模型时，
受支持块的显式表达式继续保留。原始 SLX 包不被改写；重新应用 SLX 参数会替换
独立编辑的数值快照及绑定。此阶段不提供 SLX 写回。

## 验证与独立对照

自动测试覆盖参数失败与旧缓存保护、原文保存/复制/撤销、矩阵行列、维数联动、
内部控制块与 Outport 的关联、旧版本协议拒绝、实际 WebSocket 与 CLI 执行。

`simulation/tools/block_editor_oracle.m` 是项目独立编写的可选 R2022b 探针，
查询标准库块的选定参数默认值，并求解 8 组模型参数表达式。它只输出规范化值，
不复制 MathWorks 的实现、测试、文档或诊断文字。生成数据放在忽略目录，不提交。

```powershell
& '<MATLAB R2022b>/bin/matlab.exe' -batch "addpath('simulation/tools'); block_editor_oracle('simulation/.openmat/block-editor-oracle')"
$env:OPENMAT_BLOCK_EDITOR_ORACLE = (Resolve-Path simulation/.openmat/block-editor-oracle/block-editor.json).Path
cargo test --manifest-path simulation/Cargo.toml --locked -p openmat-sim-slx --test native_parameters -- --include-ignored
```

这一对照仅验证选定默认参数和常量表达式，不等同于完整 Simulink 仿真、库、Mask、
数据类型或版本兼容。数值运行仍需各功能的独立对照和回归测试。

本阶段本机验收：前端全套 625 项、独立仿真工作区 133 项常规测试、服务器 82 项
单元测试及 8 项常规仿真传输测试通过；另行运行 R2022b 参数探针和 LLVM/CVODE
传输验收通过。工作区内需要其他外部运行库/历史生成探针的 31 项测试未在常规
命令中执行，不能据此宣称这些对照已经全部重跑。

真实浏览器完成 `2*K` 编辑/求值失败保留、双击弹窗、模型变量修改、保存重开、
Enable/Outport 策略修改及 LLVM + CVODE 运行、SLX 原始参数与可编辑参数衔接。
浏览器保存的参数化反馈模型由 CLI 重算，末值为 `0.2499999995`。
