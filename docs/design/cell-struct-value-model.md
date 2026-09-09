# Milestone 6 Cell Arrays 与 Structures 值模型提案

状态：**设计提案，尚非 accepted contract**。本文件供协调者审阅并拆分后续实现任务；
它不修改、不取代 `spec/`、accepted RFC、Milestone 6 roadmap 或任何外部 ABI/协议。

审计基线：`pre-publication baseline`（2026-08-24）。结论来自
当前 OpenMat checkout 的只读审阅，以及本机 MATLAB R2022b 的最小 clean-room
黑盒探针。探针只执行本任务自行编写的表达式，不复制 MATLAB 源码、测试、文档或
诊断消息。后续集成状态修正（以下“当前 checkout”段仍描述该审计基线）：value-stack
已落地 bytecode v11、kernel-v1 精确会话、Server v1 transport 与 CLI schema-v2 scalar
observation；aggregate 分支已落地 postfix brace/dynamic field/`EndIndex` HIR 和
Cell/Struct Value/COW core。RFC 0004 与 `spec/protocol/kernel-v2.md` 已接受并取代本文的
合同建议；下一 bytecode 版本确定为 v12。

## 1. 冻结摘要

1. `CellArray` 与 `StructArray` 都是有 canonical `Shape` 的 value container。shape 至少
   二维、保留空维度、按列主序线性化、clone 后采用 COW；不能用 Rust `Vec` 长度或
   “是否有字段”推断 scalar/empty 状态。
2. cell 的每个 element 是完整 `Value`；struct 的所有 element 共享一个有序 field
   schema，每个 field column 按 struct element 的列主序保存完整 `Value`。`{}` 是
   `0x0 cell`，`{[]}` 是包含一个 `0x0 double` 的 `1x1 cell`；`struct()` 是无字段的
   `1x1 struct`，`struct([])` 是无字段的 `0x0 struct`。
3. Rust `Clone` 只表示内部 COW clone，不是完整语言赋值边界。runtime 必须统一提供
   `language_copy`：普通 value/aggregate 共享 COW 存储，value-class 写时复制，
   handle-class 保留 identity alias。向 aggregate 写入、从 aggregate 跨赋值/参数边界
   取出以及 chained write-back 都必须经过这套策略。
4. `C(...)` 是同类 cell subarray，`C{...}` 是 cell contents；后者产生临时
   comma-separated value pack。非 scalar struct array 的 `S.field` 也产生 pack。
   pack 不是 `Value`、不能进入 workspace、cell slot、struct field、协议或 conformance
   payload；它只能被赋值、调用参数、拼接或 cell construction 等明确 consumer 消费。
5. 当前 `ParenApply` 必须原样保留 unresolved call-vs-index ambiguity。`x(...)` 继续到
   runtime 才依据 resolved `Value` 选择 call 或 paren indexing；新增的 postfix
   `x{...}` 是无调用歧义的 `BraceApply`。`struct(...)` 仍是普通 unresolved apply，
   不能在 parser/HIR 阶段硬编码为 constructor。
6. MATLAB 没有另一套 struct literal 语法。首 tranche 的 struct value construction
   是 `struct()` built-in 加后续 field assignment；CST/HIR 只需要 static field、dynamic
   field 和 apply/index place，不需要 `StructLiteralExpr`。
7. switch tranche 只消费 cell case value 的 element iterator：对每个 cell element 调用
   switch 自己的 case-match primitive 并短路。它不调用、也不定义一般 cell equality，
   更不能因为 `case {a,b}` 可工作就增加 `CellArray == CellArray`。
8. `openmat-kernel-v1` 的 `PreviewValue` 是封闭集合，只允许 char、string、logical、
   八类 integer、double/single 所对应的 canonical kinds。精确递归 cell/struct inspect
   需要新 accepted RFC 和新协议版本；不能向 v1 偷加 preview kind。
9. conformance schema-v2 已有递归 `items`、`fields`、`records` 形状和递归 comparator，
   但当前 OpenMat CLI 仍只接受 schema-v1，kernel inspect 也不能传 aggregates；现有
   MATLAB serializer/PowerShell validator 还没有全局 node/depth/cycle budget。实现前必须
   先接受统一的 bounded-observation 决策。
10. 本提案建议 bytecode 从 `11.0` 升为 `12.0`。新增 instruction、typed pack register 和
    place/write-back 语义无法由旧 runtime 安全忽略；当前 verifier 又只接受完全相等的
    `CURRENT_BYTECODE_VERSION`，因此不能把它伪装成无影响的 `11.x` minor change。

## 2. 当前 checkout 的真实边界

### 2.1 syntax、parser 与 HIR

当前 syntax 已有 `LBrace`/`RBrace`、`CellExpr`/`CellRow`、`ParenApplyExpr` 和
`FieldExpr`：

- prefix `{...}` 通过 `parse_matrix(true)` 形成 cell literal；
- postfix loop 只识别 `(...)`、`@Base(...)`、`.identifier` 和 transpose；不识别
  postfix `{...}`；
- `.` 后只接受 identifier，不接受 `.(expression)`；
- HIR 有 `ExprKind::Cell(Vec<Vec<Expr>>)`、unresolved `ExprKind::ParenApply` 和 static
  `ExprKind::Field { target, name }`；没有 `BraceApply`、dynamic field 或 index-bound
  `end`；
- standalone `:` 已降为 `ExprKind::AllIndex`；`end` 当前只是 `Name("end")`，compiler
  会把它当普通 name，而不是与目标 shape/参数位置绑定的 index expression；
- parser 允许一般 expression 出现在 `=` 左边，HIR assignment 也保留完整 target，
  但 compiler 目前只实现 name、static object field、name-rooted paren index 和一行
  name-list assignment。

因此 parser 层不能声称 brace indexing、dynamic field 或 chained aggregate assignment
已经存在。新增节点必须仍然 lossless/error-tolerant，并允许 `a(1){2}.f.(name)` 这种
postfix 链按源码顺序嵌套；compiler 是否支持整条链是独立语义问题。

### 2.2 array 与 value

`openmat-array::Shape` 已冻结了可复用基础：至少二维、去除第二维以后的尾随 singleton、
保留内部 singleton/zero extent、checked `u64` product、列主序 stride，以及少于 stored
rank 时最后一个 subscript collapse 的规则。`DenseArray<T>` 使用 `Arc<Vec<T>>`，clone
共享 storage、第一次 mutable access 经 `Arc::make_mut` detach；index result 采用
`MaterializeCopy`，不暴露 strided view。

当前 `Value` 只有 `Nothing`、logical/double/complex scalar、string、numeric/logical
array、object/object array 和 function。没有 cell/struct variant。当前
`shares_array_storage_with` 也只覆盖 numeric/logical/string/object array。`Nothing` 是 VM
内部 marker，不是 cell growth 或新 struct field 的默认语言值；默认 aggregate content
必须是一个真实的 `0x0 double Value`。

