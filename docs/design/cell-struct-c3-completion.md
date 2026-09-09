# Cell/Struct C3 completion：动态输出、place planning 与提交边界

状态：**候选实现设计，不是 accepted contract**。

本文只补全 Cell/Struct C3 的 compiler/bytecode/runtime 共享边界，不修改
`docs/rfcs/0004-cell-struct-observation-contract.md`、`spec/` 或任何 Rust 实现。
如本文与 accepted RFC 冲突，以 accepted RFC 为准；需要改变共享合同的部分必须由
coordinator 单独接受。

审计基线：

- integration HEAD：`pre-publication baseline`
  (`codex/aggregate-integration`)；
- compiler aggregate lowering：当前集成 commit
  `pre-publication baseline`，与任务指定的
  `pre-publication baseline` 在
  `crates/openmat-compiler` 下内容相同；
- accepted contract：RFC 0004 的 C3 与 VM/bytecode 12.0 章节；
- oracle corpus：现有 10 个 `aggregate_*` program、manifest 与 R2022b reference。

为避免把不同证据等级混在一起，本文使用以下标签：

- **[Probe]**：本任务在 MATLAB R2022b 上得到的 clean-room 可观察事实；
- **[Checkout]**：对当前 live checkout 的只读审阅、build 或 test 事实；
- **[Contract]**：accepted RFC 已冻结、实现必须满足的合同；
- **[Inference]**：由 probe/checkout/contract 推导、但不是直接 observation 的结论；
- **[Proposal]**：由事实与合同推导的 OpenMat 候选实现；
- **[Blocker]**：probe 与 accepted wording 冲突或合同不足以唯一决定实现，必须先由
  coordinator 接受勘误/修订；
- **[Unknown]**：本任务未验证或需 coordinator 决定的边界。

本文严格区分三种容易混用的“place”：

1. **expanded bracket target**：bracket 中只有一个 aggregate term，但该 term 展开为多个
   leaf places，例如 `[C{I}]`、`[S(I).f]`；
2. **independent LHS places**：同一 assignment 中有多个逗号分隔的 LHS terms，例如
   `[C{1}, C(2), x, B{1}]`；即使两个 term 共享 root，它们仍是独立 term；
3. **nested path inside one place**：一个 term 内从一个 root 沿多个 selector write back，
   例如 `C{end}.x(end)=v`；它不是多个独立 LHS term。

## 1. 冻结结论

1. **[Probe]** `[C{I}]` 与 `[S(I).f]` 的目标数在 RHS call 之前动态决定。
   当 bracketed LHS 含至少一个 comma-list target 时，所有 LHS term 的 index 与
   dynamic field-name expression 按源码从左到右先求值一次；之后 RHS arguments
   从左到右求值，producer 以所有 LHS term cardinality 之和作为 requested
   `nargout` 进入。
2. **[Probe]** 普通 name、普通/paren place 与 aggregate place 可以混合。普通 name 或
   固定宽度 place 贡献 1，cell brace selection 贡献 selected contents 数，struct field
   selection 贡献 selected records 数，empty selection 贡献 0。
3. **[Probe]** requested output 为 0 时 producer 仍执行且观察到 `nargout == 0`；RHS pack
   target/index 也仍求值。RHS pack 不足时在任何 LHS commit 前失败；多余 pack values
   被忽略。`deal` 的单输入复制与多输入 count 检查属于 callable 自身语义，不能由
   compiler 对 `deal` 拼写特判。
4. **[Probe]/[Contract]** 一个 expanded bracket target 内的多个 leaf place，以及一个
   place 内的完整 nested path，都是 root-local transaction。
5. **[Probe]** 多个 independent LHS places 在 R2022b 中不是 statement-wide transaction；
   各 term 在 RHS 成功并通过全局 arity barrier 后按源码顺序提交，后项失败不回滚前项。
   该结论对同一 root、不同 root、ordinary-then-aggregate 都成立。
6. **[Blocker]** RFC 0004 的 “materializes and validates the RHS values and every target
   place before committing any update” 没有说明 “every target place” 只指 expanded target
   内的 leaves，还是也包含 independent LHS terms。若包含后者，就与第 5 点直接冲突。
   在 accepted RFC 勘误/修订前，不得实现多个 independent aggregate LHS places。
7. **[Contract]** 每一个 aggregate term 内仍必须完整预验证 RHS slice 与 path、在旁路
   构造 updated root，并最多发布一次 root store。第 4 点不放宽单 term 的
   `AssignPlace` 原子性。
8. **[Probe]** RHS 可以在 LHS planning 与 commit 之间修改同一 root；最终 assignment
   保留 RHS 对 root 的修改并在其上应用已缓存的 selector。例如 RHS 把 2-element cell
   改为 3-element cell 后，前两个 planned contents 被写入，第三个 RHS-side-effect
   element 保留。因此 place token 不能把 planning snapshot 当成最终 commit base。
9. **[Probe]/[Contract]** nested lvalue `end` 绑定每个 step 的即时 target 与当前维度，
   selector/dynamic field expression 只求值一次。deletion/growth 使用 mutation 前解析的
   `end` 数值。
10. **[Proposal]** 当前 `PackRegister` 必须保留，但还需 typed `PlaceRegister`、typed
   assignment-plan token、stepwise place planning、dynamic-output `CallPack`/`ApplyPack`
   与 ordered `DistributePack`。这些是 crate-internal logical bytecode，不是 Rust ABI。
11. **[Proposal]** `aggregate_c3_transaction_rollback` 不能由当前“一次 error 后会话结束”的
   differential observation 独立证明。aggregate tranche 可先用 Rust runtime unit test
   在 `Err` 后检查 workspace/root/alias，再将该单个 oracle case 等待 try/catch；若
   acceptance 要求现有 10 个 program 全部 differential pass，则 try/catch 必须前置。

## 2. 当前 checkout 的精确边界

### 2.1 已有 bytecode 12.0

