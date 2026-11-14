#!/usr/bin/env bash
# lua-globals.sh FILE...: the global names each Lua file reads or writes, one line per file.
# Lua has no compiler to catch a local that a move or rename left behind; it shows up here as a
# new global. Compare the output before and after a change to tex/*.lua.
set -euo pipefail
for f in "$@"; do
  printf '%s: ' "$f"
  texluac -l -l -o /dev/null "$f" 2>&1 | grep -E "(GETTABUP|SETTABUP)" | grep -oE '; _ENV "[^"]+"' \
    | sed -E 's/; _ENV "([^"]+)"/\1/' | sort -u | tr '\n' ' '
  echo
done
