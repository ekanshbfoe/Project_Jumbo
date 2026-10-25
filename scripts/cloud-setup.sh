#!/usr/bin/env bash
# Sets up a fresh Debian/Ubuntu machine (e.g. an InstaBox VM) for Project Jumbo work.
# Safe to re-run: anything already installed is skipped, so run it again after a
# restart if system packages on the root disk were lost.
#
#   bash scripts/cloud-setup.sh              # install everything and run all checks
#   SKIP_DESKTOP=1 bash scripts/cloud-setup.sh   # skip the (slow, disk-hungry) Tauri build

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"

step() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
fail() { printf '\n\033[1;31mxx %s\033[0m\n' "$*" >&2; exit 1; }

SUDO=""
if [ "$(id -u)" -ne 0 ]; then SUDO="sudo"; fi

# 8 GiB of RAM runs out when rustc builds Tauri on every core.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

step "System packages"
# Tauri v2 on Linux needs WebKitGTK and friends; python3-venv is split out on Debian.
APT_PACKAGES=(
  build-essential pkg-config curl wget file git
  libssl-dev libxdo-dev librsvg2-dev libayatana-appindicator3-dev libwebkit2gtk-4.1-dev
  python3 python3-venv python3-pip
)
missing=()
for p in "${APT_PACKAGES[@]}"; do
  dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
done
if [ "${#missing[@]}" -gt 0 ]; then
  echo "installing: ${missing[*]}"
  $SUDO apt-get update -qq
  $SUDO DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${missing[@]}"
else
  echo "all present"
fi

step "Node"
command -v node >/dev/null || fail "node not found; install Node 20+ (CI uses 24)"
node_major="$(node -p 'process.versions.node.split(".")[0]')"
[ "$node_major" -ge 20 ] || fail "node $(node -v) is too old; need 20+ (CI uses 24)"
echo "node $(node -v)"

step "Rust"
# rustup lives under $HOME, which is on the persistent disk.
if ! command -v rustup >/dev/null; then
  if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
  else
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    . "$HOME/.cargo/env"
  fi
fi
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
rustup default stable
echo "$(rustc --version)"

step "JS dependencies"
npm ci

step "Python worker venv"
if [ ! -x worker/.venv/bin/python ]; then
  python3 -m venv worker/.venv
fi
worker/.venv/bin/pip install -q -e "worker[dev]"

step "Checks (same as CI)"
npm run typecheck
npm test
(cd worker && .venv/bin/python -m pytest -q)
cargo fmt --all --check
cargo clippy --workspace --exclude jumbo-desktop --all-targets -- -D warnings
cargo test --workspace --exclude jumbo-desktop

if [ "${SKIP_DESKTOP:-0}" != "1" ]; then
  step "Desktop app build"
  npm run build -w @jumbo/desktop
  cargo clippy -p jumbo-desktop -- -D warnings
fi

step "Done"
df -h "$ROOT" | tail -1 | awk '{print "disk: " $3 " used, " $4 " free of " $2}'
du -sh target 2>/dev/null | awk '{print "rust build output (target/): " $1 "  (cargo clean frees it)"}'
echo "Run the desktop app with: npm run tauri dev -w @jumbo/desktop"
