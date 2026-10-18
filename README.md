# Project Jumbo

Automated pipeline that turns raw manhwa PDFs (webtoon strips and page-based chapters) into narrated Hinglish motion-graphic recap videos.

- **Control center:** lightweight Tauri v2 (Rust + React) desktop app. All project state lives locally.
- **Worker:** ephemeral Kaggle TPU v5e-8 session running Magi v2, Qwen2.5-VL-72B, a Memory-R1 librarian, OmniVoice and Remotion.
- **Bridge:** automated Cloudflare quick tunnel with a sealed GitHub Gist rendezvous.

See [docs/architecture.md](docs/architecture.md) for the full design and roadmap.

## Repository layout

| Path | Contents |
|---|---|
| `packages/schema` | Shared zod contracts (`PanelsDoc`, `ScriptDoc`, `EpisodeSpec`, `MemoryDelta`) and their JSON Schema exports |
| `packages/motion` | `@jumbo/motion`, the Remotion component library (polygon panels, camera, animation presets) |
| `docs/` | Architecture and ADRs |

The Rust crates, the desktop app and the Python worker arrive in later roadmap steps.
