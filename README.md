# Ito (local-first fork)

Hold a hotkey, speak, release — the transcript is typed into whatever
application has focus.

This is a **single-user, local-first fork** of [heyito/ito](https://github.com/heyito/ito).
The upstream project is no longer maintained. This fork strips out everything
that phoned home: there are **no accounts, no billing, no telemetry and no
cloud sync**. Transcription runs against a local Whisper server by default;
Groq is an optional cloud provider you enable with your own API key.

There are two independent ways to run it:

| | **Electron app** | **Ito Tray** |
|---|---|---|
| Platforms | macOS, Windows | Windows only |
| What you run | Electron UI + gRPC server in Docker | one `.exe`, nothing else |
| Transcription | local Whisper (speaches) container, or Groq | embedded whisper.cpp, or Groq |
| GPU | — | optional CUDA build |
| Docs | this file | [native/ito-tray/README.md](native/ito-tray/README.md) |

If you just want dictation on Windows with the least moving parts, use **Ito
Tray** — it needs no Docker, no server and no Node.

---

## Ito Tray (Windows, standalone)

The whole pipeline (hotkey → microphone → whisper.cpp → typing) in a single
process. Hold `Ctrl+Win`, speak, release.

```powershell
cd native
cargo build --release -p ito-tray --target x86_64-pc-windows-msvc
# → native/target/x86_64-pc-windows-msvc/release/ito-tray.exe
```

Needs the Rust MSVC toolchain plus CMake and LLVM/Clang (whisper-rs compiles
whisper.cpp). With an NVIDIA GPU, add `--features cuda` for roughly an order of
magnitude speedup. Configuration lives at `%APPDATA%\Ito-tray\config.toml`.

Full manual: **[native/ito-tray/README.md](native/ito-tray/README.md)**.

---

## Electron app

### Prerequisites

- macOS 10.15+ or Windows 10+
- [Bun](https://bun.sh/) and Node.js 20+
- [Rust](https://rustup.rs/) — the native helpers are Rust binaries
- Docker — runs the gRPC server and the local Whisper container
- Microphone permission, plus Accessibility permission on macOS (needed for
  global hotkeys and text insertion)

### Running from source

```bash
git clone <your fork url>
cd ito
bun install

# Build the native helper binaries
bun build:rust:mac      # or: bun build:rust:win

# Start the gRPC server + local Whisper (from the server directory)
cd server
cp .env.example .env    # optional: adjust the Whisper model, or add GROQ_API_KEY
docker compose up --build
cd ..

# Start the app in another terminal
bun dev
```

The server is required — the Electron app sends audio to it over gRPC. See
[server/README.md](server/README.md) for details.

### Default hotkeys

| Mode | macOS | Windows |
|---|---|---|
| Transcribe | `Fn` | `Ctrl` + `Win` |
| Edit ("hey ito" commands) | `Ctrl` + `Fn` | `Alt` + `Ctrl` |

Both are configurable in Settings. Edit mode rewrites the dictation as an
instruction and needs a `GROQ_API_KEY` on the server.

### What the app gives you

- Dictation into any focused text field, system-wide
- **Dictionary** — custom vocabulary fed to the transcription model
- **Notes** — transcripts saved locally
- Interaction history in a local SQLite database
- Microphone and shortcut settings

Everything is stored on your machine. The analytics module is a no-op stub
kept only so call sites still compile.

---

## Transcription providers

The server talks to one of two backends:

- **Local Whisper** (default) — the [speaches](https://github.com/speaches-ai/speaches)
  container started by `docker compose`, listening on port 8000. Audio never
  leaves your machine.
- **Groq** — set `GROQ_API_KEY` in `server/.env`. Faster, but audio is sent to
  Groq. Also required for edit mode, which needs an LLM.

Ito Tray has the same choice, configured in its own `config.toml`.

---

## Development

### Common commands

```bash
bun dev             # electron-vite dev with watch
bun runAllTests     # lib + server + app + native tests
bun lint            # ESLint       (bun lint:fix to fix)
bun format          # Prettier     (bun format:fix to fix)
bun type-check      # tsc --noEmit
bun lint:native     # clippy       (bun lint:fix:native to fix)
bun format:native   # rustfmt      (bun format:fix:native to fix)
```

Packaging: `bun build:mac` / `bun build:win`.

### Layout

```
ito/
├── app/          Electron renderer (React + Zustand + Tailwind)
├── lib/          shared code: main process, preload/IPC, media interfaces
├── native/       Rust workspace (and one Swift package)
│   ├── audio-recorder/        microphone capture + resampling
│   ├── global-key-listener/   global hotkey capture
│   ├── text-writer/           text insertion
│   ├── active-application/    focused-window detection
│   ├── selected-text-reader/  selected text (macOS/Windows only)
│   ├── macos-text/            macOS text helpers
│   ├── cursor-context/        Swift; cursor context on macOS
│   └── ito-tray/              standalone Windows tray app
├── server/       gRPC transcription server (Bun) + docker-compose
├── scripts/      build and maintenance scripts
└── resources/    icons and build assets
```

### Native tests

The Rust modules are a Cargo workspace:

```bash
cd native
cargo test --workspace
```

`ito-tray` is excluded from the MinGW cross-compile CI check — build it with
MSVC on Windows. `selected-text-reader` does not compile on Linux.

---

## Acknowledgments

- **[heyito/ito](https://github.com/heyito/ito)** — the upstream project this
  is forked from, and the origin of essentially all of the application code.
- **[electron-react-app](https://github.com/guasam/electron-react-app)** by
  @guasam — the Electron + React template upstream was built on.
- **[whisper.cpp](https://github.com/ggerganov/whisper.cpp)** and
  [whisper-rs](https://github.com/tazz4843/whisper-rs) — embedded transcription
  in Ito Tray.
- **[speaches](https://github.com/speaches-ai/speaches)** — the local Whisper
  server used by the Electron build.

## License

GNU General Public License v3 — see [LICENSE](LICENSE).
