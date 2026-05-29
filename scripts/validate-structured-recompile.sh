#!/usr/bin/env bash
# Validate structured-recompile fidelity for editable CS2 scripts.
#
# For each `editableStructured` script, assemble its reversible .ts two ways and
# compare bytes:
#   - default mode  -> uses the embedded ASM trailer (== the original bytes)
#   - --strict-structured -> forces recompile from the recovered structured TS
# Any MISMATCH means the structured recovery does NOT round-trip byte-identically
# (a false-editable). With the recompile-fidelity gate in place this should be 0;
# it is the regression net for control-flow-recovery (relooper) work.
#
# Usage: scripts/validate-structured-recompile.sh <build> <subbuild> [sample] [cache_dir]
set -u
BLD=${1:?build}; SB=${2:?subbuild}; SAMPLE=${3:-80}
CACHE=${4:-/Users/robert/projects/alerion/cache/unpacked/$BLD}
BIN=./target/release/rs3-cache-rs
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT
RS3_CACHE_DIR=$CACHE "$BIN" --cache-dir "$CACHE" --data-dir data --build "$BLD" --subbuild "$SB" \
  transpile-scripts --out-dir "$OUT" --all-scripts >/dev/null 2>&1
python3 - "$OUT/transpile-diagnostics.json" "$OUT" "$SAMPLE" <<'PY'
import json,sys,os
d=json.load(open(sys.argv[1])); out=sys.argv[2]; n=int(sys.argv[3])
ed=[s for s in d['scripts'] if s.get('editableStructured')]
print(f"editable={len(ed)} (sampling {min(n,len(ed))})")
open(os.path.join(out,'.sample'),'w').write("\n".join(s['export_name'] for s in ed[:n]))
PY
ok=0; mismatch=0; err=0
while read -r name; do
  [ -z "$name" ] && continue
  ts="$OUT/$name.ts"; [ -f "$ts" ] || ts=$(ls "$OUT/${name}"*.ts 2>/dev/null | head -1)
  [ -f "$ts" ] || { err=$((err+1)); continue; }
  RS3_CACHE_DIR=$CACHE "$BIN" --cache-dir "$CACHE" --data-dir data --build "$BLD" --subbuild "$SB" \
    assemble-script --input "$ts" --output "$OUT/a.cs2" --no-verify >/dev/null 2>&1 || { err=$((err+1)); continue; }
  RS3_CACHE_DIR=$CACHE "$BIN" --cache-dir "$CACHE" --data-dir data --build "$BLD" --subbuild "$SB" \
    assemble-script --input "$ts" --output "$OUT/b.cs2" --strict-structured --no-verify >/dev/null 2>&1 || { echo "STRICT-FAIL $name"; err=$((err+1)); continue; }
  if cmp -s "$OUT/a.cs2" "$OUT/b.cs2"; then ok=$((ok+1)); else echo "MISMATCH $name"; mismatch=$((mismatch+1)); fi
done < "$OUT/.sample"
echo "RESULT build=$BLD ok=$ok mismatch=$mismatch err=$err"
[ "$mismatch" -eq 0 ]
