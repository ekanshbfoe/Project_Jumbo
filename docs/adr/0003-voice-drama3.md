# ADR 0003: Fish Audio Drama 3 is the voice

**Status:** accepted 2026-10-26. Replaces OmniVoice.

## Decision
- Narration and character voices are Fish Audio Drama 3 (35B MoE, 3B active), run by speech.cpp 0.8.11
  built for CUDA sm_75, with our patch for a second GPU and a memory cap.
- Weights: `sakasegawa/drama-3-GGUF`, a conversion checked stage by stage against Fish's implementation.
  The quant and replica layout come from spike 5.
- Scripts for voicing are Devanagari or Mixed Hinglish only (`hinglish.ts`, `rules.script_problems`).

## Why
- **OmniVoice (spikes 3, 3b):** a heavy foreign accent on Hinglish. Native Hindi references fixed the
  code-switching but not the naturalness.
- **Drama 3 (spike 4, one T4, Q6_K):** judged natural. That held for Devanagari and Mixed script, emotion cues
  (`[laughs]`, `[whispers]`, ...) and two-speaker scenes.
- **Romanized Hindi** read noticeably worse, hence the writing rule.

## How it fits a T4
- Fish's own int4 build needs sm_80 (GPTQ-Marlin), and Kaggle's containers have no Vulkan driver, so
  speech.cpp is compiled for CUDA (about 25 min). The build is kept as a Kaggle dataset.
- Experts that do not fit the card run on the CPU.
  - Spike 4: 14 of 40 layers' experts on the card; peak 14.7 GB GPU, 20.8 GB RAM.
  - Load 120 s; 1.6 s per second of audio.
- The patch:
  - caps planned GPU memory at the pool's ceiling (`SPEECH_GPU_FRACTION`);
  - can put later layers' experts on a second card (`SPEECH_DRAMA3_SECOND_DEVICE`).

## Consequences
- **No speed or duration control.** Panels follow the audio.
  - `buildEpisodeSpec` holds each beat for its clip, its pause and the next transition.
  - The voice stage checks each take and speaks failures again with a new seed, at most 3 tries.
  - Only digital silence is trimmed. Nothing is stretched.
- **Pace and emotion are text cues**, written by the runner from the script's fields.
- **The TPU** has no ggml backend. Drama 3 on TPU means Fish's PyTorch code under TorchTPU, still to be tested.
