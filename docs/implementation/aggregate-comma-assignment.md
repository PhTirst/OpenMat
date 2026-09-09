# Aggregate comma-assignment compiler lowering

This note records the compiler-only completion available on integration commit
`90341d4`, whose bytecode generation is `13.0`. It does not amend RFC 0004 or
the candidate C3 design.

## Version-13 lowering

A bracketed LHS containing exactly one aggregate place is distinguished from a
direct aggregate assignment. For example, `[cells{[1, 4]}]` is an expanded
comma-list target, while `cells{[1, 4]}` remains the simple brace-assignment
form and requests one RHS value.

When every final selector has a statically known positive cardinality, the
compiler now lowers an RHS application as follows:

1. Load the name-rooted aggregate and evaluate each encoded selector once.
2. Preserve the complete ordered `PlaceStep` path.
3. Lower the unresolved RHS application once, requesting the static place
   cardinality through the ordinary version-13 `Apply.outputs` vector. The
   compiler does not inspect the callee spelling, so `deal`, a user function,
   and any other unresolved callable use the same path.
4. For more than one result, preserve output-register order by constructing one
   temporary `1 x N` cell and brace-expanding it with `:` into a pack register.
5. Pass that pack to one transactional `AssignPlace`. Store the returned root
   only after `AssignPlace` succeeds.
6. Emit automatic named display only when the HIR statement does not suppress
   output. A semicolon therefore affects display but not selector, RHS, place,
   or root-store lowering.

The static selector subset is intentionally conservative: scalar numeric or
`end` indices and rectangular matrix literals made from those scalar indices.
Multi-subscript cardinalities use checked products. The original matrix rows
remain in bytecode, so runtime index resolution still determines the selected
places in MATLAB column-major order. RHS output registers are repacked in their
original order.

Ordinary fixed-width forms such as `[a, ~, b] = f()` retain the pre-existing
RHS-first `Apply(outputs = 3)` lowering. `~` consumes one requested output but
does not create a store or display instruction. Multiple independent aggregate
LHS terms remain rejected by `MultipleAggregateAssignmentTargets`.

## Explicit version-13 boundary

Version 13 cannot request a runtime-sized number of call outputs. A bracketed
aggregate call whose cardinality depends on a value such as `indices`, `:`, a
range, or the runtime size of a direct struct field now receives the structured
`AggregateAssignmentOutputArity` unsupported diagnostic. This replaces the
previous semantically incorrect fallback that always requested one output.
Pack-valued RHS expressions remain representable because they already produce
a `PackRegister` and do not need a requested call arity.

The register-to-pack remap is sufficient for the accepted numeric and character
conformance case, but it is not a general dynamic-C3 interface. In particular:

- selector validation still occurs in `AssignPlace`, after the RHS call;
- `AssignPlace.root` is the root value loaded before the RHS and cannot rebase
  on an RHS mutation of the same binding;
- the temporary cell and brace expansion cross additional `language_copy`
  boundaries, which may be observable for value-class copy hooks;
- nested-place `end`, empty dynamic selections, and runtime cardinalities still
  need stepwise planning.

A general fix therefore requires a shared bytecode revision with, at minimum,
independent checked `PlaceRegister` and `AssignmentPlanRegister` files plus
equivalent operands for:

```text
BeginPlace { dst_place, root }
ExtendPlace { dst_place, base_place, step }
ResolvePlaceEnd { dst, place, argument_index, argument_count }
BuildAssignmentPlan { dst_plan, targets }
ApplyPack { dst_pack, target, arguments, requested_by }
DistributePack { assignment, source }
```

`targets` must distinguish discard, local/global binding, and one name-rooted
planned place. `ApplyPack.requested_by` obtains the checked total cardinality
without spelling-based builtin handling. `DistributePack` owns the global arity
barrier and a single expanded-place transaction. Enabling multiple independent
aggregate targets still depends on the accepted-contract clarification recorded
in `docs/design/cell-struct-c3-completion.md`.

## Verification

Compiler tests exercise the exact HIR shapes for:

```matlab
[cells{[1, 4]}] = deal(101, 404);
[records([1, 3]).alpha] = deal('left', 'right');
```

They assert requested output arity, complete place paths, output-register to
pack-register mapping, root-store ordering, column-major selector preservation,
ordinary multi-assignment with `~`, and semicolon/display behavior. The existing
`aggregate_c3_comma_assign` conformance program also passes through the real
parser, CST-to-HIR lowering, compiler, verifier, runtime, kernel-v2 inspection,
and schema-v2 CLI producer.
