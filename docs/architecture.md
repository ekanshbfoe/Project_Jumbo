# Project Jumbo — Architecture: Automated Manhwa → Motion-Recap Pipeline

## Context

Project Jumbo turns raw manhwa PDFs (vertical webtoon strips **and** page-based layouts) into narrated Hinglish motion-graphic recap videos.
- All heavy work (vision, LLM, TTS, Remotion rendering) runs on an **ephemeral Kaggle TPU v5e-8** session.
- A **lightweight Rust desktop app (Tauri v2 + React)** is the control center. It is the only place that stores persistent state.
- The two are connected by an **automated Cloudflare quick tunnel**, with no custom domain and no manual URL copy-paste.

**Status:** approved 2026-10-18. Implementation follows the roadmap in section 8.

Decisions taken during design review:
- **UI:** Tauri v2 + React.
- **Bridge:** quick tunnel with an automated handshake through a private rendezvous.
- **Input formats:** mixed (webtoon strips and page-based).

---

## 0. Hard technical realities that shape the design (read first)

These are facts about the hardware and models. The design below is built around them.

| # | Reality | Design consequence |
|---|---|---|
| R1 | A TPU v5e-8 is **8 chips × 16 GB HBM**, not one 128 GB pool. **A chip is owned by exactly one process** (libtpu lock). | The HBM ceiling (`hbm_ceiling` in `worker/model_specs.json`: 0.80 by default, 0.90 if S0 spike 6 passes its soak) is enforced **per chip** (12.8 or 14.4 GB/chip), so the total stays under it by construction. The worker pool schedules **chip leases**, not shared VRAM. |
| R2 | Qwen2.5-VL-72B in int8 is about 76 GB of weights. It needs tensor parallelism of 8 (64 attention heads / 8 KV heads), which is about 9.5 GB/chip and leaves about 3.3 GB/chip under the cap for KV cache. | While Qwen is active it **holds all 8 chips**. Nothing else can use the TPU at that time, so other stages run on the **host CPU** (many vCPUs) or in separate TPU phases. "Dynamic scaling" means phase-based chip leasing plus CPU-side concurrency. |
| R3 | Magi v2 outputs **axis-aligned boxes** (panels, text, characters) plus OCR and speaker links. It does **not** output polygons. It was trained on manga *pages*, not borderless webtoon strips. | Add a **polygon refinement stage**: OpenCV gutter/contour analysis, with SAM-2 inside the Magi box as a fallback. Add a **layout router** that slices webtoon strips into pseudo-pages before Magi runs. |
| R4 | Qwen2.5-VL native context is 32,768 tokens. Images cost tokens in proportion to their pixel count. | The 70% ceiling is about **22.9K tokens**, covering everything including the output reserve. A deterministic **Context Assembler** enforces it with the real tokenizer and caps image pixels per panel. |
| R5 | Memory-R1 is a research method (ADD/UPDATE/DELETE/NOOP memory manager on a 7-8B base). Released weights need to be verified. | The Librarian sits **behind an interface**. Fallback: Qwen2.5-7B-Instruct, or the 72B itself, with grammar-constrained output in the same op schema. Every op is **validated deterministically** before it touches the ledger. |
| R6 | Kaggle TPU sessions: roughly a 9 h hard cap and a weekly quota (~20 h). Machines are queued and ephemeral. | Worker has a **dead-man switch** (idle timeout) and a **hard deadline auto-Stop** at about 8h30m. Every intermediate artifact streams back to local, so a dead session loses only in-flight work. |
| R7 | Quick tunnels (`trycloudflare.com`) have about a **100 MB request-body limit**, **no SSE**, about 200 concurrent requests, and no uptime SLA. | Use **WebSocket** for events, **chunked, resumable, content-addressed** uploads, and **HTTP Range** downloads. Handle tunnel loss with **epoch republish** and automatic reconnect. |
| R8 | Running vLLM / torch-xla on TPU varies by model (dynamic shapes cause recompiles). | A **Phase-0 spike track** (inside Step 1) decides TPU vs CPU per model. The `ModelSpec` registry makes that a config change, not a code change. |

Operational risks to note:
- Check Kaggle's acceptable-use terms for long-running tunnel servers.
- Remotion needs a paid company license above 3 employees.
- Recapping copyrighted manhwa publicly risks platform claims.

---

## 1. Tech Stack

