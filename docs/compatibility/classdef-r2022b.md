# MATLAB R2022b `classdef` compatibility baseline

This matrix audits the release-one contract in
`spec/language/classdef-core.md` against clean-room conformance programs. The
MATLAB side was recorded by treating MATLAB R2022b as a black box. The programs,
manifests, and normalized observations are OpenMat-authored; no MATLAB source,
tests, documentation, diagnostic identifiers, or diagnostic text are retained.

## Snapshot and status vocabulary

The OpenMat measurements below were made on 2026-08-24 at implementation
baseline `pre-publication baseline`.

- **R2022b recorded** means that the checked-in MATLAB black-box observation
  matches the case manifest. The 15 classdef references are part of the 44
  checked-in R2022b observations.
- **OpenMat passed** means that the OpenMat runner executed the case, produced an
  OpenMat observation, and matched it against both the manifest expectation and
  the checked-in R2022b reference.

At this baseline the OpenMat classdef selection passes 15 of 15 cases: 15
passed, 0 failed, 0 unsupported, and 0 internal errors. The complete current
corpus independently passes 44 of 44 through the same runner.

The MATLAB references were not regenerated for this OpenMat measurement. Their
black-box recording procedure is documented in `tests/conformance/README.md`;
the checked-in references are oracle evidence, while the counts above are
current OpenMat implementation results. Neither finite result is a claim of
complete MATLAB R2022b compatibility or of exhaustive coverage within the
sampled classdef features.

## Contract coverage matrix

| Release-one contract item | Conformance cases | MATLAB R2022b baseline | OpenMat at `bd23229` | Coverage boundary |
| --- | --- | --- | --- | --- |
| Class definitions in class files | All 15 classdef cases use `cases/support` class files | Recorded | Passed | Exercises the resolver, parser, compiler, runtime, object layer, and runner end to end for the sampled files; it does not cover every legal class-file form. |
| Value classes | `value_class_copy`, `class_argument_semantics`, `class_homogeneous_array`, `class_plus_overload` | Recorded | Passed | Covers scalar copying, callee isolation, a row object array, and an overloaded operation. |
| Handle-semantics classes derived from `handle` | `handle_class_alias`, `class_argument_semantics` | Recorded | Passed | Covers direct `handle` inheritance, alias assignment, and mutation through a function argument. Indirect handle inheritance through a user superclass is not sampled. |
| Constructors | `class_constructor`, `class_inheritance_override` | Recorded | Passed | Covers property initialization and explicit superclass-constructor invocation. |
| Instance methods | `value_class_copy`, `handle_class_alias`, `class_public_access`, `class_inheritance_override` | Recorded | Passed | Covers value-returning mutation, in-place handle mutation, ordinary calls, and dispatch from an inherited method. |
| Static methods | `class_static_constant` | Recorded | Passed | The static call is made without constructing an instance. |
| Properties with default values | `class_property_defaults`, `class_access_controls` | Recorded | Passed | Covers scalar and row-vector defaults plus defaults at all three access levels. |
| Public get/set and method access | `class_public_access`, `class_access_controls` | Recorded | Passed | External property get/set and public method calls are observed. |
| Protected get/set and method access | `class_access_controls`, `class_protected_access_error` | Recorded, including normalized `access-violation` | Passed | A subclass gets and sets a protected property and calls a protected method; external property access is rejected. |
| Private get/set and method access | `class_public_access`, `class_private_access_error`, `class_access_controls` | Recorded, including normalized `access-violation` | Passed | Internal private property get/set and private method access are observed; external private property access is rejected. External private-method rejection is not a separate case. |
| Constant properties | `class_static_constant` | Recorded | Passed | Covers class-qualified read and use by a static method. |
| Dependent properties and conventional get/set accessors | `class_dependent_property` | Recorded | Passed | Both getter and setter behavior are observed before and after assignment. |
| One user-defined superclass | `class_inheritance_override`, `class_access_controls`, `class_reflection_predicates` | Recorded | Passed | Covers explicit base construction, inherited access, and `isa` across one superclass. |
| Ordinary dispatch and overriding | `class_inheritance_override` | Recorded | Passed | A base method calls `object.score()`, which dispatches to the derived override. |
| Basic arithmetic operator overload | `class_plus_overload` | Recorded | Passed | Covers same-class binary `plus` dispatch returning a numeric result. No comparison overload is sampled. |
| Basic homogeneous object arrays | `class_homogeneous_array` | Recorded | Passed | Records an opaque `1x2` value-class array with exact class, shape, `ndims`, and `numel`. Handle arrays, empty arrays, and multidimensional object arrays are not sampled. |
| `class`, `isa`, and `isobject` | `class_reflection_predicates` | Recorded | Passed | Checks exact class name, own class, user superclass, unrelated `handle`, object input, and numeric non-object input. |
| Value/handle assignment semantics | `value_class_copy`, `handle_class_alias` | Recorded | Passed | The value copy remains unchanged; both handle aliases observe mutation. |
| Value/handle argument-passing semantics | `class_argument_semantics` | Recorded | Passed | A callee mutation is isolated for a value object and visible for a handle object. |
| Unsupported attributes produce structured diagnostics | No dedicated differential case | Not recorded by this matrix | Not claimed | Existing parser tests are outside this corpus. This diagnostic contract needs a separate OpenMat-owned diagnostic corpus. |