当前 object runtime 已有正确的关键分界：`language_copy` 对 value-class object/array
创建 assignment copy，对 handle-class 保持引用；普通 array/string 用 COW clone。这套
行为应被 aggregate 调用，而不是在 `CellArray` 或 `StructArray` 内复制一套 object store。

### 2.3 bytecode、compiler 与 runtime

当前集成 value-stack 的 bytecode `11.0` 是 register-oriented；除下列既有能力外，
`SwitchMatch` 也已经落地：

- `BuildMatrix`；
- unresolved `Apply`/`ReturnApply`；
- `ApplyArgument::{Value, Colon}`；
- numeric/object paren `IndexAssign`；
- class-object `GetField`/`SetField`/`ApplyField`。

compiler 已保留 `ParenApply` 到 runtime；runtime `apply_value` 先检测 callable，否则执行
numeric/object indexing，并明确拒绝 indexing 的多输出。`IndexAssign` 也只分 numeric
与 object。`GetField` 等指令目前是 class-object property/method boundary，不能在不区分
receiver kind 的情况下直接把 struct 的 pack 语义塞进原实现。

当前 compiler 对 `ExprKind::Cell` 报 `CellLiteral` unsupported，对 standalone colon 仅在
apply arguments 中转成 `ApplyArgument::Colon`；非-cell switch 已由 `SwitchMatch` 执行，
但 cell literal 与 cell-valued case list 仍 unsupported。现有
array index resolver 已能产生 column-major offsets 与 result shape，可推广给 aggregates，
但 numeric `index_assign` 没有 deletion 路径，也没有 aggregate growth/default-fill。

### 2.4 kernel、protocol 与 conformance

`ExecutionEngine::inspect` 当前只返回 `MatrixPreview`。kernel-v1 精确 char/integer 会话、
协商与对应 canonical preview 已经落地；kernel runtime 对合同允许的 scalar/array kinds
逐 element 生成 `PreviewValue`，`Nothing`/function unsupported。v1 contract 的封闭边界
仍然是：

- v1 canonical classes 只有 char/string/logical、八类 integer、double/single；
- unknown `PreviewValue.kind` 是 decode failure；
- contract 明说增加 kind 需要另一协议版本；
- v0/v1 downgrade 对不可无损值返回 `workspace.unsupportedValue`。

schema-v2 的 case/observation 文件已声明：cell 用递归 `items`；struct 用有序 `fields`
加 column-major `records`。`OpenMat.Conformance.psm1` 已递归验证和比较 nested values，
检查 `items/records` count、record field set、每个 node 的 `numel <= 4096` 和每个 string
node 的 code-unit bounds。缺口是：

- 没有全树 node/element/code-unit 预算；
- 没有显式 recursion depth 或 cycle guard；
- fields 没有统一 hard count/code-unit bound；
- MATLAB normalizer 直接递归；
- CLI manifest parser仍硬编码 `schema_version == 1`；
- CLI observation header仍固定 schema-v1；
- kernel-v1 没有递归 aggregate wire value。

## 3. Canonical 内存模型

### 3.1 共同 shape 与 storage 约束

Cell/struct 必须直接复用 `openmat_array::Shape`，不再定义第二套 shape canonicalization：

- `dimensions.len() >= 2`；
- `numel == checked_product(dimensions)`；
- element/record 的线性顺序是 MATLAB column-major；
- 空 shape 保留，例如 `0x3` 不能变成 `0x0`；
- indexing 的 one-based boundary 只存在于 runtime/index API，Rust slice offset 一律
  zero-based；
- clone 后共享存储，成功写入前才 detach；索引/field 验证失败不得提前 detach；
- selection result materialize 为新的 contiguous aggregate；首版不暴露 view。

建议内部类型如下。名称允许在 accepted interface task 中机械调整，语义不能改变：

```rust
pub struct CellArray {
    storage: DenseArray<Value>,
}

pub struct FieldName(Arc<str>);

pub struct StructSchema {
    names: Vec<FieldName>,              // observable order
    lookup: BTreeMap<FieldName, usize>, // lookup only; never defines order
}

pub struct StructArray {
    shape: Shape,
    schema: Arc<StructSchema>,
    columns: Arc<Vec<Arc<Vec<Value>>>>,  // one column per field
}
```

每个 struct field column 长度严格等于 `shape.numel()`，内部顺序为 struct elements 的
column-major order。field-major storage 是本提案的 canonical Rust model，原因是 field
read/write、加字段和 COW 都只需触碰相关 column；wire/conformance 仍重建 record-major
`records`，Rust layout 从不成为 ABI。

构造器必须拒绝：field 重名、column count 与 field count 不同、column length 与
`numel` 不同、无法放入 host `Vec` 的 shape、无效 field name。`StructSchema.lookup` 只
用于精确名称查找；field order 永远以 `names` 为真值，不能从 `BTreeMap` 排序恢复。

### 3.2 CellArray 状态

Cell 没有 scalar optimization variant。统一 storage 能避免 `{[]}`、显式 `1x1` cell、
empty shaped cell 在 runtime/API 中走不同分支：

| 状态 | shape | items |
| --- | --- | --- |
| `{}` | `0x0` | 0 |
| `cell(0,3)` | `0x3` | 0 |
| `{[]}` | `1x1` | 一个 `0x0 double` |
| `{a,b;c,d}` | `2x2` | `a,c,b,d` 列主序 |

建议 API：

```rust
impl CellArray {
    pub fn from_values(shape: Shape, values: Vec<Value>) -> Result<Self, AggregateError>;
    pub fn filled_empty_double(shape: Shape) -> Result<Self, AggregateError>;
    pub fn shape(&self) -> &Shape;
    pub fn numel(&self) -> u64;
    pub fn values(&self) -> &[Value];
    pub fn value_at_offset(&self, offset: usize) -> Option<&Value>;
    pub fn replace_at_offset(&mut self, offset: usize, value: Value)
        -> Result<Value, AggregateError>;
    pub fn shares_storage_with(&self, other: &Self) -> bool;
}
```

`from_values` 接受的 values 必须已经由 runtime 按语言 copy policy 准备好；value crate
不知道 object store，不能自行 deep-copy opaque object handles。

### 3.3 StructArray 状态与 field order

Struct 也不使用 scalar special variant：

| 状态 | shape | fields | records |
| --- | --- | --- | --- |
| `struct()` | `1x1` | `[]` | 一个空 record |
| `struct([])` | `0x0` | `[]` | 0 |
| `struct('f',cell(0,3))` | `0x3` | `[f]` | 0 |
| scalar shaped struct | `1x1` | creation order | 一个 record |
| struct array | 任意 canonical shape | 全数组共享 schema | 每 field 有 `numel` values |

