# Milestone 6 char 与 integer 值模型冻结建议

状态：设计冻结候选，供后续实现任务直接采用；本文件不修改或取代任何
accepted spec/RFC。

基线：`pre-publication baseline`。结论来自当前 OpenMat
代码与规范的只读审阅，以及本机 MATLAB R2022b 的临时黑盒探针。探针只执行
OpenMat 自行编写的表达式，未保留 MATLAB 源码、测试、文档或诊断消息。

## 1. 冻结结论

1. `char`、`string` 和 `uint16` 是三个不同的语言类，不能共用一个动态值标签。
   `char` 的基本元素是一个 UTF-16 code unit，不是 Unicode scalar value，也不是
   UTF-8 字节。
2. `char` 使用 `ArrayData::Char(DenseArray<CharCodeUnit>)`；`CharCodeUnit` 是封装
   `u16` 的新类型。不能直接令 `char` 与 `uint16` 都实现为 `DenseArray<u16>`，因为
   一个 Rust 元素类型不能同时拥有两个 `ArrayElement::DTYPE`，而且两类的动态
   class、拼接、索引结果和运算规则不同。
3. `string` 保留独立的 `Value::String(StringValue)`。每个 string element 使用
   UTF-16 code-unit 序列和独立 missing 位；不得继续以 `Arc<str>` 作为规范存储，
   因为 MATLAB R2022b 可保存孤立代理项，而 Rust `str` 不能无损表示它。
4. 八个整数类都使用定宽 Rust 整数分量。`ArrayData` 增加一个嵌套的
   `Integer(IntegerArrayData)` variant；`IntegerArrayData` 显式枚举八个实整数和
   八个复整数存储。不能先转成 `f64`，否则 `int64`/`uint64` 在 `2^53` 以上丢失
   可观察精度。
5. 不为整数增加优化 scalar variant。语言层 integer scalar 是相应 class 的
   `1x1 ArrayData::Integer`。保留现有 logical、double、complex double 和 string
   scalar 优化；以后只有在测量证明必要时才增加 integer scalar 优化。这避免在
   `Value`、运行时和所有内建函数中复制十六套 scalar/array 分支。
6. Rust enum 布局、wrapper 布局和泛型实例都不是稳定 ABI。跨进程值预览必须使用
   新的版本化协议，不能把 Rust 内存布局、native endian 字节或 serde enum 的偶然
   形状当作 ABI。
7. `openmat-kernel-v0` 已冻结，不能向它的 `PreviewValue` enum 直接加入新 variant。
   这会使现有 v0 客户端在反序列化未知 preview kind 时失败。无损 char/integer/
   UTF-16 string preview 应进入 `openmat-kernel-v1`，并保留 v0 协商与降级路径。

## 2. MATLAB R2022b 可观察模型

### 2.1 char array

`char` 是动态 class，不是 string 的显示形式。

| 表达式/构造 | class | shape | 关键结果 |
| --- | --- | --- | --- |
| `'abc'` | `char` | `1x3` | code units 为 `97,98,99` |
| `''` | `char` | `0x0` | `numel == 0`，不是一个空 string scalar |
| `char(zeros(0,3))` | `char` | `0x3` | 空 shape 必须保留 |
| `'don''t'` | `char` | `1x5` | 单引号 literal 用成对单引号转义 |
| `char([0xD83D,0xDE42])` | `char` | `1x2` | 一个非 BMP 字符占两个元素 |
| `char(0xD83D)` | `char` | `1x1` | 孤立 high surrogate 可被保存 |

shape 遵守统一的 MATLAB 数组规则：至少二维、列主序、尾随 singleton 规范化、
空维度不得塌缩。单引号 literal 自身产生一行 char vector，空 literal 是特殊的
`0x0`；后续 `reshape`、拼接和构造函数可以产生其他 shape。

char 的 equality、索引、`numel` 和 conformance payload 均按 code unit，而不是按
用户感知字符或 Unicode code point。索引可拆开代理对：上述代理对的 `(1)` 结果是
`1x1 char` high surrogate，`(:)` 是 `2x1 char`。

### 2.2 string scalar 与 string array

双引号 literal 产生 string，而不是 char：

