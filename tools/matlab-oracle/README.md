# MATLAB R2022b oracle harness

This Windows-only harness invokes a user-supplied MATLAB executable in batch
mode and executes only the OpenMat-authored conformance programs under
`tests/conformance`. It never reads or modifies MATLAB installation files, and
it does not retain MATLAB stdout, stderr, source, tests, documentation, or full
diagnostic messages.

## Requirements and usage

- PowerShell 7 or newer (`pwsh`)
- A licensed MATLAB R2022b installation

Set `MATLAB_EXE` to the full path of that installation's `bin/matlab.exe`, or
pass the path explicitly with `-MatlabPath`. No installation path is assumed.

From the repository root:

```powershell
pwsh -NoProfile -File tools/matlab-oracle/Invoke-MatlabOracle.ps1 `
    -MatlabPath $env:MATLAB_EXE `
    -Tag smoke `
    -TimeoutSeconds 120 `
    -ResultDirectory tools/matlab-oracle/results
```

Omit `-Tag` to run every case. Multiple tags use inclusive (any-tag)
selection. `-List` prints the selection without starting MATLAB.

The runner copies the project-owned case directory and wrapper into a unique
temporary directory, uses .NET's argument list API so paths with spaces are
passed without shell re-parsing, and launches `matlab.exe -batch`. The timeout
applies to the complete selected batch. Temporary files are removed in a
`finally` block by default; `-KeepTemporary` preserves that unique directory
for harness debugging. Neither success nor failure cleans or deletes the case
tree or the requested result directory. Only selected `<case-id>.json` files
and `run-summary.json` are written there.

Exit codes are stable for automation:

- `0`: all selected cases matched their manifests
- `2`: one or more normalized observations mismatched
- `3`: harness setup or launch failure
- `4`: MATLAB wrapper failure or missing output
- `124`: timeout (the spawned process tree is terminated)

## Data boundary

`openmat_oracle_run.m` serializes observable values into the project schema:
class, shape, `ndims`, `numel`, and a typed column-major payload. Floating real
and imaginary parts use invariant precision-preserving strings; logicals,
strings, missing markers, character code units, and integers have distinct
fields. For schema version 2, cells are traversed by column-major item order and
structs by column-major record order followed by their ordered `fieldnames`.
Every nested value is normalized recursively; display text is never consulted
or parsed. Object values, function handles, invalid first-tranche field names,
and other unsupported leaves are rejected instead of being serialized as
exact values.

Schema-v2 normalization applies the accepted complete-tree ceilings before a
successful observation is written:

- at most 4,096 elements in every exact node;
- at most 16,384 value nodes and 65,536 aggregate elements;
- root depth zero and maximum depth 32;
- at most 16,384 UTF-16 code units in one string element and 65,536 across
  char data, non-missing string data, and each struct field schema;
- at most 1,048,576 UTF-8 bytes for the complete pretty-printed observation,
  including its trailing newline and outer oracle envelope.

Nodes and elements are charged depth-first in pre-order. A struct field name is
charged once per struct node, not once per record, and missing strings charge
zero code units. Counts use checked bounded additions, and a failure discards
the partial value. Limit and unsupported failures become the OpenMat-owned
normalization categories `payload-limit` and `unsupported-payload`;
`cyclic-payload` and `invalid-observation` are reserved for the other accepted
producer failures. These diagnostic envelopes are intentionally not valid
reference truth under the current case/observation schemas. The PowerShell
harness validates their narrow shape, saves them for diagnosis, marks the case
failed, and never compares or promotes them as a MATLAB language error.

MATLAB language errors are separately mapped from identifiers in memory to
short OpenMat-owned categories, and neither the identifier nor message is
saved.

The runner compares each observation with the `expected` object in its case
manifest. A future OpenMat runner can execute the same source contract, emit
the same observation schema, and reuse the comparison policy documented in
`tests/conformance/README.md`.