### 1.1 Desktop (local control center)
- **Shell:** Tauri v2 (Rust core; WebView2 on Windows, WebKitGTK on Linux). About 40–80 MB RAM, sub-second startup, MSI/NSIS bundler.
- **Frontend:** React 19 + TypeScript + Vite, styled with plain CSS (`src/styles.css`). Heavier libraries are added only when a screen needs them:
  - React Flow for the character and faction relationship graph.
  - Monaco diff editor for the script and memory-delta diffs.
  - **`@remotion/player`** for live motion preview of the exact `EpisodeSpec` inside the Approval Gate. This is DOM playback, not encoding, so it is consistent with "no rendering locally" and catches bad animation before TPU time is spent on it.
- **IPC (UI ↔ Rust):** Tauri commands for request/response, plus Tauri events for streaming progress and logs. The typed TS client is hand-written in `apps/desktop/src/api.ts`, next to the Rust command structs it mirrors.
- **Rust crates:**
  - Runtime and networking: `tokio`, `reqwest` (rustls), `tokio-tungstenite`.
  - Data and files: `serde`/`serde_json`/`toml`, `blake3` (content addressing), `notify` (fs watch), `fs4` (file locks).
  - Crypto: `x25519-dalek` + `hkdf` + `hmac`/`sha2` + `chacha20poly1305` (handshake crypto).
  - Credentials: **`keyring`** stores `kaggle.json` and the GitHub token in the OS store (Windows Credential Manager, macOS Keychain, Linux Secret Service). Without one, they fall back to an owner-only file in the app config dir. Never in the project.
  - Other: `tracing`.
- **Hub architecture:** Cargo workspace with a `HubModule` trait (`id`, `nav entry`, `commands`, `settings schema`, `lifecycle hooks`). Modules are compiled in through Cargo features. The frontend lazy-loads one route bundle per module. Manhwa Studio is module #1, and later tools plug in the same way without touching the core.

### 1.2 Kaggle worker
- **Python 3.11**.
  - **FastAPI + Uvicorn** (single process, asyncio) as the control plane: REST for jobs and assets, WebSocket for events.
  - **pydantic v2** for models generated from the shared schema.
  - JSON lines over each replica subprocess's stdin/stdout for RPC to model workers (stdlib only; the child moves fd 1 to stderr so library output cannot corrupt the protocol).
- **Inference:**
  - Three compute modes, CPU, GPU and TPU, run the same pipeline and contracts (ADR 0004).
  - **Director:** Qwen3-VL-32B Instruct. On the T4s: llama.cpp, Q6_K across both cards (spike 7b). On the TPU: vLLM on the chips, bf16 at TP=8 (spike 9), with llama.cpp Q8_0 on the host CPUs as its fallback. Output is constrained to the ScriptDoc schema.
  - **Librarian:** Qwen2.5-7B Instruct (chosen in spike 6), guided MemoryDelta output; vLLM at TP=2 on the TPU.
  - **Magi v2:** on the T4s, or on the TPU through TorchTPU (which replaces torch-xla), or on CPU.
  - **Voice:** Fish Audio Drama 3 through speech.cpp (ggml): Q5_K split over both T4s (ADR 0003), or Q8_0 on the TPU host's CPUs alongside the chips. Drama 3 on the chips through TorchTPU is a later spike.
- **Vision support:**
  - PyMuPDF for rasterization.
  - OpenCV for strip slicing, gutters and contours, and polygon simplification.
  - SAM-2 (optional) for polygon refinement.
  - LaMa (optional, CPU) for speech-bubble inpainting to produce "clean" panels.
- **Render:** Node 20 + `@remotion/renderer` (`renderMedia`), invoked as a subprocess. It runs on host CPU cores with high concurrency, so it overlaps with TPU work.
- **Bootstrap:** the app embeds the worker's source in the kernel it pushes (`crates/jumbo-bridge/src/bootstrap.py`), so every session runs exactly the worker version the app was built with.
- **Boot speed:** pip wheelhouse, Node `node_modules` tarball, the `cloudflared` binary, model weights (pre-quantized), and the XLA persistent compilation cache all ship as **private Kaggle Datasets/Models**. They are attached through `kernel-metadata.json` `dataset_sources`/`model_sources`, so nothing large is downloaded at boot.

### 1.3 Shared contract (single source of truth)
- **`packages/schema`**: zod schemas for `EpisodeSpec`, `PanelsDoc`, `ScriptDoc`, `MemoryDelta`, `LedgerEvent`, and `SessionTicket`.
  - Exported to JSON Schema, then generated into **pydantic** (`datamodel-code-generator`) and **Rust** (`typify`).
  - One contract is used by Qwen post-processing, Remotion, the Tauri preview, the ledger validator, and the worker API. Each payload carries a `schema_version`.

