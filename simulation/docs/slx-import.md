# SLX 导入与控制模型兼容范围

现在可以直接读取一部分 MATLAB/Simulink R2022b 的 `.slx` 模型，并交给
OpenMat 现有仿真引擎执行。当前源码中的 Web/Desktop 模型编辑器使用
`hybrid-v1` 配置；命令行保留旧配置为默认值，也可以显式选择新配置。
新配置叠加了 [多速率](multirate.md) 和 [非线性控制 / 复位事件](hybrid-control.md)，
保存的旧模型重新应用参数时仍沿用自己的导入配置。下面分节说明各版范围。
这些功能需要从当前源码构建，已发布安装包不会自动获得更新。

“文件结构可以读取”和“模型可以正确仿真”分别检查。遇到不支持的方块、
求解配置或参数，结构仍尽量保留，但运行会返回兼容性诊断。

## control-v1：层级、参数和连续控制方块

在模型编辑器中导入 `.slx` 后，左侧显示参数区和系统层级，中间显示原始
结构，右侧显示所选方块的原始参数。双击子系统进入，使用路径导航或
“返回上层”离开。缺少 `K`、`Ts` 等变量时，在参数区直接填写，或选择一个
自己准备的 `.m` 参数文件，然后点击“应用参数并检查”。例如：

```matlab
K = 2;
Ts = 0.05;
A = [-1 1; 0 -2];
B = eye(2);
C = eye(2);
D = zeros(2);
x0 = [1; -1];
```

这里使用普通 m 语法解析器读取常量赋值，支持标量、矩阵、转置、四则运算、
标量展开、矩阵乘法，以及 `zeros`、`ones`、`eye`、`reshape`、`sin`、`cos`、
`tan`、`exp`、`log`、`sqrt`、`abs`。参数按文件顺序定义；不访问全局工作区，
不执行任意脚本、`eval`、模型回调、函数定义、循环、索引赋值或数据字典。
缺失变量、尺寸错误、非有限值均报错。上限：64 KiB 文本、256 个变量，
单数组 4096 个数值，总存储 65536 个数值，并限制表达式深度和计算量。

除下文的六类基础方块外，新配置支持：

| 方块 | 当前范围 |
| --- | --- |
| SubSystem + Inport/Outport | 普通虚拟子系统、多层嵌套；端口从 1 连续编号；保留 SID 与模型路径 |
| Mux / Demux | 固定宽度实数向量拼接/拆分；不等宽拆分使用明确的宽度向量 |
| Ground / Terminator | 接地信号与未使用信号终止 |
| Product / Bias | 逐元素乘除和偏置；标量展开或匹配向量宽度 |
| Sine Wave | 时间模式、仿真时间输入、明确的连续采样 `SampleTime=0` |
| State-Space | 实数 A/B/C/D；1–32 个状态、输入、输出；初值和直接馈通检查 |
| Transfer Fcn | SISO 真有理或等阶传递函数，0–32 个状态；零初值 |
| Step | 明确的连续采样 `SampleTime=0`；有限时间、前后值与标量展开 |

信号是标量或固定宽度向量；参数矩阵不等同于矩阵信号。模型仍采用明确
`ode4`/`FixedStepDiscrete` 设置和单一 UnitDelay 周期。Sine Wave/Step 的
R2022b 内置缺省采样时间是继承值 `-1`，本配置会拒绝，必须显式设为 `0`。
根级 Inport/Outport 的外部数据输入输出尚未实现。原子、启用、触发子系统、
多速率、总线、Mask、库链接、S-function、Model Reference 和 Stateflow
仍不在支持范围内。

通过检查后可直接运行，或切到“查看 / 编辑数值模型”。该视图显示展开后的
OpenMat 运行图，支持独立修改；原始 SLX 结构保持供导航和检查使用。
重新应用参数会由原 SLX 重新生成运行图，界面会提示替换独立修改。
参数待应用或检查失败时，编辑器与 CLI 都禁止执行旧快照。

保存为 `.omsim`（JSON schema 4）会同时保存原始 SLX 包、层级、参数和
生成的 m 数值源码。单文件可以移动并重新打开运行。内嵌生成源码可以查看，
通过 SLX 参数重新生成；暂不单独导出到组件库。没有 SLX 写回/导出功能。
内嵌源码最多 64 个文件，单文件 64 KiB、总计 1 MiB；超过限制会给出诊断。

