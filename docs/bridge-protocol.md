# Bridge protocol v1

How the desktop app (Rust, `crates/jumbo-bridge`) and the Kaggle worker (Python, `worker/`) find and trust each other over a Cloudflare quick tunnel. Test vectors that both implementations must pass live in `protocol/bridge-v1.vectors.json`.

## Keys

- **App:** a fresh X25519 keypair per session (`app_sk`, `app_pk`). Only `app_pk` ever leaves the PC.
- **Worker:** a fresh X25519 keypair per boot (`worker_sk`, `worker_pk`).
- **Session key `K`:**
  - `shared = X25519(own_sk, peer_pk)`, 32 bytes.
  - `K = HKDF-SHA256(ikm = shared, salt = "jumbo-bridge-v1", info = session_id, len = 32)`.

All binary values travel as standard base64 with padding.

## Session ticket

The launcher embeds the ticket in the kernel source it pushes. The ticket contains no secrets:

```json
{
  "proto": 1,
  "session_id": "<32 hex chars>",
  "app_pk": "<b64>",
  "deadline_s": 30600,
  "idle_s": 1800,
  "rendezvous": { "kind": "gist", "gist_id": "<id>" }
}
```

- `deadline_s`: seconds after boot when the worker stops itself. 8.5 h leaves a margin under Kaggle's 9 h cap.
- `idle_s`: the dead-man switch. The worker stops itself after this many seconds without an authenticated request.
- `rendezvous`: where to publish. The gist token itself never appears in the ticket; on Kaggle it comes from the private `jumbo-secrets` dataset the app attaches (see [kaggle-setup.md](kaggle-setup.md)), or a Kaggle Secret `JUMBO_GIST_TOKEN`.

## Rendezvous record

When its tunnel is reachable, the worker writes `session-<session_id>.json` to the private gist:

```json
{
  "proto": 1,
  "session_id": "…",
  "epoch": 1,
  "worker_pk": "<b64>",
  "sealed": "<b64 of libsodium sealed box to app_pk of {\"url\": \"https://…trycloudflare.com\", \"boot\": {…}}>",
  "ts": 1760860800
}
```

- `epoch` increases every time the tunnel is re-created. The app always follows the highest epoch it has seen.
- Only the app can open `sealed`, so the gist never exposes the tunnel URL.

## Request authentication

Every request except `GET /health` carries:

```
Authorization: Jumbo <session_id>.<ts>.<sig>
sig = hex(HMAC-SHA256(K, "v1\n" + METHOD + "\n" + PATH_AND_QUERY + "\n" + ts + "\n" + hex(blake3(body))))
```

- `ts` is Unix seconds. The worker rejects a `ts` more than 60 s from its own clock.
- WebSocket upgrades sign `GET` with an empty body.
- A leaked tunnel URL is useless without `K`.

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| GET | `/health` | Unauthenticated liveness: `{"ok": true, "proto": 1}` |
| GET | `/v1/session` | Session id, epoch, timings and host facts. `warming` is true while models load; the app waits for it to clear |
| GET | `/v1/pool` | Model pool snapshot: TPU phase, HBM ceiling, per-chip owner, reserved GB and peak, CPU use, replicas, queued requests, models that cannot start. `{"enabled": false}` on a worker without a pool |
| POST | `/v1/session/stop` | Start draining; the worker exits after in-flight work |
| POST | `/v1/blobs/missing` | `{"hashes": [...]}` → `{"missing": [...]}` |
| GET | `/v1/blobs/{hash}/status` | `{"received": n, "complete": bool}` (resume point) |
| PUT | `/v1/blobs/{hash}?offset=N&total=T` | Upload one chunk of at most 32 MiB. The final chunk is verified against `hash` (blake3) |
| GET | `/v1/blobs/{hash}` | Download, with `Range` support |
| POST | `/v1/jobs` | `{"kind", "input", "params"}` → `{"job_id"}`. The id is derived from the inputs, so retries are idempotent |
| GET | `/v1/jobs/{job_id}` | Job status and output blob |
| WS | `/v1/events` | Server pushes `job.progress`, `job.done`, `job.failed`, `session.draining` and `log`, plus pool events: `pool.status` (same body as `/v1/pool`, sent when it changes), `pool.phase`, `pool.replica_ready`, `pool.replica_failed`, `pool.hbm_soft` / `pool.hbm_ok` (throttle on/off) and `pool.hbm_kill` |

Blobs are content-addressed by lowercase-hex blake3. Chunking keeps every request under the quick tunnel's 100 MB body limit, and `Range` makes downloads resumable.
