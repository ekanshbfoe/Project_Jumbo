# ADR 0002: S0 model availability (desk research)

**Date:** 2026-10-26 · **Status:** proposed; the TPU spikes confirm or overturn each line

What could be settled before any TPU run, from public model cards and the vLLM TPU project.

| Model | Finding | Consequence |
|---|---|---|
| Qwen2.5-VL-72B on vLLM TPU | vLLM's TPU backend (tpu-inference, now vllm-jax) lists Qwen2.5-VL as its only supported multimodal model, verified for the **7B** on a single v6e chip. The 72B is not validated on any TPU, and no published recipe covers v5e. | Spike 6 is the critical risk. If 72B does not run at TP=8 on v5e-8, the Director needs a different model or backend, which goes back to the project lead. |
| Qwen2.5-VL-72B int8 weights | Official quantized builds are AWQ **int4**. No official int8 GPTQ for 2.5-VL. A Keras port offers weight-only int8. | Spike 7 decides between int8 (on the fly or Keras), int4 AWQ, or bf16 sharded (about 144 GB, too big for 128 GB HBM). |
| Memory-R1 (Librarian) | No official code or weights found for arXiv 2508.19828. A follow-up, **Memory-R2 7B** (Qwen2.5-7B-Instruct + LoGo-GRPO, Apache-2.0), publishes a memory manager with INSERT/UPDATE/DELETE (no NOOP). | Per R5, the Librarian stays behind its interface. Candidates: Qwen2.5-7B-Instruct with constrained `MemoryDelta` decoding (the fallback), and Memory-R2 7B with INSERT mapped to ADD. Spike 4 compares them on the same chapter. |
| OmniVoice (voice) | `k2-fsa/OmniVoice`: 0.6B, 600+ languages (Hindi not named), 24 kHz, zero-shot cloning from a short reference clip plus its transcript. Documented for CUDA and Apple Silicon only. **Pretrained weights are CC-BY-NC.** No statement on code-switching. | Spike 3 checks Hinglish quality and whether it runs on TPU or only on CPU. **The licence forbids commercial use**; a monetised channel needs a different model or a licence. Hinglish-specific alternatives exist (XTTS-v2 Hinglish fine-tune, IndicF5-based). |

## Sources

- vllm-project/tpu-inference: support matrix, multimodal example, issue #2468 (Qwen3-VL fails on v5e; "Qwen2.5-VL is the only supported multi-modal")
- AI-Hypercomputer/tpu-recipes: Qwen2.5-VL on Trillium (7B verified, 72B not)
- huggingface.co/k2-fsa/omnivoice (model card, licence)
- huggingface.co/ahmedehabb/Memory-R2 (model card)
