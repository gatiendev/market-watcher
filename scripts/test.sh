#!/usr/bin/env bash
# Run the whole test suite: Rust (in Docker, no host toolchain needed) + Vitest.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "== backend tests =="
docker run --rm \
  -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/cargo \
  -e RUSTUP_HOME=/usr/local/rustup \
  -v mw-cargo:/cargo \
  -v "$ROOT/backend":/app \
  -v mw-target:/app/target \
  -w /app \
  rust:1-bookworm \
  /usr/local/cargo/bin/cargo test --color=always

echo
echo "== frontend tests =="
export PATH="$HOME/.hermes/tools/node-26.12.1-linux-x64/bin:$PATH"
cd "$ROOT/frontend"
npm test
