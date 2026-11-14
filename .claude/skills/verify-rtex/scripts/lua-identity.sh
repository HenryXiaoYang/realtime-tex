#!/usr/bin/env bash
# lua-identity.sh RUN BASE_REV PROJECT...: for a change to tex/ that must not change output.
# Runs rtex verify and rtex probe on a copy of each project twice, with the support files of
# BASE_REV and with the working tree's, and compares every page list and live display list byte
# for byte, and the units file with the build folder's name masked. Exit 1 on any difference.
# Work goes to build/verify-run/RUN/lua-identity, the report to build/verify-evidence/RUN/.
set -euo pipefail
run=${1:?run id}; base=${2:?base revision}; shift 2
root=$(git rev-parse --show-toplevel)
w=$root/build/verify-run/$run/lua-identity; e=$root/build/verify-evidence/$run
rm -rf "$w"; mkdir -p "$w/base" "$e"
git -C "$root" archive "$base" tex | tar -x -C "$w/base"
rtex=$root/target/release/rtex; fail=0; : >"$e/lua-identity.txt"
for proj in "$@"; do
  n=$(basename "$proj"); cp -R "$proj" "$w/$n"
  for side in old new; do
    if [ $side = old ]; then export RTEX_TEXDIR=$w/base/tex; else unset RTEX_TEXDIR; fi
    "$rtex" verify --project "$w/$n" --build "$w/$n-$side-verify" >"$w/$n-$side-verify.txt" 2>&1 || true
    "$rtex" probe --project "$w/$n" --build "$w/$n-$side-probe" --n 20 >"$w/$n-$side-probe.txt" 2>&1 || true
  done
  unset RTEX_TEXDIR
  files=0; diffs=0
  for f in "$w/$n-old-verify"/capture/main.rtex-page*.bin; do
    files=$((files+1)); cmp -s "$f" "${f/-old-verify/-new-verify}" || diffs=$((diffs+1))
  done
  [ "$(ls "$w/$n-new-verify"/capture/main.rtex-page*.bin | wc -l)" = "$(ls "$w/$n-old-verify"/capture/main.rtex-page*.bin | wc -l)" ] || diffs=$((diffs+1))
  files=$((files+1))
  cmp -s <(sed "s/$n-old-verify/$n-X-verify/g" "$w/$n-old-verify/capture/main.rtex.json") \
         <(sed "s/$n-new-verify/$n-X-verify/g" "$w/$n-new-verify/capture/main.rtex.json") || diffs=$((diffs+1))
  live=0; ldiff=0
  for f in "$w/$n-old-probe"/*.dl; do
    [ -e "$f" ] || continue
    live=$((live+1)); cmp -s "$f" "${f/-old-probe/-new-probe}" || ldiff=$((ldiff+1))
  done
  line="$n: capture files $files, differing $diffs; live lists $live, differing $ldiff; $(grep -E 'layer 1' "$w/$n-new-verify.txt" | sed -E 's/layer 1 \(fast vs extractor\): //' | cut -c1-90)"
  echo "$line" | tee -a "$e/lua-identity.txt"
  [ "$diffs" = 0 ] && [ "$ldiff" = 0 ] || fail=1
done
exit $fail