命令行用法：

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- inspect-slx 'C:/models/plant.slx' --slx-profile control-v1 --parameters 'C:/models/parameters.m'
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- import-slx 'C:/models/plant.slx' --slx-profile control-v1 --parameters 'C:/models/parameters.m' --output 'C:/models/plant.omsim'
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run 'C:/models/plant.omsim'
```

未指定 `--slx-profile control-v1` 时仍采用下面的旧导入配置。

## 旧配置使用方式

从仓库根目录运行，先把示例路径替换成自己的文件：

```powershell
$model = 'C:/models/feedback.slx'
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- inspect-slx $model
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- check $model
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run $model
```

`inspect-slx` 返回模型结构、`runnable` 和 `issues`。它的 `ok: true` 只表示
结构解析成功；能否执行以 `runnable` 为准。包损坏、引用丢失或结构歧义会
直接报错。其他命令遇到不兼容模型时退出码为 1，诊断以 JSON 写入 stderr。
方块诊断尽量包含原始 SID、参数名和所在 XML 部件，便于定位。

运行结果中的 `scopes` 说明每个 Scope 对应的扁平数值区间，`frames` 包含
时间和数值。当前 Scope 记录引擎接受的各个时刻，没有绘图窗口；也不复现
Simulink Scope 的抽点、记录文件和外观配置。

使用已经准备好的 LLVM 22 运行时：

```powershell
$env:OPENMAT_SIM_LLVM_LIBRARY = & ./simulation/tools/Prepare-Llvm.ps1
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run $model --backend llvm
```

该后端生成并执行真实的 LLVM ORC 数值机器码。LLVM 的准备和平台边界见
[仿真工作区说明](../README.md)。SLX 文件自身不能指定要加载的本机库。

也可以导出当前引擎使用的数值模型：

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- import-slx $model --output 'C:/models/imported.omsim.json'
```

输出路径必须尚不存在。这是已有实验性 JSON 格式的数值快照，会丢失 SLX
特有的编辑和未知元数据；原始 SLX 不会被修改，也没有 SLX 写回功能。
未来面向 PDE/有限元的 OpenMat 自有 OPC 格式不属于本次实现。

## 旧配置支持范围

| 方块 | 支持的配置 | 主要限制 |
| --- | --- | --- |
| Constant | 实数 double 标量或一维向量字面量；默认值 1 | 向量需按 1-D 输出，采样时间为 `inf` |
| Gain | 标量系数或与输入等宽的逐元素系数；默认值 1 | 不支持矩阵乘法及标量输入扩展为向量输出 |
| Sum | 2–64 个输入，`+`/`-` 符号或输入个数 | 输入等宽；不支持单输入求和及隐式广播 |
| Integrator | 内部初值；默认值 0；标量初值可以按输入宽度扩展 | 不支持复位、限幅、状态或饱和端口 |
| UnitDelay | 一个明确的正采样周期；默认初值 0 | 不支持继承周期、帧模式或多速率 |
| Scope | 一个输入，可观察向量 | 当前是数值观察器 |

执行要求 R2022b、normal 模式、单层模型，求解器为 `ode4` 或
`FixedStepDiscrete`，`FixedStep` 是明确的正数，`StartTime=0`，`StopTime`
有限且落在步长网格上。所有 UnitDelay 的周期必须相同，并且是步长的整数倍。
`FixedStepDiscrete` 模型中不能包含 Integrator。

字面量例子包括 `1`、`-0.5`、`1e-3`、`[1 2]`、`[1;2]`。`K`、`sin(1)`、
`[1 2;3 4]` 等工作区变量、表达式或矩阵不会被当成数值悄悄代入。

子系统结构、未知方块及未知部件可保留用于检查，但当前不执行子系统、
库链接、Model Reference、Mask、总线、复数/定点信号、代数环或零交叉事件。
模型回调、工作区/字典依赖和自定义方块默认值需要后续初始化机制，当前
会阻止运行。导入器本身不会启动 MATLAB，也不会执行这些内容。

已知外观、记录和代码生成元数据以及其他求解器的非活动选项只保留。
例如配置文件中保存的 `AbsTol` 不参与固定步长 RK4；这不意味着 OpenMat
能够执行该配置中的变量步长求解器。未知方块参数、求解器属性或配置组件
会给出诊断。当前兼容范围不是整个 Simulink 产品的等价实现。

## 本机对照验收

新配置使用独立编写的生成器：

```powershell
$env:OPENMAT_SLX_CONTROL_ORACLE_DIR = & ./simulation/tools/Invoke-SlxOracle.ps1 -Profile control
$env:OPENMAT_SIM_LLVM_LIBRARY = & ./simulation/tools/Prepare-Llvm.ps1
cargo test --manifest-path simulation/Cargo.toml --locked -p openmat-sim-slx --test control_oracle -- --ignored --nocapture
```

它在本机 R2022b 中生成 16 个可运行对照模型及两个继承采样时间的拒绝样例。
参考与 LLVM 后端分别逐时刻比较数值。常量速率信号在 MATLAB 中可能仅记录
一次；此时也检查 OpenMat 各帧保持相同数值。