### 1.4 Monorepo layout
```
project-jumbo/
  apps/desktop/            # Tauri v2 (src-tauri/ Rust shell + src/ React)
  crates/
    jumbo-core/            # project fs, manifests, state machines, locks
    jumbo-ledger/          # event-sourced Receipt Book, validator, snapshots
    jumbo-bridge/          # Kaggle launcher, rendezvous, auth, transport, journal
    jumbo-hub/             # HubModule trait + registry
    mod-manhwa/            # Manhwa Studio module (commands, orchestration client)
  packages/
    schema/                # zod contract → JSON Schema → pydantic / Rust
    motion/                # @jumbo/motion Remotion component library
  worker/
    jumbo_worker/          # app, jobs, pool/ (resource manager), pipeline stages, tunnel
    model_specs.json       # ModelSpec registry and the HBM ceiling (shipped in the worker bundle)
  docs/adr/                # architecture decision records
```

---

## 2. Bridge & Automated Headless Handshake

### 2.1 One-time setup (done once, guided by an in-app wizard)
1. The user imports `kaggle.json`, which is stored in the OS keychain.
2. The user creates a classic GitHub PAT with **only the `gist` scope**, stored in the keychain. The app creates one **secret gist** (`jumbo-rendezvous`) and stores its ID.
3. Kaggle's API cannot attach Kaggle Secrets, so the app keeps the gist token in a private dataset `<user>/jumbo-secrets` and attaches it to every kernel it pushes. The first push creates the `jumbo-worker` kernel. There are no manual Kaggle steps.

### 2.2 Per-session boot sequence
```
Desktop (Rust)                         Kaggle API        Worker notebook              GitHub Gist
 1 gen session_id + ephemeral X25519 keypair (app_priv/app_pub)
 2 ensure the private jumbo-secrets dataset holds the current gist token (upload if not)
 3 kernels/push  (bootstrap script embedding the worker source and the ticket {session_id, app_pub, proto};
                   TPU v5e-8, internet on, dataset/model sources) ───────▶ queued → running
                                                     4 install wheelhouse, start FastAPI on :8000
                                                     5 gen worker X25519 keypair
                                                     6 spawn cloudflared (supervised), parse URL
                                                     7 self-check: GET https://<url>/health via tunnel (DNS ready)
                                                     8 write session-<id>.json ──────────────────▶ {session_id, epoch:1,
                                                                                               worker_pub,
                                                                                               sealed(url, boot_info → app_pub), ts}
 9 poll gist (authenticated, ETag/If-None-Match every 3s; 304s are free) ◀─────────────────────
10 K = HKDF(X25519(app_priv, worker_pub), session_id)
11 open WSS with HMAC auth → worker verifies → "Authenticated"
12 worker streams model warm-up progress → "Ready"
```
- **Request auth:** `Authorization: Jumbo <session_id>.<ts>.<HMAC_K(method|path|ts|blake3(body))>` with a 60 s replay window. WebSocket auth happens on upgrade. A leaked tunnel URL is useless without K. The pushed kernel contains only the *public* key, so there are no secrets in the Kaggle version history.
- The `Rendezvous` trait (gist implementation now, ntfy or another relay later) keeps the relay swappable.

### 2.3 Boot state machine and failure recovery (in `jumbo-bridge`)
`Pushing → Queued → Running → AwaitingRendezvous → Authenticating → Warming → Ready → (Draining → Consolidating → Closed)`

| Failure | Detection | Recovery |
|---|---|---|
| Push or quota error | API error | Show the reason (quota, auth). No retry loop on quota errors. |
| Stuck in queue | Timeout (configurable, default 20 min) | Offer cancel or keep waiting. |
| Notebook crash during boot | `kernels/status` = error (polled every 30 s out-of-band) | Fetch `kernels/output` log, show it in the UI, retry once with backoff. |
| No rendezvous record | Timeout (10 min) while the kernel is running | Fetch the partial log. Common cause: missing secret. Show a guided fix. |
| Tunnel dies mid-session | WS close / ping timeout | The worker's cloudflared supervisor restarts it and republishes with **epoch+1**. The app enters `Reconnecting`, watches the gist for a higher epoch, and resumes. |
| Worker or session dies | Kernel status complete/error, or no epoch for 3 min | `SessionLost`. Local state is intact because everything is checkpointed. "Relaunch & Resume" replays the **local job journal** and re-sends only missing blobs. |
| Kaggle hard limit approaching | Worker clock | Automatic Stop/Consolidate at T-30 min. |
| App closed or PC asleep | No authenticated traffic for N min (default 30) | Worker dead-man switch: finish in-flight jobs, park results, shut down to save quota. Results are fetched on the next session, or re-run from the journal. |