field order 是可观察 value metadata：

- `struct(name,value,...)` 按 pair 出现顺序建 schema；重复 name 是 error；
- static/dynamic assignment 新增 field 时 append 到末尾；
- 更新已有 field 不重排；
- paren indexing、reshape/growth 保留 destination schema order；
- struct-to-struct assignment 以 field name 对齐。只要 field set 相同，即使 RHS order
  不同也可赋值，destination order 不变；本任务的 R2022b 探针已确认这一点；
- `orderfields`/按模板显式重排不在首 tranche，但未来只能通过返回新 value 实现。

建议 API：

```rust
impl StructArray {
    pub fn empty(shape: Shape, fields: Vec<FieldName>) -> Result<Self, AggregateError>;
    pub fn from_columns(
        shape: Shape,
        fields: Vec<FieldName>,
        columns: Vec<Vec<Value>>,
    ) -> Result<Self, AggregateError>;
    pub fn shape(&self) -> &Shape;
    pub fn numel(&self) -> u64;
    pub fn field_names(&self) -> &[FieldName];
    pub fn field_index(&self, name: &str) -> Option<usize>;
    pub fn value_at(&self, field: usize, offset: usize) -> Option<&Value>;
    pub fn replace_at(
        &mut self,
        field: usize,
        offset: usize,
        value: Value,
    ) -> Result<Value, AggregateError>;
    pub fn append_field(
        &mut self,
        name: FieldName,
        default_values: Vec<Value>,
    ) -> Result<(), AggregateError>;
    pub fn shares_schema_with(&self, other: &Self) -> bool;
    pub fn shares_field_storage_with(&self, other: &Self, field: usize) -> bool;
}
```

添加 field 时为所有既有 records 填真实 `0x0 double`，然后只覆盖被赋值的 records。
空 struct 仍保存 schema；`numel == 0` 绝不能导致 fields 丢失。

### 3.4 COW、nested values 与 class copy 边界

必须区分三件事：

1. **storage clone**：Rust `Value::clone`/aggregate clone，仅共享 Arc buffer；它不是
   对外 ABI，也不单独承诺 value-class copy 已 materialize。
2. **language assignment copy**：workspace/local/argument/capture/aggregate insertion 的
   统一 runtime operation。numeric、char、string、cell、struct 走 COW clone；
   value-class object 创建 assignment copy；handle-class object 保留 handle identity。
3. **place write-back**：对 `D{1}.x = 9`、`S(2).f(1) = 9` 等 chained lvalue，先验证
   整条 selector path，再从最内层生成更新后的 value，逐层 detach/copy-back，最后一次
   store root。不能取出 opaque handle 原地写后忘记回填 value container。

Aggregate clone 不必立即递归复制所有 nested value-class objects；否则复制一个巨大
cell 变成 O(total nested graph)。语义隔离由统一 write-back 保证：第一次修改某一条
nested value-class path 时，复制该 receiver，并把新 handle 写回已经 detach 的 aggregate
路径。handle-class path 则写回同一 handle，其他 alias 可观察同一状态。

以下边界必须调用 `language_copy`：cell literal/`cell` assignment 输入、struct constructor
field value、field/brace assignment、函数参数、函数返回值落入变量、从 aggregate 提取后
落入变量。仅作 index resolution、switch case iteration 或只读 preview 时不得制造
assignment copy。

`Value`/`ValueKind` 增加 `Cell`、`Struct`；`class_name`、`dimensions`、`numel`、workspace
summary、display、changed detection 和 size accounting 必须 exhaustive 更新。Rust
`PartialEq` 即使为测试/changed detection 存在，也不是 MATLAB equality dispatch。

## 4. Cell 语义与 staged acceptance

本节的 Phase C1-C3 是同一 Milestone 6 aggregate tranche 内部交付顺序。只有 C3 与累计
gate 都通过后，才能宣称 roadmap 的“Cell arrays and structures” tranche 完成。

### 4.1 Phase C1：construction、scalar read 与 scalar write

`{...}` cell literal：

- 每个 source element 从左到右求值；source row 之间形成二维 shape；
- 普通 element 原样成为一个 cell item，不执行 numeric concatenation；
- 行宽必须在 comma-list expansion 后一致；不一致报 structured concatenation error；
- `{}` 产生 `0x0`，不产生无 shape 的 internal marker；
- `{pack...}` consumer 按 pack 顺序把多个 values splice 为多个 cell items；
- 构造 storage 前对每个插入值执行 language assignment copy。

`cell(dims...)` 是 runtime built-in：dims 经统一 shape/size validation，所有 items 填
`0x0 double`。首 tranche 可只接受 non-negative real integer scalar dims；其他 overload
必须稳定 unsupported，不能猜测。

Scalar access：

- `C(k)`/`C(i,j,...)` 返回 shape 由统一 index resolver 决定的 `CellArray`；即使只选
  一个 element，class 仍是 cell；
- `C{k}`/`C{i,j,...}` 产生长度 1 的 `ValuePack`，single-value consumer 取该 content；
- braces 对 non-cell receiver 报 aggregate type error，不进入 call resolution；
- target 与每个 index expression 都只求值一次。

Scalar assignment：

- `C(k) = {value}` 写一个 cell slot；RHS 必须是 cell；
- `C{k} = value` 写 content，`value == []` 时存一个 `0x0 double`，不是删除 cell；
- 未定义 name 的 scalar growth 以及超出当前 linear extent 的 growth 用 `0x0 double`
  填 gap；probe 中 `{1}; C{3}=9` 得到 `1x3` 且 `C{2}` 为空 double；
- 所有 validation、allocation reservation 和 language-copy 必须先成功，再原子发布更新。

### 4.2 Phase C2：general indexing、colon 与 end

Paren 与 brace 共用一个 resolved `IndexPlan`：numeric integer indices、logical mask、
range、standalone colon、linear/multidimensional shape rules、collapsed final subscript 和
one-based bounds 都不能各写一套。结果 offsets 按 selection 的 column-major traversal
顺序排列。

`:`：

- `C(:)` 返回 column cell vector；
- `C{:}` 产生所有 contents 的 pack，顺序是 cell storage 的 column-major order；
- multidimensional colon 的 result shape 遵守现有 array index resolver，不由 element
  type 特判。

`end` 不是普通 `Value`。HIR 建议增加 `ExprKind::EndIndex`，只在 apply/index argument
expression 的 lexical scope 内合法。compiler 在已经求得 target register 后，为每个
`end` 发出：

```rust
InstructionKind::ResolveEnd {
    dst: Register,
    target: Register,
    argument_index: u32,
    argument_count: u32,
}
```

