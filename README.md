# rs3-cache-rs

CLI-first Rust toolkit for RS3 cache extraction, CS2, and overlay semantic trees.

Supported native base revisions:

- build `948.1` — OpenRS2 cache id `2573` (current; prepared 2026-06-10)
- build `947.1` — OpenRS2 cache id `2519`
- build `910.0` — separate first-class base revision

`910`, `947`, and `948` are all native targets in this repo. Treat them as separate first-class revisions with their own opcode books, command semantics, roundtrip checks, and transpile expectations. Do not frame `910` as secondary compatibility mode or `947` as sole runtime truth.

## Adding a new build (how 948 was added)

1. Decompress the OpenRS2 tar into `../../cache/static-caches/`, extract flat
   to `../../cache/unpacked/<build>/` (`tar -xf ... --strip-components=1`).
2. Derive the CS2 opcode book — opcodes are fully rescrambled every build:
   `cargo run --release -- derive-opcode-book --old-cache .../unpacked/947
   --new-cache .../unpacked/<build> --old-book data/opcodes-947.txt
   --out data/opcodes-<build>.txt` (aligns unchanged scripts across caches;
   948: 20,416 scripts aligned, 1,230 commands, 0 conflicts, all 20,621
   scripts then decode under code-length validation). `opcodes-948.txt` was
   later replaced wholesale by the richer upstream **zwyz/rs3-cache**
   `data/opcodes-948.txt` (2,244 names — a verified strict superset of the
   1,230 derived names: 0 id conflicts, +1,014 engine commands with no 910
   counterpart). It carries a `// Synced from …` provenance header and is a
   hand-synced upstream INPUT, never regenerated from the registry.
3. Fix format drift surfaced by consume-full-payload errors. 948 needed two:
   material v0 gained flag bit 23 → one BE float (`parse_material_v0`), and
   VFX flow shape kind 8 (3 BE floats, `decode_flow_shape`) — both validated
   archive-wide on old+new builds (0 regressions).
4. `prepare-overlay` then `unpack --skip-audio --best-effort-maps` into
   `../../cache/rs3-cache/<build>-all`, plus `cs2 --out-dir .../cs2`.

948 known follow-ups: 2,230/141,575 mapsquare groups have parse errors under
`--best-effort-maps` (not yet compared against a 947 baseline). The 1,014
upstream-only commands (now present in `opcodes-948.txt`) are unused by any
live script and have no 910 counterpart, so they never enter the registry —
they only matter if future scripts use them.

## What it does

- Extract and text-render interfaces (decode for all component types)
- Encode/decode interface **components** byte-perfectly for the container types — layer (0), rectangle (3), graphic (5) — via `src/interface_codec.rs`; this powers the `examples/gen_*` interface-clone generators (other component types are decode-only)
- Decode varps and varbits
- Decode and decompile CS2 scripts; dependency trees for interfaces, scripts, varps, varbits, and configs (`dep-tree-*`)
- Transpile CS2 to reversible TypeScript (`--output-style high-ts|reversible`) and assemble it back to CS2 (`assemble-script`, batch via `assemble-script-batch`)
- Render the same CS2 IR as **byte-exact RuneScript** — a second reversible surface (`render_runescript` / `parse_runescript`) validated by the `RS3_RUNESCRIPT_GATE` round-trip (0 failures on 910 + 948); proven, not yet the default editing surface
- Validate donor `947 -> 910` script/interface slices with migration audit (`migrate-check`, `migrate-script`)
- Build native overlay plan JSON for Bun `cacheoverlay` wrapper (`overlay-plan`), plus the overlay's semantic-tree inputs (`prepare-overlay`, `dump-raw-flat`, `dump-refs`, `dump-configs`)
- Decode RT7 models, maps (`verify-map-archive`), and build the NXT-model clip-flag collision grid per map square (`build-collision`)
- Extract audio (`jaga` + embedded `ogg`, direct `ogg`)
- Run full unpack flow with top-level exports (`worldmap`, `maps`, `vfx`, `animator`, `cutscene2d`, defaults, `areas.png`, `ttf`, `fontmetrics`, `binary`)

## Requirements

- Rust toolchain (`cargo`)
- Cache data available one of two ways:
  - extracted flat cache dir (default: `../eval/cache-flat/cache`)
  - OpenRS2 tar (default: `../cache-runescape-live-en-b947.1-2026-04-20-10-45-34-openrs2#2519.tar`)
- Opcode/name tables in `data/` (default `--data-dir data` when run from this crate; Alerion: `tools/rs3-cache-rs/data`)

### Opcode data sources

`data/cs2/registry-910.json` (extracted from the client by `extract-cs2-registry`) is the canonical opcode truth for build **910**. Its three 910 txt files — `opcodes-910.txt`, `opcodes-large-910.txt`, `opcode-aliases-910.txt` — are **generated views** of that registry: run `generate-cs2-data` after `extract-cs2-registry`, or `generate-cs2-data --check` to gate drift. The registry also carries an `id_948` column, but `opcodes-948.txt` is **not** generated and stays hand-derived: the registry is anchored to the 1,432-case 910 dispatch switch and cannot represent the donor-only opcodes 948 adds (e.g. `sub` at id 824), which are used when decoding the live 948 cache. **947, 948, and every other build remain hand-derived txt** (`scripts/derive-opcode-book.py` / `sync-legacy-data.sh`).

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
- `deps/` — typed component/script dependency artifacts and coverage
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

Native planner for donor `947`:

```bash
cargo run --release -- \
  --cache-dir /path/to/cache/unpacked/910 \
  --data-dir /path/to/tools/rs3-cache-rs/data \
  --build 910 --subbuild 0 \
  overlay-plan --manifest /path/to/cacheoverlay-manifest.json --out-file /tmp/overlay-plan.json --audit-dir /tmp/overlay-plan-audit
```

### Cross-build migration proof (`migrate-check` / `migrate-script`)

Prove a donor `947` interface group or script against the `910` target before splicing it. `--out-file` is required; `--audit-dir` writes the split audit (`summary.json`, `scripts_failed.jsonl`, `unsupported_sites.jsonl`, …):

```bash
cargo run --release -- \
  --cache-dir /path/to/cache/unpacked/910 --data-dir data --build 910 --subbuild 0 \
  migrate-check --interface-group 105 --out-file /tmp/migrate-interface.json \
  --source-build 947 --source-subbuild 1 --remap --validate-target --audit-dir /tmp/migrate-interface-audit

cargo run --release -- \
  --cache-dir /path/to/cache/unpacked/910 --data-dir data --build 910 --subbuild 0 \
  migrate-script --script-id 621 --out-file /tmp/migrate-script.json \
  --source-build 947 --source-subbuild 1 --remap --validate-target --audit-dir /tmp/migrate-script-audit
```

### Collision (`build-collision`)

Builds the RS clip-flag collision grid for one map square (NXT collision model — terrain flags + loc clips):

```bash
cargo run --release -- \
  --cache-dir /path/to/cache/unpacked/910 --data-dir data --build 910 --subbuild 0 \
  build-collision --map-x 50 --map-z 50 --out /tmp/collision-50-50.json   # omit --out for a stdout summary
```

### Interface component codec + examples

`src/interface_codec.rs` byte-perfectly encodes/decodes the container component types — layer (0), rectangle (3), graphic (5); other component types raise `UnsupportedType` and remain decode-only. This is enough to clone and retarget whole interface groups built from those types. The `examples/` directory uses it:

- `gen_necro_page.rs` / `gen_necro_primary.rs` / `gen_necro_action_window.rs` / `gen_necro_ribbon_icon.rs` — clone 910-native interface groups into new ids (e.g. magic ability page 1459 → necromancy 1207) with retargeted component refs, re-encoding each component and verifying the round-trip
- `gen_skillguide_slots.rs` / `skillguide_grid.rs` — skill-guide/HUD-grid discovery and generation
- `verify_branch_targets.rs` — decode a `.cs2` file and assert every branch/switch target is in range (run after any mid-script CS2 instruction insertion)
- `decode_cs2.rs` — standalone CS2 decode for inspection

```bash
cargo run --release --example verify_branch_targets -- /tmp/out.cs2
cargo run --release --example gen_necro_page -- /path/to/cache/unpacked/910 /tmp/necro-page-out
```

# CS2 workflow (`910` and `947` first-class)

Canonical workflow: [docs/workflows/cs2-cache.md](../../docs/workflows/cs2-cache.md).

Each revision should be run, validated, and reasoned about on its own terms. Use build-specific cache inputs, opcode metadata, and roundtrip checks for both.

**947** — subbuild `1`:

```bash
C947="--cache-dir /path/to/cache/unpacked/947 --data-dir /path/to/tools/rs3-cache-rs/data --build 947 --subbuild 1"

# Typed defs + transpiled corpus
cargo run --release -- $C947 ts-export --out-dir /path/to/cache/rsmv/947/clientscript
cargo run --release -- $C947 transpile-scripts --out-dir /path/to/cache/rsmv/947/clientscript --all-scripts

# Validate / assemble / deps
cargo run --release -- $C947 validate-script --script-id 4330
cargo run --release -- $C947 assemble-script --input script.asm.ts --output /tmp/out.cs2
cargo run --release -- $C947 assemble-script --input script.ts --output /tmp/out.cs2 --strict-structured
cargo run --release -- $C947 dep-tree-script --id 4330 --out-file /tmp/deps.json
```

**910** — subbuild `0`: same commands with `--build 910 --subbuild 0` and `cache/unpacked/910`.

**Roundtrip tests** default to `100` scripts per build. Set `RS3_ROUNDTRIP_LIMIT=0` or `RS3_ROUNDTRIP_LIMIT_910=0` for full-corpus proof:

```bash
RS3_CACHE_DIR=.../947 cargo test asm_encode_roundtrip_byte_perfect --release
RS3_CACHE_DIR_910=.../910 cargo test asm_encode_roundtrip_byte_perfect_910 --release
```

**Repack into Alerion runtime:** patch temp copy of revision-appropriate `raw-flat` tree, then `bun run js5pack:pack-flat --archives scripts` in `server/` (see workflow doc).

Alerion env defaults:

- `RS3_CACHE_DIR` → `cache/unpacked/947` for `947` tests
- `RS3_CACHE_TAR` → `cache-runescape-live-en-b947.1-2026-04-20-10-45-34-openrs2#2519.tar` override for `947` archive backfill
- `RS3_CACHE_DIR_910` → `cache/unpacked/910` for `910` tests
- `RS3_CACHE_TAR_910` → `cache-runescape-live-en-b910-2019-12-11-00-00-00-openrs2#1730.tar` override for `910` archive backfill
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
- `RS3_CACHE_DIR_910`
- `RS3_CACHE_TAR_910`
- `RS3_ROUNDTRIP_LIMIT`
- `RS3_ROUNDTRIP_LIMIT_910`
- `RS3_DATA_DIR`
