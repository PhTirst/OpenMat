# OEX 容器 API（ABI 1.3）

`include/openmat/oex.h` 新增 42 个函数，桥接层共提供 73 个函数。
`include/openmat/oex.hpp` 和 `sdk/rust/oex` 同步覆盖这些接口。
插件使用 ABI 1.3 头文件重新编译，并与对应的 `openmat_oex.dll` 一起使用。
OEX ABI 版本与内核通信协议版本分别管理。

## 接口范围

下表中的函数名省略该行对应的前缀。

| 前缀 | 函数 | 行为 |
| --- | --- | --- |
| `OexValue_` | `GetDimensions`, `GetClassName` | 通用尺寸查询；运行时 class 名，包括已注册的原生对象类 |
| `OexString_` | `Create`, `CreateFromUtf8`, `CreateFromUtf16` | 创建指定形状的 String 数组，默认元素为非 missing 的空文本 |
| `OexString_` | `GetElementView`, `GetElements`, `CopyElementUtf8` | 单元素或批量借用 UTF-16；复制转换后的 UTF-8 |
| `OexString_` | `SetElementUtf8`, `SetElementUtf16`, `SetMissing`, `SetElementsUtf8`, `SetElementsUtf16` | 单元素及批量写入，批量失败不提交部分结果 |
| `OexCell_` | `Create`, `GetElement`, `SetElement`, `GetElements`, `SetElements` | 任意 Value 的单元素及批量读写 |
| `OexStruct_` | `Create`, `GetFieldCount`, `GetFieldName`, `FindField` | 指定形状与有序字段集合，按名字查询字段索引 |
| `OexStruct_` | `GetField`, `SetField`, `GetFieldValues`, `SetFieldValues` | 单记录字段读写；整个字段通过同形状的 Cell 交换 |
| `OexStruct_` | `AddField`, `RemoveField`, `SetFieldNames` | 全数组增删字段、原子替换有序字段名；允许一次交换两个名字 |
| `OexTable_` | `Create`, `GetRowCount`, `GetVariableCount`, `GetVariableName`, `FindVariable` | 显式指定行数，查询变量及其名字 |
| `OexTable_` | `GetVariable`, `SetVariable`, `AppendVariable`, `RemoveVariable`, `SetVariableNames` | 以完整 Value 读写变量和修改 schema |
| `OexTable_` | `GetRowNames`, `SetRowNames`, `ClearRowNames` | 区分未设置行名与已设置的空行名列表 |

## 编码

String 数据在运行时保持 UTF-16。插件可以使用两条路径：

- UTF-8 接口接收或复制 `char` 字节，创建/写入时转换到 UTF-16，读取时转换回 UTF-8。
- UTF-16 接口使用 `uint16_t` 码元，可以无损保存嵌入 NUL、代理对和孤立代理项。读取 view 不转码。

长度始终显式给出：UTF-8 计字节，UTF-16 计码元，不计结尾终止符。
接口不使用 `strlen`，不自动追加 NUL。非法 UTF-8 或无法转换成 UTF-8 的孤立
UTF-16 代理项返回 `OEX_ERROR_ENCODING`，不静默替换为 U+FFFD。

`missing` 独立于文本内容；空文本和 missing 都可能具有零长度。
UTF-8 批量输入使用并行的 `uint8_t` missing 数组，NULL 表示全部非 missing；
UTF-16 输入使用 `OexStringElementView.isMissing`。missing 的输入长度必须是零。
`CopyElementUtf8` 通过单独的输出参数报告 missing。

字段名、变量名、类名等元数据使用 UTF-8，继续遵守各类型现有的运行时约束。
这不意味着内部所有名称也要转换成 UTF-16。目前 Struct 字段名采用 ASCII
标识符子集；Table 的变量名、行名允许 Unicode 和空格，但不能重复、为空或包含 NUL。
Char 数组继续通过已有的 `OEX_DATA_CHAR16` dense 接口访问。

## 所有权、形状与失败语义

- C 索引从零开始，数组按列主序排列。形状与 dense builder 一致：至少二维，
  第二维以后不保留末尾的单例维；允许零长度维度。
- Create 和子值 Get 返回 owned handle，调用方负责 `OexValue_Release`。
  Set 借用源值，要求目标是 owned handle，不消耗输入。
- 获取或存入子值使用运行时的语言复制语义。普通数组、String、Cell、Struct、
  Table 保持值语义和写时复制；对象按自身的 value/handle 语义处理。
  仅创建 standalone `BuiltinContext` 时，没有解释器的对象复制服务，相关操作会报错。
