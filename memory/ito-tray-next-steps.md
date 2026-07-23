---
name: ito-tray-next-steps
description: Agreed follow-up work on ito-tray after the CUDA/Groq/overlay round (July 2026)
metadata:
  type: project
---

Next steps for `native/ito-tray`, agreed 2026-07-22 after GPU + Groq work landed:

1. ~~**Redo the REC overlay visuals**~~ — **done**: replaced with a
   sound-reactive level bar above the taskbar (premultiplied-BGRA DIB through
   `UpdateLayeredWindow`, so it has real per-pixel alpha and anti-aliased caps).
2. **Easier online/local switching** — choosing between the Groq API and the
   local CUDA model currently means hand-editing `provider` in config.toml and
   restarting. Should be a tray-menu toggle that applies without a restart.
   (The Language submenu added in the same round is the pattern to copy.)
3. ~~**Pin HR/EN language selection**~~ — **done**: a Language submenu in the
   tray writes `language` to the config and applies immediately.

Context: transcription now runs on the GPU (see [[ito-tray-windows-build]]);
`large-v3-turbo` is the active model (~1.6 GB in VRAM of the RTX 2060 SUPER's 8 GB).
