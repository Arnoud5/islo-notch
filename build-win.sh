#!/bin/sh
# Cross-compile for Windows from Linux (used to produce the shipped Islo.exe).
# On Windows itself you just run: cargo build --release
export RUSTC_BOOTSTRAP=1
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
exec cargo build --release --target x86_64-pc-windows-gnu -Zbuild-std=std,panic_abort "$@"