| 表达式/构造 | class | shape | missing | element payload |
| --- | --- | --- | --- | --- |
| `"abc"` | `string` | `1x1` | false | 3 个 UTF-16 code units |
| `""` | `string` | `1x1` | false | 0 个 code units |
| `strings(0,0)` | `string` | `0x0` | 无元素 | 无 payload |
| `strings(0,2)` | `string` | `0x2` | 无元素 | 无 payload |
| `["a",missing,"bc"]` | `string` | `1x3` | `false,true,false` | 每元素独立序列 |

空 string scalar、missing string scalar 和空 string array 是三个不同状态：

- `""` 有一个非 missing 元素，payload 长度为零；
- `string(missing)` 有一个 missing 元素；其 code-unit payload 规范化为空，但 missing
  位为 true；
- `strings(0,0)` 没有元素。

R2022b 的 string element 也必须按 UTF-16 边界保存。黑盒探针中，由代理对构造的
一个 `1x1 string` 的 `strlength` 为 2，转回 char 得到两个 code units；由孤立 high
surrogate 构造的 string 仍为非 missing `1x1 string`，`strlength` 为 1。因此内部
`Arc<str>` 和协议 JSON text 都不能作为唯一真值。

建议冻结如下内部结构；名字可机械调整，但语义不可改变：

```rust
pub struct StringElement {
    code_units: Arc<[u16]>,
    missing: bool,
}

pub enum StringValue {
    Scalar(StringElement),
    Array(StringArray), // DenseArray<StringElement>
}
```

`StringElement` 构造器必须维持 `missing => code_units.is_empty()` 的规范形式；不能用
空 code-unit 序列推断 missing。显示层可以把非法 UTF-16 以 replacement character
有损显示，但 equality、`strcmp`、索引、协议和 conformance 不得经过该有损文本。

### 2.3 char 与 string 的拼接、索引和运算差异

| 操作 | char | string |
| --- | --- | --- |
| 水平 bracket 拼接 | `['ab','cd']` 是 `1x4 char`，拼 code units | `["a","b"]` 是 `1x2 string`，拼 elements |
| 纵向 bracket 拼接 | char 行列必须满足数组拼接 shape | string 按 element shape 拼接 |
| 混合 bracket 拼接 | `['a',"b"]` 和 `["a",'b']` 均产生 `1x2 string` | char operand 转成 string element 后拼接 |
| `+` | `'a'+'b'` 进入数值语义，结果为 `double(195)` | `"a"+"b"` 拼 element 内容，结果是一个 `1x1 string` |
| 圆括号索引 | 返回 char code-unit 子数组，可拆代理对 | 返回 string element 子数组，class 仍为 string |
| 花括号索引 | 不属于 char array 索引 | `s{1}` 提取该 element 的 char row vector |
| condition | real char array 使用数值 all-nonzero 规则 | string condition 报 type error |

编译器必须保留引号 delimiter。当前 `lower_string` 虽收到 delimiter，却把单、双引号
都降为 `Constant::String(String)`，这是 Milestone 6 必须关闭的语义缺口。

### 2.4 strcmp 边界

`strcmp` 不得继续把 char 与 string 合并成一个 `StringValue` 契约：

- char 对 char 比较完整 char array 的 code units；标量文本比较返回 scalar logical；
  `strcmp('','')` 为 true；
- string array 逐 element 比较并保留/broadcast element shape；
- char 标量文本可与 string array 比较，按 string scalar 扩展；
- missing 与任何 element（包括 missing）比较为 false；
- 比较必须按 UTF-16 code units 精确进行，不能先转为有损 UTF-8。

黑盒探针还确认 `strcmp(['ab';'ac'],'ab')` 返回 scalar false；不能凭现有限制文档中的
“character-matrix row rules”自行把 char matrix 拆成逐行字符串。cellstr/cell array
overload 属于另一个明确阶段。

## 3. integer 值模型

### 3.1 class、范围、shape 与复整数

| class | Rust 实分量 | 最小值 | 最大值 |
| --- | --- | ---: | ---: |
| `int8` | `i8` | -128 | 127 |
| `uint8` | `u8` | 0 | 255 |
| `int16` | `i16` | -32768 | 32767 |
| `uint16` | `u16` | 0 | 65535 |
| `int32` | `i32` | -2147483648 | 2147483647 |
| `uint32` | `u32` | 0 | 4294967295 |
| `int64` | `i64` | -9223372036854775808 | 9223372036854775807 |
| `uint64` | `u64` | 0 | 18446744073709551615 |

