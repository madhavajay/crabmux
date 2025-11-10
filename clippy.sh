#!/usr/bin/env bash
set -euo pipefail

# Run from the repo root so cargo picks up the correct workspace
SCRIPT_DIR="$(cd -- "$(dirname "$0")" >/dev/null 2>&1 && pwd)"
cd "$SCRIPT_DIR"

# Enforce formatting for the entire workspace
cargo fmt --all

# Lint everything (lib, bins, tests, benches, examples), treat warnings as errors
cargo clippy \
  --fix \
  --allow-dirty \
  --all-targets \
  --all-features \
  --no-deps \
  -- -D warnings
