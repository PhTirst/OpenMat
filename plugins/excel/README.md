# OpenMat Excel plugin

独立的 Rust OEX 插件，使用 [calamine](https://docs.rs/calamine/0.36.1/calamine/)
读取电子表格，使用 [rust_xlsxwriter](https://docs.rs/rust_xlsxwriter/0.99.0/rust_xlsxwriter/)
生成 XLSX。无需安装 Excel、Python 或 COM 组件。

本目录拥有独立 Cargo 工作区和锁文件。插件只依赖官方 `oex` SDK 和上述文件处理库；
不链接 OpenMat 的 Rust 内核 crate，不改动根工作区、内置函数或 C ABI。
`tests/host` 是独立测试程序，只有它依赖内核 crate。

## 构建和加载

当前目标：Windows x64，Rust 1.90+，OEX ABI 1.2。
从仓库根目录执行（`BridgePath` 指向 OpenMat 发行包提供的兼容桥接 DLL）：

```powershell
./plugins/excel/build.ps1 -BridgePath 'C:/OpenMat/openmat_oex.dll'
```

开发环境已有 `target/debug/openmat_oex.dll` 时可省略 `BridgePath`。
脚本默认构建 release 插件，生成 `plugins/excel/dist/`；不构建或修改宿主软件。
这个目录包含插件、桥接 DLL、本文档、示例和插件第三方依赖许可文件。部署时保持两个 DLL 在一起。

通过软件现有的加载参数使用：

```powershell
openmat-cli run model.m --oex-plugin C:/work/OpenMat/plugins/excel/dist/openmat_excel.dll
# 或启动提供给客户端连接的服务端：
openmat-server --stdio --oex-plugin C:/work/OpenMat/plugins/excel/dist/openmat_excel.dll
```

函数使用 `excel_` 前缀，不替换 `readmatrix`、`writematrix` 等内置函数。

## m 语言接口

所有文本参数使用单引号 **char 向量**，支持中文及有效 UTF-16 字符。
当前 OEX 尚无 string/cell/table 的完整访问接口，所以不接受双引号 string 值。
相对文件路径按内核进程工作目录解析。

```matlab
excel_write('data.xlsx', [1, 2; 3, 4], '实验', 'B2');
[A, kinds, origin] = excel_read('data.xlsx', '实验', 'B2:C3');
n = excel_sheet_count('data.xlsx');
name = excel_sheet_name('data.xlsx', 1);
```

### 读取

```matlab
A = excel_read(filename);
[A, kinds, origin] = excel_read(filename, sheet, range);
```

- `sheet`：从 1 开始的整数序号（double 标量），或工作表名称；省略或 `''` 选择第一张表。
- `range`：`'B2:D8'`、`'C4'` 等有界 A1 区域，支持 `$B$2` 和小写列名。
  省略或 `''` 返回解析器识别的已用矩形区域。显式区域保持完整形状，区域外无数据的位置为空白。
  不接受整列 `A:C`、命名区域或包含工作表名的 `Sheet!A1`。
- `A`：列主序 double 矩阵。数字直接读取；逻辑值转为 0/1；其余类别为 NaN。
  数字文本如 `'123'` 不自动转成数字。日期不隐式转换为 MATLAB 日期序号。
- `kinds`：可选的同形 uint8 矩阵，区分下面的类别。
- `origin`：可选的 `[起始行, 起始列]`，从 1 开始。读取空表且未指定区域时，返回 0×0 矩阵和 `[1, 1]`。

| kinds | 单元格类别 | A 中的值 |
| --- | --- | --- |
| 0 | 空白或区域外没有数据 | NaN |
| 1 | 数字 | double |
| 2 | 逻辑 | 0/1 |
| 3 | 文本 | NaN |
| 4 | 日期、时间、时长 | NaN |
| 5 | Excel 错误值 | NaN |

公式读取文件中保存的**缓存结果**，不重新计算，也不保证缓存最新。
未合并展开单元格：合并区域通常仅左上角有值；隐藏行列不被过滤。
读取支持 calamine 的 XLSX/XLSM/XLSB/XLS/ODS 路径；自动测试实测 XLSX 和 ODS。
加密工作簿、宏执行、样式提取和混合 cell/table 输出不在本版范围内。

### 写入

```matlab
excel_write(filename, A);
excel_write(filename, A, sheetName, startCell, overwrite);
```

- 只生成 `.xlsx`，一次调用生成一张工作表；默认名称 `'Sheet1'`、起始单元格 `'A1'`。
- `A` 是非空的二维实数或 logical 稠密矩阵，支持 double、single 和整数类型。
  logical 写为 Excel 布尔值。复数、稀疏数组、文本矩阵不接受。
- NaN 写为空白；Inf/-Inf 报错。空白边缘不会记录原矩阵形状；需保留形状时用显式区域读取。
  为防止整数到 double 的静默舍入，int64/uint64 超出 ±2^53 / 2^53 时拒绝写入。
  Excel 数值本身也有精度限制，标识符等精确长整数不适合存为数字。
- 默认 `overwrite=false`，已有路径报 `OpenMat:Excel:Exists`。
  显式 `true` 或 `1` 会**替换整个工作簿**，不是修改原文件中的某张表或某个区域。
- 在目标目录的临时文件中完成序列化，成功后才发布到目标路径。
  参数错误、取消或序列化失败时，保留原文件；目标父目录须已存在。

```matlab
% 明确允许替换整个文件：
excel_write('data.xlsx', [5, 6; 7, 8], '实验', 'A1', true);
```

不支持追加工作表、原位编辑现有文件、写公式或保留原文件格式。
读写每次最多输出/写入 10,000,000 个单元格；A1 坐标限制为 XFD1048576。
读取时底层库仍可能解析整个工作表，此输出限制不是文件解析内存上限。
插件在处理数据及提交文件前检查取消；底层解析和 ZIP 序列化期间不能立即中断。

### 错误

错误通过 Rust `Result` 转为可被 m 语言 `try/catch` 捕获的 OEX 错误。
插件错误前缀为 `OpenMat:Excel:`：`Argument`、`Sheet`、`Range`、`Size`、
`Format`、`Exists`、`NonFinite`、`Precision`、`IO`、`Write`、`Allocation`。
SDK 的类型/参数数量错误和取消使用既有 OEX 错误约定。

## 验证

```powershell
./plugins/excel/verify.ps1
# 编译缓存也可以放在其他磁盘：
./plugins/excel/verify.ps1 -BuildRoot 'C:/temp/openmat-excel-build'
```

运行格式检查、严格 Clippy、Rust 测试，再构建 release 插件并用真实内核加载 DLL 执行
`tests/integration.m`。测试在临时目录生成本项目自行编写的数据，覆盖中文路径/表名、
列主序、区域和原点、类型标记、缓存公式、错误传播、覆盖保护及整数/逻辑数据。
测试程序另外用 calamine 检查 m 语言生成的工作簿。
开发验证只编译既有宿主源码，不修改宿主；不依赖 CLI 当前是否可构建。

依赖锁定在本目录的 `Cargo.lock` 中。calamine 为 MIT 许可，rust_xlsxwriter 为
MIT OR Apache-2.0；构建脚本将 Windows 插件依赖的许可文件汇集到 `dist/third-party/`。
