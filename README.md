# Project Jumbo

Automated pipeline that turns raw manhwa PDFs (webtoon strips and page-based chapters) into narrated Hinglish motion-graphic recap videos.

- **Control center:** lightweight Tauri v2 (Rust + React) desktop app. All project state lives locally.
- **Worker:** ephemeral Kaggle TPU v5e-8 session running Magi v2, Qwen2.5-VL-72B, a Memory-R1 librarian, OmniVoice and Remotion.
- **Bridge:** automated Cloudflare quick tunnel with a sealed GitHub Gist rendezvous.

See [docs/architecture.md](docs/architecture.md) for the full design and roadmap.

## Repository layout

| Path | Contents |
|---|---|
| `packages/schema` | Shared zod contracts (`PanelsDoc`, `ScriptDoc`, `EpisodeSpec`, `MemoryDelta`, `LedgerEvent`, `SessionTicket`), their JSON Schema exports and cross-language fixtures |
| `packages/motion` | `@jumbo/motion`, the Remotion component library (polygon panels, camera, animation presets) |
| `crates/jumbo-contracts` | Rust types generated at build time from `packages/schema/json` (typify) |
| `crates/jumbo-core` | Project folder layout, manifests, chapter state machine, single-writer lock, job journal |
| `crates/jumbo-ledger` | The Receipt Book: hash-chained memory ledger, delta validation, views, snapshots and lock |
| `crates/jumbo-cli` | `jumbo` developer CLI over projects and the Receipt Book |
| `crates/jumbo-bridge` | Desktop side of the Kaggle bridge: launcher, rendezvous, signed worker client, boot state machine |
| `worker/` | Python worker for Kaggle: signed API, blob transfer, jobs, cloudflared tunnel, gist rendezvous, generated pydantic contracts |
| `protocol/` | Bridge test vectors shared by the Rust and Python implementations |
| `apps/desktop` | Tauri v2 + React desktop app (hub shell, chapters, Receipt Book) |
| `docs/` | Architecture, bridge protocol and Kaggle setup (ADRs go in `docs/adr/`) |

Setting up Kaggle for the bridge: [docs/kaggle-setup.md](docs/kaggle-setup.md).

## Development

On a fresh Debian or Ubuntu machine, `bash scripts/cloud-setup.sh` installs everything below and runs the same checks as CI.

```sh
npm install
npm test && npm run typecheck          # TypeScript packages
cargo test --workspace                  # Rust crates
npm run tauri dev -w @jumbo/desktop     # desktop app (Vite + Tauri)
cargo run -p jumbo-cli -- --help        # developer CLI
python -m venv worker/.venv && worker/.venv/bin/pip install -e "worker[dev]"   # worker (Windows: .venv/Scripts/)
cd worker && .venv/bin/python -m pytest   # worker tests
npm run studio -w @jumbo/motion         # Remotion Studio
npm run codegen                         # after changing packages/schema: JSON Schema + worker pydantic models
```
