---
name: ito-tray-windows-build
description: How to build/run the ito-tray Rust app on the user's Windows machine
metadata:
  type: project
---

**This applies only to the archived `ito-tray`.** The active app,
`native/ito-groq`, has no local model and therefore needs none of the below —
plain `cargo build --release -p ito-groq` with the MSVC toolchain is enough
(~1 min from cold, 4.4 MB exe).

Building `native/ito-tray` on this Windows machine (MSVC, Ryzen 7 3700X) requires CMake and LLVM/Clang for whisper-rs. They were installed via winget (`Kitware.CMake`, `LLVM.LLVM`) but are **not on the persistent PATH** — each build shell must prepend them and set libclang:

```
$env:Path = "C:\Program Files\CMake\bin;C:\Program Files\LLVM\bin;" + $env:Path
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
cargo build --release -p ito-tray --target x86_64-pc-windows-msvc
```

GPU builds add `--features cuda` and need `CUDA_PATH` plus the toolkit's `bin`
on PATH (CUDA 13.3 installed at `C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3`;
NVIDIA driver 610.74). The winget CUDA install **requires an interactive UAC
prompt** — it fails with `0x800704c7 canceled by the user` when run in a
background shell. First CUDA compile takes ~4-10 min and is easily interrupted
by session teardown; it resumes from cached objects.

Exe: `native/target/x86_64-pc-windows-msvc/release/ito-tray.exe` (gitignored). First run downloads the ggml `small` model (~465 MB) to `%APPDATA%\Ito-tray\models\`. rustfmt/clippy components had to be added (`rustup component add rustfmt clippy`). Local checkout is CRLF (git autocrlf), so `cargo fmt --check` spams "Incorrect newline style" — ignore that noise, only real content diffs matter.