所有 scalar 都报告 `1x1`；所有数组（包括 `0xN`）原样保留 canonical shape、列主序
和 COW 语义。整数不能存成 double：例如 `uint64` 最大值转成 double 已变成
`1.8446744073709552e+19`，不能再还原原值。

MATLAB R2022b 允许复整数。`int8(complex([1.4,-1.5,200],[99,-99,1]))` 的 class
仍是 `int8`，可观察元素为 `1+99i`、`-2-99i`、`127+1i`。实部和虚部分别执行同一
整数转换规则。若转换后所有虚部分量为零，R2022b 的 integer 结果为 real；只要有
非零虚部分量就为 complex。实现不能把所有 integer 假定为 real，也不能把复整数
提升为 complex double。

建议结构：

```rust
pub enum DType {
    F64,
    ComplexF64,
    Logical,
    Char,
    I8, ComplexI8, U8, ComplexU8,
    I16, ComplexI16, U16, ComplexU16,
    I32, ComplexI32, U32, ComplexU32,
    I64, ComplexI64, U64, ComplexU64,
}

pub struct CharCodeUnit(u16);
pub struct ComplexInteger<T> { re: T, im: T }

pub enum IntegerArrayData {
    I8(DenseArray<i8>),
    ComplexI8(DenseArray<ComplexInteger<i8>>),
    // 对其余七个 class 成对枚举。
}

pub enum ArrayData {
    F64(DenseArray<f64>),
    ComplexF64(DenseArray<Complex64>),
    Logical(DenseArray<Logical>),
    Char(DenseArray<CharCodeUnit>),
    Integer(IntegerArrayData),
}
```

嵌套 `IntegerArrayData` 将 `ArrayData` 的匹配面控制为一个 integer 分支，同时保留
编译期宽度和 signedness，避免 `Vec<u8>` 加运行时 reinterpretation。所有 primitive
integer 和八个 `ComplexInteger<T>` 实例实现 sealed `ArrayElement`；
`CharCodeUnit` 单独实现 `DType::Char`。这些类型均不使用 `repr(C)` 承诺 ABI。

`DType`/`IntegerArrayData` 应提供统一方法：`shape`、`numel`、`class_name`、
`is_complex`、`component_width_bytes`、同 variant COW sharing、逐元素访问和精确
十进制格式化。调用者不得用 16 个随手复制的 match 实现各自版本。

### 3.2 integer scalar 决策

不增加 `Value::Int8`、`Value::UInt8` 等 top-level variant，也不增加一个包含 16 个
分支的 `IntegerScalar`。integer scalar 一律是 `1x1 ArrayData::Integer`：

- `class`、shape、索引和 COW 路径只有一个真值；
- integer constructor 与数组索引不需要在 scalar/array 间反复装箱/拆箱；
- `Value::is_scalar()` 已按 `numel == 1` 工作；
- `NumericView` 需要支持 `1x1` array 的 scalar dispatch，而不是以 Rust enum variant
  判断语言 scalar。

现有 `Value::Logical`、`Double`、`Complex` 和 `StringValue::Scalar` 保留。显式的
`1x1 double/logical/string array` 仍可保留 array storage，语言元数据与优化 scalar
一致。

### 3.3 integer conversion 冻结规则

首个 integer conversion tranche 必须一次覆盖八个 constructor，使用一个按目标
class 参数化的 checked conversion 核心；禁止使用 Rust `as` 作为语言转换规则。

对 logical、char、real integer、real single/double 输入：

1. 有限非整数值舍入到最近整数；恰好半值时远离零。实测 `-2.5 -> -3`、
   `-1.5 -> -2`、`-0.5 -> -1`、`0.5 -> 1`、`1.5 -> 2`、`2.5 -> 3`。
2. 舍入后低于目标最小值或高于目标最大值时饱和；unsigned 的负数饱和为 0。
3. `NaN -> 0`；`+Inf` 饱和到最大值；signed 的 `-Inf` 饱和到最小值，unsigned 的
   `-Inf -> 0`。
4. integer 到不同 integer class 同样饱和，不 wrap。例如 `uint8(255) -> int8(127)`，
   `int8(-1) -> uint8(0)`。
5. char 输入按其无符号 16-bit code unit 数值参与转换；`uint16(char_value)` 必须逐
   code unit 精确保留。

