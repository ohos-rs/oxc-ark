#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

run() {
  echo "+ $*"
  "$@"
}

cd "$repo_root"

run pnpm run format:check
run cargo clippy --workspace --all-targets -- -D warnings
run pnpm --filter @ohos-rs/oxk lint
run cargo test --workspace --lib --bins
run pnpm run build
run pnpm --filter @ohos-rs/oxk run build:oxlint-runtime
run pnpm --filter @ohos-rs/oxk run test:build
run pnpm --filter @ohos-rs/oxk run build --target wasm32-wasip1-threads
run pnpm --filter @ohos-rs/oxk run test:wasi

# Restore native generated wrappers after the WASM target build rewrites them.
run pnpm --filter @ohos-rs/oxk run build:debug

run git diff --check
run git diff --cached --check
