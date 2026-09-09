# MATLAB R2022b table and delimited-text boundary

Status: clean-room design and first-stage implementation contract. This note is
not an accepted RFC and does not override `spec/` or accepted files under
`docs/rfcs/`.

The observations in this note target repository baseline
`pre-publication baseline`. They were measured on 2026-08-29
with the locally installed MATLAB R2022b executable at
`$env:MATLAB_EXE`.

## 1. Evidence boundary and method

All probe programs and input files were authored for OpenMat and placed under
`%TEMP%\openmat-table-r2022b-probes`. The probes retained only labels, success
or rejection, class names, shapes, variable names, normalized values, and
OpenMat-authored delimited-file contents. No MATLAB source, tests,
documentation, or diagnostic prose is copied into this repository. Octave was
not used as an oracle.

The three focused commands were:

```powershell
& $env:MATLAB_EXE -batch `
    "cd('C:/work/openmat-table-r2022b-probes'); probe_table_core"

& $env:MATLAB_EXE -batch `
    "cd('C:/work/openmat-table-r2022b-probes'); probe_table_io"

& $env:MATLAB_EXE -batch `
    "cd('C:/work/openmat-table-r2022b-probes'); probe_table_edges"
```

The word **measured** below means that one of those probes observed the stated
R2022b result. A **phase-one decision** is an OpenMat implementation boundary
chosen from the measured cases. Anything listed as **deferred** or
**unmeasured** must not be advertised as compatible merely because a generic
container or CSV dependency happens to accept it.

## 2. First-stage public surface

The first executable tranche should expose:

- the `table` value class and constructor;
- `class`, `size`, `height`, `width`, and `istable` for tables;
- variable reads through dot indexing, table selection through parentheses,
  and contents extraction through braces;
- whole-variable dot assignment, variable deletion, and
  `Properties.VariableNames` reads and replacement;
- `readtable` and `writetable` for CSV and delimited text;
- the high-frequency options `Delimiter`, `ReadVariableNames`,
  `WriteVariableNames`, `VariableNamingRule`, and `FileType='text'` where this
  note gives a measured boundary.

`VariableNames` belongs to the `table` constructor and the public
`Properties.VariableNames` property. The measured R2022b `readtable` and
`writetable` calls rejected a direct `VariableNames` name-value argument. An
import-options object can carry replacement names, but a general
`detectImportOptions` object model is not required for the first tranche. A
caller can read and then rename through `T.Properties.VariableNames`.

## 3. Table value and shape model

A table is a two-dimensional heterogeneous container with an independent row
count and an ordered list of variables. Its public size is:

```text
[row_count, variable_count]
```

One table variable is one ordinary OpenMat value. It is not flattened into one
scalar cell per table element. Every variable must have `size(value, 1)` equal
to the table row count, but a variable may have additional columns. The
measured `3x2` numeric value stored as one variable produced a `3x1` table;
dot and brace extraction of that variable both returned the original `3x2`
numeric value. A `1x3` row vector is therefore one table row, not three rows.

The implementation-oriented representation should have this shape:

```text
TableValue
|- row_count: usize
|- variables: ordered Vec<TableVariable>
|  `- TableVariable { name_utf16, value }
`- properties: TableProperties
   `- variable_names: ordered unique names
```

The table and each stored value use the ordinary OpenMat copy-on-write rules.
Selecting or assigning a table must not introduce a second, mutable alias to a
stored array.

### 3.1 Empty shapes

The following shapes are distinct and measured:

| Construction | Table size | Meaning |
| --- | --- | --- |
| `table()` | `0x0` | no rows and no variables |
| two named `0x1` variables | `0x2` | no rows, two variables |
| size-based construction with three rows and no variables | `3x0` | three rows, no variables |

Deleting the final variable from a measured `3x1` table produced `3x0`; it did
not discard the row count. Adding a `3x1` variable to that table preserved its
three rows. Adding the first `3x1` variable to `table()` established a row count
of three.

### 3.2 Constructor behavior

Measured constructor rules:

- named input expressions use their source binding names (`A`, `B`);
- non-name expressions receive positional names `Var1`, `Var2`, and so on;
- `VariableNames` accepts a row cell array of character vectors or a row string
  array and preserves the supplied order;
