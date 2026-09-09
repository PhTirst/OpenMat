# MATLAB R2022b `classdef` lifecycle black-box contract

This note records clean-room observations made with OpenMat-authored programs
against the locally installed MATLAB R2022b executable. It is a compatibility
target for a future OpenMat finalizer design. It does not implement or prescribe
a garbage collector.

The repository baseline was
`pre-publication baseline`. The probes ran through the existing
bounded MATLAB oracle and observation-v2 schema. They save only OpenMat-owned
numeric event traces and booleans. No MATLAB source, tests, documentation,
identifier, diagnostic text, or warning text is copied into the corpus.

## Observation encoding

All handle probes use `OpenMatGcLifecycleHandle`. Each case has an independent
numeric case ID. The global event store contains `(case ID, event code)` rows,
and a case serializes only rows carrying its own ID. This prevents a delayed
finalizer from contaminating another case's result.

The shared event codes are:

- `100 + object ID`: constructor body reached the observation point;
- `200 + object ID`: `delete` entered for that object;
- `300 + 10 * isvalid(object) + object ID`: validity observed inside `delete`;
- `400 + 10 * isvalid(rescued_alias) + object ID`: validity immediately after
  `delete` stores `self` in the probe's global rescue root;
- `500 + numel(delete_argument)`: callback argument size, enabled only by the
  handle-array case;
- `1` through `99`: case-local phase markers described by the program; and
- `90`: separator before case-local boolean results.

Object IDs are one through three, so these ranges do not overlap. A code such
as `301` therefore means that object 1 was already invalid inside its own
`delete` method. Stored properties remain readable by that method in the
sampled callback, which is how it obtains its case and object IDs.

## Recorded cases

| Case | R2022b observation locked by the reference |
| --- | --- |
| `gc_lifecycle_acyclic_overwrite_clear` | Constructing the replacement completes before the overwritten object is finalized. The old object is finalized before the next statement. Named `clear` finalizes the replacement before the following statement. |
| `gc_lifecycle_function_return` | An unreturned acyclic local is finalized during callee return and before the caller resumes. A returned handle remains live at callee return and is finalized when the caller clears its root. |
| `gc_lifecycle_script_scope` | A script shares the invoking function workspace: the object is still live immediately after the script finishes and is finalized only when that function returns. |
| `gc_lifecycle_cycle_unrooted` | A two-node graph with only the mutual `Peer` references remaining is reclaimed on function return. Construction is `1, 2`; finalization is `2, 1` for this exact graph. |
| `gc_lifecycle_cycle_root_survives` | Clearing one local name does not collect a mutual cycle while the other local remains a root; both nodes report valid. After the probe breaks the cycle and clears the remaining root, nodes finalize `2, 1`. |
| `gc_lifecycle_explicit_delete_alias` | Explicit `delete` runs the destructor once, invalidates both aliases, and a repeated `delete` has no second destructor effect. |
| `gc_lifecycle_no_resurrection` | Assigning `self` to a global from `delete` leaves a nonempty but invalid alias. Property access through that alias fails; the object cannot be resurrected. |
| `gc_lifecycle_handle_array` | Explicit deletion of the sampled `1 x 3` handle array invokes three scalar `delete` callbacks in linear order `1, 2, 3`; every element is invalid afterward. |
| `gc_lifecycle_constructor_failure` | A constructor that fails after setting identity and logging construction still runs `delete` for the partial handle before control enters the surrounding `catch`. |
| `gc_lifecycle_value_delete_method` | Clearing a value-class instance does not call its method named `delete`. Calling `delete(value)` invokes that ordinary method, and a later clear has no finalizer effect. |
| `gc_lifecycle_error_unwind` | An acyclic function local is finalized while its frame unwinds, before the caller's `catch` runs. |
| `gc_lifecycle_destructor_error_explicit` | A destructor-thrown OpenMat probe error during explicit `delete` does not transfer control to the surrounding `catch`. Execution continues, warning presence is observable, and the handle is invalid. |
| `gc_lifecycle_destructor_error_automatic` | The same destructor-thrown probe error during automatic destruction on `clear` does not transfer control to the surrounding `catch`. The clear completes, execution continues, and warning presence is observable. |