对 complex numeric 输入，实部和虚部分别应用上述规则；任一转换后虚部非零则产生
complex integer storage，否则产生 real integer storage。`logical(complex integer)` 和
complex integer condition 在 R2022b 中报 type error；不能套用 OpenMat 当前对
complex double 的“任一分量非零”逻辑。

从 integer 到 double/single 必须执行 IEEE 目标精度转换，并承认大整数可能舍入；
从 integer 到 logical 对 real integer 使用 `value != 0`。char constructor 的完整
跨类表属于 char conversion tranche，但至少冻结：`char(uint16)` 精确保留全部
`0..65535` code units，负整数输入饱和到 code unit 0，不能经 Unicode scalar 检查
拒绝代理项。

### 3.4 arithmetic、overflow 与 mixed-class 分阶段边界

分阶段的原则是“明确报 unsupported/type error”，绝不悄悄提升成 double 或使用 Rust
wraparound。

#### Tranche I：值、转换和数组基础

必须实现：八类 real/complex storage、constructors、shape/COW、class/isa、精确
索引与 indexed assignment、logical/condition、char/string literal 与基本拼接、
inspect/conformance。除 equality/inequality 所需的精确比较外，integer arithmetic
可暂时返回结构化 `runtime.unsupportedArrayOperator`。该阶段不声称 arithmetic
兼容。

#### Tranche II：已冻结的 real integer operator core

按 operator 建立显式 dispatch 表和 R2022b differential cases：

- 同 integer class 的 `+`、`-`、`*`、`.*`、`/`、`./`、`^`、`.^` 返回同 class；
  中间结果在足够宽的精确域计算，最终按相同“半值远离零 + 饱和”规则落回目标。
  实测 `int8(127)+int8(1)==int8(127)`、`int8(100)*int8(2)==int8(127)`、
  `int8(5)/int8(2)==int8(3)`。
- unsigned unary minus 返回同 unsigned class 并饱和，例如 `-uint8(1)==uint8(0)`。
- real integer 与 real double **scalar** 的已支持 arithmetic 可双向组合，结果保持
  integer class；real double 非 scalar 与 integer arithmetic 报错。
- 不同 integer class 的 arithmetic 报错；single 或 logical 与 integer arithmetic
  报错。不得从“都是数值”推导共同提升类型。
- comparisons 与 arithmetic 分表：同类 integer、不同 integer class、integer 与
  shape-compatible double 均可产生 logical comparison；比较不得先统一转成 f64。

上述规则不能外推到未测 operator、matrix division、隐式扩展、char/integer 混合或
complex integer。每增加一格 dispatch 表，都要有本机 R2022b 黑盒 case。尚未进入表
的组合必须稳定报错。

#### Tranche III：完整 mixed/complex/char numeric 兼容

逐项加入 complex integer arithmetic、char 与 integer 的数值运算、reductions、
matrix operators、colon 的完整混合规则和 single。已知边界包括：

- `int8(1):int8(3)` 产生 `1x3 int8`；
- `'A'+uint8(1)` 产生 `uint8(66)`，但这不能用来推断所有 char/integer operator；
- non-real complex integer 不可用于 condition、logical conversion 或 index。

Tranche III 完成前，相关组合保持结构化 unsupported，不得由 `NumericView` 的通用
`Complex64` accessor 偶然“支持”。

### 3.5 logical、condition 与 index acceptance

冻结 acceptance：

- real integer scalar/array 可传给 `logical`；0 为 false，其余为 true；
- real integer condition 与现有 MATLAB 数组 condition 一样：非空且所有元素非零
  才为 true，空 integer array 为 false；
- real char condition 使用 code unit 数值的同一规则；string condition 报 type error；
- non-real complex integer 的 `logical` 和 condition 报 type error；
- index 接受 logical mask，以及值为有限、正整数的 real double、single、char 和八个
  integer class；0、负数、fractional、NaN、Inf 和 string 拒绝；
- complex double 即使虚部数值为 0，只要仍是 complex value，也拒绝作为 index；
  integer conversion 若已规范化为 real storage则可索引，non-real complex integer
  拒绝；
- integer index 的范围检查必须在其原始精确域进行。特别是 `uint64` 不能先转 f64，
  超界值必须确定地报告 index-out-of-bounds/size-limit，而不是因舍入指向别处。

索引结果保持 target class。integer/char 单元素索引分别返回 `1x1` 同 class array；
不能像当前 double/logical `gather` 一样把它们错误转成 `Value::Double` 或
`Value::Logical`。

