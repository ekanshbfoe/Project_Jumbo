# ADR 0001: Kaggle bridge, first live runs

**Date:** 2026-10-26 · **Status:** accepted for the bridge; TPU access open

## What was tested

`crates/jumbo-bridge/tests/live_kaggle.rs` (ignored by default) runs the desktop app's Launch path against real Kaggle, GitHub and Cloudflare:

secrets dataset → kernel push → Kaggle queue → worker boot → cloudflared quick tunnel → sealed gist rendezvous → signed session → 8 MiB echo job round trip → forged request rejected → Stop → kernel finishes → gist record removed.

## Results

| Run | Outcome | Ready after | 8 MiB up / down | Host |
|---|---|---|---|---|
| Kaggle CPU | pass | 42 s | 2.6 s / 1.0 s | 4 vCPU, 31.3 GB RAM |
| `TpuV5E8` requested | pass, but **ran on CPU** | 35 s | 1.8 s / 1.0 s | 4 vCPU, 31.3 GB RAM, no TPU devices, 0 s TPU quota used |

Bugs the live run found, now fixed: Kaggle omits enum fields that hold their first value (`{}` for a QUEUED kernel), and an omitted dataset status means NOT_YET_PERSISTED.

## TPU through the API

A batch kernel pushed through the API never received a TPU on this account. Five probes, all with `machine_shape: TpuV5E8` and `enable_tpu: true`:

| Probe | Result |
|---|---|
| Existing kernel (first created as CPU) | CPU image, CPU host |
| Same kernel, `docker_image_pinning_type: latest` | image pin kept, CPU host |
| Fresh kernel, `latest`, no image | CPU image, CPU host |
| Fresh kernel, explicit `gcr.io/kaggle-tpu-images/python-tpuvm` | replaced by the CPU image, CPU host |
| Interactive `CreateKernelSession` with `TpuV5E8` | no TPU time used |

For comparison, `NvidiaTeslaT4` through the same push path does get 2× Tesla T4 with CUDA, so accelerator selection works in general. Kaggle's maintainers attribute TPU-on-push failures to a pinned CPU image (Kaggle/kaggle-cli#1197) and say pushed TPU runs are batch sessions limited to one per user. Neither workaround changed the outcome here. The weekly TPU quota (20 h) shows the account has never run a TPU session.

## Decision

- The bridge design (quick tunnel + secret gist rendezvous + private secrets dataset) is verified live and stays as decided.
- The launcher sends `enableTpu`/`enableGpu` with the machine shape and `dockerImagePinningType: latest` for TPU, matching Kaggle's CLI and the maintainer's guidance.
- The worker's `tpu` boot fact is the check that a TPU was really granted; the app must not treat a TPU launch as TPU unless the worker reports it.
- **Open:** how this account gets a TPU from an automated launch. S0 spikes that need the TPU are blocked until then.
