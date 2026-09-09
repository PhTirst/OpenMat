# OEX v1 complete C plugin

This example exercises the OEX v1 surface needed by a low-level numerical
plugin:

- owned input transfer and writable zero-copy dense storage;
- synchronous callbacks into arbitrary m-language function handles;
- one-off sparse triplet construction;
- a native handle class with a read/write property and ordinary methods;
- persistent sparse-pattern reuse;
- checked native-object borrowing and native-object creation;
- explicit cancellation, error-path cleanup, and destructor ownership.

From the repository root, build the bridge and plugin with UCRT64 GCC:

```powershell
cargo build -p openmat-oex
gcc -shared -std=c11 -O2 -Wall -Wextra -Werror `
  -I .\include `
  .\examples\oex-v1-plugin\oex_v1_plugin.c `
  .\target\debug\openmat_oex.dll `
  -o .\examples\oex-v1-plugin\oex_v1_plugin.oex.dll
Copy-Item .\target\debug\openmat_oex.dll .\examples\oex-v1-plugin\
```

Then run the checked-in script through the CLI:

```powershell
cargo run -p openmat-cli -- run `
  .\examples\oex-v1-plugin\demo.m `
  --oex-plugin .\examples\oex-v1-plugin\oex_v1_plugin.oex.dll
```

The DLL is trusted in-process code. Do not load an OEX plugin from an untrusted
source.
