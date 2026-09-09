# MATLAB class-folder compatibility with R2022b

OpenMat resolves a classdef stored as `@Class/Class.m` from the current folder
or an ordinary search-path root. A packaged class uses
`+package/@Class/Class.m` and keeps its qualified runtime name. The `@Class`
directory itself is never an effective path entry and `genpath` does not
traverse it.

The behavior was measured against the locally installed MATLAB R2022b on
2026-09-03 using OpenMat-authored black-box probe files.

## External method bodies

A body-less signature in a non-abstract `methods` block declares a sibling
method file. For example, `result = bump(object, amount)` is implemented by
`@Class/bump.m`. Abstract-block signatures remain abstract dispatch slots and
do not trigger filesystem loading.

The compiler records the declaration's method kind, access, and exact function
signature. The kernel then resolves only the sibling file inside the owning
class directory, compiles it as an ordinary function file, compares fixed or
variadic input/output shape, and binds the resulting bytecode function to the
class method. It does not search an unrelated `bump.m` on the current path.
External bodies run with the declaring class's access context, so private and
protected member access follows the same runtime checks as an inline body.

## Tagged-struct classes

OpenMat also supports MATLAB's older value-class form when construction occurs
inside the matching `@Class/Class.m` constructor:

```matlab
function object = Point(x, y)
data.x = x;
data.y = y;
object = class(data, 'Point');
end
```

The struct fields become private stored properties on a runtime value class;
the input struct itself is not mutated or globally reinterpreted. Sibling
`@Class/method.m` files are selected from the actual argument types for direct
function calls, `feval`, and named function handles. An unrelated same-named
function remains the fallback for non-object arguments. Old-style objects use
value-copy behavior, report their tag through `class`, satisfy `isobject` and
`isa`, and do not support classdef-style `object.method(...)` syntax.

The inherited form `class(data, 'Child', parent1, parent2, ...)` supports an
ordered list of scalar tagged-struct parents, including inherited method
dispatch, `isa` checks for every parent ancestry, and base-private field access
from base methods. Method lookup follows the declared parent order. Repeated
storage reached through multiple parent paths (diamond inheritance) and
old-style inheritance over nonscalar struct arrays remain outside this tranche.