一个 subscript 时 `end == numel(target)`；多个 subscripts 时按 `Shape::effective_dimensions`
取当前 argument 的 extent，包括最后 supplied dimension 的 collapse。`end-1` 先执行
`ResolveEnd` 再走普通 arithmetic。若 unresolved `ParenApply` 最终解析为 callable，任何
绑定该 target 的 `end`/colon 都产生 structured invalid-call-argument error，而不是改作
全局 name lookup。内层 `A(B(end))` 的 `end` 必须绑定 B，不绑定 A。

### 4.3 Phase C3：comma list、assignment、deletion 与 growth

Brace selection 总是先产生 ordered `ValuePack`。本机观察冻结以下 consumption rule：

- single LHS 只消费第一个 value，忽略 trailing values；
- N 个 LHS 消费前 N 个 values，忽略 trailing values；
- pack 少于 N 个 values 时为 arity error，且不得部分赋值；
- zero-length pack 被任何正输出数 consumer 消费时为 arity error；
- 作为 call/apply arguments、cell literal 或明确 splice operand 时，pack 全部展开；
- pack 不得被 `StoreLocal`/`StoreGlobal` 或 aggregate storage 接受。

简单 `C{1:2}=9` 和 `C{1:2}=[]` 在 R2022b probe 中都失败；multi-target content
assignment 使用 bracketed comma-list place，例如 `[C{1:2}] = deal(7)`。compiler 必须
先 materialize/validate RHS pack 与全部 target places，再提交更新，避免前半 elements 已
改变而后半 arity/type 失败。

Paren assignment：

- `C(selection) = rhsCell`；RHS cell numel 等于 selection numel，或是 scalar cell 并
  扩展到整个 selection；probe 已确认 `C(1:2)={8}` 的 scalar expansion；
- RHS 非-cell（除 deletion token `[]`）是 type error；
- `C(selection)=[]` 删除 cell elements，不把 contents 设为空。多维 deletion 首版仅
  接受 R2022b 允许的整行/整列/单维线性 deletion shapes；任意不合法 hole deletion
  必须报 dimension error，不得偷偷 flatten；
- `C{selection}=[]` 是 content assignment，只有 selection 恰为 scalar 的简单形式合法；
  bracketed multi-place 可把多个 slots 都设为空 double；
- growth shape、gap fill、empty selection no-op、logical assignment 与 deletion 的
  详细 cases 必须各有 R2022b oracle case。

## 5. Struct 语义

### 5.1 construction：`struct(...)` 而非新 parser literal

`struct` 是可 shadow 的 callable name。parser/HIR 继续产生 unresolved
`ParenApply(Name("struct"), args)`：workspace 里若有同名 value，则 runtime 依现有
resolution precedence 执行 indexing；只有 resolver 选中 built-in 才进入 constructor。

首 tranche constructor overload：

- `struct()`：无字段 `1x1 struct`；
- `struct([])`：无字段 `0x0 struct`；
- `struct(name1,value1,...)`：偶数个 field/value 参数，name 为 scalar char 或 non-missing
  scalar string；
- duplicate field 是 error；field order 是 pair order；
- non-cell field value 作为一个完整 value，复制到所有 output records；
- cell field value 提供逐-record contents；所有 non-scalar cell inputs 必须 shape 相同，
  scalar cell 可扩展；任一 shaped empty cell 决定 shaped empty struct；
- probe 中 `struct('x',{1,2},'y',99)` 为 `1x2`，两个 `y` 都是 99；
- 构造器存每个 field value 时执行 language copy，handle identity/value-class boundary 与
  3.4 相同。

`struct(existingStruct)`、object-to-struct conversion、name/value cell-vector shorthand、
`rmfield`、`orderfields` 和任意 standard-library convenience overload 不在首 tranche；
应以 structured unsupported 拒绝，而不是通过通用 callable path产生近似结果。

### 5.2 static/dynamic field read

HIR 建议将 field name 表达清楚：

```rust
pub enum FieldSelector {
    Static(Name),
    Dynamic(Box<Expr>),
}

ExprKind::Field {
    target: Box<Expr>,
    selector: FieldSelector,
}
```

static `.field` 保留 identifier token/span；dynamic `.(expr)` 保留完整 expression，runtime
要求其结果为 scalar char row 或 non-missing scalar string，并验证 field-name contract。
首 tranche 至少支持 ASCII MATLAB identifier subset；完整 Unicode identifier 规则在
accepted compatibility decision 前列为未知，不能声称已覆盖 R2022b 全集。

Struct field read：

- scalar struct 返回长度 1 pack，single-value context 得到 field value；
- non-scalar struct 按 records 的 column-major order 返回 value pack；probe 中
  `S=struct('f',{10,20}); x=S.f` 取第一个，`[x,y]=S.f` 得到 10、20；
- shaped empty struct field read 返回 zero-length pack；
- missing field 报 `aggregate.missingField` 类别，不返回 `Nothing`/empty double；
- dynamic read 与 static read除 name 求值外完全同义；dynamic expression只求值一次；
- class-object static field/method dispatch保留现有 access-control 语义。runtime 应先按
  receiver `ValueKind` 分派 struct vs object，不把 struct fields注册到 class registry。

### 5.3 field write、array expansion 与 COW

Field write 是 place update，不是 object property side effect：

- scalar `S.field=value` 更新一个 record；
- non-scalar `S.field=value` 的简单赋值不是 scalar expansion；probe 中它失败。多 record
  field assignment 使用 bracketed comma-list target，例如 `[S.field]=deal(value)`；
- 对任一 element 添加新 field 时，field append 到全数组 schema，其他 records 对应值
  填 `0x0 double`；probe 中 `S(2).y=7` 后 `S(1).y` 为空；
- dynamic field write走同一 API；
- missing root/超界 scalar element可以触发 struct growth，gap records 的全部 fields 填
  `0x0 double`；
- successful write 只 detach被修改 field column；其他 field columns和未修改 nested
  storage继续共享；
- validation失败、field name无效或 RHS language-copy失败时原 struct保持不变。

Paren struct indexing/assignment：

- `S(indices)` 返回同 schema 的 `StructArray`，selection shape按统一 resolver；
- `S(selection)=scalarStruct` 对 selection scalar-expands；probe 已确认两个 elements 都被
  替换；non-scalar RHS 必须与 selection numel/shape规则兼容；
- RHS field set 必须与 destination相同。order 不同时按 name重排到 destination order；
  probe 已确认 `{a,b}` destination可接受 `{b,a}` RHS且保持 `{a,b}` order；
- field set不同是 schema error，不能隐式增删字段；添加字段只能经 field assignment；
- `S(selection)=[]` 是 record deletion并保留 schema；`S(1)=[]` 从 `1x2` 得 `1x1`；
- scalar growth填空 records；probe 中 `G(3)=struct('a',9)` 产生 `1x3`，`G(2).a` 为空。