### 2.4 Transport
- **Assets are content-addressed by blake3.** The app sends a manifest, the worker answers with the missing hashes, and uploads go in **32 MB chunks** (under the 100 MB tunnel limit) with resume.
- Results stream back as they are produced (panels JSON, crops, script drafts, deltas, audio clips, MP4) using Range-resumable GETs. Every artifact is checksummed and written atomically (temp file + fsync + rename) into the project.
- **WebSocket events:** `job.progress`, `job.done`, `gate.ready`, `pool.status` (chip/CPU usage, for a live resource meter), `log`.

---

## 3. Local Directory Schema (single source of truth)

```
<ProjectRoot>/
  jumbo.project.toml            # id, title, schema_version, language=hinglish, voice profile, style defaults, arc boundaries
  .jumbo/
    project.lock                # single-writer OS file lock (prevents two app instances)
    journal/  jobs.jsonl        # append-only job journal → crash recovery & resume
    blobs/    <blake3>          # content-addressed cache (dedupes uploads/downloads)
    sessions/ <session_id>.json # session record: chapters touched, status, consolidation state
  chapters/
    ch_0001/
      chapter.toml              # state: imported→rasterized→analyzed→directed→librarian→approved→voiced→rendered→consolidated (+stale flag)
      source/   original.pdf
      pages/    page_001.webp … # rasterized pages OR strip slices (layout: "strip"|"page")
      panels/
        panels.json             # PanelsDoc: ids p_0001…, reading order, bbox, polygon (normalized), bubbles, chars, OCR, speaker
        crops/  p_0001.webp
        clean/  p_0001.webp     # optional bubble-inpainted
        chars/  c_0003_p_0001.webp
      script/
        draft.v1.json …         # Director output versions (ScriptDoc: lines, beats, presets, tone tags)
        notes.md                # Qwen raw episodic notes
        approved.json           # frozen at Approval Gate (immutable once voiced)
      memory/
        delta.proposed.json     # Librarian ops (provisional)
        delta.approved.json     # what was committed to the ledger
      audio/
        l_0001.wav …
        audio.manifest.json     # durations, sample rate, line→clip map
      render/
        episode.json            # EpisodeSpec = Remotion input props (paths are project-relative)
        ch_0001.mp4
        ch_0001.srt
  receipt_book/                 # "The Receipt Book" – master story state
    ledger.jsonl                # APPEND-ONLY event log; every op carries receipts (chapter, panel ids, quote)
    state/                      # materialized views – always regenerable by replaying ledger
      characters/ <char_id>.json
      factions.json  locations.json  relationships.json  items.json
      plot_threads.json         # open loops: status open|resolved, opened_at, receipts
    tiers/
      t1_active/  ch_0041.receipts.json   # full episodic receipts (recent chapters)
      t2_chapters/ ch_0001.summary.json   # factual chapter summaries
      t3_arcs/    arc_01.recap.json       # recursive arc recaps
    snapshots/
      s_0007/ manifest.json     # hash-chained checkpoint at each session end (ledger offset + state hash + tier files)
    LOCK.json                   # {locked: true, snapshot: s_0007, ledger_offset, hash} – set by Stop/Consolidate
  exports/                      # user-promoted final deliverables
  logs/                         # local app + fetched worker logs per session
```

**Key invariants**
- **The Rust app is the only writer** of `receipt_book/`. Kaggle only *proposes* deltas. The ledger validator rejects any op that:
  - references an unknown entity ID,
  - lacks a receipt pointing to an existing panel, or
  - contradicts a locked fact without an explicit UPDATE citing new evidence.
- `state/` is always rebuildable from `ledger.jsonl`, and snapshots are hash-chained. Corruption is detectable and repairable.
- All writes are atomic. `chapter.toml` state transitions are the resume points.

---

## 4. Kaggle Dynamic Worker Architecture

### 4.1 Components (inside `jumbo_worker`)
```
FastAPI control plane ──▶ Pipeline Orchestrator (per-chapter DAG, asyncio)
                               │ enqueue StageJobs
                               ▼
                    Stage Queues (priority, idempotent jobs keyed by input hash)
                               │
                    Resource Manager ── ChipLedger (8 × ceiling × 16 GB budget) + CpuPool (cores/RAM)
                               │            ▲
                    Autoscaler ┘            └── Memory Watchdog (per-chip HBM sampling)
                               │
                    Worker Replicas = subprocesses bound to chip subsets / CPU cores
                               │
                    Weight Cache: /kaggle/input (disk) → /dev/shm (host RAM) → HBM
```

