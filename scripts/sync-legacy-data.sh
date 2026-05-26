#!/usr/bin/env bash
# Merge opcode/name data from another checkout into tools/rs3-cache-rs/data.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${RS3_CACHE_RS_DATA:-$ROOT/data}"
SOURCE=""

usage() {
  cat <<'EOF'
Usage: sync-legacy-data.sh <source-data-dir> [--delete-source <project-root>]

  source-data-dir   Directory containing opcodes-*.txt, names/, commands/, etc.

Copies into tools/rs3-cache-rs/data and prints a diff summary. Optional
--delete-source removes a project root after you confirm the merge.

Examples:
  ./scripts/sync-legacy-data.sh /path/to/external/rs3-cache/data
  ./scripts/sync-legacy-data.sh ../other-tool/data --delete-source ../other-tool
EOF
}

DELETE_ROOT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --delete-source)
      DELETE_ROOT="${2:?--delete-source requires a path}"
      shift 2
      ;;
    *)
      if [[ -z "$SOURCE" ]]; then
        SOURCE="$1"
        shift
      else
        echo "Unexpected argument: $1" >&2
        usage >&2
        exit 1
      fi
      ;;
  esac
done

if [[ -z "$SOURCE" ]]; then
  usage >&2
  exit 1
fi

if [[ ! -d "$SOURCE" ]]; then
  echo "Source data directory not found: $SOURCE" >&2
  exit 1
fi

mkdir -p "$DEST"
echo "Syncing $SOURCE -> $DEST"
rsync -a --checksum "$SOURCE"/ "$DEST"/

echo ""
echo "Post-sync check (extras only in DEST are OK):"
diff -rq "$SOURCE" "$DEST" || true

if [[ -n "$DELETE_ROOT" ]]; then
  if [[ ! -d "$DELETE_ROOT" ]]; then
    echo "Source project root not found, skip delete: $DELETE_ROOT" >&2
    exit 0
  fi
  echo ""
  read -r -p "Delete source project at $DELETE_ROOT? [y/N] " confirm
  if [[ "$confirm" == [yY] ]]; then
    rm -rf "$DELETE_ROOT"
    echo "Removed $DELETE_ROOT"
  else
    echo "Skipped delete."
  fi
fi

echo "Done. Canonical data dir: $DEST"
