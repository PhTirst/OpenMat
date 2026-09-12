# SLX 导入 v0

现在可以直接读取一部分 MATLAB/Simulink R2022b 的 `.slx` 模型，并交给
OpenMat 现有仿真引擎执行。当前入口是独立的仿真命令行，还没有接入网页
编辑器或桌面安装包。

“文件结构可以读取”和“模型可以正确仿真”分别检查。遇到不支持的方块、
求解配置或参数，结构仍尽量保留，但运行会返回兼容性诊断。

## 使用

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

## 当前支持范围

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
