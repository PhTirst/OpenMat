# Pinned release-license fallbacks

Cargo packages normally carry their own license files, and
`Generate-ThirdPartyLicenses.ps1` copies those files directly. A small number
of published crates declare an SPDX license but omit the corresponding text.
The files in this directory are pinned, unmodified upstream texts used only for
those packages. An unknown omission remains a hard release error.

| Fallback file | Exact upstream source |
| --- | --- |
| `rust-alloc-no-stdlib-BSD-3-Clause.txt` | `dropbox/rust-alloc-no-stdlib`, commit `6032b6a9b20e03737135c55a0270ccffcc1438ef`, `LICENSE` |
| `equator-MIT.txt` | `sarah-ek/equator`, commit `9d4107bdd6a75598aed4a60fe46200c9e9f4e653`, `LICENSE` |
| `faer-MIT.txt` | `sarah-quinones/faer`, commit `8dfcceee8eb3d81231366acc9d3f157ac5e7057a`, `LICENSE` |
| `hdf5-rust-Apache-2.0.txt` | `metno/hdf5-rust`, commit `25418ae50cdfee8463fd83602a448aadbe6e1efe`, `LICENSE-APACHE` |
| `libffi-rs-Apache-2.0.txt` | `libffi-rs/libffi-rs`, commit `8309adab3629990d06da20f3da656faf95b23100`, `LICENSE-APACHE` |
| `nano-gemm-MIT.txt` | `sarah-ek/nano-gemm`, commit `806b012a6f5ee0615042bf64675180fb97ff2ccc`, `LICENSE` |
| `profiling-Apache-2.0.txt` | `aclysma/profiling`, commit `8271551172eb6fa4cba47369aedd93790c623df9`, `LICENSE-APACHE` |
| `pulp-MIT.txt` | `sarah-quinones/pulp`, commit `5eb07fd7b68edf0a5e19f71737d315f72a510295`, `LICENSE` |
| `stylo-MPL-2.0.txt` | `servo/stylo` packaged as `cssparser 0.36.0`, commit `9339ddd71443463dfb85d1185ad50cd98b34bc8f`, `LICENSE` |
| `rust-unic-Apache-2.0.txt` | `open-i18n/rust-unic`, commit `5878605364af97a3358368a6eaef02104af2e016`, `LICENSE-APACHE` |
| `webview2-rs-MIT.txt` | `wravery/webview2-rs`, commit `b74dc5e2b394044bea5191052868ce7a106c202c`, `LICENSE` |
| `NSIS-3.08-COPYING.txt` | NSIS 3.08 `COPYING`, pinned from the Tauri NSIS tool bundle used by the Windows release |

Dual-licensed wrappers use the Apache-2.0 option where the upstream SPDX
expression permits it. The actual bundled HDF5 and stock zlib licenses are
extracted separately from their locked source packages, while OpenBLAS is
verified against its pinned release checksum.
