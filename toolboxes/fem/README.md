# OpenMat FEM：纯 m 语言计算基础包

第一版使用普通 `classdef` 值类、数值数组、函数句柄和 `sparse`，不包含
native 插件、网格文件读取、网格生成、绘图或内置物理问题求解器。

## 安装与运行

把本目录加入路径，**不要把 `+fem` 本身加入路径**：

```matlab
addpath('C:/work/OpenMat/toolboxes/fem');
```

然后执行本目录的 `demo_poisson.m` 或 `run_tests.m`。也可以在 OpenMat
编辑器中打开脚本；本轮没有验证图形界面执行入口。当前默认 Windows debug
stdio 服务器会在深层调用中栈溢出，建议先按后面的独立构建命令验证运行。
示例把 5 个完整自由度缩减为 1 个独立未知量，
求解后恢复完整解；它验证非零 Dirichlet 条件下的仿射解。

以下路径中的 `C:/work/OpenMat` 仅为本仓库示例位置，可替换成实际路径。

## 对象职责

| 类 | 数据与职责 |
| --- | --- |
| `fem.ReferenceCell` | 二维参考单元、局部顶点和有向边的编号约定 |
| `fem.Mesh` | 几何顶点坐标、混合单元连接、外部 ID、边拓扑与集合 |
| `fem.FiniteElement` | 参考基函数、局部 DOF 的归属和含义、变换回调 |
| `fem.DofMap` | 完整局部—全局编号及共享 CSC 变换表，不处理约束 |
| `fem.FunctionSpace` | Mesh 快照、有限元定义表、每单元定义编号与 DofMap |
| `fem.QuadratureRule` | 参考域积分点与权重 |
| `fem.GeometryMap` | 几何映射、Jacobian、有符号行列式、正的积分尺度 |
| `fem.AssemblyPlan` | 配对单元的测试／试探映射，复用 COO 装配坐标 |
| `fem.ConstraintMap` | 仿射关系 `u = C*q + b`，系统缩减和解恢复 |

所有基础对象均为值类，属性公开可读、外部不可直接赋值，保持派生数据一致。
读取到的数组仍是普通数组；修改数组不会隐式修改原对象。修改拓扑或有限元
分配时显式构造新对象和装配计划。`withSet` 返回新 Mesh，必须接收返回值。
数值数组遵循宿主的 COW 语义，不承诺任意索引提取是零拷贝。

## Mesh、编号与集合

```matlab
coordinates = [0, 1, 0, 1; 0, 0, 1, 1]; % 每列一个几何顶点
types = {'triangle'; 'triangle'};
pointers = [1; 4; 7];
vertices = [1; 2; 3; 2; 4; 3];
mesh = fem.Mesh(coordinates, types, pointers, vertices);
```

- m 接口的内部索引全部从 1 开始，包括指针数组。
- `CellPointers` 长度为 `NumCells+1`，末项是 `numel(CellVertices)+1`。
- 一个单元的连接为 `CellVertices(ptr(e):ptr(e+1)-1)`。
- 三角形参考顶点为 `(0,0),(1,0),(0,1)`，有向边为 `1→2,2→3,3→1`。
- 四边形参考顶点为 `(0,0),(1,0),(1,1),(0,1)`，有向边沿此环序。
- `CellTypes` 可用 cell 字符数组或 string 数组；不能只由顶点数猜测类型。
- 必须保留单元局部顶点顺序。只对用于识别公共边的端点对排序。
- 公共边方向从较小的**内部顶点索引**指向较大的索引，与文件 ID 无关。
- `EdgeCellPointers/EdgeCells/EdgeLocalNumbers` 保存边的单元邻接。允许超过
  两个邻接单元的拓扑；`boundaryEdges()` 精确定义为仅有一个邻接单元的边。

外部 ID 可以为零、不连续、无序，并在顶点和单元两个类别中分别唯一：

```matlab
options = struct();
options.VertexIds = uint64([0; 10; 42; 100]);
options.CellIds = uint64([900; 3]);
options.Connectivity = 'ids';
mesh = fem.Mesh(coordinates, types, pointers, ...
    uint64([0; 10; 42; 10; 100; 42]), options);

mesh.vertexIndex(uint64(42))  % 3
mesh.vertexId(3)              % uint64(42)
mesh.cellIndex(uint64(3))     % 2
```

默认 `Connectivity='indices'`；不存在自动判断 0/1 起点的逻辑。外部 ID 保存为
精确 `uint64`，大于 `2^53` 的 ID 必须由调用者以整数数组提供，不能先经过
double。查询结果统一为列向量。批量 ID 转换通过精确整数分组完成，不按最大
ID 分配数组，也不在积分循环中查 ID。

```matlab
mesh = mesh.withSet('wall', 1, mesh.boundaryEdges());
mesh = mesh.withSet('corners', 0, [1; 3]);
mesh = mesh.withSet('material_A', 2, [1; 2]);
edges = mesh.edgeSet('wall');
[cells, localEdges] = mesh.edgeCells(edges(1));
```