- all variables must have equal first-dimension lengths; a mismatch rejects the
  construction;
- a source input named `Row` was rejected because it collided with the table's
  public dimension name.

Phase one must implement the measured ordinary named/expression cases and must
reject wrong name counts and duplicate names. Collision resolution among
generated `VarN` names and the complete set of dimension-name collisions remain
unmeasured and require new probes before claiming full constructor parity.

### 3.3 Queries

For a measured `3x2` table:

| Expression | Result |
| --- | --- |
| `class(T)` | character result `table` |
| `size(T)` | double row vector `[3 2]` |
| `height(T)` | double scalar `3` |
| `width(T)` | double scalar `2` |
| `istable(T)` | logical scalar true |
| `istable(numeric_value)` | logical scalar false |

The table's width is the number of variables, not the sum of the secondary
widths of their stored values.

## 4. Read indexing

### 4.1 Dot indexing

`T.Name` returns the stored variable itself, preserving its class and complete
shape. A measured numeric variable returned `double 3x1`; a measured text
variable returned `cell 3x1`; the matrix-valued variable described above
returned `double 3x2`.

The first tranche must support identifier dot names and dynamic character or
string names so that variables whose public names are not language identifiers
remain addressable.

### 4.2 Parenthesis indexing

`T(rows, variables)` returns another table, even for one selected scalar. It
preserves the selected row order, variable order, variable names, and each
variable's class. Measured results included:

| Selection | Result shape and class |
| --- | --- |
| one row and one variable | `table 1x1` |
| rows `[3 1]`, variables `[2 1]` | `table 2x2`, in requested order |
| all rows, names `{'Text','Num'}` | `table 3x2` |
| all rows, character name `'Num'` | `table 3x1` |
| no rows, all variables | `table 0x3`, names retained |
| all rows, no variables | `table 3x0` |
| logical row mask | table containing the true rows |
| logical variable mask | table containing the true variables |

Phase one therefore accepts colon, one-based numeric vectors, logical masks,
one character/string variable name, and a cell/string vector of variable names
for the applicable selector. Bounds, mask lengths, and unknown names must be
validated before publishing the result.

### 4.3 Brace indexing

`T{rows, variables}` selects the underlying variable values and concatenates
them horizontally. It does not return a table:

- one numeric element returned `double 1x1`;
- one numeric variable returned its `double 3x1` value;
- one text variable returned its `cell 3x1` value;
- a double and a logical variable produced a `double 3x2` result under the
  ordinary concatenation conversion rules;
- numeric and cell variables could not be concatenated and the operation was
  rejected;
- two homogeneous numeric variables produced an ordinary numeric matrix;
- selecting zero rows from a two-variable numeric table produced `double 0x2`;
- selecting zero variables from that table produced `double 2x0`.

The implementation must route brace extraction through the same class and
shape rules as ordinary horizontal concatenation. It must not invent a special
table-only coercion. The number of output columns is the sum of selected
variables' secondary widths, not necessarily the number of selected table
variables.

## 5. Dot assignment and variable names

Measured whole-variable mutation rules:

- assigning `T.Score = [1;2;3]` appends a variable at the end;
- assigning a `single 3x1` value to the existing variable replaces both its
  value and class without moving it;
- assigning a two-row value or a scalar into a three-row table is rejected;
  there is no scalar expansion for whole-variable dot assignment;
- a failed append leaves the prior names, values, shape, and row count intact;
- a dynamic dot name containing a space is accepted and retained exactly;
- assigning `[]` to an existing variable deletes that variable but preserves
  the table row count.

Mutation is copy-on-write and atomic at the table root: validate the new value,
name, and row count before replacing the root visible to the caller.
Parenthesis/braces assignment, row growth, and row deletion are deferred from
this tranche.

## 6. `Properties.VariableNames`

For a three-variable table, `T.Properties.VariableNames` was measured as a
`cell 1x3`. Each element was a `char 1xN` row vector. The order exactly matched
the table variable order.

Replacing the property accepted both a row cell array of character vectors and
a row string array. Replacement renamed variables without changing their data,
class, order, or table size. Duplicate replacement names were rejected and the
old names remained visible. Names need not be valid identifiers: a measured
name containing a space survived exactly.