Struct COW 覆盖 outer shape/schema、field column和 nested value 三层。`S2=S1` 后修改
`S2.payload` 不得改变 `S1.payload`；本机 probe 得到原值 1、修改值 9。handle-class field
仍保持 alias；value-class field写时复制并回填当前 struct column。

## 6. Comma-separated list 的 VM 边界

### 6.1 不把 pack 放进 Value

建议增加与普通 value register 分离的 typed pack register；不要把 frame 的普通 register
改成一个到处需要分支的 union：

```rust
pub struct PackRegister(u32);

struct ValuePack {
    values: Vec<Value>,
}

pub enum ValueSource {
    One(Register),
    Expand(PackRegister),
}

struct Frame {
    registers: Vec<Value>,
    packs: Vec<ValuePack>,
    // existing frame state omitted
}
```

这些都是 crate-internal Rust types，不是 ABI。`Function` 增加 checked
`pack_register_count`；普通 arithmetic、condition、index value、workspace store根本不能
引用 `PackRegister`。pack-aware instruction显式接受 `PackRegister`/`Expand`。verifier只需
检查两套register index各自在对应frame bound内，不需要控制流type inference，也从表示上
阻止malformed bytecode把pack泄漏到language value。

建议 `ValuePack` API：

```rust
impl ValuePack {
    fn from_values(values: Vec<Value>) -> Self;
    fn len(&self) -> usize;
    fn consume_outputs(self, requested: usize) -> Result<Vec<Value>, AggregateError>;
    fn splice_into(self, destination: &mut Vec<Value>) -> Result<(), AggregateError>;
}
```

`consume_outputs` 按第 4.3 节规则：不足报错，trailing忽略；requested 为 0 时不强制
求出/复制 contents之外的额外 language values，但 target/index side effects仍已发生。

### 6.2 Bytecode 12.0 最小新增面

建议新增而非重载 class-object 指令：

```rust
InstructionKind::BuildCell {
    dst: Register,
    rows: Vec<Vec<ValueSource>>,
}
InstructionKind::BraceApply {
    dst_pack: PackRegister,
    target: Register,
    arguments: Vec<ApplyArgument>,
}
InstructionKind::GetAggregateField {
    dst_pack: PackRegister,
    target: Register,
    field: FieldOperand,
}
InstructionKind::AssignPlace {
    dst: Register,
    root: Register,
    path: Vec<PlaceStep>,
    source: ValueSource,
    mode: AssignmentMode,
}
InstructionKind::ResolveEnd {
    dst: Register,
    target: Register,
    argument_index: u32,
    argument_count: u32,
}
InstructionKind::Unpack {
    outputs: Vec<Register>,
    pack: PackRegister,
}

pub enum FieldOperand {
    Static(ConstantId),
    Dynamic(Register),
}

pub enum PlaceStep {
    Paren(Vec<ApplyArgument>),
    Brace(Vec<ApplyArgument>),
    Field(FieldOperand),
}

pub enum AssignmentMode {
    Store,
    Delete,
}
```

`ApplyArgument` 增加 `Expand(PackRegister)`，使 `f(C{:})`、`A(C{:})` 都能在 unresolved
Apply boundary展开 pack；`Colon`继续是 index descriptor。`BuildMatrix`未来也应将 operands
升级为 `ValueSource` 以消费 `[C{:}]`，但若 aggregate tranche不在同一 commit覆盖该
consumer，compiler必须对它报明确 unsupported，不能把 pack当 scalar。

`AssignPlace` 是最小共享 write-back boundary：compiler把 name-rooted完整 path编码一次，
runtime验证、更新并返回新 root，compiler再通过已有 local/global store保存。它避免为
paren/brace/static field/dynamic field的每种嵌套组合增加一条 instruction。首 tranche只
允许可证明 root 是一个 assignable name/local；临时值链 assignment稳定 unsupported。

现有 `GetField`/`SetField`/`ApplyField` 保持 class-object含义。未来若 coordinator决定按
receiver kind合并，必须先更新 bytecode contract和所有 verifier/runtime tests，不能只改
一个 exhaustive match。

### 6.3 Compiler lowering

Lowering 顺序：

1. 求值 unresolved apply/brace/field target一次；
2. 在 target register context下降 index arguments，使 `end` 正确绑定；
3. 普通 expression需要一个 value时，对 pack发 `Unpack` with one output；
4. multiple assignment为每个 RHS producer构造/拼接 pack，再一次性 `Unpack`；
5. call/apply argument将 pack标为 `Expand`，普通 value标为 `One`；
6. lvalue先编译为 root + `PlaceStep`，RHS求值后发一个 transactional `AssignPlace`；
7. `ParenApply` 始终发 unresolved `Apply`，不得因源码 target拼写为 `cell`/`struct`而提前
   选择 indexing或built-in。

Compiler diagnostics 至少区分 malformed syntax、unsupported pack consumer、invalid
assignment target和statically invalid dynamic-field form。runtime负责动态 type、shape、
missing field、arity、growth/deletion errors。

## 7. Runtime dispatch 与稳定错误

建议把现有 numeric-specific helper推广为共享 index plan，并新增 aggregate module；不把
cell/struct逻辑继续堆进 `array_ops.rs` 的 numeric matches：

```rust
fn resolve_index_plan(
    shape: &Shape,
    arguments: &[IndexInput],
    cancellation: &CancellationToken,
) -> Result<ResolvedSelection, RuntimeErrorKind>;

fn paren_index_aggregate(... ) -> RuntimeResult<Value>;
fn brace_index_cell(... ) -> RuntimeResult<ValuePack>;
fn paren_assign_aggregate(... ) -> RuntimeResult<Value>;
fn assign_place(... ) -> RuntimeResult<Value>;
fn read_struct_field(... ) -> RuntimeResult<ValuePack>;
fn switch_case_items(cell: &CellArray) -> impl Iterator<Item = &Value>;
```

建议在现有 `InvalidExecutionState` 兼容壳下增加精确 `AggregateRuntimeError` detail，至少有：

- `InvalidOperand { operation, actual }`；
- `InvalidFieldName`；
- `MissingField { name }`；
- `SchemaMismatch`；
- `AssignmentSizeMismatch { selected, supplied }`；
- `CommaListArity { requested, available }`；
- `InvalidDeletionShape`；
- `GrowthLimit`；
- `PackEscaped`（只表示 invalid bytecode/internal，不是用户 type error）。

kernel映射使用 OpenMat-owned categories，例如 `aggregate.type`、
`aggregate.missingField`、`aggregate.schemaMismatch`、`aggregate.assignmentSize`、
`aggregate.arity`、`aggregate.deletionShape`、`aggregate.sizeLimit`。不得复制或比较 MATLAB
identifier/message。cancellation、checked size和host allocation failure沿用现有策略；长
selection、copy、field-column construction和recursive observation都定期 poll。

## 8. Switch 的 cell case 消费边界

