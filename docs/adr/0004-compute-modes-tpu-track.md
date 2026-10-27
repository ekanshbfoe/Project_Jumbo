# ADR 0004: Three compute modes; the TPU track runs the full pipeline

**Status:** accepted 2026-10-27 (project lead). Engines on the TPU chips are confirmed by spike 9.

## Decision
- The app runs one pipeline in three **compute modes**: **CPU**, **GPU** (Kaggle T4 ×2) and **TPU** (Kaggle v5e-8).
  Every mode produces the same contracts (PanelsDoc, ScriptDoc, MemoryDelta, AudioManifest, EpisodeSpec). Only
  the engine, precision and replica layout differ.
- **GPU** stays exactly as built: Magi v2 on the cards, Qwen3-VL-32B Q6_K on llama.cpp across both T4s (spikes
  7/7b), Drama 3 Q5_K split over both T4s (ADR 0003).
- **TPU is a core track, not a fallback.** It holds the full architecture at full precision and is where the
  system grows. It uses the chips *and* the host at once.
- **Queued TPU sessions are never cancelled** except on the project lead's command, or after more than an hour
  without any reply from them. A cancelled request goes to the back of a 24 h+ queue.

## The TPU host (measured, probe 2026-10-27)
v5litepod-8: 8 chips (16 GB HBM each, 128 GB in all; bf16 and int8 in hardware), **224 vCPUs, 378 GB RAM,
164 GB `/dev/shm`**, Python 3.12, `/kaggle/working` 20 GB.

## Model placement on TPU (budgets at the 0.80 HBM ceiling: 12.8 GB per chip)

| Stage | Model | Engine | Precision | Lease | Per chip | Notes |
|---|---|---|---|---|---|---|
| Vision | Magi v2 | TorchTPU (`model.to("tpu")`) | bf16/fp32 | 1 chip × up to 8 replicas | ~2–4 GB | same runner and fingerprints as GPU |
| Director | Qwen3-VL-32B Instruct | vLLM on TPU (tpu-inference, or vllm-torchtpu) | **bf16, lossless** | 8 chips, TP=8 | ~8.2 GB weights + KV | constrained ScriptDoc JSON |
| Librarian | Qwen2.5-7B Instruct | vLLM on TPU | bf16 | 2 chips, TP=2, up to 4 replicas | ~7.6 GB | guided MemoryDelta JSON |
| Voice | Drama 3 | speech.cpp on the **host CPUs** | **Q8_0 (full 8-bit)** | 0 chips, 112 threads per replica | — | runs during any chip phase |
| Render | Remotion | node on host CPUs | — | 0 chips | — | 224 cores |

- **Using everything at once:** the chips run one phase at a time (`Vision → Director → Librarian`, the
  existing phase arbitration in `pool/autoscaler.py`), while the host CPUs run Drama 3 and Remotion for approved
  chapters in parallel. Voice is the longest stage and never waits for chips.
- **Drama 3 on the chips** (Fish's PyTorch code under TorchTPU; 35B MoE, bf16 ≈ 70 GB → TP=8 ≈ 8.8 GB per chip)
  is a later spike. It replaces the CPU placement only if it is faster at equal quality.
- **Director throughput:** decode is bound by HBM bandwidth (819 GB/s per chip, ~8.2 GB read per chip per token):
  an estimated **40–70 tokens/s per stream before batching**, against 5.2 on the T4s. Continuous batching lets
  scenes of different chapters (speculative chaining) share the chips. Spike 9 measures it.

## Context budget per mode (the dynamic-context system, architecture §5)
The tiers (T1 receipts → T2 chapter summaries → T3 arc recaps), the overflow order and "never truncate silently"
are the same in every mode. Only the ceiling changes, and it comes from memory, not from the model (Qwen3-VL's
native window is 256K):

| Mode | Director context ceiling | Source |
|---|---|---|
| GPU (Q6_K, q8 KV) | 16,384 tokens | measured, spike 7 |
| TPU (bf16, TP=8) | 64K planned (KV ≈ 33 KB per chip per token: ~100K fit; 64K leaves room for batching) | spike 9 measures |
| CPU (TPU host, Q8_0) | 32K | spike 8 |

A scene of 20 panels at 512 px plus its OCR text is ~5.9K tokens (spike 7), so the GPU window holds one scene
with its memory tiers, and the TPU window holds several scenes or a much larger history.

## Registry and sessions
- `model_specs.json` gets one entry per **(model, mode)**: engine, precision, lease and measured footprint. The
  pool already detects chips, GPUs and cores at boot (`PoolConfig`), so the mode follows from the host the
  session landed on.
- The app keeps a TPU session request queued while it works on GPU. Kaggle runs a GPU and a TPU session at the
  same time (observed 2026-10-27: the TPU probe started while spike 7b held the T4s), so when the TPU session
  starts, new chapters route to it and the GPU session finishes what it holds.
- **Quality reference:** TPU outputs are full precision. GPU outputs (quantized) are compared against them on
  the same chapters whenever both modes have run them.
- **Boot speed:** bf16 weights are staged once into `/dev/shm` (164 GB holds Qwen3-VL-32B, Qwen2.5-7B, Magi and
  Drama 3 Q8_0 together). The XLA compile cache (`VLLM_XLA_CACHE_PATH`) is saved as a private dataset between
  sessions.

## Risks
- Both vLLM TPU engines call v5e **experimental**. Qwen3-VL on v5e is unverified: an old report of a vision
  encoder failure (tpu-inference #2468) predates the Qwen3-VL fixes in tpu-inference v0.26. If neither engine
  serves it, the Director falls back to llama.cpp Q8_0 on the host CPUs (spike 8) while the chips run Magi and
  the Librarian.
- TorchTPU is new (Oct 2026). Magi's remote code may need small changes for it.

## Spikes
- **Spike 8** (queued): Drama 3 Q8_0 and Qwen3-VL-32B Q8_0 on the host CPUs.
- **Spike 9** (queued after this ADR): Qwen3-VL-32B bf16 TP=8 on the chips with tpu-inference, then
  vllm-torchtpu if the first fails; the same three scenes and schema as spike 7b; load time, tokens/s,
  context ceiling and compile time.
