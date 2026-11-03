#!/usr/bin/env bash
# Model-checks every Quint spec (ADR-0016): typecheck, random simulation, then an
# exhaustive TLC search over the spec's bounded state space.
set -euo pipefail
cd "$(dirname "$0")/../specs"
for spec in *.qnt; do
  echo "== $spec"
  quint typecheck "$spec"
  # A spec with constants is checked through each instance after its first module.
  mains=$(grep -oP '^module \K\w+' "$spec" | tail -n +2)
  for main in ${mains:-$(basename "$spec" .qnt)}; do
    echo "-- $main"
    quint run "$spec" --main="$main" --invariant=invariants --max-samples=20000 --max-steps=40 --seed=1
    quint verify "$spec" --main="$main" --backend=tlc --invariant=invariants
  done
done