The first-stage `Properties` surface is deliberately narrow. It must expose
`VariableNames` with the public representation above. Description, units,
dimension names, user data, row names, custom properties, and property-specific
display behavior are deferred.

## 7. `readtable` for CSV and delimited text

### 7.1 Header recognition

The measured detector separated header recognition from generated variable
names:

- an obvious CSV header `Number,Word` selected data lines beginning at line 2
  and returned names `Number`, `Word`;
- headerless rows beginning with `10,alpha` selected data lines beginning at
  line 1 and returned `Var1`, `Var2`;
- forcing `ReadVariableNames=true` on that headerless file consumed the first
  row as names and returned one data row; the numeric-looking name was modified
  to a valid default name;
- forcing `ReadVariableNames=false` on the obvious-header fixture generated
  `Var1` through `VarN`, but the already detected header line was still not
  returned as data.

Phase one should implement a deterministic two-step policy for these ordinary
cases: detect the first data line and header candidacy, then choose header or
generated variable names. It need not reproduce every undocumented heuristic.
Ambiguous files should require an explicit option rather than silently claiming
R2022b parity.

By default, a measured header `First Name,1value,First Name` became
`FirstName,x1value,FirstName_1`. With
`VariableNamingRule='preserve'`, it became
`First Name,1value,First Name_1`: spelling was preserved where possible, while
the duplicate was still made unique.

### 7.2 Default column inference

The measured default inference for a three-row CSV was:

| Input column | Returned variable |
| --- | --- |
| decimal integers `1,2,3` | `double 3x1` |
| `true,false,true` | `cell 3x1` containing character vectors, not logical |
| ordinary words with one blank field | `cell 3x1`; blank becomes empty char |
| numeric values with one blank field | `double 3x1`; blank becomes `NaN` |
| an entirely blank column | `double 3x1` containing `NaN` values |

The first-phase inference algorithm should therefore be intentionally narrow:

1. if every nonempty field in a column parses as a supported phase-one double
   token, produce a double column and map blank fields to `NaN`; the measured
   tokens cover ordinary decimal values and the writer's `NaN` token;
2. if every field is blank, produce a double column of `NaN`;
3. otherwise produce a cell column of character row vectors, mapping blank
   fields to empty char;
4. do not infer logical, datetime, duration, categorical, or string storage in
   this tranche.

Exact numeric token grammar beyond the measured decimal and `NaN` cases is
unmeasured. Locale-dependent decimal separators and thousands separators must
not be accepted accidentally by the host locale.

### 7.3 Delimiters, extensions, and quoting

Measured delimiter behavior:

- `.csv` is recognized directly;
- a `.txt` file accepts explicit semicolon or tab `Delimiter` values;
- `.tsv` is not recognized by R2022b from the extension alone;
- supplying only a tab delimiter still does not make `.tsv` recognizable;
- `.tsv` succeeds when `FileType='text'` and tab `Delimiter` are both supplied.

Quoted CSV fields were measured with embedded commas, doubled quote characters,
and embedded line feeds. Each logical record, including its multiline field,
became one table row. The parser must therefore use a record-aware CSV state
machine, preferably the Rust `csv` crate or an equivalent bounded parser; it
must not split the file into lines and then split each line by the delimiter.

## 8. `writetable`

The measured default CSV output:

- writes variable names as the first record;
- writes logical values as `1` and `0`;
- writes numeric `NaN` as the token `NaN`;
- writes an empty character cell as an empty field;
- quotes text containing a delimiter, quote, or line break and doubles embedded
  quote characters;
- uses CRLF record endings on the measured Windows host, while an embedded line
  feed inside a quoted field remains part of that field.

`WriteVariableNames=false` omits the header. Tab-separated output follows the
same `.tsv` boundary as reading: the measured `.tsv` write required
`FileType='text'` together with a tab delimiter.

A `writetable`/`readtable` text round trip does not promise class identity. In
the measured round trip, a logical column was serialized as `1/0` and read back
as double. Text returned as cell-of-char. Phase-one tests must compare the
specified textual and imported representations rather than assume an invisible
schema in CSV.

Writers should stream rows and perform bounded field escaping. They must not
materialize a second JSON-like copy of the entire table. A temporary file plus
atomic replace is preferred for overwrite mode so an encode or I/O failure does
not leave a partially replaced destination.