- 修改取出的子值不会自动修改父容器，需要 Set 写回。这个规则不改变 handle
  对象本身共享实例的语义。
- 变更先写入临时值，成功且未取消时才替换目标。维度、类型、编码、名称、
  范围检查失败或取消都不会提交部分写入。返回错误时 owned 输出置 NULL。
- UTF-16、字段名、变量名 view 借用源存储，不延长其寿命；源值变更、释放或
  重新进入运行时前应停止使用。类名文本由调用上下文持有，最迟在回调结束失效。
- `GetDimensions` 和 `CopyElementUtf8` 接受 NULL + 0 容量查询所需长度。
  缓冲区不足返回 `OEX_ERROR_RANGE`，提供所需长度并保持数据缓冲区不变。
  找不到字段或变量返回 `OEX_ERROR_NOT_FOUND`。

Table 的一个变量可以是多列数组，例如一个 `N×3` 数组仍是一个变量。
创建、追加、替换要求变量第一维等于 Table 行数，其他维度保留。
允许创建 `N×0` Table；删除最后一个变量也保留行数。追加不会隐式改变行数，
包括向 `0×0` Table 追加的情况。`GetRowNames` 返回 owned String 列数组；
未设置行名时返回 `0×1` String，并设置 `hasNames=0`。

## C++20 示例

C++ 直接返回值，错误抛出 `oex::Error`。missing/查找缺失使用 `std::optional`。
以下函数在插件的 C++ 回调内部调用；最外层 C 导出回调负责捕获异常。

```cpp
#include <openmat/oex.hpp>

oex::Value make_table(oex::Call& call) {
    const std::array<std::uint64_t, 2> shape{2, 1};
    const std::array<std::optional<std::string_view>, 2> text{
        std::string_view("hello"), std::nullopt};
    auto names = call.make_strings_utf8(shape, text);
    const std::array<std::string_view, 1> variables{"Name"};
    const std::array<oex::ValueView, 1> columns{names.view()};
    auto table = call.make_table(2, variables, columns);

    auto column = table.table_variable(call, 0);
    column.set_string_utf8(call, 1, "updated");
    table.set_table_variable(call, 0, column.view());
    return table;
}
```

`Value` 自动释放 handle。String 的借用 view 使用 `std::span<const uint16_t>`，
拥有副本的 UTF-8 读取返回 `std::optional<std::string>`。
完整可执行示例见 `crates/openmat-oex/tests/fixtures/container_plugin.cpp`。

## Rust 示例

```rust
use oex::{Call, Result, Value};

fn make_table<'call>(call: &Call<'call>) -> Result<Value<'call>> {
    let names = call.strings_utf8(&[2, 1], &[Some("中文"), None])?;
    let mut table = call.table(2, &[("Name", names.as_value_ref())])?;

    let mut column = table.table_variable(call, 0)?;
    column.set_string_utf8(call, 1, "更新")?;
    table.set_table_variable(call, 0, column.as_value_ref())?;
    Ok(table)
}
```

Rust 使用 `Result`、`Option`、借用和 `Drop`。String 的 UTF-16 借用为 `&[u16]`；
`string_utf8` 返回 `Result<Option<String>>`。借用检查阻止在使用 String view 时
同时修改它；owned 子值受回调生命周期约束。完整验证插件位于
`sdk/rust/tests/plugin/src/lib.rs`。

## 验证

在仓库根目录运行：

```powershell
cargo fmt --all --check
cargo test -p openmat-value -p openmat-runtime -p openmat-oex --locked
cargo check --workspace --exclude openmat-cli --all-targets --locked
./sdk/rust/verify.ps1
```

C 与 C++ 测试用 GCC/G++ 在 Windows x64 构建真实 DLL，并通过 OEX loader 调用。
覆盖 UTF-8/UTF-16、NUL、missing、失败原子性、借用写入拒绝、取消、嵌套容器、
Table 多列变量与零变量表。Rust 验证包含严格 Clippy、rustdoc、C ABI 布局和
函数签名检查、借用约束 compile-fail 测试、M 语言加载 Rust 插件的端到端测试，
包括原生对象经 Cell 传递与实际类名查询。

完整的 `cargo check --workspace --all-targets --locked` 仍在 `openmat-cli`
报协议 V3 枚举 match 不完整的 E0004；排除该包后检查通过。未修改 CLI 代码。
本次扩展没有改变 Value 分支，没有新增 Table 索引语法或扩展 Struct 字段名规则。