Cell tranche只向 switch tranche提供以下概念接口：

```rust
pub enum CaseCandidates<'a> {
    One(&'a Value),
    CellItems(CellItemIter<'a>),
}
```

switch lowering/runtime规则由 switch任务拥有：selector只求值一次；逐 case求值；若
case value是 cell，按 column-major item order调用
`switch_scalar_case_match(selector, candidate)`并在首个 true短路；empty cell永不匹配；
first matching case执行且不fall through。

以下明确不由 aggregate tranche定义：

- 一般 `eq`/`isequal` 对 cell/struct的规则；
- cell selector与cell candidate的递归 equality；
- nested cell candidate自动flatten；
- object overload、NaN或跨numeric class promotion的switch match。

若 candidate本身是当前 match primitive不支持的 cell/struct/object，switch任务应产生其
accepted semantics规定的 no-match或structured error；不能递归调用case-list逻辑。Probe
只确认 numeric selector 2 可在 `case {1,2}` 中命中，不足以推出一般 cell equality。

## 9. Kernel exact observation 与协议升级请求

### 9.1 为什么 v1 不能承载

kernel-v1 的 `PreviewValue` internally tagged enum和class/kind table是封闭合同；unknown
kind是failure，规范又明确新增kind需要新版本。Cell item本身是有class/shape/payload的
完整 nested value；struct record还需要有序field schema。将它们 stringify为`text`、
object placeholder `missing` 或逐地址display会丢失value truth，违反inspect边界。

因此 exact bounded cell/struct observation **需要新 RFC + `openmat-kernel-v2`（建议名）**。
在 accepted contract 落地前：

- v1/v0对任何cell/struct inspect返回`workspace.unsupportedValue`；
- list-workspace仍可报告class/dimensions，bytes若不能checked计算则为unknown；
- display可有显式lossy text，但CLI/conformance不得从display重建value；
- 本提案不直接修改kernel-v1或accepted RFC。

### 9.2 建议的 v2 wire shape

v2 bootstrap仍使用v0 envelope，client offer顺序为`[v2,v1,v0]`，选择后禁止mid-session
fallback。`inspect` response把payload改为versioned union：flat scalar array继续使用已有
`MatrixPreview`；aggregate使用`AggregatePreview`：

```text
AggregatePreview {
  class: "cell" | "struct",
  dimensions: [safe integer, ...],
  selectedRange: {start, size},
  kind: "cell" | "struct",
  items?: [ExactValue, ...],
  fields?: [string, ...],
  records?: [{fieldName: ExactValue, ...}, ...],
  truncation: {truncated, omittedElements},
  usage: {nodes, elements, codeUnits, depth}
}
```

Cell使用`items`，struct使用`fields+records`。返回数组是selected range的最长完整
column-major prefix；`returned + omitted == product(selectedRange.size)`。每个returned
top-level item/record必须完整，nested node禁止partial payload。`ExactValue`字段名与
conformance schema-v2对齐：`class,size,ndims,numel,complex,kind`加对应flat payload，
cell/struct递归使用`items/fields/records`。JSON object property顺序不表示field order；
只有`fields`数组表示order，每个record key set必须与它完全相同。

需要RFC冻结的协商上限建议为：

- top-level `maxPreviewElements <= 4096`（沿用）；
- `maxAggregateNodes <= 16384`；
- `maxAggregateElements <= 65536`（全树所有array numel之和）；
- `maxAggregateDepth <= 32`；root depth为0；
- 每个string element `<=16384` UTF-16 units、全preview string/char/field-name code units
  `<=65536`；
- encoded frame仍不得超过既有1 MiB hard frame bound。

producer在预算耗尽前停在top-level element边界并报告omitted。若当前top-level element
单独违反per-node numel、per-string、depth、cycle或unsupported-value hard rule，整个
inspect失败，不能返回半个nested value。建议稳定protocol categories：

- `workspace.previewLimit`：node/element/code-unit/frame budget；
- `workspace.previewDepth`：深度超过协商上限；
- `workspace.cyclicValue`：observer跟踪的object identity在active path重复；
- `workspace.unsupportedValue`：function handle、未接受object representation或其他kind。

纯cell/struct value graph按值语义通常无cycle；cycle可由handle-class/object graph引入。
首aggregate exact tranche不遍历object internals，因此object/function handle作为nested
value返回unsupported。observer仍应维护active identity set，为未来object records和恶意
adapter防护预留稳定结果；cycle绝不能靠递归直到stack overflow发现。

## 10. Conformance schema-v2 映射与安全边界

### 10.1 Canonical mapping

Cell：

```json
{
  "class": "cell",
  "size": [2, 2],
  "ndims": 2,
  "numel": 4,
  "complex": false,
  "kind": "cell",
  "items": ["value(1,1)", "value(2,1)", "value(1,2)", "value(2,2)"]
}
```

示例中的字符串是顺序说明，不是实际payload；实际每项都是完整递归value object。
`items.length == numel`，包括empty时0；每项按自己的class/shape编码，绝不stringify。

Struct：

```json
{
  "class": "struct",
  "size": [1, 2],
  "ndims": 2,
  "numel": 2,
  "complex": false,
  "kind": "struct",
  "fields": ["beta", "alpha"],
  "records": [
    {"beta": "value(1).beta", "alpha": "value(1).alpha"},
    {"beta": "value(2).beta", "alpha": "value(2).alpha"}
  ]
}
```

实际record values同样是递归value objects。`records.length == numel`；每个record必须
恰好有`fields`集合，不多不少；比较field order只比较`fields`数组，JSON member order
忽略。无字段scalar struct映射为`records:[{}]`；有字段shaped empty struct保留fields且
`records:[]`。

### 10.2 现有 schema-v2 能做什么、还缺什么

现有schema和PowerShell semantic validator已经接受上述结构、递归检查每node的
shape/payload count，并递归exact compare。因此aggregate mapping本身不要求schema-v3。
但fixed global traversal budget会影响producer/validator一致性，必须由新RFC接受后在
MATLAB normalizer、PowerShell module、CLI serializer和kernel adapter四处用同一组
常量/计数顺序实现；不能只依赖`ConvertFrom-Json -Depth 100`。

建议depth-first、field-order/column-major deterministic traversal：进入node时先计1个
node和该node `numel`，再按payload顺序递归；char/string/field names按UTF-16 code units
计数。超过任何budget时立即停止且不写partial observation文件。

Conformance runner的稳定非语言结果：

| 原因 | synthetic category | summary status |
| --- | --- | --- |
| unsupported nested kind | `unsupported-payload` | `unsupported` |
| node/element/code-unit/depth budget | `payload-limit` | `unsupported` |
| repeated active object identity | `cyclic-payload` | `unsupported` |
| serializer invariant/invalid record set | `invalid-observation` | `internal` |