集合以 `(Name, Dimension)` 定位，维数 0/1/2 分别指顶点／边／单元；允许重叠，
同名同维集合会被 `withSet` 显式替换。集合成员使用内部索引，文件集合的 ID
应先转换。集合不携带边界条件或材料的物理含义。

## FiniteElement 与每单元分配

内置 `fem.lagrange('triangle',1/2)` 和
`fem.lagrange('quadrilateral',1/2)`。局部 DOF 顺序为：顶点、边中点（若有）、
Q2 单元中心。`lagrange` 是生成共享定义的工厂函数，而不是每个阶数一个类。

```matlab
elements = {fem.lagrange('triangle',1), fem.lagrange('triangle',2)};
V = fem.FunctionSpace(mesh, elements, [1; 2], 'discontinuous');
fe = V.element(2);
```

同形状的不同单元可以选择不同定义，同一 Mesh 可以建立多个 FunctionSpace。
默认共享方式为 `'conforming'`。上述 P1/P2 邻接组合在 conforming 模式下会被拒绝，
因为边迹不兼容；库不会默默把它当作连续空间。使用显式 DofMap 加 ConstraintMap
可以自行表达这种情况，或明确采用 discontinuous 模式。

自定义有限元通过 `fem.FiniteElement(def)` 提供以下全部字段：

| 字段 | 约定 |
| --- | --- |
| `ReferenceCell` | 参考单元对象或名称 |
| `NumDofs` | 正整数局部系数个数 |
| `ValueShape` | 正整数分量形状，例如标量 `1`、二维向量 `2` |
| `EntityDofs` | `EntityDofs{dim+1}{entity}`：该实体拥有的局部 DOF 索引 |
| `EntityKeys` | 同一结构，元素为有序规范 DOF 泛函列表的身份字符串 |
| `MapType` | `'identity'`、`'covariant'` 或 `'contravariant'` |
| `TabulateFcn` | `callback(points,order)` 返回基函数数组结构体 |
| `TransformFcn` | `callback(edgeDirections)` 返回局部方向变换 `T` |

`EntityDofs` 必须让每个局部 DOF 恰好归属于一个实体。与边闭包相关的 DOF
由 `entityClosureDofs(1,edge)` 查询，包含端点 DOF，不能和边自身拥有的 DOF 混淆。

`EntityKeys` 不是任意材料标签：**相同 key 是作者对泛函含义、归一化和规范顺序
完全相同的声明**。自动编号信任这个声明，并验证数量一致；不会只按数量共享，
也不会推导任意自定义基函数之间的兼容性。不同形状的标准 P2/Q2 可以共享
匹配的边中点 DOF。单元内部 DOF 从不跨单元共享。

回调应为确定性的计算函数，不修改捕获的 handle 状态。
同一有限元定义和方向组合的 T 只计算一次；数值相同的 T 在表中显式复用。
需要在匿名回调中调用包函数时，先捕获其命名句柄：

```matlab
tabulator = @mybasis.tabulate;
def.TabulateFcn = @(points,order) tabulator(parameters,points,order);
```

这也规避当前 OpenMat 匿名函数中直接限定包名调用的解析限制。

## 数组形状与几何

| 数组 | 维度 |
| --- | --- |
| `points`、`rule.Points` | `2 × Nq`，参考坐标 |
| `rule.Weights` | `Nq × 1`，不包含物理积分尺度 |
| `basis.Values` | `Ndof × Ncomponent × Nq` |
| `basis.Gradients` | `Ndof × Ncomponent × 2 × Nq`，参考导数 |
| `geometry.Points` | `2 × Nq`，物理坐标 |
| `geometry.Jacobians` | `2 × 2 × Nq`，`dx/dxi` |
| `geometry.DetJ`、`geometry.Measure` | `Nq × 1`；后者为 `abs(DetJ)` |

标量值仍保留概念上的分量维度 1；MATLAB 数组显示可能省略尾部单例维。
`tabulate(points)` 默认只计算值，`tabulate(points,1)` 同时计算一阶参考导数。

`fem.quadrature(ref,degree)` 提供 degree 0..8 的固定高斯规则。
三角形保证参考多项式总次数，四边形保证各坐标次数；不是自适应积分或物理
积分误差估计。曲边、变系数等非多项式被积函数由用户选择并检查积分阶数。

`fem.GeometryMap(mesh)` 使用顶点定义的仿射三角形或双线性四边形，拒绝退化和
翻折单元；整体顺时针编号可用，符号保留在 DetJ 中。默认四边形在全部参考
角点检查 Jacobian 符号。几何节点与场 DOF 独立，可提供纯 m 的
`GeometryMap(mesh,callback)`，让回调利用单独的高阶几何节点返回 Points 和
Jacobians。自定义映射仅检查被查询点，作者还需保证整个单元内的合法性。

