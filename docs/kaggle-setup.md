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

## 3. How the worker gets the gist token (automatic)

The worker needs the GitHub token to publish its rendezvous record. Kaggle's API cannot attach Kaggle Secrets to a kernel, so the app delivers the token itself:

- Before each launch it makes sure a **private** dataset `<you>/jumbo-secrets` holds one file, `gist_token`. It uploads only when the token changed, and each upload deletes the older versions, so a rotated token does not linger in the dataset history.
- Every kernel it pushes lists that dataset as a source, and the worker reads it from `/kaggle/input/`.
- The kernel itself (`<you>/jumbo-worker`) is created by the first push if it does not exist yet.

Only the gist-scoped token goes into the dataset. Anyone with access to your Kaggle account could read it, which is why it must have nothing but the `gist` scope. A Kaggle Secret named `JUMBO_GIST_TOKEN`, if you attach one by hand, still works as a fallback.

## 4. In the desktop app (Manhwa Studio → Session)

1. **Credentials:** import `kaggle.json` (or paste an API token), then paste the GitHub token. Both go into the system credential store (Windows Credential Manager, macOS Keychain or the Linux Secret Service), or a private file in the app settings folder when there is none.
2. **Bridge settings:** set the kernel `<you>/jumbo-worker` and the gist id.
3. Pick **Kaggle CPU** first and press **Launch worker**. This tests the whole cloud path (push, queue, cloudflared tunnel, rendezvous, signed API) without spending TPU quota.
4. When it shows **Ready**, press **Run bridge test**, then **Stop session**.
5. Switch to **Kaggle TPU v5e-8** for real runs.

**Local worker** mode needs none of the above. The app starts the worker on this PC from `worker/.venv`.

## What a session costs

- **Idle:** the worker stops itself after 30 minutes without requests from the app.
- **Deadline:** every session ends by 8.5 hours, under Kaggle's 9-hour cap.
- **Stop session:** ends the kernel immediately, which returns the remaining quota.
