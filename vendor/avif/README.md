# Bundled AVIF decoder

Windows x64 MSVC uses the release-built static `native/x86_64-pc-windows-msvc/aom.lib`.
Cargo links it directly, including when Velocity is a dependency. No CMake, NASM,
libclang, codec DLL, environment override, or build-time download is needed.
Other target triples currently produce an explicit build error.

The library is libaom 3.11.0, decoder only, optimized with MSVC `/O2` and the
dynamic CRT (`/MD`). CPU dispatch and the upstream SIMD implementations remain
enabled. The native encoder is disabled. Rust bindings are checked in.

Sources:

- `decode.rs`: avif-decode 1.0.2, BSD-3-Clause, https://github.com/kornelski/avif-decode
- `aom/`: aom-decode 0.2.14, BSD-2-Clause, https://gitlab.com/kornelski/aom-decode
- `ffi.rs`: libaom-sys 0.17.2+libaom.3.11.0, BSD-2-Clause, https://github.com/njaard/libavif-rs
- Native library: libaom 3.11.0 from the same libaom-sys crate, BSD-2-Clause,
  https://aomedia.googlesource.com/aom/

Local Rust changes adapt imports to private modules, omit the unused aom-decode
AVIF convenience module, and mark generated extern blocks unsafe for Rust 2024.
The crates.io aom-decode package declares BSD-2-Clause but omits a license file;
`LICENSE-aom-decode` reproduces that license with the package author's attribution.
Keep the supplied license and patent notices with redistributed binaries.

Rebuild from the repository root:

```powershell
./tools/build-avif.ps1 -Nasm 'C:/path/to/nasm-2.16.03/nasm.exe'
```

Only maintainers rebuilding this file need CMake, MSVC C/C++ tools, NASM 2.x,
and `tar`. The script downloads a checksum-pinned source archive, builds Release,
and updates the library and SHA-256 sidecar. Ordinary Cargo builds never run it.
