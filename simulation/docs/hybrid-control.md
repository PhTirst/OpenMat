# 非线性控制与复位事件

当前源码的模型编辑器使用 `/simulation/v6`，与 kernel 共用服务器端口。
新增模型保存为 JSON schema 6；旧 schema 1–5 和旧 SLX 配置仍可使用。
普通 m 语言执行器没有变化，下面的 LLVM 编译仅针对方块图数值程序。

## 方块与编辑

方块库的“控制逻辑”分类中提供以下方块，均有独立图标和检查器。
支持有限实数标量、固定宽度向量；一般支持标量广播，不支持动态尺寸、
矩阵信号、复数、定点或整数溢出语义。

| 方块 | 输入与行为 |
| --- | --- |
| Saturation | `in0`，上下限可以是标量或与信号等宽的向量 |
| Switch | `in0` 为条件成立时的数据，`in1` 为控制信号，`in2` 为条件不成立时的数据；支持 `>`、`>=`、`~=0` |
| Relational Operator | 两个输入；支持六种比较，输出 logical |
| Logical Operator | AND/OR/XOR/NAND/NOR/NXOR，2–64 个输入；NOT 一个输入；输出 logical |
| Abs | 逐元素绝对值 |
| MinMax | 2–64 个输入时逐元素极值；一个输入时将整个向量归约为标量 |
| Reset Integrator | 连续积分，`in` 为被积信号，`reset` 为标量复位信号 |
| Reset Discrete Integrator | 自身离散时钟上的 Forward Euler 积分与复位 |

复位支持上升沿、下降沿和任意边沿，触发后恢复内部初值；支持积分增益。
普通 Integrator / Discrete Integrator 的检查器也可启用外部复位端口。
修改端口数量时，删除已经不存在的输入端口连线，撤销可恢复完整操作。
检查模型后可查看输出类型与事件面数量；运行后 Scope 显示事件标记和时间列表。
事件列表表示整个模型的事件，灰色虚线为穿越，红色虚线为实际复位。
采样 Scope 仅记录自身时钟上的点，事件标记仍显示其他时刻发生的事件。

logical 在数值接口中使用准确的 0/1 表示。Switch 的两个数据输入类型必须
一致。logical 信号经过 Unit Delay / Rate Transition 时，初值当前要求
为 0/1；其他数值转换请显式完成，不会静默模拟完整 Simulink 类型系统。

## 时间与求解器

固定步长 RK4 在各级计算控制方块的即时输出，在接受的主步上检测并执行复位。
例如连续复位信号在 0.137 s 穿过零、步长为 0.01 s 时，复位发生在 0.14 s。
离散积分器的周期为 0.05 s 时，同一信号在 0.15 s 才被检测。

配置 SUNDIALS 后选择 CVODE Adams 或 BDF，可在主步之间定位连续零交叉。
上面的连续复位可定位到 0.137 s。CVODE 的试算和根函数求值都不会发布状态；
定位成功后，调度器统一处理复位、重新计算输出，再重启积分。根函数使用纯
数值 IR 的参考执行器；主输出、导数和离散更新继续使用所选 reference/LLVM 后端。
在同一时刻命中离散时钟时，先提交待更新的离散状态，再检测复位，并在提交帧前
重新计算采样输出和下一次离散状态。事件失败保留上一接受帧。

本版用独立编写的 R2022b 模型确认了零值边沿语义：

- 连续积分器区分负、零、正。负到零及零到正都可能形成上升沿；同一次穿越在
  相邻主步经过零时不会重复复位。零保持更长时间后离开零，则可形成新的边沿。
- 离散积分器按“正值 / 非正值”检测：非正到正为上升沿，正到非正为下降沿。
- 初始时刻以初值开始，并初始化边沿记忆，不把初始信号当成一次外部边沿。

schema 6 使用“周期 × 整数序号”构造离散命中时间，与对照模型的时间网格匹配；
时钟身份仍由整数计数维护。浮点阈值附近的相等判断会影响边沿，应显式设置
Step 时间与采样周期，不把求解容限理解为比较运算的容差。

限制：最多 256 个事件面、每帧 512 条事件、每次运行 100000 条事件、16 轮
复位传播；编译和运行另有原有预算。直接或间接的复位状态环会给出诊断。
可以使用真正有状态的延迟打断环；同一步可采样的 Hold 不会隐藏该依赖。
暂不支持 level reset、外部初值端口、状态输出端口、限幅积分器、触发/启用
子系统、一般代数环求解。根定位需要适合括根的有限连续函数；不保证检测
切触零点或一步内的任意多个根。可关闭不需要的控制方块零交叉检测。

## SLX 导入

新导入默认使用 `hybrid-v1`，在 `multirate-v1` 范围上新增上述方块和复位。
保存的老模型保持原导入配置，重新应用参数沿用其配置。检查不支持的参数时
保留原始 SID、模型路径和参数名；不忽略关键行为后勉强运行。
Logic 的 R2022b 缺省 `AllPortsSameDT=on` 要求 logical 输入，可连接比较输出。

```powershell
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- import-slx model.slx --slx-profile hybrid-v1 --output imported.omsim
cargo run --manifest-path simulation/Cargo.toml --locked -p openmat-sim-cli -- run imported.omsim
```

`.omsim` 中保存运行图、编辑信息、SLX 原包及其结构、已应用参数和内嵌源码。
本阶段的控制方块直接编译为数值运算，不生成多余的 m 回调文件。
原 SLX 结构与独立编辑后的数值图分别保留；仍没有 SLX 写回功能。

## 立即体验

模型编辑器右上角“示例模型”提供三个完整示例，默认使用可直接运行的 RK4：

- `saturated-pi.omsim.json`：限幅执行器与反算抗积分饱和；分别观察对象响应和控制量。
- `switched-control.omsim.json`：2 秒切换反馈增益，观察闭环响应变化。
- `periodic-reset.omsim.json`：同一周期信号驱动连续与 50 ms 离散积分器；触发函数
  内嵌在文件中，可切换到 CVODE 对比事件时间。

这些文件都是自包含的 `.omsim` 文档，可单独复制到其他目录执行。
CLI 的执行参数由命令行指定；浏览器保存的求解器选择不会覆盖 CLI 参数。

## 验证与边界

`tools/slx_hybrid_oracle.m` 使用公开模型 API 创建 21 个 OpenMat 自编模型，
覆盖六种运算、闭环 PI/切换、向量归约，以及连续/离散三种复位边沿与零值保持。
在本次本地 MATLAB R2022b 验证中，reference 和 LLVM 的轨迹最大绝对差约
`1.33e-15`。这证明这些具体配置的行为，不代表完整 Simulink 兼容。
生成的 SLX、观测值与本机路径留在忽略目录，不进入公开源码。

```powershell
pwsh -NoProfile -File simulation/tools/Invoke-SlxOracle.ps1 -Profile hybrid
# 将上一步输出目录设为 OPENMAT_SLX_HYBRID_ORACLE_DIR，并配置 LLVM DLL 后：
cargo test --manifest-path simulation/Cargo.toml -p openmat-sim-slx --test hybrid_oracle -- --ignored --nocapture
cargo test --manifest-path simulation/Cargo.toml --workspace
# 配置 SUNDIALS 目录后验证解析复位时刻、周期重启和回调失败边界：
cargo test --manifest-path simulation/Cargo.toml -p openmat-sim-sundials --test events -- --ignored
```
