# `migrate.rs` decomposition — architecture refactor plan

**Goal:** split the 2,635-LOC flat `src/migrate.rs` into `src/migrate/mod.rs` (cli + analyzer core +
shared methods) + `types.rs` (the report/diff/validation data model) + `interface.rs` + `script.rs`
(the domain-specific `MigrationAnalyzer` method blocks) + `tests.rs`. **Strictly behavior-preserving**
— pure code movement, no migration-logic changes.

## Why (evidence, measured 2026-06-15)
`src/migrate.rs` = **2,635 LOC** mixing:
- **Data model (~450 LOC, lines ~22–468):** `ConflictStatus`(enum), `ConflictEntry`, `FieldDiff`,
  `ConflictReport`, `ConflictSummary`, `ScriptReport`, `TargetValidationSummary/Report`,
  `ComponentTargetValidation`, `ScriptTargetValidation`, `RemapTable`, `VarpRemapTarget`,
  `ReferenceUpdate`, `RefUpdateEntry`, `AllocationInfo`(+impl), `RangeAlloc` (+ late
  `PreparedScriptOverlay`, `DependencySiteValidation`).
- **The `MigrationAnalyzer` impl (~1,700 LOC, lines ~480–2194)** interleaving interface-migration,
  script-migration, and shared validation/allocation methods (all sharing analyzer state).
- **Inline tests (~370 LOC, lines ~2380–end):** `build_ctx` + a few `#[cfg(test)]` fns.
- **The cli command layer** (`run_check`, `run_script`, `MigrateCheckOpts`, `MigrateScriptOpts`,
  `MigrateSource`, `RemapOpts`) — moved here by the cli refactor; the orchestration entry points.

## API + coupling notes
- **Public API to preserve:** `MigrateSource`, `RemapOpts`, `MigrationAnalyzer`, `run_check`,
  `MigrateCheckOpts`, `run_script`, `MigrateScriptOpts`, `ConflictReport`, `ScriptReport`,
  `TargetValidationReport`, `ReferenceUpdate` (+ anything else currently `pub`). Define/re-export from
  `mod.rs` so `crate::migrate::X` resolves unchanged.
- **The analyzer methods are inherent methods on `MigrationAnalyzer`.** Use the proven validate
  pattern: **multiple inherent `impl MigrationAnalyzer` blocks across child modules**; child modules
  read the parent type's **private fields directly (no visibility widening)**; cross-method calls
  (`self.walk_component_deps()`, `self.allocate_free_ids()`, …) resolve regardless of which module the
  callee lives in. (If clippy `redundant_pub_crate`/`elidable_lifetime_names` fire on your *wrapper*
  lines, use plain `pub`/elided lifetimes per the overlay_plan/validate precedents — those are
  authored scaffolding, not moved bodies.)

## Target architecture (5 modules)
- `src/migrate/types.rs` — the entire data model above (all structs/enums + `AllocationInfo`'s impl).
  These are serde/report types — **do not change field names/order/attrs/types**.
- `src/migrate/interface.rs` — `impl MigrationAnalyzer` block with the clearly-interface methods:
  `walk_component_deps`, `compare_component`, `remap_interface`, `validate_interface_target`,
  `validate_interface_components`.
- `src/migrate/script.rs` — `impl MigrationAnalyzer` block with the clearly-script methods:
  `walk_script`, `compare_script`, `remap_script`, `validate_script_target`, `prepare_script_overlay`,
  `rewrite_script_for_target`, `collect_script_ref_updates`, `validate_target_scripts_from_overlays`.
- `src/migrate/tests.rs` — `#[cfg(test)] mod` with the test module **verbatim**.
- `src/migrate/mod.rs` (the core) — the cli layer (`run_check`/`run_script` + the `*Opts`/`MigrateSource`/
  `RemapOpts`), the `MigrationAnalyzer` struct + its **entry + shared/cross-cutting** methods (`new`,
  `analyze_interface`, `analyze_script`, `entity_summaries`, `allocate_free_ids`,
  `build_reference_updates`, `validate_dependency_site`, `build_target_validation_context_from_overlays`,
  `summarize_target_validation`, and the small free helpers like `push_diff`/`compare_pair`), the
  `pub mod` declarations, and the re-exports. (Keep the cross-cutting methods here — both
  interface.rs and script.rs call them via `self.`; that's fine across modules.)

If a method's home is ambiguous, leave it in `mod.rs` — only move the *clearly* domain-specific ones.
Goal: `mod.rs` ≈ 700–900 LOC core, the rest redistributed.

## Behavior-preservation proof
- **Byte-identical movement:** reconstruct from `git show HEAD:src/migrate.rs` and diff moved blocks →
  byte-identical (modulo module headers / `use` / visibility / your `impl` wrapper lines). Report it.
- **Tests:** the full suite (**518**) must pass after every step and at the end (the migrate tests +
  the migrate-check/script oracles exercise this). Do NOT edit `tests/`.

## Execution discipline — incremental, ALWAYS GREEN
`git mv migrate.rs migrate/mod.rs` FIRST. Then one extraction at a time, `cargo build --release` after
each: (1) `tests.rs`, (2) `types.rs`, (3) `interface.rs`, (4) `script.rs`, (5) trim `mod.rs`.

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No migration-logic or serde (field names/order/attrs/types) changes.
  Public names/signatures unchanged. Move test bodies verbatim.
- **clippy:** `cargo clippy --release --all-targets` stays at **0**.
- **Scope:** only `src/migrate.rs` → `src/migrate/**`. Do NOT touch `src/commands/migrate.rs` (the cli
  command shim, if any), `server/`, generated files, or `lib.rs` (the rename suffices). Tree is clean.
- Use Edit/Write; `python3` to search (hook blocks grep/rg on repo files — grep works on `/tmp`).
  Large verbatim moves via a `python3` script writing the file is fine — moved code byte-identical.
- Do **not** commit or push.

## Final verification (run all, report verbatim)
```bash
cd tools/rs3-cache-rs
cargo build --release                       # clean
cargo test --release                        # 518 passed / 0 failed
cargo clippy --release --all-targets        # 0
wc -l src/migrate/*.rs ; git diff --stat | tail
# + the byte-identity diff result
```

## Done criteria
- `migrate.rs` → `migrate/` = `mod.rs` (~700–900 core) + `types.rs` + `interface.rs` + `script.rs` +
  `tests.rs`. Public API identical; build green, 518 tests pass, clippy 0, byte-identity confirmed.

## Budget valve
This is the most tangled target so far. If the interface/script impl-split proves too coupled to do
cleanly, **the guaranteed wins are `types.rs` + `tests.rs`** (lift those, leave the impl in `mod.rs`) —
land that green rather than forcing a messy split. If you near limits, **stop at a green state**
(compiles + all tests pass) and report what remains. Partial-but-green is a success; a broken build is
not.
