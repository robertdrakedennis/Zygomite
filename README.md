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
- Java repo data files for opcode/name lookups (default: `../rs3-cache/data`)

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
