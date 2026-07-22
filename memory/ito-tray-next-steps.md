---
name: ito-tray-next-steps
description: Agreed follow-up work on ito-tray after the CUDA/Groq/overlay round (July 2026)
metadata:
  type: project
---

Next steps for `native/ito-tray`, agreed 2026-07-22 after GPU + Groq work landed:

1. **Redo the REC overlay visuals** — the current `overlay.rs` indicator (dark
   rectangle, hand-drawn 5x7 "REC" bitmap font, red dot) looks bad. It renders
   correctly now, but needs a proper design: rounded/pill shape, real text
   rendering, possibly a level meter or pulse animation.
2. **Easier online/local switching** — choosing between the Groq API and the
   local CUDA model currently means hand-editing `provider` in config.toml and
   restarting. Should be a tray-menu toggle that applies without a restart.
3. **Pin HR/EN language selection** — auto-detection still guesses (whisper's
   first pass produced `bs`/`pt`/`nn` before the candidate fallback corrected
   it). Wants Croatian/English fixed explicitly rather than detected.

Context: transcription now runs on the GPU (see [[ito-tray-windows-build]]);
`large-v3-turbo` is the active model (~1.6 GB in VRAM of the RTX 2060 SUPER's 8 GB).