**[Contract]** RFC 0004 接受了独立 `PackRegister`、pack-aware operands、`ResolveEnd` 与
transactional `AssignPlace`；pack 不能成为 language `Value`，`ParenApply` 必须保持
call-vs-index unresolved。

**[Checkout]** 当前 `openmat-bytecode` 已有：

- `PackRegister` 与 per-function `pack_register_count`；
- `ValueSource::{One, Expand}`、`ApplyArgument::Expand`；
- `BuildCell`、`BraceApply`、`GetAggregateField`、`Unpack`；
- `ResolveEnd`；
- 单个 `root + Vec<PlaceStep> + source + mode` 的 `AssignPlace`；
- fixed-output `Call { outputs: Vec<Register>, ... }` 与
  `Apply { outputs: Vec<Register>, ... }`；
- current bytecode version `12.0`，verifier 对 value/pack register 分别做 bounds check。

`cargo test --locked -p openmat-bytecode -p openmat-compiler` 在外部临时 target 目录通过：
bytecode 27 tests、compiler 56 tests。

### 2.2 当前 lowering 的四个关键缺口

**[Checkout]** `c473048` 对应 lowering 当前行为如下：

1. `matrix_contains_aggregate_place` 对多个 syntactic aggregate LHS term 报
   `MultipleAggregateAssignmentTargets`；
2. 单个 `[C{I}] = rhsCall(...)` 会进入 `lower_place_assignment`，但 RHS call 通过
   `lower_expression` 固定请求 1 个 output，不会根据 `I` 的 runtime selection size
   请求动态 `nargout`；
3. nested place 的第二个及后续 paren/brace step 含 `end` 时直接报
   `NestedPlaceEnd`；所有 step 的现有 `lower_apply_arguments` 也只拿 root register
   作为 `ResolveEnd.target`，无法表示即时 nested target；
4. `Unpack.outputs`、`Call.outputs`、`Apply.outputs` 都在 compile time 固定宽度，不能把
   runtime cardinality 的 pack 分配给动态 place range。

这解释了为什么 `PackRegister` 已能表达 `cells{:}` 的 read pack，却不能完整表达
`[cells{I}] = producer(...)`。

### 2.3 runtime 尚未接上 v12 aggregate surface

**[Checkout]** `cargo check --locked -p openmat-runtime` 在外部临时 target 目录
失败于两个静态边界：interpreter 尚未覆盖 6 个 aggregate instruction variant，且 index
argument conversion 尚未覆盖 `ApplyArgument::Expand`。因此当前 checkout 的 10 个
aggregate oracle program **end-to-end 为 0/10**；不能把 compiler/bytecode unit test
通过写成 runtime 或 differential 通过。

## 3. R2022b clean-room probe 方法

**[Probe]** 使用：

```text
$env:MATLAB_EXE -batch <self-authored probe>
```

探针位于系统临时目录，只调用本任务自写的 side-effect/counter/producer functions；没有
复制 MATLAB 源码、测试、文档或诊断消息。caught failures 只归一化为 success/failure
boolean，不记录 identifier 或 message。

最终 probe suite 连续执行两轮：

- 每轮 32 条规范化 observation；
- 两轮逐字相同；
- 规范化输出 SHA-256：
  `A00D1579C7EA182D09DB400E78CE2F53C890FAE4F7E7424D4126F4CDF0EFA873`。

## 4. 求值顺序与 dynamic requested outputs

### 4.1 含 comma-list target 的 bracketed assignment

以下阶段是 **[Probe]**，也是本文建议 compiler/runtime 固定的顺序：

1. 按 LHS term 源码顺序进行 planning；
2. 一个 term 内按 postfix chain 顺序求值 target selector：index arguments 从左到右，
   dynamic field name 在其 receiver/index 之后求值；每个 expression 恰好一次；
3. 计算每个 term cardinality 并求和；若任一 LHS plan 失败，停止，RHS 不求值；
4. RHS arguments 从左到右求值；
5. callable producer 以总 cardinality 作为 requested `nargout` 执行；pack producer 则
   materialize 自身实际 pack；
6. 对实际 RHS pack 做 statement arity barrier；
7. 按 LHS term 源码顺序分配 RHS slice 并提交。

第 7 步是 **[Probe]** 的 R2022b 行为；对多个 independent targets 的 OpenMat 实现仍受
第 5.4 节 accepted blocker 约束。单 expanded target 不受该 blocker 影响。

混合探针的事件顺序是：

```text
cell-index -> struct-index -> dynamic-field -> rhs-arg-1 -> rhs-arg-2
-> producer(nargout=6)
```

对应 LHS 为一个 ordinary name、2 个 cell contents、另一个 ordinary name、2 个 struct
field leaves，总 requested outputs 为 `1 + 2 + 1 + 2 = 6`。另一个混合探针证明 ordinary
indexed term 位于 expanding term 前后时，它们的 index expression 也都在 RHS 之前按
LHS 顺序求值。

**[Probe]** 如果 bracketed LHS 没有任何 dynamic comma-list cardinality，已测的纯
fixed-width cases 是 RHS arguments/producer 先执行，随后 indexed LHS selector 才执行。
本 C3 completion 不应无意改变这条既有 ordinary assignment 顺序。

### 4.2 Cardinality 规则

| LHS term | **[Probe]/[Contract]** requested output contribution |
| --- | ---: |
| ordinary name、`~`、固定宽度 paren place | 1 |
| final `C{I}` | resolved selection 的 content 数 |
| final `S(I).f` / `S(I).(name)` | selected struct record 数 |
| nested chain 最终只指向一个 leaf | 1 |
| empty brace/field selection | 0 |

Cell contents 与 struct records 的映射顺序都是 resolved selection order。探针中
`I = [2, 1]` 时，第一个 output 写 record/content 2，第二个写 record/content 1。

### 4.3 0、short、extra 与 callable-specific behavior

**[Probe]/[Contract]** 通用 pack consumption：

- total requested 为 0：producer 仍被调用并观察 `nargout == 0`；RHS pack 的 target/index
  仍求值；不发生 LHS store，也不因 RHS pack 非空而报 arity error；