## 4. 对当前代码面的逐项影响

### 4.1 `openmat-array`: DType、ArrayElement、ArrayData

- `DType` 增加 `Char`、八对 real/complex integer storage dtype，并提供 MATLAB
  class 映射；`ComplexI8` 与 `I8` 的 class 都是 `int8`。
- 新增 `CharCodeUnit` 和 `ComplexInteger<T>`；所有构造器保持表示有效。
- sealed `ArrayElement` 覆盖八个 primitive integer、八个 complex instantiation 和
  `CharCodeUnit`。
- `ArrayData` 增加 `Char` 与一个嵌套 `Integer` variant；`dtype/shape/numel`、COW、
  equality 和 byte estimate 必须穷尽新 variant。
- 不增加 endian/native byte serialization API；协议从 typed elements 逐值编码。

### 4.2 `openmat-value`: Value 与 ValueKind

- `Value::String` 保留，但 payload 改为 UTF-16 `StringElement`；增加 missing API、
  code-unit API 和仅用于显示的 lossy UTF-8 API。
- char/integer 都走 `Value::Array`。`ValueKind` 增加 `Char`、`Integer`，并在
  `Value::Array` 中按 dtype 返回；`class_name` 返回精确八类或 `char`。
- `dimensions/numel/is_scalar/shares_array_storage_with/value equality/condition` 穷尽
  新 storage。
- `as_real_number`/`as_complex_number` 不得吸收 integer 或 char；这两个现有 API
  返回 f64，会破坏大整数。为 dimension、index、comparison 分别增加 exact typed
  accessor。
- string equality 比较 missing 位和 UTF-16 code units；missing 不等同空 string。

### 4.3 compiler Constant 与 literal lowering

当前 `Constant::String(String)` 同时承担标识符/属性名和语言 string literal，必须拆开：

```rust
Utf8Symbol(String)       // 名称、属性、函数引用等元数据
CharLiteral(Vec<u16>)    // 单引号 literal
StringLiteral(Vec<u16>)  // 双引号、非 missing scalar string literal
```

具体 variant 名可调整，但三种语义不能再复用一个 variant。`lower_string` 使用传入的
delimiter 选择 `CharLiteral` 或 `StringLiteral`，并在 delimiter unescape 后调用
`encode_utf16`。OpenMat 源文件仍是 UTF-8，因此源 literal 本身只含 Unicode scalar；
孤立 surrogate 由运行时 `char`/numeric construction 产生。integer 没有专用 literal，
数字 token 仍是 double；八类 integer 由 built-in constructor 产生，因此本 tranche
不增加 integer Constant。

runtime constant materialization 分别创建 `1xN/0x0 char` 和优化 `1x1 string`。
`Utf8Symbol` 永远不能作为语言值装入普通寄存器，除非指令语义明确要求类名文本；
这些场合应显式构造 char 或 string，而不是泄漏内部 symbol 类型。

### 4.4 runtime NumericView、拼接、运算与索引

当前 `NumericView` 把 logical、double 和 complex double 都暴露为 `Complex64/f64`。
不得简单再加 integer 分支并复用 `complex_at`。建议分层：

- `ArrayView`：只负责 class、shape、numel 和列主序 element 定位；
- `NumericView`：带 `NumericClass::{Logical,Char,Double,Integer(IntegerClass)}` 与
  real/complex 标志，但不做隐式提升；
- `ExactIndexView`：按 signed/unsigned/floating/logical 分支校验 index；
- operator-specific promotion/dispatch：决定输出 class、精确中间域、舍入和饱和。

`build_matrix` 先按 MATLAB class 拼接规则选择输出 class，再逐值转换；不能用现有
`complex`/`logical` 两个 bool 决定所有 numeric class。`gather`、indexed assignment、
transpose、for iteration、colon 和 COW promotion 都必须保留 integer width、
signedness、complexness 及 char class。

### 4.5 builtins conversions、shape helpers 与 strcmp

- 注册 `int8`、`uint8`、`int16`、`uint16`、`int32`、`uint32`、`int64`、`uint64`、
  `char` 和 string construction 边界；八个 integer built-in 共享一个目标类参数化
  conversion core。
