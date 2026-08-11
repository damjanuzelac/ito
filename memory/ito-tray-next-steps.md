---
name: ito-tray-next-steps
description: State of the dictation apps after ito-groq replaced ito-tray (August 2026)
metadata:
  type: project
---

**Superseded on 2026-08-11.** `native/ito-tray` is archived; the active
dictation app is `native/ito-groq` (single `ito.exe` + `ito.ini`, transcription
through the Groq API only). Local/offline dictation is delegated to
[handy.computer](https://handy.computer) rather than maintained here.

The July 2026 follow-up list is closed:

1. ~~Redo the REC overlay visuals~~ — done, and the sound-reactive bar carried
   over to `ito-groq` unchanged.
2. ~~Easier online/local switching~~ — **moot**: `ito-groq` has no local
   provider to switch to.
3. ~~Pin HR/EN language selection~~ — done, carried over (tray menu writes
   `language` to `ito.ini`, applies without a restart, `auto` is the default).

Open items for `ito-groq`:

- **Measure before optimizing further.** Every dictation appends a timing line
  to `ito.log` beside the ini (`rec / prep / groq / type / total`). Only if
  upload dominates is FLAC-instead-of-WAV or a chunked streaming upload worth
  building — both were deliberately left out of v1.
- **Rotate the Groq API key.** The old one lived in
  `%APPDATA%\Ito-tray\config.toml` and was pasted into a chat session.
- **Copy `ito.exe` somewhere stable** outside `native/target/` and add a
  `shell:startup` shortcut.
- The NeMo Croatian ASR question is only relevant to a local engine, so it no
  longer applies to this app. See [[ito-tray-windows-build]] for how to build
  the archived local version if that ever changes.
