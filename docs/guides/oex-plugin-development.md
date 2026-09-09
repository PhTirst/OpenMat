# Build an OEX native plugin

OEX loads trusted Windows x64 DLLs into an OpenMat kernel. The public contract
is [`include/openmat/oex.h`](../../include/openmat/oex.h).

The checked-in [complete C plugin](../../examples/oex-v1-plugin/README.md)
builds independently and demonstrates dense mutation, sparse construction,
native classes, properties, object bridging, cancellation, and cleanup.

## Minimal function and plugin descriptor

```c
#include <openmat/oex.h>

static OexStatus OEX_CALL add2(OexCall *call) {
    double a;
    double b;
    OexValue *result = NULL;

    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 0), &a));
    OEX_TRY(OexValue_GetFloat64(OexCall_BorrowInput(call, 1), &b));
    OEX_TRY(OexValue_CreateFloat64(call, a + b, &result));
    return OexCall_SetOutput(call, 0, result);
}

static const OexFunctionDefinition FUNCTIONS[] = {
    {sizeof(OexFunctionDefinition), 0, OEX_UTF8_LITERAL("add2"),
     2, 2, 1, 1, add2},
};

static const OexPluginDefinition PLUGIN = {
    sizeof(OexPluginDefinition), OEX_ABI_VERSION, 0, 0,
    OEX_UTF8_LITERAL("example"), OEX_UTF8_LITERAL("1.0.0"),
    FUNCTIONS, 1, NULL, 0,
};

OEX_PLUGIN_EXPORT const OexPluginDefinition *OEX_CALL OexPlugin_Init(void) {
    return &PLUGIN;
}
```

The descriptor, text, and arrays it references must remain valid for the life
of the loaded DLL. OpenMat copies metadata but pins the DLL because callbacks
remain plugin-owned code.

ABI 1.1 is the first sealed v1 header. Recompile plugins that used the formative
ABI 1.0 native-class descriptor.

## Build

Build `openmat_oex.dll` first, then link the plugin to it. With UCRT64 GCC:

```powershell
cargo build -p openmat-oex
gcc -shared -std=c11 -O2 -Wall -Wextra -Werror `
  -I C:\work\OpenMat\include add2.c C:\work\OpenMat\target\debug\openmat_oex.dll `
  -o add2.oex.dll
Copy-Item C:\work\OpenMat\target\debug\openmat_oex.dll .
```

Keep `openmat_oex.dll` beside the plugin, or install the bridge at the location
used by the OpenMat distribution. MSVC can link the generated
`openmat_oex.dll.lib` instead.

## Load

Plugin loading is an explicit trust decision because an OEX library executes
native code inside the kernel process. Pass each library separately; OpenMat
canonicalizes the paths, rejects duplicate canonical libraries, and loads them
in command-line order:

```powershell
openmat-cli run model.m --oex-plugin .\add2.oex.dll
openmat-server --stdio --oex-plugin .\add2.oex.dll
openmat-server --listen 127.0.0.1:0 `
  --workspace-root . --oex-plugin .\add2.oex.dll
```

Relative plugin paths resolve against the process working directory. Descriptor
validation or language-name collisions fail engine initialization with the
stable `oex.load` category; no partially constructed session is published.

## Zero-copy dense output

Use a builder when the plugin will fill a complete new array:

```c
static OexStatus OEX_CALL scale2(OexCall *call) {
    OexDenseView source;
    OexMutableDenseView output;
    OexDenseBuilder *builder = NULL;
    OexValue *value = NULL;
    uint64_t i;

    OEX_TRY(OexDense_GetView(OexCall_BorrowInput(call, 0), &source));
    if (source.dataType != OEX_DATA_F64) return OEX_ERROR_TYPE;
    OEX_TRY(OexDenseBuilder_CreateUninitialized(
        call, source.dataType, source.rank, source.dimensions,
        &builder, &output));

    for (i = 0; i < source.elementCount; ++i)
        ((double *)output.data)[i] = 2.0 * ((const double *)source.data)[i];

    OEX_TRY(OexDenseBuilder_Commit(builder, &value));
    return OexCall_SetOutput(call, 0, value);
}
```

No plugin temporary array and no final host copy are involved.

## Sparse output

For one-off assembly, request writable triplet buffers. Indices are zero-based:

```c
OexSparseTripletBuilder *builder = NULL;
OexSparseTripletView view;
OexValue *value = NULL;

OEX_TRY(OexSparseTripletBuilder_CreateUninitialized(
    call, OEX_DATA_F64, rows, columns, capacity, &builder, &view));
/* Initialize view.rowIndices[i], view.columnIndices[i], and
   ((double *)view.values)[i] for 0 <= i < storedCount. */
OEX_TRY(OexSparseTripletBuilder_Commit(builder, storedCount, &value));
return OexCall_SetOutput(call, 0, value);
```

Commit sorts entries, combines duplicate coordinates, and removes explicit
zeros. For repeated assembly with stable connectivity, create one persistent
`OexSparsePattern`, keep it in the native object, and use
`OexSparsePatternBuilder_CreateUninitialized` on every call. That path exposes
only the writable values buffer; release the pattern in the object's
destructor. Logical, `double`, and complex-`double` are supported.

## Native handle classes

A class descriptor registers a constructor, destructor, ordinary methods, and
optional dependent properties:

```c
static const char ASSEMBLER_TYPE_TOKEN = 0;

