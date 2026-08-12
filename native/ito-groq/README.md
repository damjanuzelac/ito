# ito

Voice dictation for Windows in one executable. Hold a hotkey, speak, release —
the transcript is typed into whatever application has focus.

Transcription runs on [Groq](https://groq.com)'s speech API. There is no local
model, no database, no server and no background sync. If you want offline
dictation, use a tool built for that ([handy.computer](https://handy.computer))
and keep this one for when you want speed.

## Setup

1. Build it (see below) or copy `ito.exe` somewhere permanent — anywhere works,
   it does not install.
2. Run it once. It creates `ito.ini` next to the exe and puts a circular icon
   in the tray.
3. Paste your Groq API key into `ito.ini` (get one at
   [console.groq.com/keys](https://console.groq.com/keys)).
4. Tray menu → **Reload ito.ini**.
5. Hold `Ctrl`+`Win`, speak, release.

To start it with Windows, put a shortcut to `ito.exe` in
`shell:startup` (Win+R → `shell:startup`).

## ito.ini

The file lives next to the exe. If the exe sits somewhere unwritable (Program
Files), it falls back to `%APPDATA%\Ito\ito.ini` instead.

```ini
groq_api_key = "gsk_..."
language = "auto"                  # "auto" | "hr" | "en"
model = "whisper-large-v3-turbo"
hotkey = ["ControlLeft", "MetaLeft"]
microphone = "default"
dictionary = []                    # names and jargon the model keeps getting wrong
min_speech_ms = 300
min_rms = 200.0
ignore_phrases = ["Titlovi HRT", "Thank you."]
type_via_clipboard = false
```

- **`language`** — `auto` lets Groq detect it. Pin it to `hr` or `en` when
  detection keeps guessing wrong on short clips. Also switchable from the tray
  menu, which writes the choice back to this file.
- **`dictionary`** — a hint, not a rule. Useful for proper nouns.
- **`min_speech_ms` / `min_rms`** — clips below either threshold are dropped
  without an API call, which is what stops a silent press from being
  transcribed into an invented sentence.
- **`ignore_phrases`** — the invented sentences that get through anyway.
  Matched case-insensitively, ignoring trailing punctuation.
- **`type_via_clipboard`** — turn on only if some application ignores the
  synthesized keystrokes. It borrows the clipboard for about a second.

## Tray menu

- **Listening enabled** — unregisters the hotkey without quitting.
- **Croatian / English / Auto-detect** — same setting as `language`, saved
  immediately, no restart. The recording bar is red for Croatian, blue for
  English, grey for auto.
- **Reload ito.ini** — after editing the file by hand.
- **Open ito.ini folder**, **Quit Ito**.

## While recording

A thin bar appears just above the taskbar and breathes while the microphone is
open. It is click-through, so it never gets in the way. The tray icon turns red
while recording and amber while transcribing.

That tray icon is drawn at runtime as a plain filled circle rather than being
the app icon, because its **colour is the status** — a static logo there would
say nothing. The Ito logo (`icon.ico`, the same one the Electron app uses) is
embedded in the exe instead, where Explorer, the taskbar, Alt+Tab and the
Startup shortcut all pick it up.

## Speed

Every dictation logs where its time went:

```
[ito] rec 4.2s | prep 3ms | groq 240ms | type 2ms | total 260ms | 88 chars
```

`total` is measured from the moment the hotkey came up, so it is the delay you
actually feel. Two design choices exist purely to keep that number down:

- **The connection is opened when you press the hotkey**, not when you release
  it. By the time there is audio to send, the TLS handshake is already paid
  for. This is worth 150–350 ms.
- **The transcript is typed with `SendInput`**, not pasted. Pasting means
  taking the clipboard, waiting for it to settle, and handing it back — around
  a second of the app being busy after every single dictation.

If `groq` dominates the log, try `whisper-large-v3-turbo` if you are not on it
already. If the total is dominated by neither, the upload is the suspect and
the next step would be sending FLAC instead of WAV.

## Build

Needs the MSVC toolchain. No CUDA, no CMake, no LLVM — this is the entire
reason the crate exists separately from the archived `ito-tray`.

```bash
cd native
cargo build --release -p ito-groq --target x86_64-pc-windows-msvc
```

The result is `native/target/x86_64-pc-windows-msvc/release/ito.exe`, about
5 MB, no runtime dependencies.

Other useful commands:

```bash
cargo test -p ito-groq
cargo run -p ito-groq -- --transcribe-file clip.wav   # 16 kHz mono, no mic needed
```

## Layout

| File | What is in it |
|---|---|
| `src/main.rs` | `ito.ini` handling, the session state machine, timing |
| `src/groq.rs` | The API client, connection pre-warming, prompt hints |
| `src/input.rs` | Hotkey chord matching and text insertion |
| `src/audio.rs` | Enhancement, WAV encoding, the loudness gate |
| `src/overlay.rs` | The recording bar |
| `src/tray.rs` | Tray icon and menu |
| `icon.ico` | App icon, embedded into the exe by `build.rs` |

Microphone capture and the global key hook come from the sibling crates
`audio-recorder` and `global-key-listener`; they are statically linked, so the
build still produces a single exe.