`fem.mapBasis` 映射 identity 型基函数及其梯度，也支持二维向量的 Piola 值
映射。它不自动推导 Piola 场的一般物理导数；带导数的请求明确报错。

## DofMap、装配与约束

```matlab
g = V.Dofs.cellDofs(e);
T = V.Dofs.cellTransform(e);
uLocal = T * u(g);
```

T 统一为普通 CSC；它可以是单位矩阵、符号、置换或一般小矩阵。
手动映射构造形式：

```matlab
dofs = fem.DofMap(pointers, globalDofs, numDofs, transforms, transformIds);
V = fem.FunctionSpace(mesh, elements, cellElementIds, dofs);
```

`AssemblyPlan(testDofs,trialDofs,form)` 接受不同的测试和试探空间，产生矩形或
方形矩阵。调用者保证两边的单元列表按同一物理单元配对。

```matlab
plan = fem.AssemblyPlan(V.Dofs, V.Dofs); % 默认 bilinear
K = plan.assemble(localMatrices);      % cell 数组，每单元一块
K = plan.assemble(@elementMatrix);     % 或 callback(e)，逐单元计算
F = plan.assembleVector(localVectors); % 列向量，或 callback(e)
```

输入必须是**未应用 T 的局部基函数顺序**。装配器只应用一次变换：
`Ttest.'*Ke*Ttrial`；显式选择 `'sesquilinear'` 时改用 `Ttest'`。
最终 `sparse(I,J,values,...)` 合并重复贡献、删除抵消后的零。
复用的是 COO 行列索引，不是最终 CSC 的值槽位。回调模式不存全部 Ke，
但仍需要存储全部 COO 贡献；大型网格的分块装配尚未实现。

`FunctionSpace.edgeDofs(edges)` 返回边闭包所涉及的规范全局 DOF，包括变换
耦合到的列。它不计算边界函数值：点值、法向矩和切向矩的赋值办法并不相同。

```matlab
constraints = fem.ConstraintMap.fixed(V.Dofs.NumDofs, fixedDofs, prescribedValues);
[Kr, Fr] = constraints.reduce(K, F);
u = constraints.expand(Kr \ Fr);
```

DofMap 和完整 K/F 保留被指定的自由度。约束计算
`Kr=C.'*K*C`、`Fr=C.'*(F-K*b)`，不会漏掉非零指定值对右端项的影响。
复数半双线性形式明确用 `reduce(K,F,'sesquilinear')`。
`ConstraintMap` 自身不调用求解器。无约束、全部固定和空空间均有测试。

一般多点关系由 `ConstraintMap(C,b,independentDofs)` 提供。
必须满足 `C(independentDofs,:)=I`、`b(independentDofs)=0`，因此 q 就是这些
独立系数的值，也能在不把全局 C 转稠密的情况下保证列满秩。重复固定同一
DOF 会报错。首版要求用户提供已展开的关系，不自动处理约束依赖链或循环。
边界力、通量等积分仍由用户单元／边界计算代码贡献到 K/F，不属于此约束类。

## 验证

测试全部为本项目编写，不依赖第三方有限元代码或 MATLAB 私有实现。

```powershell
# 独立构建目录避免覆盖正在运行的开发服务器。
# m 调用帧由运行时在堆上调度，不需要加大 Windows 主线程栈。
cargo build -p openmat-server --target-dir toolboxes/fem/.build
pwsh -NoProfile -File toolboxes/fem/Run-OpenMat.ps1
pwsh -NoProfile -File toolboxes/fem/Run-OpenMat.ps1 -Entry demo_poisson.m

# 本机 MATLAB R2022b，运行相同的 m 测试和示例。
& $env:MATLAB_EXE -batch "cd('C:/work/OpenMat/toolboxes/fem'); run_tests; demo_poisson; check_code"
```

`Run-OpenMat.ps1` 只为验证启动独立 stdio 服务器，使用现有 v0 协议，不修改协议
或连接用户正在使用的会话。可通过 `-ServerPath` 指定别的构建；超时会终止
它自己启动的测试进程。m 包本身没有 Windows 专用逻辑。

19 组回归覆盖：大整数 ID、无效输入、拓扑与集合、P/Q 基函数节点插值和
有限差分导数、共享编号、混合形状／不同阶数、积分精度、线性／曲线几何、
退化／翻折检查、矩形装配、复数形式、符号／置换／一般小矩阵 T、零值抵消
后的重复装配、固定和一般约束、空空间、完整仿射补丁算例。

## 首版边界

- 仅二维平面三角形／四边形；尚未实现三维面拓扑、曲面网格或其他参考单元。
- 内置标量 P1/P2/Q1/Q2。自定义定义可接入，但暂未内置 RT/Nedelec 等完整族。
- 不自动生成 hp、周期或悬挂节点约束；提供显式 DofMap 和仿射 ConstraintMap。
- 不含边／面数值积分便利接口、DG 内界面双侧装配或几何逆映射搜索。
- 不含文件 IO、前后处理、材料模型、PDE DSL 或求解器包装。
