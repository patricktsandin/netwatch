#!/usr/bin/env bash
# Model-checks every Quint spec (ADR-0016): typecheck, random simulation, then an
# exhaustive TLC search over the spec's bounded state space.
set -euo pipefail
cd "$(dirname "$0")/../specs"
for spec in *.qnt; do
  echo "== $spec"
  quint typecheck "$spec"
  quint run "$spec" --invariant=invariants --max-samples=20000 --max-steps=40 --seed=1
  quint verify "$spec" --backend=tlc --invariant=invariants
done
