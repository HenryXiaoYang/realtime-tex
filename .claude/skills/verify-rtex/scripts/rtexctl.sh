#!/usr/bin/env bash
# rtexctl.sh launch|doctor|cleanup RUN_ID — one isolated rtex verification run.
# Scratch:  build/verify-run/RUN_ID   (binary link, generated project, session build dir)
# Evidence: build/verify-evidence/RUN_ID   (kept by cleanup)
set -euo pipefail
cmd=${1:?launch|doctor|cleanup}; run=${2:?run id}
root=$(git rev-parse --show-toplevel)
scratch=$root/build/verify-run/$run
evidence=$root/build/verify-evidence/$run
rtex=$root/target/release/rtex
case $cmd in
  launch)
    (cd "$root" && cargo build --release -p rtex-cli)
    mkdir -p "$scratch" "$evidence"
    "$rtex" gen-book --pages 10 --fonts lm-tfm --out "$scratch/project" >"$evidence/gen-book.txt" 2>&1
    git -C "$root" rev-parse HEAD >"$scratch/revision"
    echo "ready: project $scratch/project, evidence $evidence"
    ;;
  doctor)
    test -d "$scratch/project" || { echo "no run $run: launch first"; exit 1; }
    "$rtex" doctor
    echo "binary: $rtex ($("$rtex" --version))"
    echo "revision: launched at $(cat "$scratch/revision"), HEAD $(git -C "$root" rev-parse HEAD), $(git -C "$root" status --short | wc -l | tr -d " ") uncommitted files"
    newer=$(find "$root/crates" -name '*.rs' -newer "$rtex" | head -1)
    [ -z "$newer" ] || { echo "stale binary: $newer is newer than $rtex: launch again"; exit 1; }
    echo "project: $scratch/project ($(wc -c <"$scratch/project/main.tex") bytes of main.tex)"
    ;;
  cleanup)
    if [ -f "$scratch/serve.pid" ]; then kill "$(cat "$scratch/serve.pid")" 2>/dev/null || true; fi
    rm -rf "$scratch"
    echo "removed $scratch; evidence kept in $evidence"
    ;;
  *) echo "unknown command $cmd"; exit 2;;
esac
