# rs3-cache-rs

CLI-first Rust toolkit for RS3 cache extraction, CS2, and overlay semantic trees.

Current target snapshot:

- build `947.1`
- OpenRS2 cache id `2519`

## What it does

- Extract and render interfaces
- Decode varps and varbits
- Decode and decompile CS2 scripts
- Decode RT7 models
- Extract audio (`jaga` + embedded `ogg`, direct `ogg`)
- Run full unpack flow with top-level exports (`worldmap`, `maps`, `vfx`, `animator`, `cutscene2d`, defaults, `areas.png`, `ttf`, `fontmetrics`, `binary`)

## Requirements

- Rust toolchain (`cargo`)
- Cache data available one of two ways:
  - extracted flat cache dir (default: `../eval/cache-flat/cache`)
  - OpenRS2 tar (default: `../cache-runescape-live-en-b947.1-2026-04-20-10-45-34-openrs2#2519.tar`)
- Opcode/name tables in `data/` (default `--data-dir data` when run from this crate; Alerion: `tools/rs3-cache-rs/data`)

## Build

```bash
cargo build --release
```

## CLI usage

Top-level help:

```bash
cargo run -- --help
```

Main options:

- `--cache-dir <PATH>` flat cache root
- `--cache-tar <PATH>` tar used to backfill missing archive/group files
- `--data-dir <PATH>` data files for names/opcodes
- `--build <N>` cache build for versioned decoding/opcodes (default `947`)
- `--subbuild <N>` cache subbuild for opcodebook lookup (default `1`)

### Full unpack (recommended)

```bash
cargo run -- \
  --cache-dir ../eval/cache-flat/cache \
  --cache-tar ../cache-runescape-live-en-b947.1-2026-04-20-10-45-34-openrs2#2519.tar \
  --data-dir data \
  unpack --out-dir /tmp/rs3-cache-rs-out
```

Fast model sample run:

```bash
cargo run -- unpack --out-dir /tmp/rs3-cache-rs-out --sample-models --skip-audio
```

RS3 build `910` example:

```bash
cargo run -- \
  --cache-dir /tmp/rs3-cache-rs-910/cache \
  --cache-tar /Users/robert/projects/ignis/static/cache-runescape-live-en-b910-2019-12-11-00-00-00-openrs2#1730.tar \
  --data-dir data \
  --build 910 \
  --subbuild 0 \
  unpack --out-dir /tmp/rs3-cache-rs-910-out-audio --sample-models --max-audio-files 500
```

### Individual commands

```bash
# interfaces
cargo run -- interfaces --out-dir /tmp/rs3-if

# varps (all domains)
cargo run -- varps --out-file /tmp/varps.json

# varps (single domain)
cargo run -- varps --domain player --out-file /tmp/varps-player.json

# varbits
cargo run -- varbits --out-file /tmp/varbits.json

# configs
cargo run -- configs --out-dir /tmp/configs

# cs2 decompile + summary json
cargo run -- cs2 --out-dir /tmp/cs2 --out-file /tmp/scripts.json

# models
cargo run -- models --out-dir /tmp/models --out-file /tmp/models.json

# audio (limit for quick scan)
cargo run -- audio --out-dir /tmp/audio --max-files 5000
```

### CacheOverlay semantic tree (`prepare-overlay`)

Writes everything the Alerion overlay needs under one directory:

- `raw-flat/` — lossless JS5 group bytes for repack
- `refs/` — structured config dependency graph (`obj.json`, `npc.json`, …)
- `.rs3-cache-manifest.json` — artifact fingerprints for stamp/skip logic

Legacy `config/dump.*` text is not produced here; use the separate `dump-configs` command only if you need human-readable dumps for inspection.

```bash
cargo run --release -- \
  --cache-dir /path/to/cache/unpacked/947 \
  --data-dir /path/to/tools/rs3-cache-rs/data \
  --build 947 --subbuild 1 \
  prepare-overlay --out-dir /path/to/cache/rs3-cache/947-all
```

Alerion server shortcut: `bun run cache:semantic:sync-947` (947 + 910 trees), then `bun run cacheoverlay:ensure-947-overlay`.

# CS2 workflow (947 active, 910 base)

Full agent workflow: [docs/cs2_roundtrip_workflow.md](../../docs/cs2_roundtrip_workflow.md).

**947 (donor / runtime truth)** — subbuild `1`:

```bash
C947="--cache-dir /path/to/cache/unpacked/947 --data-dir /path/to/tools/rs3-cache-rs/data --build 947 --subbuild 1"

# Typed defs + transpiled corpus
cargo run --release -- $C947 ts-export --out-dir /path/to/cache/rsmv/947/clientscript
cargo run --release -- $C947 transpile-scripts --out-dir /path/to/cache/rsmv/947/clientscript --all-scripts

# Validate / assemble / deps
cargo run --release -- $C947 validate-script --script-id 4330
cargo run --release -- $C947 assemble-script --input script.asm.ts --output /tmp/out.cs2
cargo run --release -- $C947 dep-tree-script --id 4330 --out-file /tmp/deps.json
```

**910 (overlay base)** — subbuild `0`: same commands with `--build 910 --subbuild 0` and `cache/unpacked/910`.

**Roundtrip tests** (100 scripts per build):

```bash
RS3_CACHE_DIR=.../947 cargo test asm_encode_roundtrip_byte_perfect --release
RS3_CACHE_DIR=.../910 cargo test asm_encode_roundtrip_byte_perfect_910 --release
```

**Repack into Alerion runtime:** patch a temp copy of `cache/rs3-cache/947-all/raw-flat`, then `bun run js5pack:pack-flat --archives scripts` in `server/` (see workflow doc).

Alerion env defaults:

- `RS3_CACHE_DIR` → `cache/unpacked/947` (947 tests) or `910` (910 tests)
- `RS3_DATA_DIR` → `tools/rs3-cache-rs/data`

## Expected unpack output

`unpack --out-dir <X>` writes:

- `<X>/interface`
- `<X>/config/*.json` + `*.defaults`
- `<X>/script/decompiled` + `scripts.json`
- `<X>/model/decoded` + `models.json` or `models_sample.json`
- `<X>/audio` (unless `--skip-audio`)
- `<X>/worldmap`, `<X>/maps`, `<X>/vfx`, `<X>/animator`, `<X>/cutscene2d`
- `<X>/uianim`, `<X>/uianimcurve`, `<X>/ttf`, `<X>/fontmetrics`, `<X>/binary`
- `<X>/areas.png`

## Quality gates

Run strict lint:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Run tests:

```bash
cargo test
```

Real-cache tests support env overrides:

- `RS3_CACHE_DIR`
- `RS3_CACHE_TAR`
- `RS3_DATA_DIR`
