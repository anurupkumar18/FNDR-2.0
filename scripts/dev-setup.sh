#!/usr/bin/env bash
# T-108: environment bootstrap. Takes a bare macOS checkout to a green
# `make test`. Safe to re-run; every step is idempotent.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v rustup >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  echo "Installing rustup (rust-toolchain.toml pins the toolchain version)..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup toolchain install >/dev/null 2>&1 || rustup toolchain install

if ! command -v node >/dev/null 2>&1; then
  echo "Node 20+ is required and not on PATH (use nvm, asdf, or brew)." >&2
  exit 1
fi
(cd ui && npm install)

make test
echo
echo "dev-setup: done. Read CONTRIBUTING.md before your first PR."