- actual pack 少于 requested：arity failure，任何 LHS term 都不 commit；cell root、struct
  schema 与既有 fields 保持不变；
- actual pack 多于 requested：消费前 requested 个，trailing values 忽略；
- 固定声明两个 outputs 的自写 function 被请求三个 outputs 时，function body 没有进入；
  LHS planning side effect 已发生，LHS 未 commit；
- producer body 自身失败时，LHS plan 和 RHS argument side effects 已发生，producer body
  side effect 已发生，LHS 未 commit。

**[Probe]** `deal` 的额外观察：

- 一个 input 可按 requested count 复制到 0、2、3 等 outputs；
- 多 inputs 与 requested count 不同，无论少或多，所有 input expressions 先求值，然后
  callable 失败，LHS 不 commit。

**[Proposal]** 这些规则通过普通 unresolved dynamic-output call 实现。compiler 不检测
callee 名称是否为 `deal`；builtin 与 user function 都从同一个 runtime requested-output
字段读取 `nargout`。

## 5. 原子性与失败后副作用

### 5.1 一个 expanded bracket target 内的多个 leaf places

**[Contract]** 一个 `AssignPlace` 必须：

1. 验证完整 selector path；
2. 验证并 language-copy 该 term 消费的完整 RHS slice；
3. 在不可观察的 working root 上执行所有 leaf updates；
4. 成功后最多 store root 一次；
5. 失败时不 detach/publish root，所有 aliases 保持原值。

**[Probe]** `[C{[1,2,3]}] = R{[1,2]}` 在 RHS arity shortage 后 `C` 全部不变；
`[S([1,2]).newf] = R{1}` 后 `newf` 没有进入 schema，既有 field values 不变。
同一 expanded place 含 duplicate offsets 时按 selection order 应用，后一个 value 胜出。

因此 **[Proposal]** `[C{I}]` 和 `[S(I).f]` 各自是一个 root-local transaction，不为每个
selected offset 单独 store root。这里 bracket 内只有一个 aggregate term；`I` 展开得到
的 leaves 不是多个 independent LHS terms。

### 5.2 一个 place 内的 nested path

**[Contract]** `AssignPlace` 的 name-rooted `path` 可以包含 paren、brace、static field 与
dynamic field steps。完整 path 与 RHS 必须共同验证，updated root 在旁路从最内层向外层
write back，成功后最多发布一次 root store。`C{end}.x(end)=v` 因此是一个 place、一个
root transaction，不是分别提交 `C{end}`、`.x` 与 `(end)`。

**[Probe]** 本任务验证了 nested selector/field expressions 只求值一次、即时 `end` 绑定与
成功 writeback；没有用 R2022b probe 制造一个 nested inner failure 后再检查 outer root。
**[Proposal]** nested failure rollback 先由 host-side runtime test 证明，不能由现有
`aggregate_c3_transaction_rollback` 外推。

### 5.3 多个 independent LHS places

**[Probe]** MATLAB R2022b 可观察为顺序提交，不是整个 statement rollback：

| LHS 形态 | 后项失败后的可观察状态 |
| --- | --- |
| `[C{1}, C(2)] = producer(...)` | `C{1}` 已更新；第二项失败；同 root 不回滚第一项 |
| `[A{1}, B(1)] = producer(...)` | `A{1}` 已更新；`B` 失败且不变；cross-root 不回滚 `A` |
| `[x, C(1)] = producer(...)` | `x` 已更新；`C` 失败且不变 |
| `[C(1), x] = producer(...)` | 第一项失败；`x` 未提交 |
| `[C{1}, C{1}] = producer(...)` | 成功，第二项覆盖第一项 |
| `[C{1}, C{2}] = producer(...)` | 成功，两次更新都保留 |

另一个 cross-root short-pack probe `[C{1},D{1}] = R{:}` 证明共同 RHS pack 只有一个 value
时，两个 roots 都不改变。因此 **[Probe]** dynamic statement 先有全局 arity barrier，
通过后才出现上表的 term-by-term commits。

全 statement 仍有一个共同 **arity barrier**：RHS pack shortage 在任何 term commit 前
失败。这与 term-local type/shape/schema validation 不同；后者在该 term 即将 commit 时
执行。

**[Proposal] [Blocker]** R2022b-compatible transaction boundary 应是“一个 syntactic
aggregate term 的一个 name-rooted update”，不是“整个 bracketed statement”，也不是
“按 root 自动合并所有 term”。即使两个相邻 term 共享 root，也保留源码顺序与后项失败
时前项已发布的事实。但第 5.4 节的 accepted wording 冲突未解决前，compiler 继续拒绝
多个 independent aggregate LHS places；不得先实现该建议再事后解释合同。

### 5.4 RFC 0004 逐条对照与 blocker

| RFC 0004 wording | expanded bracket target | independent LHS places | nested path |
| --- | --- | --- | --- |
| “materializes and validates the RHS values and every target place before committing any update” | 与 short-pack/schema probe 相容：所有 leaves 在该 root 发布前共同验证 | 若 “every target place” 包含所有 independent terms，则与 same-root/cross-root 顺序提交 probe 直接冲突 | 可解释为完整 nested path 在一次 root store 前验证，和 VM invariant 相容 |
| VM minimum boundary 是 “one `AssignPlace` containing a name-rooted path” | 一个 term/一个 root 可由一个 instruction 表示 | 当前 contract 没有 statement-wide multi-root transaction representation | 正是一个 name-rooted multi-step path |
| VM invariant 6 要求 `AssignPlace` 完整验证、旁路构造、最多一次 root store | 相容 | 只约束每个 `AssignPlace` 时相容；若把整个 LHS 当一个 transaction，则表达不足 | 相容 |
| C3 要求 failed type/shape/field/arity/copy checks 留下 original root/aliases | 对一个 term 的 root 明确相容 | “original root” 若指失败 term 则相容；若要求回滚同 statement 已成功的同 root 前项，则与 probe 冲突 | 对完整 path 的 root 相容 |
| negative acceptance 要求 failed transactional write-back | transaction unit 可明确为一个 expanded term | transaction unit 未定义，不能据此声称全局原子性 | transaction unit 可明确为一个 full path |