static const OexNativeMethodDefinition METHODS[] = {
    {sizeof(OexNativeMethodDefinition), 0, OEX_UTF8_LITERAL("assemble"),
     1, 1, 1, 1, assemble},
};

static OexStatus OEX_CALL get_dofs(OexCall *call, void *instance) {
    Assembler *assembler = (Assembler *)instance;
    OexValue *result = NULL;
    OEX_TRY(OexValue_CreateFloat64(call, (double)assembler->dofs, &result));
    return OexCall_SetOutput(call, 0, result);
}

static const OexNativePropertyDefinition PROPERTIES[] = {
    {sizeof(OexNativePropertyDefinition), 0, OEX_UTF8_LITERAL("Dofs"),
     get_dofs, NULL}, /* read-only */
};

static const OexNativeClassDefinition CLASSES[] = {
    {sizeof(OexNativeClassDefinition), OEX_CLASS_HANDLE,
     OEX_UTF8_LITERAL("Assembler"), construct, destroy,
     METHODS, sizeof(METHODS) / sizeof(METHODS[0]),
     &ASSEMBLER_TYPE_TOKEN,
     PROPERTIES, sizeof(PROPERTIES) / sizeof(PROPERTIES[0])},
};
```

Place `CLASSES` and its count in `OexPluginDefinition`. Constructor instance
pointers become ordinary OpenMat handle objects; method dispatch and `class`
use the registered name, and the destructor runs exactly once when the object
is cleared or collected. Native value classes are not supported yet.

A property getter receives zero call inputs and must set one output. A setter
receives the assigned value as input zero and sets no output. The native
instance pointer is supplied separately to both callbacks. Use `NULL` for the
missing side of a read-only or write-only property; at least one callback is
required. `obj.Dofs` uses this property bridge directly, while ordinary methods
continue to use `obj.assemble(...)`.

When another native object is passed as an ordinary argument, resolve it with
the active call and its exact type token:

```c
Mesh *mesh = NULL;
OEX_TRY(OexNativeObject_BorrowInstance(
    call, OexCall_BorrowInput(call, 0),
    &MESH_TYPE_TOKEN, (void **)&mesh));
```

The pointer is borrowed only for the callback. A plugin that keeps shared
native state longer must apply its own reference-counting policy.

A factory method can transfer a newly allocated instance to OpenMat:

```c
Mesh *refined = create_refined_mesh(mesh);
OexValue *result = NULL;
OexStatus status = OexNativeObject_Create(
    call, &MESH_TYPE_TOKEN, refined, &result);
if (status != OEX_OK) {
    destroy_mesh(refined); /* ownership did not move */
    return status;
}
status = OexCall_SetOutput(call, 0, result);
if (status != OEX_OK) OexValue_Release(result);
return status;
```

After `OexNativeObject_Create` succeeds, OpenMat owns the instance even if a
later operation fails; do not destroy it from the plugin. Release only the
owned `OexValue` wrapper when `SetOutput` fails.

## Ownership checklist

| Resource | How it is obtained | How long it is valid | Cleanup or transfer |
| --- | --- | --- | --- |
| Borrowed `OexValue` | `BorrowInput` | Current callback or until its slot is taken | Never release |
| Owned `OexValue` | `TakeInput`, `Retain`, create, or builder commit | Until released or transferred | `OexValue_Release`, or successful `SetOutput` |
| Dense/CSC view | A view function | Current callback and while the backing handle remains valid | No separate cleanup |
| Dense/sparse builder | Builder create | Current callback | Commit or abort; unfinished builders auto-abort |
| Sparse pattern | Create or retain | Independent of the call | One `OexSparsePattern_Release` per owned handle |
| Borrowed native instance | `OexNativeObject_BorrowInstance` | Current callback | Never destroy |
| New native instance | Plugin allocation | Until `OexNativeObject_Create` succeeds | Plugin destroys on Create failure; OpenMat destroys after success |

Returning an error does not release plugin-owned `OexValue` or sparse-pattern
handles. Use explicit cleanup paths after the first ownership transfer. No C++
exception, Rust panic, or other unwind may cross an OEX callback boundary.

## Taking and mutating an input

Use `TakeInput` when the result can reuse an input representation:

```c
OexValue *value = NULL;
OexMutableDenseView view;

OEX_TRY(OexCall_TakeInput(call, 0, &value));
OEX_TRY(OexDense_GetMutableView(value, &view));
/* write view.data */
return OexCall_SetOutput(call, 0, value);
```

Obtaining the mutable view first performs the necessary OpenMat COW detach.
If a later operation fails before `SetOutput`, call `OexValue_Release(value)`.
Do not release a value after `SetOutput` succeeds because ownership has moved
to the host.

## Loading

Loading remains explicit:

```rust
// SAFETY: the application trusts this in-process native DLL.
let mut engine = unsafe {
    openmat_kernel::RuntimeEngine::with_oex_plugins(["C:/plugins/add2.oex.dll"])?
};
```

The descriptor name is then registered in ordinary OpenMat name resolution,
so the example is called as `add2(2, 3)`.
