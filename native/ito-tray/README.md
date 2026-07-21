# Ito Tray

A **standalone Windows tray application** for local voice dictation — the
whole Ito pipeline in a single `.exe`, with no Electron, no server and no
Docker. Hold the hotkey, speak, release: the transcript is typed into the
focused application. Transcription runs in-process via
[whisper.cpp](https://github.com/ggerganov/whisper.cpp) (whisper-rs).

## Features

- **Hold-to-talk dictation** — default hotkey `Ctrl+Win` (suppressed from the
  focused app while held, including Start-menu protection)
- **Edit mode** — hold `Ctrl+Alt` (or start with "hey ito") to treat the
  dictation as a command; requires a Groq API key in the config
- **Custom dictionary** — words from the config are fed to Whisper as a
  vocabulary hint
- **History** — transcripts are stored in a local SQLite database
- **Auto-downloaded model** — the ggml Whisper model is fetched from Hugging
  Face on first run
- **Status tray icon** — blue when idle, red while recording, amber while the
  model loads or transcribes
- **Constrained language detection** — with `language = "auto"`, detection is
  restricted to `auto_languages` (default `["en", "hr"]`) so short clips aren't
  mis-detected as an unrelated language

## Building (Windows)

Requirements:

- [Rust](https://rustup.rs/) (MSVC toolchain)
- [CMake](https://cmake.org/download/) and
  [LLVM/Clang](https://releases.llvm.org/) — needed by whisper-rs to build
  whisper.cpp (set `LIBCLANG_PATH` if bindgen can't find clang)

```powershell
cd native
cargo build --release -p ito-tray --target x86_64-pc-windows-msvc
# → native/target/x86_64-pc-windows-msvc/release/ito-tray.exe
```

Note: MinGW (`x86_64-pc-windows-gnu`) cross-compilation is not supported for
this crate; CI compile checks exclude it.

### GPU acceleration (CUDA)

With an NVIDIA GPU, local transcription can run on the GPU (an order of
magnitude faster than CPU). Install the CUDA Toolkit (matching your driver's
supported CUDA version) and build with the `cuda` feature:

```powershell
cargo build --release -p ito-tray --target x86_64-pc-windows-msvc --features cuda
```

## Configuration

Created on first run at `%APPDATA%\Ito-tray\config.toml`:

```toml
hotkey_transcribe = ["ControlLeft", "MetaLeft"]  # Ctrl+Win
hotkey_edit = ["Alt", "ControlLeft"]             # Ctrl+Alt
microphone = "default"           # or an exact input device name
provider = "local"               # local (embedded whisper) | groq (cloud API)
model = "small"                  # tiny | base | small | medium | large-v3
language = "auto"                # or e.g. "en", "hr"
auto_languages = ["en", "hr"]    # candidates when language = "auto"
dictionary = []                  # e.g. ["Zagreb", "Postgres", "Uzelac"]
no_speech_threshold = 0.6
groq_api_key = ""                # optional; enables edit mode + groq provider
groq_model = "openai/gpt-oss-120b"
groq_transcription_model = "whisper-large-v3-turbo"
```

With `provider = "groq"` (and a `groq_api_key`), clips are sent to Groq's
transcription API instead of the local model — much faster than CPU inference,
at the cost of audio leaving the machine. The local model is not downloaded or
loaded in this mode. Changing `provider` requires an app restart. If a Groq
request fails while the local model is loaded, transcription falls back to it.

Key names are raw [rdev](https://github.com/heyito/rdev) names, e.g.
`ControlLeft`, `MetaLeft` (Win key), `Alt`, `ShiftLeft`, `KeyA`, `Function`.

Models are stored in `%APPDATA%\Ito-tray\models\`, history in
`%APPDATA%\Ito-tray\ito-tray.db` (`interactions` table, same shape as the
Electron app's local database). Use **Reload config** from the tray menu after
editing (changing `model` requires an app restart).

## Development on Linux/macOS

The tray UI is Windows-only, but the whole pipeline can be exercised
headlessly:

```bash
cd native
cargo test -p ito-tray
cargo run -p ito-tray -- --transcribe-file test.wav   # one-shot pipeline run
cargo run -p ito-tray -- --listen                     # headless hotkey loop
```

## Architecture

- `main.rs` — CLI parsing, wiring, headless modes
- `tray.rs` (Windows) — tao event loop + tray-icon menu and status tooltip
- `hotkeys.rs` — exact-chord matching (ported from the Electron app) on top of
  the shared `global-key-listener` library; hold starts, release completes
- `session.rs` — state machine owning capture (`audio-recorder` lib), the
  Whisper engine, text insertion (`text-writer` lib) and history
- `audio_pipeline.rs` — enhancement (DC removal, 80 Hz high-pass, −3 dBFS
  normalization) ported from the server
- `whisper.rs` / `model_download.rs` — whisper-rs wrapper + ggml auto-download
- `prompt.rs` — vocabulary prompt (224-token budget) and "hey ito" detection
- `groq.rs` — optional edit-mode LLM call