本阶段的 Windows 本机验收中，两种后端均通过上述对照，最大绝对误差为
`7.105e-15`。浏览器完成了嵌套子系统导航、参数文件读取、F5 运行、内嵌回调
查看和保存后重开。将 State-Space 的 `.omsim` 单独复制到新目录后，CLI
仍能运行；21 个采样点、两个输出通道与 R2022b 对照的最大误差为 `2.22e-16`。
这些结果只覆盖本节列出的原创模型和配置，不代表完整 Simulink 兼容性。

连续 Step 在固定 ode4 中按 RK4 各阶段的时间求值，恰好到达阶跃时刻的阶段
使用阶跃后值；不会改成理想分段积分。显式选择 CVODE 时，求解器停在已知
阶跃边界，以左极限结束前一段，再用右极限重启。这是两种明确的求解方式，
不声称 CVODE 复现 Simulink 的可变步长求解器，也未实现一般零交叉定位。

旧配置的原有验收方式继续保留：

需要单独安装并许可 MATLAB/Simulink R2022b；普通构建和运行 SLX 不需要它。
PowerShell 7 下运行：

```powershell
$env:OPENMAT_SLX_ORACLE_DIR = & ./simulation/tools/Invoke-SlxOracle.ps1
$env:OPENMAT_SIM_LLVM_LIBRARY = & ./simulation/tools/Prepare-Llvm.ps1
cargo test --manifest-path simulation/Cargo.toml --locked -p openmat-sim-slx --test oracle -- --ignored --nocapture
```

脚本通过 PATH 寻找 MATLAB，也可传入 `-MatlabPath 'C:/path/to/matlab.exe'`。
`-OutputDirectory` 必须是新目录，以防旧结果被误当作本次验收。默认输出到
被 Git 忽略的 `simulation/.openmat/` 子目录，最多等待 600 秒。
Windows 启动逻辑集中在此脚本；其他平台可从 MATLAB 直接调用
`slx_oracle(output_directory)`，Rust 比较器没有 Windows 路径依赖。

生成器通过公开 API 创建四个 OpenMat 自编模型，并保存其 SLX 和数值观察：

| 模型 | 方程/行为 | 对照样本数 |
| --- | --- | --- |
| `om_slx_feedback` | `x' = 1 - x` | 21 |
| `om_slx_vector` | `x' = [1,2] - [1,2].*x`，标量初值展开 | 21 |
| `om_slx_counter` | `y[k+1] = 1 + y[k]`，周期 0.1 | 4 |
| `om_slx_sampled` | `x' = 1 - q`，q 为 x 的上一拍采样 | 9 |

比较器检查所有采样时刻和分量，容差为 `2e-12 * max(1, abs(expected))`。
首次本机 Windows 验收中，参考后端和 LLVM 后端均通过：前两个模型最大
绝对误差为 `2.22e-16`，后两个为零。这是上述模型的实测结果，不能外推为
所有 SLX 模型的兼容性保证。

普通 CI 使用内存中构造的原创 XML/ZIP 测试，不需要 MATLAB。Windows CI
额外要求一个 SLX 模型通过真实 LLVM ORC 执行；MATLAB 对照是显式的本机
可选验收，没有准备依赖时不会假装通过。生成的 SLX、观察值和日志不提交
到公开仓库。

## 实现与参考

代码分为通用 `openmat-opc`、SLX 文档与语义转换 `openmat-sim-slx`，再接
已有 `openmat-sim`/`openmat-sim-llvm`。包内关系决定部件位置，没有把
`simulink/systems/system_root.xml` 之类的固定路径当成唯一合法布局。
原始部件字节与节点所在部件/字节范围可通过 Rust API 检查。

OPC v0 支持普通 Stored/Deflated ZIP 和 UTF-8 XML；限制文件、部件、解压
大小、XML 节点和模型规模。不支持 ZIP64、加密、签名验证、DTD 或外部资源
获取；多重可能的 ZIP 结束记录会被作为歧义拒绝。详见
[RFC 0011](../../docs/rfcs/0011-slx-import-v0.md)。

SLX 采用 OPC 的依据是 MathWorks 的
[保存模型说明](https://www.mathworks.com/help/simulink/ug/save-models.html)；
通用包结构参考 [ECMA-376 Part 2](https://ecma-international.org/publications-and-standards/standards/ecma-376/)。
自建模型使用 [Simulink 的公开模型管理 API](https://www.mathworks.com/help/simulink/ug/create-load-open-save-and-close-models-programmatically.html)。
方块行为参考官方 [Gain](https://www.mathworks.com/help/simulink/slref/gain.html)、
[Unit Delay](https://www.mathworks.com/help/simulink/slref/unitdelay.html) 说明，并以本机 R2022b 的原创测试程序作数值对照。
