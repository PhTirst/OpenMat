# RFC 0020: common block authoring and parameter compatibility

Status: implementation contract for the user-approved common block editor milestone.

Scope: `simulation/`, `apps/web/src/simulation/`, simulation server routes/tests,
and milestone documentation. Existing accepted RFCs, language specifications,
root manifests and numerical execution ABI remain unchanged.

The numerical model remains schema 1 through 8. Authoring document schema 9
adds optional model-scoped `parameters`: `{source, bindings}`. Source is bounded
assignment-only m text; bindings map numerical block IDs to canonical block
parameter names and expression strings. The saved numerical model contains the
last resolved values for inspection, but bindings are authoritative: server and
CLI reevaluate them before checking/running. Failed resolution cannot run a stale
cached model. Unbound values retain their existing semantics. Model parameters
are independent of the interactive m workspace and never invoke arbitrary code.

`/simulation/v9` adds `resolveParameters` and an optional `parameters` field to
check/run requests, using the existing bounded native parameter evaluator.
Earlier routes reject these operations/fields. Resolution is atomic and validates
target block/parameter names, shape, finite values and resource bounds. It may
resolve an incomplete editing graph; compilation still validates connectivity.

A shared, versioned common-block descriptor catalog defines canonical names,
independently authored labels, defaults, groups, editors and supported options.
Both native parameter forms and SLX property presentation use it. Unsupported
options are explicitly identified and never silently applied. This milestone
does not introduce matrix-valued signals, fixed-point arithmetic, matrix Gain,
arbitrary workspace scripts, masks, or SLX writeback. R2022b is the compatibility
baseline; metadata and numerical tests are independently authored.

Conditional Enable/Trigger controls are editable internal diagram elements,
projected from the single authoritative parent execution descriptor. Optional
schema-9 editor control positions persist their layout. Changing or deleting a
control edits parent execution; copying a subsystem copies its control layout.
Outport owns the user-facing initial-output/disabled-output form, mapped by port
number to that same execution descriptor. No duplicate state policy is stored.
Ordinary virtual nesting and existing conditional runtime restrictions persist.

Common computational blocks receive compact, individually drawn mathematical
symbols, parameter-aware content, external names and orientation-aware ports.
The existing React editor, colors, numerical backend and solver choices remain.
The sidebar and double-click parameter dialog share one form implementation;
subsystem double-click keeps navigation behavior. Import source properties remain
preserved and distinguish explicit values from descriptor defaults.

Validation covers expression resolution/rejection/stale-cache protection,
source retention, undo/copy/delete/save/reopen, condition-control editing and
Outport association, parameter grouping and unsupported options, real browser
and server execution, and independently authored R2022b comparisons. Generated
oracles, caches, screenshots and binaries remain ignored local artifacts.