### 4.2 ModelSpec registry (`worker/model_specs.json`, profiled values filled in by the spikes)
| model | backend | chips/replica | HBM/chip (profiled) | CPU fallback | min/max replicas | batchable |
|---|---|---|---|---|---|---|
| magi_v2 | torch-xla or cpu | 1 | ~3–5 GB | yes | 0 / 8 | yes |
| qwen3vl_32b | vllm-tpu (bf16) / llama.cpp (Q6_K on 2 T4s, Q8_0 on CPU) | 8 (TP=8) | ~8.2 GB weights + KV | host CPU (Q8_0) | 0 / 1 | continuous batching |
| librarian (Qwen2.5-7B) | vllm-tpu | 2 (TP=2) | ~7.6 GB | slow | 0 / 4 | yes |
| drama3 | ggml (speech.cpp, CUDA) | 2 T4s, one Q5_K replica split | ≤ ceiling (11.5 GiB per card), 8 of 40 layers' experts on CPU | experts only | 0 / 1 | one line at a time |
| remotion | node/cpu | 0 | — | n/a | 0 / ⌊cores/concurrency⌋ | per-chapter |

### 4.3 How the HBM ceiling is guaranteed (three layers)
The ceiling is one config value, `hbm_ceiling` (default 0.80; spike 6 may raise it to 0.90). The per-chip budget, allocator caps and watchdog thresholds all derive from it (`jumbo_worker/pool/`).

1. **Admission control (static):** the registry refuses any model whose `hbm_gb_per_chip` exceeds `hbm_ceiling × 16 GB`, and chips are leased whole, as aligned 1/2/4/8-chip slices, best-fit. Chips are process-exclusive (R1), so per-chip correctness implies the total stays under the ceiling.
2. **Framework caps (runtime):** each replica starts with its allocator capped at the ceiling:
   - vLLM `gpu_memory_utilization` = ceiling (passed as `JUMBO_GPU_MEMORY_UTILIZATION`).
   - JAX `XLA_PYTHON_CLIENT_MEM_FRACTION` = ceiling with `PREALLOCATE=false`.
   - torch-xla equivalent PJRT limits.
   - Chip binding through `TPU_VISIBLE_CHIPS`, `TPU_CHIPS_PER_PROCESS_BOUNDS`, `TPU_PROCESS_BOUNDS`, and `TPU_PROCESS_PORT`.
