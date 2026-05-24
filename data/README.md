# Opcode and name tables (`data/`)

Canonical `--data-dir` for `rs3-cache-rs` in Alerion.

| Use | Path / env |
|-----|------------|
| CLI (from `tools/rs3-cache-rs`) | `--data-dir data` (default) |
| Server overlay | `tools/rs3-cache-rs/data` via `DEFAULT_RS3_DATA_DIR` / `RS3_DATA_DIR` |
| Override | `export RS3_DATA_DIR=/path/to/this/dir` |

## Contents

- `opcodes-<build>.txt` — per-revision CS2 opcode tables (e.g. `opcodes-947.txt`, `opcodes-910.txt`)
- `opcodes-<build>-<subbuild>.txt` — subbuild-specific tables when present
- `opcodes-unscrambled.txt` — fallback opcode list
- `names/` — script/config name tables
- `commands/` — command name tables

## Merging from another checkout

If you have opcode data elsewhere (old Java tree or upstream `rs3-cache/data`):

```sh
cd tools/rs3-cache-rs
./scripts/sync-legacy-data.sh /path/to/other/data
```

Review the diff summary before deleting the source tree.