- `logical`、`double` 支持 real integer；complex integer 遵守上面的拒绝/转换规则。
- `class`/`isa` 使用动态 class，不看 Rust storage type 名。
- `positive_integer_dimension` 与 reshape/size helpers 若接受 integer，必须 exact 提取，
  不得先 `as_real_number -> f64`。char 是否作为 size 参数属于单独 oracle case，未测
  前不扩大 acceptance。
- `strcmp` 按 2.4 分派 char/string/missing；现有统一 `StringValue` 路径只能保留为
  string-array 子路径。cell/cellstr overload 留待后续并稳定报 unsupported。
- reductions 与 linalg 不得自动接收 integer；每个 built-in 的 native/double 输出
  class 选项必须按 tranche 明示。

### 4.6 kernel inspect、workspace summary 与 display

- `variable_summary` 为 char/integer 返回精确 class/shape；complex integer 设置
  `complex: true`。
- `value_bytes`：char 为 `numel * 2`；integer 为 `numel * component_width`，complex
  再乘 2；string 至少按 `2 * code_units + missing bitmap/offset metadata` 给出稳定的
  payload estimate。字段仍是 estimate，不声称 Rust allocator footprint。
- `values_changed/arrays_changed` 按 exact integer bits、char code units、string missing
  与 code units 比较。
- display 是非规范层，可以有损替换非法 UTF-16并截断；inspect 和 workspace delta
  不能复用 `bounded_text(&str)` 作为真值。
- `preview_value` 新增 char code-unit、integer exact component 和 UTF-16 string element
  编码；v0 下无法无损表示时返回 `workspace.unsupportedValue`，不能伪装成 Number、
  Text 或 Missing。

### 4.7 protocol PreviewValue

`openmat-kernel-v1` 建议冻结以下 JSON-safe preview kinds：

```text
charCodeUnit { value: 0..65535 }
integer { real: canonical decimal string, imaginary: canonical decimal string }
string { codeUnits: [0..65535], missing: boolean }
```

integer class 来自 `MatrixPreview.class`，每个 component 使用十进制字符串，因此
`uint64` 全范围安全；real integer 的 `imaginary` 固定为 `"0"`，避免 optional 字段
带来两种等价 wire shape。char 每个 preview value 对应一个数组元素；string 每个
preview value 对应一个 string element。这一区别使 selected range 与 values count
仍保持现有一致性。

v1 保留 Number、Complex、Logical、Special、Missing；旧 Text 仅可作为兼容读取项，
新 kernel 不用它传 string 真值。所有新序列继续受 `MAX_PREVIEW_ELEMENTS` 限制；
string element 另加 per-element code-unit 上限和总 code-unit 上限，防止一个 element
绕过元素数限制。

### 4.8 CLI observation、conformance schema 与 MATLAB oracle

当前 observation schema v1 已能无损表达 real/complex integer 的 MATLAB 文本形式
（`integer: [string]`）和 char（flat column-major `code_units`），CLI 只需从 v1
integer/char preview 填入现有字段。但当前 CLI 对 `char` 和八个 integer class 明确
落入 unsupported；必须增加对应 match，integer 格式化不得经 f64。

schema v1 的 string payload 是 JSON string，不能作为孤立 UTF-16 surrogate 的跨
Rust 无损表示。建议新增 observation schema v2，而不是原地改变 v1：

- char 继续使用 flat `code_units`，并增加 `maximum: 65535`；
- string 使用 `string_code_units: [[u16...], ...]` 加平行 `missing: [bool...]`；
- missing element 的 code-unit sequence 规范化为空；
- integer 继续使用 MATLAB oracle 的 canonical element string，以兼容复整数；
- comparator 对 char、string code units、missing 和 integer 逐元素精确比较。

MATLAB oracle 的 `normalize_strings` 在 v2 中直接对每个 element 执行
`double(char(element))` 并保留 missing 位，不先写入 JSON text。CLI v2 从协议的
UTF-16 code units 直接构造同一 payload。case schema、observation schema、oracle、
OpenMat CLI 和 comparator 必须以同一个 `schema_version: 2` 原子切换；v1 runner 和
reference 保留只读兼容。

## 5. 协议与 schema 迁移方案

1. 不修改 `spec/protocol/kernel-v0.md` 的语义。先由协调任务接受一个新的 v1 协议
   contract；协议 ID 为 `openmat-kernel-v1`。
2. initialize 时 client 发送 `[v1,v0]`，kernel 选择双方第一个共同版本。v1 client
   必须能读取 v0；v0 client 不会收到 v1 envelope。