## 9. Error and transaction boundary

OpenMat diagnostics use OpenMat-owned categories and wording; this note does
not require MATLAB diagnostic text. The following conditions must be distinct
and visible to the caller:

- table row-count mismatch;
- duplicate or wrong-count variable names;
- unknown variable name or invalid selector;
- brace extraction whose selected values cannot concatenate;
- unsupported file extension or file type;
- malformed quoted delimited text;
- unsupported option or unsupported inferred type;
- filesystem open, read, write, and replace failures.

Table construction and root mutation validate before publication. `readtable`
does not publish a partially built table when parsing or conversion fails.
`writetable` must report the original filesystem or conversion category instead
of replacing it with a generic failure.

## 10. Suggested implementation layering

The first tranche should keep the value semantics and text transport separate:

```text
parser / evaluator indexing
        |
        v
TableValue + TableProperties
        |
        +--------------------+
        |                    |
        v                    v
table built-ins       DelimitedTableCodec
                           |
                           v
                    buffered file boundary
```

Recommended responsibilities:

- the value layer owns row count, ordered variables, names, copy-on-write, and
  selector/assignment validation;
- built-ins own argument parsing and return-count behavior;
- a delimited codec owns header detection, quoting, field inference, and row
  streaming;
- the platform filesystem boundary owns path conversion, error preservation,
  temporary files, and atomic replacement;
- kernel/workspace code treats a table as one ordinary value and does not know
  CSV details;
- the Web Variable Editor requests bounded row/variable slices rather than a
  whole-table JSON preview.

## 11. Minimum conformance corpus

Before the tranche is called closed, locally authored cases should cover at
least:

1. named, expression, and explicit-name construction;
2. mismatched row counts and `0x0`, `0xN`, and `Nx0` shapes;
3. `class`, `size`, `height`, `width`, and `istable`;
4. dot, parenthesis, and brace reads, including reordered and empty selections;
5. brace extraction of compatible numeric/logical values and rejection of an
   incompatible numeric/cell selection;
6. append, replace-with-new-class, failed replacement, dynamic-name access,
   rename, and final-variable deletion;
7. CSV header and headerless detection plus `ReadVariableNames` behavior;
8. numeric, textual, `true/false`, numeric-blank, text-blank, and all-blank
   import columns;
9. semicolon and tab delimiters, including the measured `.tsv` `FileType`
   requirement;
10. CSV write/read behavior for headers, logical values, `NaN`, empty text,
    delimiters, quotes, and multiline text;
11. malformed input and filesystem failures with no partial table or hidden
    generic error.

The differential observation should record class, exact table shape, ordered
variable names, each selected variable's class and shape, and normalized value
payloads. It should not compare localized warning or diagnostic prose.

## 12. Explicitly deferred scope

The following are outside this first-stage contract:

- datetime, duration, calendar duration, categorical, and MATLAB missing-value
  inference;
- timetable, row times, row names, and table joins or synchronization;
- `array2table`, `cell2table`, `struct2table`, `table2array`, `table2cell`, and
  `table2struct` unless assigned as a separate tranche;
- parenthesis/braces assignment, row insertion/deletion, and concatenation of
  tables;
- spreadsheet, XML, HTML, fixed-width, database, and remote-resource import;
- a complete `detectImportOptions`/`setvartype` object model;
- locale-sensitive numbers, arbitrary encodings, BOM policy, comments, custom
  line ranges, units, descriptions, and row names;
- `WriteMode`, append semantics, archive formats, and schema sidecars;
- MAT-file persistence of tables, including v7.3/HDF5;
- nested tables, objects, function handles, and graphics handles as delimited
  columns;
- exact display formatting and MATLAB warning parity.

## 13. Unmeasured R2022b edges

The probes did not establish duplicate or repeated selectors, N-D variable
values, assignment through `()` or `{}`, row-growth syntax, empty assignment to
an absent variable, all generated-name collision cases, all reserved dimension
names, import-option precedence, malformed-quote recovery, Unicode encoding and
BOM behavior, numeric overflow/underflow tokens, locale behavior, very large
streaming files, or allocation-failure atomicity. Each needs a new focused
R2022b probe before it can expand the compatibility claim.