这些synthetic observations不是程序抛出的language error，不能与MATLAB identifier对应，
不能作为成功执行的reference truth提交。Oracle case若超预算应被case author缩小或标为
unsupported；reference更新仍只能由显式oracle命令执行。新categories及其runner status
mapping需在RFC/runner tests中接受；在那之前保持现有`unsupported-payload`行为。

首aggregate corpus不得把object/function handle放进要求exact success的nested payload；
value/handle copy semantics应通过最终返回logical/numeric/char/struct摘要的程序观察，而
不是序列化opaque identity。

## 11. MATLAB R2022b clean-room observations

探针从系统临时目录直接以`matlab.exe -batch`执行；没有创建probe/source文件，因此没有
临时文件需要保留或删除。只记录以下本任务自有表达式与归一化结论，不记录MATLAB消息：

| 自有最小表达 | 自有观察结论 |
| --- | --- |
| `size(cell(0,3))` | `[0,3]`，`numel==0` |
| `C={11,22;33,44}; C(1,:)` | class仍是cell，shape `1x2`，contents为11、22 |
| `C{1,2}` | scalar content 22 |
| `[a,b]=C{1,:}` | 两个输出为11、22 |
| `[q1,q2,q3,q4]=C{:}` | 输出顺序11、33、22、44，确认列主序 |
| `x=C{:}` with two items | single receiver得到第一个item，未报错 |
| three items to two LHS | 前两个被消费，trailing item忽略 |
| two items to three LHS | 发生arity error；不记录diagnostic文本 |
| `C{1:2}=9` / `C{1:2}=[]` | 简单multi-content assignment发生error |
| `[C{1:2}]=deal(7)` | 两个contents都变为7 |
| `C(1:2)={8}` | scalar cell扩展，两个contents都为8 |
| `C(1,:)=[]` on `2x2` | 删除整行后为`1x2`，剩余33、44 |
| `C{1,1}=[]` | content为`0x0 double`，container仍`2x2` |
| `C={1}; C{3}=9` | growth到`1x3`，gap content为空double |
| `switch 2; case {1,2}` | cell-valued case list命中 |
| `s.beta=2; s.alpha=1; fieldnames(s)` | field order为beta、alpha |
| duplicate name in `struct(...)` | 发生error |
| same field set, opposite order struct assignment | assignment成功，destination order不变 |
| `struct('x',{1,2},'y',99)` | `1x2`；x为1、2，y为99、99 |
| `S(2).y=7` on two records | field y加到全schema，第一record的y为空 |
| `d.(fn)=5; d.(fn)` | dynamic write/read得到5 |
| `S=struct('f',{10,20}); x=S.f` | single receiver得到第一个field value |
| `[x,y]=S.f` | 得到10、20 |
| simple `S.f=9` on two records | 发生error |
| `[S.f]=deal(7)` | 两个field values都为7 |
| `S(2:-1:1)` | struct indexing保留`1x2`与field schema，记录顺序反转 |
| `S(1)=[]` on `1x2` | record deletion后`1x1` |
| `struct()` / `struct([])` | 分别为无字段`1x1`/`0x0` |
| `struct('f',cell(0,3))` | 有field f的`0x3` shaped empty struct |
| missing static field read | 发生error |
| scalar struct assignment to two selected records | scalar expansion，两records都更新 |
| grow struct directly to record 3 | shape `1x3`，gap record fields为空double |
| copy then mutate nested struct/cell numeric value | 原值保持1，副本变9，确认value COW |

这些probe没有覆盖所有deletion shape、logical indexing shape、Unicode field-name规则、
object overload、nested comma-list consumer或error atomicity；它们留在第13节，不能由上表
外推。

## 12. 可拆分实现任务与严格依赖

协调者应先明确接受本提案或给出interface-base修订。以下7个任务都必须遵守各自exact
path scope；若共享接口需要变化，任务在边界停止并请求协调者，不跨crate顺手修改。

### Task 1：accepted aggregate/wire decision（最先）

精确写路径：

- `docs/rfcs/0004-cell-struct-observation-contract.md`（建议新文件名）
- `spec/protocol/kernel-v2.md`（若协调者接受v2）

共享边界：确认canonical semantics引用、v2 negotiation、`AggregatePreview`/`ExactValue`
JSON、budgets/categories、v1/v0 downgrade和schema-v2 traversal policy。本任务不改Rust或
既有accepted文件；若项目政策要求修订既有spec，必须由协调者另行授权。

验收：RFC状态明确；JSON golden positive/negative examples人工审阅；v1/v0 payload逐字
不改；budget arithmetic、whole-top-level-element truncation、cycle/unsupported categories
无歧义。

### Task 2：lossless syntax 与 HIR（可与Task 3并行）

精确写路径：

- `crates/openmat-syntax/src/lib.rs`
- `crates/openmat-parser/src/lib.rs`
- `crates/openmat-hir/src/lib.rs`

共享边界：增加postfix `BraceApplyExpr`、dynamic field representation、`EndIndex` context，
保持`ParenApply` unresolved；不做runtime type resolution。

验收：lossless round-trip和span tests覆盖literal vs postfix braces、静态/动态field、长postfix
链、brace/paren assignment target、`end`/colon nested binding、missing delimiters/recovery；
运行`cargo test -p openmat-syntax -p openmat-parser -p openmat-hir`。

### Task 3：aggregate Value/COW core（可与Task 2并行）

精确写路径：

- `crates/openmat-value/src/lib.rs`
- `crates/openmat-value/src/value.rs`
- `crates/openmat-value/src/cell.rs`（新）
- `crates/openmat-value/src/struct_array.rs`（新）

共享边界：本节`CellArray`/field-major `StructArray`、`Value::{Cell,Struct}`、metadata/COW API；
不访问object store，不自行定义language equality。

验收：scalar/empty/shaped states、column-major storage、field order/duplicate rejection、
shaped empty fields、per-field detach、nested aggregate COW、invalid constructor不detach；
`cargo test -p openmat-value`。

### Task 4：bytecode 12.0 与 compiler lowering（依赖Task 2、3）

精确写路径：

- `crates/openmat-bytecode/src/lib.rs`
- `crates/openmat-bytecode/src/model.rs`
- `crates/openmat-bytecode/src/verify.rs`
- `crates/openmat-compiler/src/lib.rs`
- `crates/openmat-compiler/src/diagnostic.rs`
- `crates/openmat-compiler/src/lowering.rs`
- `crates/openmat-compiler/tests/compiler.rs`

共享边界：`PackRegister`/`pack_register_count`所需instruction contract、`ValueSource`、`BraceApply`、
`AssignPlace`、`ResolveEnd`、pack-aware lowering；在已有 v11 `SwitchMatch` 之上把版本升
至`12.0`。不得把Rust enum layout当
serialization ABI。