3. v0 kernel 遇到 char/integer/非 UTF-8-lossless string inspect 时返回
   `workspace.unsupportedValue`。已有 double/logical/valid-text 行为保持不变。
4. v1 使用新 preview kinds 和 code-unit bounds。新增字段可按版本的 unknown-field
   规则演进；新增 enum kind 仍需要下一协议版本或显式 unknown representation。
5. conformance schema v2 使用新 `$id` 和新文件，不覆盖 v1 `$id`。runner 可用 manifest
   的 schema_version 选择 serializer/comparator；不得同一 observation 混合 v1/v2
   payload。
6. 迁移期 CLI 优先协商 kernel v1：v2 case 需要 v1，否则报告 unsupported；v1 case
   可在值可无损降级时继续走 v0。reference 文件永不被隐式重写。

## 6. 后续任务拆分

先由协调任务提供一个共同 interface-base commit，冻结本文件中的 type/JSON 名称。
随后 A、B、C、D、E 五个任务可并行；它们的写路径互不重叠。若共同接口要变，任务应在
边界停止并把精确变更请求交给协调者。

### Interface-base（前置，不与并行任务混做）

精确写路径：

- `crates/openmat-array/src/dtype.rs`
- `crates/openmat-array/src/lib.rs`
- `crates/openmat-array/tests/array_core.rs`
- `crates/openmat-value/src/string.rs`
- `crates/openmat-value/src/value.rs`
- `crates/openmat-value/src/lib.rs`
- `crates/openmat-bytecode/src/model.rs`
- `crates/openmat-bytecode/src/verify.rs`

交付接口：`CharCodeUnit`、`ComplexInteger<T>`、完整 `DType`、
`IntegerArrayData`、`ArrayData::{Char,Integer}`、UTF-16/missing `StringElement`、
Value/ValueKind 元数据和 COW helper，以及暂时保留旧 `Constant::String` 的
`Utf8Symbol/CharLiteral/StringLiteral` additive variants。验收：`cargo fmt --check`，
以及 `cargo test -p openmat-array -p openmat-value -p openmat-bytecode`；覆盖全部
dtype/class/shape、孤立代理项、missing 与空 string、`uint64::MAX`、real/complex
integer COW 和 constant verifier bounds。

### Task A：literal 与 bytecode constant

精确写路径：

- `crates/openmat-compiler/src/lowering.rs`
- `crates/openmat-compiler/tests/compiler.rs`

前置接口：interface-base 的 char/string constructors 和
`Utf8Symbol/CharLiteral/StringLiteral` constant shape。验收：单、双引号产生不同
constant；`''` 为 `0x0 char`；双引号为空 string scalar；delimiter doubling；非 BMP
literal encode 为两个 code units；名称 constant 不泄漏成语言 string。运行 compiler
crate 的 fmt/test，不运行全仓高成本门。旧 `Constant::String` 的最终移除由 integration
task 在所有消费者迁移后完成。

### Task B：runtime、conversion、index 与 builtins

精确写路径：

- `crates/openmat-runtime/src/array_ops.rs`
- `crates/openmat-runtime/src/interpreter.rs`
- `crates/openmat-runtime/src/error.rs`
- `crates/openmat-runtime/tests/array_bytecode.rs`
- `crates/openmat-runtime/tests/interpreter.rs`
- `crates/openmat-builtins/src/lib.rs`
- `crates/openmat-builtins/src/core_numeric.rs`
- `crates/openmat-builtins/src/core_shape.rs`
- `crates/openmat-builtins/src/core_string.rs`
- `crates/openmat-builtins/tests/core_builtins.rs`
- `crates/openmat-builtins/LIMITATIONS.md`

前置接口：interface-base 已提供 additive constant variants，使 A/B 可并行。验收：
八类 real/complex conversion、半值
舍入、NaN/Inf、饱和、shape/COW；char/string 拼接与索引；exact integer index；
condition/logical rejection；`strcmp` char/string/missing；Tranche I 外的 arithmetic
稳定 unsupported。MATLAB differential probe 只从系统临时目录执行。

### Task C：protocol v1、kernel inspect、CLI 与 server negotiation

精确写路径：

- `crates/openmat-protocol/src/lib.rs`
- `crates/openmat-kernel/src/runtime_engine.rs`
- `crates/openmat-cli/src/lib.rs`
- `crates/openmat-server/src/lib.rs`
- `crates/openmat-server/tests/websocket_transport.rs`

