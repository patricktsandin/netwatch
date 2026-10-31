#!/usr/bin/env bash
# The definition of done (CLAUDE.md, "Workflow"): every check, failing fast.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo verus verify -p netwatch-verified
cargo kani
./scripts/check-specs.sh
