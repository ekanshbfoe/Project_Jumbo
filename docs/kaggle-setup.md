# Connecting Jumbo to Kaggle

One-time setup for running the worker on Kaggle from the desktop app. The protocol itself is described in [bridge-protocol.md](bridge-protocol.md).

## 1. Kaggle account

- Verify your phone number in Kaggle settings. Kaggle requires it for internet access and accelerators.
- Get your API credentials: **Settings → API → Create New Token** downloads `kaggle.json`.

## 2. Rendezvous gist (GitHub)

1. Create a **classic** personal access token with only the **`gist`** scope.
2. Create a **secret** gist at gist.github.com with any placeholder file (for example `README.md` containing "jumbo").
3. Copy the gist id, the last part of its URL.

The worker writes `session-<id>.json` files here. They hold only an encrypted tunnel URL and a public key, and the worker deletes them when it stops.

## 3. The worker kernel (once, in the Kaggle UI)

1. Create a new notebook and name it `jumbo-worker` (the slug becomes `<you>/jumbo-worker`). Save it.
2. In the notebook's **Add-ons → Secrets**, add `JUMBO_GIST_TOKEN` with the GitHub token, and attach it to this notebook.

The app pushes new versions of this kernel for every session, so you never edit it by hand. Kaggle's API cannot attach secrets, which is why this one step is manual. One thing still needs confirming: that the secret stays attached across versions pushed through the API. The first Kaggle CPU run will show it either way. A missing secret fails fast, with "No JUMBO_GIST_TOKEN" shown on the Session screen.

## 4. In the desktop app (Manhwa Studio → Session)

1. **Credentials:** import `kaggle.json` (or paste an API token), then paste the GitHub token. Both go into Windows Credential Manager.
2. **Bridge settings:** set the kernel `<you>/jumbo-worker` and the gist id.
3. Pick **Kaggle CPU** first and press **Launch worker**. This tests the whole cloud path (push, queue, cloudflared tunnel, rendezvous, signed API) without spending TPU quota.
4. When it shows **Ready**, press **Run bridge test**, then **Stop session**.
5. Switch to **Kaggle TPU v5e-8** for real runs.

**Local worker** mode needs none of the above. The app starts the worker on this PC from `worker/.venv`.

## What a session costs

- **Idle:** the worker stops itself after 30 minutes without requests from the app.
- **Deadline:** every session ends by 8.5 hours, under Kaggle's 9-hour cap.
- **Stop session:** ends the kernel immediately, which returns the remaining quota.