前置接口：已接受的 v1 protocol contract 与 interface-base element accessors。验收：
v0 serde golden tests 不变；v1 char/integer/string preview round-trip；uint64 最大值、
复整数、孤立代理项、missing、截断与总 code-unit bounds；v1/v0 negotiation 和 v0
unsupported 降级；CLI 对 v2 observation 的 exact payload；server transport 不把 v1
错误标记为 v0，也不改变 v0 frame 的兼容行为。

### Task D：conformance schema v2、oracle 与 corpus

精确写路径：

- `tests/conformance/schema/observation-v2.schema.json`
- `tests/conformance/schema/case-v2.schema.json`
- `tools/matlab-oracle/openmat_oracle_run.m`
- `tools/matlab-oracle/Invoke-MatlabOracle.ps1`
- `tools/openmat-conformance/Invoke-OpenMatConformance.ps1`
- `tools/openmat-conformance/OpenMat.Conformance.psm1`
- `tools/openmat-conformance/tests/Test-Comparator.ps1`
- `tools/openmat-conformance/tests/Test-RunnerExitCodes.ps1`
- `tools/openmat-conformance/tests/FakeOpenMat.ps1`
- `tests/conformance/cases/programs/char_integer_value_model.m`
- `tests/conformance/cases/manifests/char_integer_value_model.json`
- `tests/conformance/reference/matlab-r2022b/char_integer_value_model.json`

前置接口：冻结的 observation v2 JSON shape，不依赖 Task C 的 Rust 实现。验收：两个
schema draft-2020-12 validation；v1/v2 comparator routing；MATLAB R2022b 生成并精确
比较 char code units、string code units/missing、八类边界、复整数、转换饱和和 index
acceptance；reference 只能由显式 oracle 命令更新。

### Task E：web v1 decode 与 negotiation

精确写路径：

- `apps/web/src/protocol/kernel-v1.ts`
- `apps/web/src/protocol/kernel-v1.test.ts`
- `apps/web/src/App.tsx`
- `apps/web/src/state/ide-state.ts`
- `apps/web/src/state/ide-state.test.ts`
- `apps/web/src/components/UtilityPane.tsx`
- `apps/web/src/components/WorkspacePane.tsx`
- `apps/web/src/transport/kernel-transport.ts`
- `apps/web/src/transport/mock-kernel-transport.ts`
- `apps/web/src/transport/mock-kernel-transport.test.ts`
- `apps/web/src/transport/websocket-kernel-transport.ts`
- `apps/web/src/transport/websocket-kernel-transport.test.ts`

前置接口：已接受的 v1 JSON contract；不依赖 Task C 的 Rust 实现。验收：优先 v1、
可降级 v0；decoder 精确校验 char code unit、integer decimal component、string
code-unit/missing 与 bounds；workspace preview 不把孤立 surrogate 强塞入 JavaScript
string，只有显示层执行显式有损转换；现有 v0 fixtures 继续通过。

### Integration task

协调者按 interface-base、A/B/C/D/E 顺序集成，解决 exhaustive match，不让任一子任务
修改其他任务路径。集成验收至少包括：相关 crate fmt/test、protocol serde tests、
PowerShell comparator tests、schema validation、选定 R2022b oracle cases 和 OpenMat
differential cases。MATLAB 可执行验证不能由 Octave 结果替代。

## 7. 仍需协调者接受的决策

以下不是实现者可在 crate 内自行决定的细节：

1. 接受 `openmat-kernel-v1` 的新协议 contract、preview kind 名称和 code-unit limits；
2. 接受 observation/case schema v2 及 v1 reference 的保留期限；
3. 确认 Tranche I 是否把 complex integer constructors 纳入首批 acceptance。本文建议
   纳入，因为值模型和现有 MATLAB oracle 已明确观察到复整数；若延期，也必须保留
   storage/协议设计并稳定报 unsupported，不能转换成 complex double；
4. 确认 Tranche II 的首批 operator 清单。本文冻结 promotion/saturation 原则，但每个
   operator 仍需独立 R2022b case，未列入者不得被通用 numeric accessor 偶然启用；
5. accepted compatibility spec/RFC 若要引用本设计，应由协调任务另行提出，不在任何
   实现子任务中直接修改。