**[Blocker]** accepted RFC 同时使用 plural “every target place” 与 singular “one
`AssignPlace`”，没有定义 transaction unit。存在两种均可从文字读出的实现，其中一种与
R2022b probe 冲突。多个 independent aggregate LHS places 的 compiler/runtime
implementation 必须等待 accepted RFC 勘误/修订。

本文建议 coordinator 接受以下勘误以对齐 R2022b：

1. “every target place” 只指一个 syntactic aggregate term 展开的 leaves 与其 full nested
   path；
2. 整个 dynamic LHS statement 共同完成只读 planning 与 RHS arity barrier；
3. arity barrier 通过后，independent terms 按源码顺序各自做一次 root-local transaction；
4. 后一 term 失败不回滚已成功的前项，包括 same-root 前项；
5. “failed transactional write-back” 指失败 term 不发布自己的 working root，不代表
   statement-wide undo。

另一个合法 accepted decision 是保留比 MATLAB 更强的 statement-wide atomicity；若选择
该方向，必须明确记录 R2022b compatibility deviation，并为 multi-root rollback 定义新的
bytecode/runtime contract。不能把它描述成 probe-confirmed MATLAB 行为。

### 5.5 planning snapshot 与 live commit root

**[Probe]** 两种形式都得到相同结果：

```text
[root{[1,2]}] = producer_that_replaces_root_with_{70,80,90}
[root{1}, root{2}] = producer_that_replaces_root_with_{70,80,90}
```

最终 root 为 `{101,102,90}`。因此 **[Proposal]**：

- planning token 缓存 selector values、dynamic field name、resolved index plan、resolved
  `end` 数值和 cardinality；
- token 可以临时持有 planning focus，但不得拥有“最终要发布的 root snapshot”；
- 每个 term commit 前从其 local/global binding 重新读取 live root；
- 使用已缓存 selector 对 live root 重新做 type/shape/schema/bounds validation；
- 在 live root 的 COW clone 上应用该 term，再 store 一次；
- 绝不重新执行 LHS expression。

这样既保留 RHS 对 root 的合法 side effect，也能在 RHS 把 root 改为不兼容 value 时安全
失败。

## 6. Nested lvalue `end`

### 6.1 观察与绑定规则

**[Probe]** `C{end}.x(end, end-1) = 999` 在 `C` 有两个 items、第二个 `x` 为 `2x3`
matrix 时：

- outer `end` 绑定 `C.numel == 2`；
- inner 第一个 `end` 绑定即时 target `x` 的第 1 维 extent `2`；
- inner 第二个 `end-1` 绑定第 2 维 extent `3` 后做普通减法，得到 `2`；
- 更新 `x(2,2)`，不是把任一 inner `end` 绑定到 root `C`。

**[Probe]** nested chain 的 outer index、dynamic field-name、inner index 与 RHS value 的
side-effect 顺序为：

```text
outer-index -> field-name -> inner-index -> rhs
```

每项恰好一次。

### 6.2 deletion/growth 与 `end`

**[Probe]**：

- `{1,2,3}` 上 `C(end)=[]` 删除 pre-mutation last element，得到 `1x2 {1,2}`；
- `{1,2}` 上 `C(end+2)={9}` 使用旧 `end == 2`，写 index 4，得到
  `1x4 {1,2,empty-double,9}`；
- nested cell field `{10,20,30}` 先以 `x(end)=[]` 删除到 `{10,20}`，随后
  `x{end+2}=99` 使用删除后的即时 target `end == 2`，得到
  `{10,20,empty-double,99}`。

**[Proposal]** `end` 在 planning phase 对每一个 step 的当前 focus 解析成 ordinary checked
index scalar；deletion/growth 在 commit 时消费这个缓存值。commit 后不重新解析 `end`，
也不因 growth 改写它。

## 7. v12 completion 的最小逻辑模型

以下名称只冻结语义角色，**不承诺 Rust enum layout、Rust ABI 或 serialized spelling**。

### 7.1 新 typed token

**[Proposal]** 在 value/pack register 之外增加：

```rust
struct PlaceRegister(u32);
struct AssignmentPlanRegister(u32);

struct Function {
    register_count: u32,
    pack_register_count: u32,
    place_register_count: u32,
    assignment_plan_register_count: u32,
    // existing fields
}
```

内部 runtime token 的建议含义：

```rust
struct PlannedPlace {
    cached_steps: Vec<ResolvedPlaceStep>,
    cardinality: usize,
    planning_focus: Option<Value>,
}

enum RootBinding {
    Local(LocalSlot),
    Global(ConstantId),
}

enum PlannedAssignmentTarget {
    Discard,
    Binding(RootBinding),
    Place {
        root: RootBinding,
        plan: PlaceRegister,
    },
}

struct AssignmentPlan {
    targets: Vec<PlannedAssignmentTarget>,
    requested_outputs: usize,
}
```

`planning_focus` 只服务后续 nested step/`end` 解析；assignment plan 建成后可释放不再需要
的 focus snapshot。`cached_steps` 不含可再次调用的 expression，只含 evaluated values、
field name 与 resolved selector metadata。

### 7.2 Stepwise place planning

**[Proposal]** 需要等价于以下 instruction roles：

```rust
BeginPlace {
    dst_place: PlaceRegister,
    root: Register,
}

ExtendPlace {
    dst_place: PlaceRegister,
    base_place: PlaceRegister,
    step: PlaceStep,
}

ResolvePlaceEnd {
    dst: Register,
    place: PlaceRegister,
    argument_index: u32,
    argument_count: u32,
}

BuildAssignmentPlan {
    dst_plan: AssignmentPlanRegister,
    targets: Vec<PlannedAssignmentTarget>,
}
```

