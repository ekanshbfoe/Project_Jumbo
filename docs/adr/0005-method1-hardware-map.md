# ADR 0005: Where each Method 1 stage runs

**Status:** accepted 2026-10-28 (project lead). Method 1 is frozen with this map. The project is private and
internal; a public release, end-user Kaggle orchestration and branding toggles are out of scope until Method 2
is built.

## Decision

| Stage | Runs on | Why |
|---|---|---|
| PDF → page images, stitching, slicing | Local PC | Light CPU work; the PDF never leaves the PC, only page images are uploaded. |
| Magi v2 (panels, OCR) | Kaggle T4, first step of the GPU session | ~5 min per chapter on one card; a separate CPU session would take ~1 h plus its own boot. |
| Backdrops (pre-blur) | Same T4 session, host CPU | Seconds, made right after Magi. |
| Librarian | Not run | The Director's MEMORY section (spike 7e) carries the story memory. The Librarian stays paused. |
| Director, Qwen3-VL-32B Instruct | Kaggle T4 ×2, llama.cpp Q6_K | vLLM has no Turing attention backend for Qwen3-VL. ~1 h per chapter, proven in spike 7e. |
| Director, TPU option | Kaggle TPU v5e-8, vLLM bf16 TP=8 | Only on the project lead's command, with the fixes in ADR 0004 (prebuilt wheels, XLA cache, 16K context, 1–2 sequences). Used for batches of chapters, never a single chapter: queues take 11–40 h and the compile cost must be shared. |
| Pass 3 cleanup | Same T4 session, Python | Instant. |
| Drama 3 TTS | Kaggle T4 ×2, setup (c) Q5_K, after the Director unloads | Needs both cards; ~1.2 s of compute per second of audio (~20 min per chapter). 4 CPU cores are far too slow. On a TPU host its CPUs only run it alongside a chip job, never alone. |
| Soundtrack, EpisodeSpec | Local PC | Seconds. |
| Remotion render | 5 × Kaggle CPU in parallel (`--chunk=i/5`) | No GPU or TPU quota; ~7 min for a 1:48 scene. Kaggle T4s have no graphics drivers, and the local PC (7 GB RAM) took ~44 min and crashed. |
| Joining chunks, review | Local PC | Concat demuxer, stream copy, seconds. |

The `qwen25vl_72b_int8` entry in `worker/model_specs.json` is stale; the Director is Qwen3-VL-32B Instruct.

## Saving quota
- **One GPU session per batch.** Magi, the Director and Drama 3 run in one T4 session, so there is one boot
  and each model loads once. Several chapters may share a session (12 h cap) and go through the Director in
  order, so memory carries over.
- **No idle time.** A session ends as soon as its last stage finishes; outputs are saved first.
- **Prebuilt engines only.** llama.cpp and speech.cpp come from the private datasets `jumbo-llamacpp-cuda75`
  and `jumbo-speechcpp-cuda75`; nothing compiles inside a GPU session.
- **Weights** are mounted from datasets where possible, else downloaded from HF to `/kaggle/tmp` (~2–3 min).
- **No accelerator for work that doesn't need one:** rendering, joining and rasterizing.
- **Cost:** ~1.5 T4 hours per chapter, so the weekly quota covers ~15–20 chapters. TPU hours only on command.
