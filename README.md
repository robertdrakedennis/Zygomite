# rs3-cache-rs

CLI-first Rust port of `rs3-cache` for RS3 cache extraction and decoding.

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
- Java repo data files for opcode/name lookups (default: `../rs3-cache/data`; Alerion: `tools/zwyz-rs3-cache/data`)

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
  --data-dir ../rs3-cache/data \
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
  --data-dir ../rs3-cache/data \
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
  --data-dir /path/to/tools/zwyz-rs3-cache/data \
  --build 947 --subbuild 1 \
  prepare-overlay --out-dir /path/to/cache/rs3-cache/947-all
```

Alerion server shortcut: `bun run cache:semantic:sync-947` (947 + 910 trees), then `bun run cacheoverlay:ensure-947-overlay`.

# CS2 TypeScript export / transpile

```bash
cargo run --release -- \
  --cache-dir /path/to/cache/unpacked/910 \
  --data-dir /path/to/tools/zwyz-rs3-cache/data \
  --build 910 --subbuild 0 \
  ts-export --out-dir /tmp/rs3-ts-export-910
```

Writes typed definitions: `vars.ts`, `varbits.ts`, `enums.ts`, `params.ts`, `interfaces.ts` (with `ComponentId` / `InterfaceId` UIDs), `scripts.d.ts`, `named_objs.ts`, `dbtables.ts`, and `index.ts`.

Transpile CS2 to structured TypeScript (subset by default):

```bash
cargo run --release -- \
  --cache-dir /path/to/cache/unpacked/910 \
  --data-dir /path/to/tools/zwyz-rs3-cache/data \
  --build 910 --subbuild 0 \
  transpile-scripts --out-dir /tmp/rs3-transpile-910 \
  --filter-script bank_build --max-scripts 5
```

Use `--all-scripts` to transpile the full clientscript archive (slow).

Alerion defaults for integration tests:

- `RS3_CACHE_DIR=/Users/robert/projects/alerion/cache/unpacked/910`
- `RS3_DATA_DIR=/Users/robert/projects/alerion/tools/zwyz-rs3-cache/data`

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