验收：verifier拒绝pack escape/invalid field operands/path/registers；compiler snapshots
证明`f(x)`仍是Apply、`C{}`是BraceApply、dynamic field仅求值一次、nested end绑定正确、
assignment path只store root一次；两crate全部tests。

### Task 5：runtime aggregate semantics（依赖Task 3、4）

精确写路径：

- `crates/openmat-runtime/src/lib.rs`
- `crates/openmat-runtime/src/interpreter.rs`
- `crates/openmat-runtime/src/array_ops.rs`
- `crates/openmat-runtime/src/aggregate_ops.rs`（新）
- `crates/openmat-runtime/src/builtin.rs`
- `crates/openmat-runtime/src/error.rs`
- `crates/openmat-runtime/tests/interpreter.rs`
- `crates/openmat-runtime/tests/array_bytecode.rs`

共享边界：统一`ResolvedSelection`、language-copy callback、transactional place write-back、
`cell`/`struct` builtins、pack consumption和switch candidate iterator；不改protocol/kernel。

验收：第4/5节全部positive/error/atomicity cases，value/handle class nested copy boundary，
growth/default fill、deletion、end/colon、cancellation和size limit；运行
`cargo test -p openmat-runtime`及strict clippy for该crate。

### Task 6：protocol v2、kernel与CLI（依赖Task 1、3、5）

精确写路径：

- `crates/openmat-protocol/src/lib.rs`
- `crates/openmat-protocol/src/kernel_v2/**`（新）
- `crates/openmat-kernel/src/lib.rs`
- `crates/openmat-kernel/src/runtime_engine.rs`
- `crates/openmat-cli/src/lib.rs`
- `crates/openmat-server/src/lib.rs`
- `crates/openmat-server/src/websocket.rs`
- `crates/openmat-server/tests/websocket_transport.rs`

共享边界：只实现Task 1 accepted wire shape；v1/v0 codec/goldens不改；recursive observer
通过Value accessors，不读取Rust layout。

验收：v2/v1/v0 negotiation、exact nested cell/struct、field order、shaped empties、所有预算
边界、whole-element truncation、depth/cycle/unsupported、1 MiB frame和downgrade；CLI能为
schema-v2 manifest输出v2 observation；native/Web lifecycle gate。

### Task 7：schema-v2 traversal、oracle与corpus（Task 1后可准备；最终依赖Task 5、6）

精确写路径：

- `tools/matlab-oracle/openmat_oracle_run.m`
- `tools/matlab-oracle/Invoke-MatlabOracle.ps1`
- `tools/openmat-conformance/OpenMat.Conformance.psm1`
- `tools/openmat-conformance/Invoke-OpenMatConformance.ps1`
- `tools/openmat-conformance/tests/**`
- `tests/conformance/cases/programs/cell_struct_value_model.m`（新）
- `tests/conformance/cases/manifests/cell_struct_value_model.json`（新）
- `tests/conformance/reference/matlab-r2022b/cell_struct_value_model.json`（显式oracle生成）

如Task 1接受的bounds无法由现有schema-v2表达且必须改schema，Task 7必须停下，向协调者
请求额外授权`tests/conformance/schema/*-v2.schema.json`；本scope默认不修改accepted
schema files。

验收：recursive traversal deterministic且有node/depth/cycle budgets；synthetic categories
与summary status测试；R2022b reference覆盖本设计probe但不记录消息；comparator self-test、
runner exit-code self-test、选定oracle run和完整OpenMat cumulative conformance均通过。

### Integration order

严格DAG：Task 1先于6/7；Task 2与3可并行；Task 4等待2+3；Task 5等待3+4；Task 6等待
1+3+5；Task 7的schema/comparator部分可在Task 1后并行准备，但reference/完整differential
gate等待5+6。协调者按`1 -> (2 || 3) -> 4 -> 5 -> (6 || 7-prep) -> 7-final`集成，最后跑
Milestone 6累计Windows/Web gates。

## 13. 未知项与首 tranche 明确不做

### 13.1 仍需R2022b probes/accepted decision

1. 多维/ND paren deletion的完整合法shape集合，以及linear deletion后的row/column
   orientation细节。
2. logical index用于brace pack和struct field list时的result/output order边界。
3. Unicode/non-identifier dynamic field names、empty char/string、missing string field name的
   精确接受规则。
4. empty comma list进入每种consumer（function args、matrix/cell literal、nested apply）的
   完整arity/evaluation-order规则。
5. nested pack consumers如`A(C{:})`、`{S.field}`、`[S.field]`与chained temporary indexing
   的全部corner cases。
6. struct concatenation/assignment在field set相同但order不同、shaped empty、ND shape下的
   全量规则；当前只probe了scalar RHS assignment。
7. assignment发生RHS copy/allocation/object setter error时R2022b可观察的atomicity；OpenMat
   本提案选择transactional语义，兼容性仍需case确认。
8. switch的scalar match primitive对NaN、missing string、char matrix、cross-numeric class和
   object overload的规则；cell tranche只提供候选iterator。
9. object exact observation是否要使用schema-v2 `kind:object` records，以及handle identity
   graph/cycle如何表示；首aggregate exact tranche统一unsupported。
10. kernel-v2 budget数值、encoded-byte accounting和web client是否展示partial aggregate
    preview，需要accepted RFC而不是crate-local选择。

### 13.2 不在首 tranche

- 一般cell/struct equality、ordering、hashing或set semantics；
- table/timetable/categorical、sparse/GPU/distributed arrays；
- struct/object互转、`cell2struct`、`struct2cell`、`rmfield`、`orderfields`及完整reflection；
- cell/struct对所有numeric/string builtins的overload；
- heterogeneous class-object array替代cell；
- arbitrary temporary-expression lvalue、`subsref`/`subsasgn` object overload和meta-class
  dynamic dispatch；
- handle/object graph wire identity、shared-reference preservation或cycle serialization；
- strided aggregate views、JIT、Rust ABI/plugin boundary；
- 修改kernel-v1、schema-v2或accepted RFC而未先获得协调者明确授权。

## 14. 提案接受条件

协调者在启动实现前至少需明确决定：

1. 是否接受field-major `StructArray`与no-scalar-special-case模型；
2. 是否接受internal typed `PackRegister`/`AssignPlace`共享边界和bytecode `12.0`；
3. 是否接受kernel-v2而不是扩展v1，以及具体budget/categories；
4. schema-v2 traversal budget是新RFC的producer policy，还是需要新schema版本；
5. 首 tranche是否必须一次交付C1-C3全部comma-list/deletion/growth consumers；本提案建议
   是，避免roadmap acceptance声称超过实际支持；
6. Unicode field-name与object exact observation是否明确延期并以structured unsupported
   暴露。

在这些决定被accepted contract/interface commit替代前，本文件始终只是proposal。