Compiler 在下降一个 step 的 arguments 前持有 `base_place`。该 step 内的每个 `end` 发
`ResolvePlaceEnd(base_place, argument_index, argument_count)`；arguments 全部求值后才发
`ExtendPlace`，使新 token 的 focus 前进到下一即时 target。这消除了当前“所有 nested
`end` 只能引用 root register”的表示缺口。

### 7.3 Dynamic-output producer

**[Proposal]** 新增 dynamic-output pack producer，而不修改普通 register 的类型：

```rust
PackOne {
    dst_pack: PackRegister,
    value: Register,
}

CallPack {
    dst_pack: PackRegister,
    callee: Register,
    arguments: Vec<Register>,
    requested_by: AssignmentPlanRegister,
}

ApplyPack {
    dst_pack: PackRegister,
    target: Register,
    arguments: Vec<ApplyArgument>,
    requested_by: AssignmentPlanRegister,
}
```

`ApplyPack` 必须调用与现有 `Apply` 完全相同的 runtime unresolved resolution。若 target
解析为 callable，frame requested-output count 取 assignment plan 的动态总数；若解析为
indexable value，则走相同 apply/index dispatch 与多输出规则。不能依据 target 源码拼写
选择 `deal`、`cell`、`struct` 或任何 builtin。

已有 brace/field producer 直接给出实际 `PackRegister`；普通 RHS value 用 `PackOne`。
`ReturnApply` 仍负责“把当前 invocation 的动态 requested outputs 传到 tail apply”，与
assignment-owned `ApplyPack` 是不同 consumer。

### 7.4 Arity barrier 与 ordered distribution

**[Proposal]** 新增等价于：

```rust
DistributePack {
    assignment: AssignmentPlanRegister,
    source: PackRegister,
}
```

共同且不受第 5.4 节 blocker 影响的 algorithm 是：

1. checked 求 assignment plan 总 requested count；
2. 若 `source.len < requested`，在任何 store 前返回 arity error；
3. 若 `source.len > requested`，只建立前 `requested` 个 values 的消费范围；
4. 一个 expanded target 或 nested path 作为一个 `Place`，完整预验证、旁路构造并一次
   store root。

对多个 independent targets，本文建议但尚未获 accepted clarification 的后半段是：

1. 按 targets 源码顺序处理：
   - `Discard` 消费 1；
   - `Binding` language-copy 1 个 value 后 store；
   - `Place` 重新读取 live root，消费该 token 的 runtime cardinality slice，完整预验证，
     旁路构造 updated root，成功后 store root 一次；
2. 后一 target 失败时保留此前成功 stores，不维护 statement-wide undo log；
3. 指令退出时释放未消费/trailing pack values 与 planning tokens。

一个 `DistributePack` 同时编码了共同 arity barrier 与顺序 commit，避免 compiler 用一串
unchecked dynamic `Unpack` 猜测边界。每一个 `Place` 分支内部复用 accepted
`AssignPlace` transaction helper；现有 simple assignment/deletion 也可逐步迁移到相同
planner/helper，避免两套 nested semantics。

**[Blocker]** 在 RFC 勘误/修订前，runtime/compiler 只可使用 target-count 1 的
`AssignmentPlan` 来完成 `[C{I}]`、`[S(I).f]` 与 nested path；不得启用上述 independent
multi-target 顺序 commit。若 accepted decision 选择 statement-wide atomicity，
`DistributePack` 的后半段必须改成 all-target validation、multi-root working set 与共同
publish/rollback，不能复用本节的顺序 commit。

### 7.5 为什么现有四类操作不够

| 现有能力 | 缺口 |
| --- | --- |
| `PackRegister` / `BraceApply` / field pack | 能表示实际 RHS/读取 pack，不能表示动态 LHS target count 与 producer requested outputs |
| `Call` / `Apply` | outputs 是 compile-time `Vec<Register>`；不能在 call 前使用 runtime LHS cardinality |
| `Unpack` | destination count 固定；不能把一个 runtime pack range 交给一个 dynamic place token |
| `AssignPlace` | 只有一个 root/path/source；没有 pre-RHS plan token、live-root rebase 或多个 ordered LHS term |
| `ResolveEnd` | 需要 ordinary target register；当前 place lowering 没有逐 step 即时 target |

## 8. Verifier invariants

除 RFC 0004 已有 value/pack invariants 外，**[Proposal]** verifier 至少冻结：

1. `PlaceRegister` 与 `AssignmentPlanRegister` 各自使用独立 checked bounds；不得与 value
   或 pack register index space 混用。
2. ordinary arithmetic、condition、workspace store、return、protocol value 与 aggregate
   storage 都不能引用 place/assignment-plan token；token 只能出现在 planning、dynamic
   producer 与 distribution instructions。
3. `BeginPlace.root` 必须是 valid ordinary register；`ExtendPlace.base/dst` 必须是 valid
   place registers；每个 `PlaceStep` 的 value/pack register、field constant/dynamic register
   全部验证。
4. `ResolvePlaceEnd.argument_count > 0` 且 `argument_index < argument_count`；它只能引用
   step 前的 place token。
5. static field operand 必须指向 string constant；dynamic field operand 必须是 ordinary
   register，不能是 pack/place/plan。
6. `BuildAssignmentPlan` 的 local/global binding operand 必须有效；global name constant
   必须是 string；`Discard` 固定 cardinality 1；empty-selection place 的 runtime
   cardinality 0 合法。
7. duplicate root、duplicate place offset 与同 root 多 term 不能由 register-bounds verifier
   擅自定义 language policy；blocker 未解除时 compiler 不 emit independent multi-target
   plans，解除后按 accepted decision 验证/执行。
8. `CallPack`/`ApplyPack` 的 destination 必须是 pack register，`requested_by` 必须是
   assignment-plan register；arguments 沿用现有 pack expansion 验证。
9. `DistributePack` 只接受 pack source 与 assignment plan；它不能把 plan 或未展开 pack
   写进 `Value`。