## Added orthogonal observations

The six additions fill gaps without replacing the nine earlier classdef cases:

| Case | Stable observation |
| --- | --- |
| `class_access_controls` | `double` row `[10, 20, 40, 3, 6]`, proving valid public, protected, and private scopes |
| `class_protected_access_error` | Error category `access-violation`; no diagnostic identifier or message is retained |
| `class_property_defaults` | `double` row `[7, 2, 4]` from an implicit default constructor |
| `class_reflection_predicates` | Logical row `[true, true, true, false, true, false]` |
| `class_homogeneous_array` | Opaque object observation: class `OpenMatValueCounter`, size `1x2`, `ndims = 2`, `numel = 2` |
| `class_plus_overload` | Scalar `double` value `12` from same-class `plus` dispatch |

The nine retained cases remain responsible for constructor initialization,
public/private access, value and handle copying, function-argument semantics,
single inheritance and override dispatch, static/constant behavior, and
dependent get/set behavior.

## Explicit exclusions

This baseline intentionally does not turn the release-one exclusions into
execution requirements: multiple inheritance; enumeration classes; events and
listeners; dynamic properties and `dynamicprops`; complete `meta.*` reflection;
`matlab.mixin.*`; heterogeneous object arrays; full property type/size
validation; user-defined serialization; complete custom display; and complete
custom indexing through `subsref`, `subsasgn`, `subsindex`, or `end`.

Also not claimed by the current baseline are comparison-operator overloads,
indirect handle semantics through a user-defined superclass, handle-class object
arrays, empty or multidimensional object arrays, every `GetAccess`/`SetAccess`
combination, or separate negative cases for protected/private method calls.

## Reproduction

From the repository root, record or verify the MATLAB observations with a local
licensed R2022b executable:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath '<MATLAB-R2022b>\bin\matlab.exe' `
    -Tag classdef `
    -TimeoutSeconds 300 `
    -ResultDirectory tests/conformance/reference/matlab-r2022b
```

MATLAB is never invoked by CI. Recording is a deliberate local compatibility
operation; the workflow does not download or commit reference observations or
other generated output.

Reproduce the OpenMat classdef result without rebuilding inside the runner:

```powershell
cargo build --locked -p openmat-cli
pwsh -NoProfile -File tools/openmat-conformance/Invoke-OpenMatConformance.ps1 `
    -OpenMatPath .\target\debug\openmat-cli.exe `
    -Case '*class*' `
    -TimeoutSeconds 60 `
    -JsonSummary
```

The expected baseline summary is 15 passed with zero failed, unsupported, or
internal results, and exit code `0`.
