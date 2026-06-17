# `map.rs` decomposition — architecture refactor plan

**Goal:** split the 1,877-LOC flat `src/map.rs` into `src/map/mod.rs` (re-exports) + `types.rs`
(the map data model) + `decode.rs` (the decoders) + `tests.rs`. **Strictly behavior-preserving** —
pure code movement, no decode-logic or output changes.

## Why (evidence, measured 2026-06-15)
`src/map.rs` = **1,877 LOC** with an exceptionally clean three-way seam:
- **Type region, lines 1–391 — PURE types** (verified: zero impls, enums, or fns interleaved): 28
  pub structs — `MapSquare`, `LandscapeData`, `MapLoc`, `LocTransform`, `Environment`, the 11
  `Env*Settings` structs, `PointLight`, `WaterPatch`, the `MapFile5*` family, the `ChunkInstance*`
  family, `Vector3`, `Vector4`.
- **Decode region, lines ~398–1553** — the 3 pub decoders (`decode_map_square`,
  `decode_map_square_best_effort`, `decode_chunk_instance_stream`) + ~54 private decode helpers
  (biggest `decode_environment` ~123).
- **Test module, lines ~1554–end (~323 LOC)** — the `#[cfg(test)]` module.

This is the simplest decomposition in the campaign: no impls, no shared-state methods, no
interleaving — just lift three contiguous regions.

## API + coupling notes
- **Public API:** `MapSquare`, `decode_map_square`, `decode_map_square_best_effort` are used by
  `commands/`/tests; plus all 28 structs + 3 decode fns are `pub`. Re-export **every** `pub` item from
  `mod.rs` so `crate::map::X` resolves unchanged. `cargo build` is the proof.
- Free-functions + plain structs — clean `use`-wiring, no impl-block gymnastics.

## Target architecture
- `src/map/types.rs` — the 28 structs (the entire 1–391 region). serde/data types — **do not change
  field names/order/`#[serde(...)]`/types**.
- `src/map/decode.rs` — the 3 pub decoders + all private decode helpers.
- `src/map/tests.rs` — the `#[cfg(test)] mod` **verbatim**.
- `src/map/mod.rs` (thin) — `pub mod` declarations + `pub use` re-exports preserving the API.

## Behavior-preservation proof
- **Byte-identical movement:** reconstruct from `git show HEAD:src/map.rs` and diff the three moved
  regions → byte-identical (modulo module headers / `use` / visibility). Report it.
- **Tests:** the full suite (**518**) must pass after every step and at the end (the relocated map
  tests + the `verify-map-archive` path exercise this). Do NOT edit `tests/`.

## Execution discipline — incremental, ALWAYS GREEN
`git mv map.rs map/mod.rs` FIRST. Then, `cargo build --release` after each: (1) `tests.rs`,
(2) `types.rs`, (3) `decode.rs`, (4) trim `mod.rs`.

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No decode-logic or serde changes. Public names/signatures unchanged.
  Move test bodies verbatim.
- **clippy:** `cargo clippy --release --all-targets` stays at **0**. (Clippy on *your* scaffolding
  lines only → overlay_plan/validate precedents: plain `pub` inside the `pub(crate)`/private mod,
  explicit imports not `super::*` to satisfy `wildcard_imports`.)
- **Scope:** only `src/map.rs` → `src/map/**`. Do NOT touch `src/commands/`, `server/`, generated
  files, or `lib.rs` (the rename suffices). Tree is clean. Use Edit/Write; `python3` to search (hook
  blocks grep/rg on repo files — grep works on `/tmp`). Large verbatim moves via a `python3` script
  writing the file is fine — byte-identical.
- Do **not** commit or push.

## Final verification (run all, report verbatim)
```bash
cd tools/rs3-cache-rs
cargo build --release                       # clean
cargo test --release                        # 518 passed / 0 failed
cargo clippy --release --all-targets        # 0
wc -l src/map/*.rs ; git diff --stat | tail
# + the byte-identity diff result
```

## Done criteria
- `map.rs` → `map/` = `mod.rs` (thin) + `types.rs` + `decode.rs` + `tests.rs`. Public API identical;
  build green, 518 tests pass, clippy 0, byte-identity confirmed.

## Budget valve
If you near limits, **stop at a green state** (compiles + all tests pass) and report what remains.
Partial-but-green is a success; a broken build is not.