10. 所有 cardinality addition、pack slice endpoint、selection size 与 host allocation
    使用 checked arithmetic，并遵守 runtime cancellation/size limits。
11. bytecode version 仍精确匹配；不得让旧 runtime 通过 wildcard 忽略这些 variants。

**[Unknown]** 当前 v12.0 尚无可执行 aggregate runtime，也没有 serialized bytecode ABI。
若 coordinator 将本 completion 视为发布前对 RFC 所称“equivalent minimum boundary”的
补全，可继续标记 generation 12；若任何 12.0 module 已持久化或跨 build 交换，则新增 token
files/instructions 是 incompatible logical format change，必须升 major。不能在 crate 内私自
同时猜测两种格式。

## 9. Runtime frame 与 rollback ownership

**[Proposal]** frame 增加：

```text
registers: ordinary language Values
packs: ordered VM-only ValuePacks
places: ephemeral PlannedPlace tokens
assignment_plans: ephemeral ordered target plans
requested_outputs: existing per-call count
```

ownership 固定如下：

- planning phase：place token 拥有 evaluated selector metadata；不拥有可发布 root；
- RHS phase：producer/frame 拥有完整 output pack；失败时 pack 被丢弃；
- arity barrier：只读 pack 与 target cardinalities，不做 language store；
- term commit：当前 term 临时拥有 language-copied RHS slice 与 detached working root；
- term failure：只丢弃当前 working root/slice；
- term success：一次 store 把 working root 发布，之后该 store 不属于后续 term 的 rollback
  范围；
- aliases：成功 mutation 通过 COW detach 隔离 value aliases；handle-class identity 仍服从
  统一 `language_copy` policy。

任何实现都不能在 planning 时调用 object setter/subsasgn 或发布 aggregate detach。首 tranche
仍只接受 name/local-rooted aggregate place；temporary-expression lvalue 与 object overload
继续 deferred。

expanded target/nested path 的 planning、arity barrier、working-root failure/success
ownership 已由 accepted C3/`AssignPlace` contract 支持；“term success 不属于后续 rollback”
只适用于第 5.4 节建议的 R2022b-compatible
independent-target clarification。在 blocker 解除前，不实现 multi-target commit ownership。

## 10. Compiler lowering phases

**[Proposal]** 对 bracketed assignment 先分类：

1. 收集 ordered LHS terms，保留 `~`、ordinary binding、paren/brace/static/dynamic field
   chain，不按 root 合并；
2. 判断是否存在 runtime cardinality term；若不存在，保留既有 fixed-width RHS-first
   lowering；
3. 若存在 dynamic term，则对所有 LHS terms 按源码顺序 planning：
   - plain binding/`~` 只记录 fixed cardinality 1；
   - indexed term 先加载 planning root，`BeginPlace`；
   - 每个 postfix step 的 expressions 求值一次；nested `end` 对 base place 解析；
   - `ExtendPlace` 缓存 step，并在最终得到 cardinality；
4. `BuildAssignmentPlan` 冻结 ordered targets 与总 requested outputs；
5. RHS lowering：arguments 左到右；ordinary value 发 `PackOne`，brace/field RHS 使用既有
   pack，direct call 发 `CallPack`，unresolved paren apply 发 `ApplyPack`；
6. 发一个 `DistributePack`；单 target 时 runtime 做 arity barrier 与一次 root-local commit；
   multi-target 时只有在第 5.4 节 accepted blocker 解除后，才能选择顺序 commit 或新接受的
   statement-wide transaction；
7. 删除/growth 的单 place assignment 可先继续使用 `AssignmentMode`，但 nested `end`
   必须通过同一个 place planner，不能保留 root-bound shortcut；
8. `ParenApply` producer 始终 unresolved，不按 identifier spelling 特判。

需要新增稳定 compiler diagnostics：malformed LHS term、unsupported temporary root、
unsupported pack consumer、place-plan resource exhaustion；动态 type/shape/schema/arity/end
错误仍由 runtime 分类。

## 11. `aggregate_c3_transaction_rollback` 与 try/catch

### 11.1 当前依赖

**[Checkout]** parser 只把 `try`/`catch` token 识别为 unsupported statement；HIR
没有 try/catch statement variant，compiler/bytecode/runtime 也没有 exception region 或
handler stack。因此现有 `aggregate_c3_transaction_rollback.m` 无法到达 aggregate runtime。

**[Checkout]** 该 program 的两个 caught failures 分别是单独的
`cells(1:2) = {3,4,5}` 与 `records(1) = struct('b',9)`。每个 statement 都只有一个普通
paren assignment place；它们验证的是单 root cell shape mismatch 与单 root struct schema
mismatch 后 root/alias 不变。它没有覆盖：

- `[C{I}]` 或 `[S(I).f]` 这种一个 expanded bracket target；
- `[A{1},B(1)]` 或 `[C{1},C(2)]` 这种多个 independent LHS places；
- `C{1}.x(end)=...` 这种一个 place 内的 nested path。

因此它尤其**不能证明多 LHS statement-wide/global atomicity**，也不能用来消解第 5.4 节
blocker。

**[Inference]** 当前单次 conformance error observation 只能报告 failure category；执行在
error 处终止，无法在同一 program 中构造 `openmat_result`，所以不能观察 error 之后的
workspace、root 或 aliases。任何声称仅靠 `aggregate_c3_shape_error` 已证明 rollback 的
结论都是无证据的。

### 11.2 可独立验收的 aggregate 边界

**[Proposal]** 不把 try/catch 作为 aggregate runtime 的硬依赖，采用两层 gate：

1. differential gate 暂跑其余 9 个 aggregate programs；`aggregate_c3_shape_error` 只证明
   normalized error category，不声称证明 post-error state；