Every checked-in trace is in
`tests/conformance/reference/matlab-r2022b/gc_lifecycle_*.json`; each manifest's
expected value is the same complete trace.

## Finalizer contract determined by the probes

For the behavior sampled here, a compatible OpenMat implementation should use
the following language-level contract:

1. Acyclic handles finalize synchronously at the operation that removes their
   last root: overwrite, named clear, function-frame unwind, or error unwind.
   The destructor completes before the next observable statement.
2. Returning a handle transfers a root to the caller. Merely ending the callee
   does not finalize that returned object.
3. Script completion is not a separate lifetime boundary because the script
   uses its caller's workspace. The owning workspace boundary controls the
   lifetime.
4. The exact two-node unreachable cycle sampled here must be collectible even
   though the nodes retain each other. An external root to either node keeps the
   whole reachable cycle live. The observed `2, 1` order is contractual for this
   construction and edge pattern, but the probe does not justify a universal
   ordering rule for arbitrary object graphs.
5. A handle is already invalid when its `delete` method runs. All aliases become
   invalid, repeated deletion has no second finalizer effect, and storing `self`
   during `delete` cannot restore validity.
6. Explicit deletion of the sampled handle row array finalizes each element in
   column-major linear order as three scalar callbacks. Array deletion is not
   one aggregate callback in the observable trace.
7. A partially initialized handle whose constructor throws is still finalized
   before the constructor error reaches the caller.
8. A value class does not acquire finalizer semantics merely by declaring a
   method named `delete`; that method is invoked only as an ordinary explicit
   method call.
9. An error thrown from the sampled handle destructor is contained by the host:
   it does not enter a user `catch` surrounding either explicit deletion or
   automatic clear, deletion remains effective, execution continues, and a
   warning is recorded. Exact warning identifiers and text are deliberately
   outside this contract.

These are observable language rules, not an implementation mandate. Reference
counting, tracing, cycle detection, arenas, or another ownership model could
satisfy them. In particular, this work does not authorize implementing GC.

## End-of-command, `clear classes`, and shutdown boundary

The statement-level probes distinguish command completion from root removal. A
constructor assignment followed by another command does not finalize the newly
rooted object. Overwrite and `clear` do, at the points recorded above.

The current oracle normalizes `openmat_result` before its per-case function
returns. It therefore cannot serialize a finalizer that runs only after that
outer case frame or the MATLAB process has ended. A cross-case file protocol
would violate the state-independent case contract, so MATLAB process exit and
the proposed OpenMat Kernel-close boundary are not locked by this corpus.

An exploratory `clear classes` case was also rejected from the corpus. Running
that command inside the active oracle case invalidated the harness's own
serialization boundary, so it did not produce a schema-valid, independent
observation. There is consequently no `clear classes` or Kernel-close
finalizer contract in this note. A future probe needs a controller-owned side
effect channel that is read only after the child process exits and is reset for
every case.

## Reproduction and current OpenMat status

The focused reference command is:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath $env:MATLAB_EXE `
    -Tag gc-lifecycle-r2022b `
    -TimeoutSeconds 300 `
    -ResultDirectory tests/conformance/reference/matlab-r2022b
```

The focused OpenMat command is:

```powershell
pwsh -NoProfile -File tools/openmat-conformance/Invoke-OpenMatConformance.ps1 `
    -Case 'gc_lifecycle_*' `
    -JsonSummary
```

At the stated baseline plus these probe files, the MATLAB oracle passes all
13 cases. The OpenMat runner reports 13 failed, zero unsupported, and zero
internal; every OpenMat observation is the normalized category
`syntax-error`. The failure is therefore before finalizer execution, at the
current compile/accepted-syntax boundary for the added lifecycle support. It
must not be reported as an implemented GC or as a runtime finalizer mismatch.

The corpus is intentionally narrow. It does not determine arbitrary graph
finalization order, inheritance interactions between multiple destructors,
weak references, persistent/static roots, process-exit reliability, Kernel
shutdown, `clear classes`, or exact warning diagnostics.
