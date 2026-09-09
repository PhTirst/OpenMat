# OpenMat build acceleration

The Windows build entry points initialize a repository-local compiler cache
automatically. Generated tools and cached objects live below `.openmat/`, which
is ignored by Git and remains on the same drive as the checkout.

For an interactive PowerShell session, initialize the same environment with:

```powershell
& .\tools\build\Initialize-OpenMatBuildEnvironment.ps1
```

The initializer uses six Cargo jobs by default and downloads the pinned Windows
MSVC build of `sccache` 0.17.0 after verifying its SHA-256 digest. The cache is
bounded to 20 GiB. Use `-BuildJobs N`, `-CompilerCacheSizeGiB N`, or
`-DisableCompilerCache` to override those defaults.

The compiler cache accelerates Rust recompilation, especially after an isolated
Cargo target has been cleaned. It does not cache the bundled HDF5 CMake build.
HDF5 still benefits from Cargo's persistent target directory and from the six
parallel jobs passed by the `cmake` crate to `cmake --build --parallel`.