2. `openmat-runtime` host-side tests 直接执行 bytecode/interpreter API，在返回 `Err` 后检查
   workspace/root/alias，至少覆盖：
   - cell paren shape mismatch 不改变 root/alias；
   - struct schema mismatch 不改变 root/alias；
   - grouped cell/struct pack shortage 不发布任何 leaf/schema；
   - RHS producer failure不提交；
   - 同 root 多 term 后项失败的 R2022b-compatible 行为（仅在 RFC clarification 后成为
     OpenMat acceptance test）；
   - cross-root 后项失败的 R2022b-compatible 行为（同上）；
   - RHS 修改 root 后 planned selector 在 live root 上 revalidate/commit；
   - nested writeback failure不提前 detach outer aliases。

这允许 C3 aggregate semantics 在 try/catch language feature 之前独立验收，但现有
`aggregate_c3_transaction_rollback` 必须明确标为 deferred，不得伪造 differential pass。

如果 Milestone gate 要求“现有 10 cases 全部由相同 R2022b/OpenMat program differential
通过”，则 **[Proposal]** try/catch 必须前置到该 gate 之前，完整实现 syntax/HIR、exception
bytecode、runtime handler stack、caught-error workspace continuation；只在 runner 外捕获进程
error 不等价。

## 12. 现有 10 cases 的精确状态

下表的“current lowering”只表示当前 v12 compiler surface 能否表达关键 aggregate syntax，
不是 runtime pass。

| Case | Current v12 lowering | Completion 需求 |
| --- | --- | --- |
| `aggregate_c1_empty_cells` | 可编码 | runtime `BuildCell`/constructor/value semantics |
| `aggregate_c1_empty_structs` | 可编码 | runtime `struct`/shaped-empty semantics |
| `aggregate_c1_nested_structs` | 可编码 | runtime field pack/writeback |
| `aggregate_c1_scalar_access_write` | 可编码 | runtime brace/field/`AssignPlace` |
| `aggregate_c2_colon_end` | 可编码 | runtime `ResolveEnd`/aggregate indexing |
| `aggregate_c3_comma_assign` | **部分且语义不正确** | dynamic LHS planning、dynamic `nargout`、ordered distribution；不能固定请求 1 output |
| `aggregate_c3_delete_growth` | 可编码 | runtime delete/growth/default fill |
| `aggregate_c3_shape_error` | 可编码 | runtime error category 与 no-publish helper test |
| `aggregate_c3_transaction_rollback` | **不可编译完整 program** | try/catch，或按第 11 节 defer differential 并用 host-side rollback gate |
| `aggregate_c3_writeback_cow` | 可编码 | runtime nested transactional writeback/COW |

`aggregate_c3_comma_assign` 中两个写语句分别是单个 expanded bracket target；它没有多个
independent LHS terms，因此第 5.4 节 blocker 不妨碍先完成该 oracle case。

结论：

- **[Checkout]** compiler-level 关键 aggregate shape 为 8/10 可编码，1/10 部分错误，
  1/10 被 try/catch 阻断；
- **[Checkout]** runtime 当前不能编译，所以 end-to-end 为 0/10；
- **[Proposal]** 完成本设计与 aggregate runtime 后，9/10 可在 try/catch 之前进入
  differential gate，rollback 由 host-side test 先验收；全 10/10 需要 try/catch。

## 13. 分阶段精确任务边界

所有任务遵守 root `AGENTS.md`：只改分配路径，共享接口变化由 coordinator 先授权，每个
任务 focused commit 并独立验证。

### Task C3-A：bytecode completion surface

精确路径：

- `crates/openmat-bytecode/src/lib.rs`
- `crates/openmat-bytecode/src/model.rs`
- `crates/openmat-bytecode/src/verify.rs`

交付：`PlaceRegister`/assignment-plan token、stepwise planning、dynamic-output pack producer、
ordered distribution 的 logical variants 与第 8 节 verifier invariants；不实现 runtime，
不修改 accepted RFC/spec。

第 5.4 节 blocker 未解除时，contract/test 只接受 target-count 1 的 distribution semantics；
multi-target representation 可以预留，但不得冻结顺序或全局原子 commit 行为。

验证：`cargo test -p openmat-bytecode`，negative tests 覆盖四套 token bounds、invalid
field/step/end metadata、pack/token escape、invalid binding 与 exact version rejection。

### Task C3-B：compiler dynamic LHS lowering

依赖 C3-A。精确路径：

- `crates/openmat-compiler/src/lowering.rs`
- `crates/openmat-compiler/src/diagnostic.rs`
- `crates/openmat-compiler/tests/compiler.rs`
- 只有导出确有需要时才改 `crates/openmat-compiler/src/lib.rs`

交付：第 10 节 lowering；先移除 `NestedPlaceEnd` 对已支持 nested aggregate forms 的
blanket rejection，并正确 lower target-count 1 的 `[C{I}]`/`[S(I).f]` dynamic call；
`MultipleAggregateAssignmentTargets` 在第 5.4 节 blocker 解除前继续作为稳定 unsupported。
保留 temporary root/object overload 的精确 unsupported；不按 `deal`/`cell`/`struct`
拼写特判。

验证 snapshots/instruction assertions 覆盖：

- `[C{I}] = producer(args)` 与 `[S(J).(f)] = producer(args)` 的
  planning/call/distribution 顺序；
- empty/short/extra RHS pack；
- blocker 未解除时 same-root/cross-root independent targets 仍得到精确 unsupported；
- `C{end}.x(end,end-1)` 每个 `end` 引用正确 base place；
- pure fixed-width assignment 仍保留既有 RHS-first selector 顺序；
- current 10 programs 的 lowering classification 与第 12 节一致。

### Task C3-C：runtime frame、pack producer 与 place planner

依赖 C3-A；可与 C3-B 部分并行。精确路径：

- `crates/openmat-runtime/src/interpreter.rs`
- `crates/openmat-runtime/src/lib.rs`
- `crates/openmat-runtime/src/error.rs`
- `crates/openmat-runtime/src/aggregate_ops.rs`（新）
- 若 frame 在独立文件定义，则只增加该精确 frame 文件
- `crates/openmat-runtime/tests/interpreter.rs`
- `crates/openmat-runtime/tests/array_bytecode.rs`