3. **Watchdog (enforcement):** samples `device.memory_stats()` (JAX) or `xm.get_memory_info` (torch-xla) every 1 s.
   - **Soft (≥ ceiling − 4 points, and above the replica's own profiled footprint):** stop admitting new work to it until it drops back. The footprint test matters for vLLM, which pre-allocates each chip up to the ceiling by design.
   - **Hard (above the ceiling):** kill the replica and retry its requests on another (jobs are idempotent). Exactly at the ceiling is allowed, since that is where the allocator caps put a fully used replica.
   - Peaks are recorded and surfaced in `pool.status`.

**On a GPU host (Kaggle T4 ×2, no chips)** accelerator models run on the CUDA cards, which replicas share, so the same three layers work per card:
1. **Placement:** every replica reserves its profiled `hbm_gb_per_chip` on one card (`GpuLedger`). A card never has more reserved than `hbm_ceiling × gpu_hbm_gb` (15 GB usable per T4, so 12 GB at 0.80), and new replicas go to the card with the most room left.
2. **Allocator cap:** each replica sees only its card (`CUDA_VISIBLE_DEVICES`), and torch is capped at the ceiling's share of it (`JUMBO_GPU_MEMORY_FRACTION`).
3. **Card watchdog:** each replica reports its own usage (NVML per process, or torch reserved plus the CUDA context), and the card is judged on the sum. Above the ceiling, the replica furthest beyond its footprint is killed and its request retried. At the soft limit, replicas beyond their footprint are throttled. `pool.status` lists `gpu_cards` with reserved, used and peak.

### 4.4 Sleep / offload / wake
- **Sleep = replica process exits** and releases its chips. Its weights **stay resident in host RAM** (`/dev/shm` safetensors, pre-quantized), so the 330 GB host RAM acts as the offload tier. The real host RAM and vCPU counts are read at boot, not assumed.
- **Wake = spawn, mmap from `/dev/shm`, load to HBM** (seconds per GB), with XLA compile time cut by the **persistent compilation cache**. The cache is saved back as a Kaggle Dataset version between sessions.
- If vLLM's native sleep mode works on TPU (checked in the spike), it can replace process exit for Qwen. Same interface.

### 4.5 Autoscaler policy (event-driven, runs on every queue or replica event plus a 2 s tick)
- `desired[stage] = clamp(ceil(queue_depth / target_per_replica), min, max)`.
- **Placement:** best-fit bin-packing onto free chips. CPU-capable stages overflow to the CpuPool when chips are leased.
- **Phase arbitration:** Qwen needs all 8 chips, so the scheduler runs in **TPU phases**:
  - `Vision-TPU` (Magi ×N packed onto 1-chip leases) ⇄ `Director` (Qwen3-VL, 8 chips) ⇄ `Librarian`. Drama 3 runs on the host CPUs during any phase (ADR 0004).
  - A phase switch happens only when the incoming phase's backlog × cost exceeds the swap cost plus the hysteresis margin, which prevents thrashing.
  - CPU work (Remotion, polygon refinement, inpainting, CPU-fallback models) keeps running alongside any TPU phase.
- **Idle TTL** per spec (e.g. 120 s) puts replicas to sleep. Stage priority follows pipeline depth, so the stage closest to a finished deliverable goes first.
- **Cross-chapter pipelining:** while Qwen directs chapter N on TPU, the CPU renders N−1 and slices or refines N+1.

### 4.6 Per-chapter pipeline DAG
`rasterize → layout_route(strip|page) → slice → magi_detect → polygon_refine → reading_order → crop/inpaint → assemble_context → direct (Qwen) → librarian (ops) → validate → GATE(await human) → voice (OmniVoice ×lines) → build EpisodeSpec → render (Remotion) → return`

- **Speculative chaining:** while the user reviews chapter N at the Gate, the worker directs N+1 against N's *provisional* memory state. If the user edits N's delta, N+1 and later are marked `stale` and re-run automatically. This keeps the TPU busy during human review.

---

## 5. Memory Continuity & Context Rot Defense

- **Context Assembler (worker, deterministic, real tokenizer).** Hard ceiling is 70% of 32,768, about 22,900 tokens, *including* the output reserve. Default budget:
  - system + style guide 1.5K
  - T3 arc recaps 1.5K
  - T2 chapter summaries 3K
  - T1 receipts 5K
  - relevant character/thread sheets 2.5K (retrieved through Magi character clusters + name matching)
  - current scene panels about 7K (8–12 panels, `max_pixels` capped about 600 tokens/panel)
  - output reserve 2.4K

  If the budget overflows, it reduces in this order: images (fewer panels per scene) → T2 → T3. **It never truncates silently.** Every prompt's token map is logged and shown in the app.
- **Tier lifecycle:**
  - When T1 exceeds its budget, the oldest T1 chapter is promoted to a T2 factual summary.
  - When T2 exceeds its budget, contiguous chapters within an arc boundary (user-defined or Qwen-proposed) are condensed into a T3 recap.
  - Full receipts are **never deleted**. They remain in the ledger, so a summary is only a view and can always be regenerated.
- **Librarian ops schema:** `{op: ADD|UPDATE|DELETE|MERGE|NOOP, entity_type, entity_id, field, value, receipts:[{chapter, panel_id, quote}], confidence}`. MERGE folds `from_id` into `entity_id` (duplicates, a speaker cluster later named): kept values win, missing fields and aliases come over, receipts are combined, references are re-pointed and the retired id can never be used again. Decoding is grammar-constrained and then validated by `jumbo-ledger` rules (§3 invariants). Low-confidence ops are highlighted at the Gate.
- **Stop / End Session workflow:**
  1. The app sends `session.stop`. The worker immediately sets `draining`, aborts in-flight Qwen/Librarian generation, and stops dequeuing generation jobs. Voice and render jobs for *already-approved* chapters may finish (user toggle).
  2. **Consolidation pass:** for every chapter newly approved this session, the worker writes T2 summaries, updates T3 if an arc closed, and runs a Librarian reconciliation pass. This produces a `consolidation.delta`.
  3. The app shows a compact consolidation review (auto-accept allowed if validation passes). It then applies the ledger ops atomically, rebuilds `state/`, writes snapshot `s_NNNN` (hash-chained), and sets `LOCK.json`.
  4. The worker cleans up the rendezvous gist file and the kernel exits, which frees quota.
  5. **If the worker is already dead,** the session is marked `unconsolidated` and the **first job of the next session is consolidation**, so continuity is never skipped.
- **Resume (days or weeks later):** the app verifies the snapshot hash chain against the ledger, then sends the locked snapshot along with the T1/T2/T3 files and the relevant state views in the session handshake.

---

## 6. Remotion Engine Design (`packages/motion`, `@jumbo/motion`)

**Status:** the component library is locked as of 2026-10-19. New components are added only when the pipeline needs them, on the project lead's design direction.

- **`EpisodeSpec` (zod)** is consumed directly as `inputProps`. `calculateEpisodeMetadata` validates it and derives the duration from beat lengths (sized from `audio.manifest.json`), and each preset's parameters are validated by its own zod schema.
- **Building blocks:**
  - `PolygonPanel`: normalized polygon → CSS `clip-path`, with an even-odd SVG mask for bubble cut-outs, an outline-following border and a configurable drop shadow.
  - `Camera`: virtual camera transform (translate, scale, in-plane rotation, 3D `rotateX`/`rotateY` with perspective only when used, opacity, pivot, stacking order).
  - `BeatView`: one narration line, made of a blurred backdrop, an ambient layer, panels laid out by the preset, the preset overlay and narration audio. There are no captions; subtitles were dropped.
  - `Episode`: beats in a `TransitionSeries` (cut, fade, slide, wipe), plus the two overlays below.
- **Episode overlays:**
  - `ChannelWatermark`: permanent, semi-translucent mark in the bottom-right for the whole video (`watermark`).
  - `SocialPromo`: frosted-glass follow card for X, YouTube and Instagram that flips in at 5% of the runtime and has animated out by 95% (`social_promo`). It stacks above the watermark when both sit bottom-right.
- **Ambient engine** (`ambient_effect`): `dust`, `smoke`, `city_bokeh` or `none`. Each is a pure, seeded function of the episode's master clock, so it flows continuously across cuts.
- **Preset registry:** each preset is a pure function `(frame, fps, duration, params, seed) → camera transform`, optionally with its own layout, shadow and overlay. All randomness uses Remotion `random(seed)`, never `Math.random()`, so renders are deterministic.

  | Preset | Panels | Behavior |
  |---|---|---|
  | `hold` | 1 | Static |
  | `zoom` | 1 | Push in or pull out toward a focus point |
  | `pan` | 1 | Camera drift |
  | `shake` | 1 | Continuous seeded shake |
  | `action_tilt` | 1 | Spring punch-in with a dutch tilt |
  | `kinetic_jitter` | 1 | Violent jitter that decays |
  | `panoramic_fade` | 1 | Wide pan that fades in and out |
  | `split_screen` | 2–3 | Equal slots, staggered spring entrances |
  | `floating_panels` | 2–3 | Tall panels layered like photos in 3D, heavy shadows, slow drift |
  | `horizontal_slices` | 2–3 | Stacked ultra-wide strips drifting in opposite directions |
  | `hud_popup` | 1 | Game-system message box that types on |

- **Studio compositions:** `Episode` (the render target and a sample chapter) and, in the Gallery folder, `Gallery` (every preset × panel shape), `Ambient-*` previews and `SocialPromo`. The preset catalog, with descriptions, goes into Qwen's system prompt.
- The same package is imported by the Tauri Approval Gate (`@remotion/player`) and by the worker's render script, so preview and final render are identical.

---

## 7. Desktop App (Manhwa Studio module) screens

- **Hub shell:** module nav, global session/connection badge with a live chip and CPU meter from `pool.status`, settings, credentials wizard.
- **Projects:** create or open a root folder, validate it against the schema, run migrations.
- **Chapters:** drag-drop PDFs, per-chapter state-machine timeline, stale flags, retry and re-run per stage.
- **Session:** Launch / Connect / Stop-Consolidate. Boot state machine view, Kaggle log tail, quota-time remaining.
- **Approval Gate:** panel strip with polygon overlays, script editor and diff (Monaco), memory delta as an accept/edit/reject per op with clickable receipts that jump to the panel, live `@remotion/player` preview, and an "Approve → voice & render" button.
- **Receipt Book:** character cards, faction and relationship graph (React Flow), open plot threads, tier browser (T1/T2/T3), snapshot history and diff.

---

## 8. Step-by-Step Implementation Roadmap

**Step 1: Contract + Remotion component library (start here).** Spike track S0 runs in parallel.
- 1a. `packages/schema`: `PanelsDoc`, `ScriptDoc`, `EpisodeSpec`, `MemoryDelta`, plus codegen to JSON Schema, pydantic, and Rust.
- 1b. `@jumbo/motion` primitives: `PolygonPanel`, `Camera`, `BeatView`, narration audio, transitions.
- 1c. Preset registry (the full list in §6), each with a zod params schema.
- 1d. Hand-authored fixtures (sample chapter pages, polygons, dummy TTS clips), plus the Gallery and a sample Episode composition. Development uses Remotion Studio locally (a dev tool only).
- 1e. Visual regression: `renderStill` at key frames compared against golden PNGs. Determinism test: rendering twice gives identical hashes.
- **S0 feasibility spikes on a real Kaggle TPU v5e-8** (they gate Steps 4–7):
  1. Qwen2.5-VL-72B int8 on vLLM-TPU at TP=8: load, per-chip HBM at util 0.8, tokens/s, image-token cost.
  2. Magi v2 on TPU vs CPU throughput.
  3. Voice: OmniVoice failed the Hinglish test (accent); Fish Audio Drama 3 passed (spikes 4 and 5, ADR 0003).
  4. Memory-R1 weight availability (otherwise the fallback).
  5. `kernels/push` with the secret staying attached, quick tunnel, and gist rendezvous ("hello worker").
  6. Remotion render throughput on Kaggle host CPUs.
  7. Actual host RAM and vCPU counts.

  Each result is recorded as an ADR and written into `worker/model_specs.json`.

**Step 2: Local core (Rust, no GUI yet).** `jumbo-core` (project manifest, dir schema, state machines, atomic writes, project lock, journal) and `jumbo-ledger` (event log, validator, materializer, snapshots plus hash chain, LOCK). Driven by a dev CLI and tests.

**Step 3: Bridge + worker skeleton.**
- `bootstrap.ipynb`, the FastAPI app with HMAC auth, the cloudflared supervisor, and the gist rendezvous.
- `jumbo-bridge`: Kaggle REST launcher, boot state machine, reconnect by epoch, chunked content-addressed transfer, WS events.
- A minimal Tauri shell with the Session screen.
- Exit criterion: one click goes from launch to `Ready` with an echo pipeline round-tripping a file.

**Step 4: Resource manager.** `ChipLedger`, `CpuPool`, `ModelSpec` loader, replica subprocess launcher with chip binding, autoscaler with phase arbitration and hysteresis, watchdog, weight cache in `/dev/shm`, compilation-cache persistence. First validated with **fake models** that declare footprints, then with real ones.

**Step 5: Vision stage.** Rasterize, layout router (strip vs page), strip slicer, Magi v2, polygon refinement (OpenCV, optional SAM-2), reading order, crops, optional LaMa inpaint, then `panels.json` streamed back.

**Step 6: Director + Librarian + memory tiers.** Context Assembler with token budget, Qwen prompts with the preset catalog and constrained `ScriptDoc` output, Librarian ops and validation, tier promotion, speculative chaining with stale invalidation.

**Step 7: Voice + render.** Drama 3 replicas (checked takes, retakes, silence trim; panels follow the audio), audio manifest, `EpisodeSpec` builder, Remotion `renderMedia` on the worker, MP4 and SRT returned.

**Step 8: Desktop UI.** Projects, Chapters, Approval Gate (diffs, op-level review, Player preview), and the Receipt Book visualizer.

**Step 9: Stop/Consolidate and resilience hardening.** Drain, the consolidation pass, LOCK, deadline auto-stop, dead-man switch, the unconsolidated-session path, and journal-replay resume.

**Step 10: Hub extraction and packaging.** Finalize the `HubModule` registry (Manhwa Studio as module #1), the settings and credentials wizard, the MSI/NSIS installer, and optional auto-update.

---

## 9. Verification

- **Step 1:**
  - `npx remotion render Gallery` produces an MP4 of every preset on every fixture shape.
  - `renderStill` golden-frame diffs pass.
  - Rendering twice gives byte-identical frames (determinism).
  - Fixture JSON that is invalid against the schema fails zod validation.
- **Step 2:**
  - Property tests: replaying `ledger.jsonl` always reproduces `state/`.
  - The validator rejects ops with missing or invalid receipts and unknown IDs.
  - Kill-during-write tests leave no corrupt files.
  - A tampered snapshot breaks the hash chain.
- **Step 3:**
  - From the app, launch to `Ready` with no manual input.
  - Chaos tests recover automatically: kill `cloudflared` (epoch reconnect), kill the kernel (`SessionLost` then relaunch & resume), unauthenticated requests rejected, and a 500 MB upload resumes after an interruption.
- **Step 4:**
  - A randomized load simulation with fake models, at ceilings 0.80 and 0.90, asserts that no chip is shared or over budget and that all work completes across phase switches (`worker/tests/test_pool_manager.py`).
  - On the real TPU, watchdog peak logs stay ≤ the ceiling per chip across a full chapter run, and sleep/wake timings are recorded.
- **Steps 5–7:** a golden sample chapter in each layout (strip and page) produces `panels.json` with polygons, a script, a validated delta, audio, and an MP4. Results are reviewed manually in the Gate and the Player.
- **End-to-end continuity test:**
  - Process chapters 1–3, press Stop, and confirm the snapshot is locked.
  - Reopen the project in a new session and process chapter 4.
  - Check that character and thread continuity holds (e.g. no re-introduction of established characters, open threads referenced).
  - Check that the context token map never exceeds 70%.
