# Ito Server

The Ito transcription server provides a single gRPC (Connect-RPC) endpoint —
`ItoService.TranscribeStreamV2` — that streams audio from the Ito desktop app
and returns a transcript. This build is **single-user and local-first**: no
database, no object storage, no authentication and no billing.

## 🚀 Quick Start

### Prerequisites

- **Docker & Docker Compose** — the recommended way to run everything
- (Only for running outside Docker) **Bun** package manager

### Run with Docker Compose (recommended)

```bash
# Optional: adjust the local Whisper model or add a Groq API key
cp .env.example .env

docker compose up --build
```

This starts two containers:

| Service           | Port | Purpose                                                                                       |
| ----------------- | ---- | --------------------------------------------------------------------------------------------- |
| `ito-grpc-server` | 3000 | The Ito Connect-RPC server the desktop app talks to                                           |
| `whisper`         | 8000 | Local Whisper server ([speaches](https://github.com/speaches-ai/speaches)), OpenAI-compatible |

The first transcription downloads the Whisper model (configurable via
`LOCAL_WHISPER_MODEL`), so expect a short delay on first use.

Verify it's up:

```bash
curl http://localhost:3000/health   # → {"status":"ok"}
curl http://localhost:8000/v1/models
```

### Run without Docker

```bash
bun install
bun run dev        # or: bun run dev:win on Windows
```

You'll also need an OpenAI-compatible Whisper server reachable at
`LOCAL_WHISPER_BASE_URL` (default `http://localhost:8000`), e.g.:

```bash
docker run --rm -p 8000:8000 ghcr.io/speaches-ai/speaches:latest-cpu
```

## ⚙️ Configuration

All configuration is optional (`.env`, see `.env.example`):

| Variable                 | Default                        | Purpose                                                                     |
| ------------------------ | ------------------------------ | --------------------------------------------------------------------------- |
| `LOCAL_WHISPER_BASE_URL` | `http://localhost:8000`        | OpenAI-compatible transcription endpoint                                    |
| `LOCAL_WHISPER_MODEL`    | `Systran/faster-whisper-small` | Whisper model id (Hugging Face repo id for speaches)                        |
| `GROQ_API_KEY`           | _(unset)_                      | Enables the optional `groq` ASR provider and edit mode ("hey ito" commands) |
| `SHOW_ALL_REQUEST_LOGS`  | `false`                        | Verbose request logging                                                     |

### ASR providers

- **`local`** (default) — sends audio to the local Whisper server. Fully offline.
- **`groq`** — sends audio to Groq's cloud Whisper API. Requires `GROQ_API_KEY`.

The provider can be overridden per-request from the app's advanced settings
(`asrProvider`).

### Edit mode

If a dictation starts with "hey ito", the transcript is treated as a command
and rewritten by an LLM. This is the only feature that requires Groq
(`GROQ_API_KEY`); without a key, plain dictation works normally and edit mode
returns a clear error.

## 🛠️ Development

```bash
bun run dev              # Start with hot reload (tsx watch)
bun run dev:win          # Start with hot reload on Windows (bun --watch)
bun run build            # Type-check / build with tsc
bun run proto:gen        # Regenerate protobuf types (server + app)
```

Tests live next to the source (`*.test.ts`) and run with the repo-root
`bun runServerTests`.

### Protocol buffers

The API is defined in `src/ito.proto`. After changing it, regenerate the
TypeScript types for both the server and the Electron app:

```bash
bun run proto:gen
```

The `buf/validate` proto dependency is vendored at `src/buf/validate/validate.proto`
so code generation works offline.
