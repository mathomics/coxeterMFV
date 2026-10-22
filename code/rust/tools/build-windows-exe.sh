#!/usr/bin/env bash
# Builds the Windows application on Linux, with MinGW-w64:
#
#     sudo apt install mingw-w64          (once)
#     rust/tools/build-windows-exe.sh
#
# Writes rust/target/x86_64-pc-windows-gnu/release/coxeter.exe, a single file
# that needs nothing beside it: the C and C++ runtimes are linked in.
set -euo pipefail

rust_dir="$(realpath "$(dirname "$0")/..")"
target=x86_64-pc-windows-gnu

# MinGW's posix thread model, which HiGHS's std::thread needs.
cxx=x86_64-w64-mingw32-g++-posix
command -v "$cxx" >/dev/null || { echo "MinGW-w64 is missing: sudo apt install mingw-w64" >&2; exit 1; }
rustup target list --installed | grep -qx "$target" || rustup target add "$target"

# highs-sys links the C++ library by name, and rustc asks for names
# dynamically, which would leave libstdc++-6.dll to be shipped beside the exe.
# A folder holding only the static archive, searched first, makes the name
# resolve to it.
static_libs="$rust_dir/target/$target/static-libs"
mkdir -p "$static_libs"
ln -sf "$("$cxx" -print-file-name=libstdc++.a)" "$static_libs/libstdc++.a"

export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-posix
export CXX_x86_64_pc_windows_gnu="$cxx"
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C target-feature=+crt-static -L native=$static_libs"

cargo build --release --target "$target" -p coxeter-app --manifest-path "$rust_dir/Cargo.toml" "$@"
echo "$rust_dir/target/$target/release/coxeter.exe"