交付：pack/place/assignment-plan frame files、stepwise focus、dynamic requested-output call、
arity barrier、live-root reload、单 expanded target/nested path transaction、COW rollback
ownership。Windows 行为不得扩散出既有 platform boundary。

验证：第 4、5、6、9 节不受 blocker 影响的 probe-derived cases，以及
cancellation/checked allocation；independent multi-target observations 只保留为 C3-M 的
compatibility fixtures。运行 `cargo test -p openmat-runtime` 和该 crate strict clippy。

### Task C3-M：independent multi-LHS semantics（blocked）

前置条件：coordinator 已接受第 5.4 节 RFC 勘误/修订。随后才可在 C3-A/B/C 的精确路径
内增加或启用 multi-target `AssignmentPlan`、compiler lowering 与 runtime commit policy。

若接受 R2022b-compatible clarification，验收 same-root、cross-root、ordinary/aggregate
mixed、global arity barrier、duplicate root/offset 与后项 failure persistence。若接受更强的
statement-wide atomicity，则必须先另行冻结 multi-root working set 与共同 publish/rollback
contract；不得复用顺序提交 tests。

### Task C3-D：aggregate operations 与 builtins

依赖 C3-C。精确路径由 coordinator 在 runtime owner 中从下列范围拆分：

- `crates/openmat-runtime/src/aggregate_ops.rs`
- `crates/openmat-runtime/src/builtin.rs`
- 对应 runtime test files

交付：cell/struct indexing、field packs、delete/growth/default fill、schema alignment 与
language-copy callback；`deal` 只作为普通 builtin 读取 requested outputs，不与 assignment
lowering耦合。

### Task C3-E：conformance integration without try/catch

依赖 C3-B/C/D。精确路径：

- `tests/conformance/cases/programs/aggregate_*.m` 只读，除非 coordinator 明确授权更新；
- `tests/conformance/cases/manifests/aggregate_*.json` 只读；
- `tests/conformance/reference/matlab-r2022b/aggregate_*.json` 只读；
- runtime tests 仅写 C3-C 指定 test paths；
- conformance runner 的 skip/expected-set 改动如有需要，必须另行分配精确路径。

交付：其余 9 cases 的 differential gate 与 rollback host-side gate 分开报告，绝不把 deferred
case 写成 pass。

### Task TRY-1：try/catch 前置（仅当要求 10/10）

这是独立 language/runtime feature，不应塞入 aggregate crate 修改。需要 coordinator 先定义
exception bytecode contract，再分别分配：

- `crates/openmat-syntax/src/lib.rs`
- `crates/openmat-parser/src/lib.rs`
- `crates/openmat-hir/src/lib.rs`
- `crates/openmat-bytecode/src/{lib.rs,model.rs,verify.rs}`
- `crates/openmat-compiler/src/{diagnostic.rs,lowering.rs}` 与 compiler tests
- `crates/openmat-runtime/src/{interpreter.rs,error.rs}` 与 runtime tests

完成 caught-error continuation 后再启用
`aggregate_c3_transaction_rollback` differential case。

## 14. 尚未知项与不应外推的范围

1. **[Unknown]** RHS 把 planned root 缩短、改 type、改 schema 后，各类 cached selector 的
   R2022b 精确 failure category；本提案统一在 term commit 对 live root revalidate；是否
   保留 independent 已提交前项服从第 5.4 节 accepted decision。
2. **[Unknown]** object `subsasgn` overload、property setter、value-class copy hook 在多 LHS
   failure 中的完整 side-effect/rollback 顺序；首 aggregate tranche 不接受这些 temporary/
   object-overload places。
3. **[Unknown]** 多层 chain 的非 scalar intermediate comma list 后继续 postfix selection 的
   所有 MATLAB forms；planner 首版应只接受能证明 intermediate focus 唯一或最终 step
   明确定义 pack cardinality 的 forms。
4. **[Unknown]** v12.0 label 是否仍可用于发布前 completion，见第 8 节；semantic design 不因
   version label 选择而改变。
5. **[Unknown]** try/catch 的 exception object、identifier/message exposure 不属于本文；
   OpenMat 仍只使用自有 structured error categories，不复制 MATLAB messages。
6. **[Contract]** call-vs-index unresolved、one-based boundary、column-major order、COW、
   stable C ABI/versioned serialized protocol 与 no-Rust-ABI 原则保持不变。

## 15. Completion acceptance checklist

- [ ] LHS planning/RHS argument/producer/commit phase tests与第 4 节一致；
- [ ] dynamic producer 在 0、N outputs 下获得正确 requested count；
- [ ] short pack 全 statement 零 commit，extra pack trailing ignored；
- [ ] expanded bracket target、independent LHS places、nested path 三类边界在测试名与报告中
  明确区分；
- [ ] 每个 expanded place 与完整 nested path 一次 root publication；
- [ ] independent same-root/cross-root semantics 等待第 5.4 节 accepted blocker 解除；若接受
  R2022b-compatible 方案，再验收顺序提交且不做后项 rollback；
- [ ] RHS root mutation 后从 live binding revalidate，不覆盖无关 RHS side effects；
- [ ] nested `end` 使用即时 place focus，selector expression 只求值一次；
- [ ] deletion/growth 使用 mutation 前缓存的 `end`；
- [ ] verifier 阻止 value/pack/place/assignment-plan 四类 token escape；
- [ ] `ApplyPack` 保持 unresolved，不对 `deal`/`cell`/`struct` 拼写特判；
- [ ] 8/10 lowering-ready、9/10 pre-try differential、10/10 post-try 三种状态分开报告；
- [ ] runtime rollback unit tests在 `Err` 后直接验证 root 与 aliases；
- [ ] 不把 `aggregate_c3_transaction_rollback` 的单 paren assignment failures 当成 multi-LHS
  global atomicity 证据；
- [ ] 不修改 accepted RFC/spec，不声明 Rust ABI。
