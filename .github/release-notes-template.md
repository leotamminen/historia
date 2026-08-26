Prebuilt `historia` binaries for six platforms, built directly from this
tagged commit by `.github/workflows/release.yml`. No installer - each archive
contains one self-contained `historia` binary plus `LICENSE` and `README.md`.
Extract it and put `historia` on your `PATH`.

| Platform | Asset |
|---|---|
| Windows x64 | `historia-<version>-x86_64-pc-windows-msvc.zip` |
| Windows ARM64 | `historia-<version>-aarch64-pc-windows-msvc.zip` |
| macOS (Intel) | `historia-<version>-x86_64-apple-darwin.tar.gz` |
| macOS (Apple Silicon) | `historia-<version>-aarch64-apple-darwin.tar.gz` |
| Linux x64 | `historia-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `historia-<version>-aarch64-unknown-linux-gnu.tar.gz` |

Windows binaries statically link the CRT (`+crt-static`) and have no external
DLL dependency. macOS binaries link only the system libraries, as normal.

### Known limitation: Linux glibc

The Linux builds link glibc dynamically, so they run on most modern
distributions but need a reasonably current glibc - they will not run on very
old distros. A fully static `musl` build (no glibc dependency at all) is
planned for a later checkpoint but is not part of this release.

### From source

`cargo install --path .` always works if none of the above fits (see the
README's Install section).
