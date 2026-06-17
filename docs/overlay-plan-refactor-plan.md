# `overlay_plan.rs` decomposition — architecture refactor plan

**Goal:** split the 4,102-LOC flat module `src/overlay_plan.rs` (132 fns, 30 structs, 8 enums, 5
impls) into `src/overlay_plan/mod.rs` (entry + orchestration + re-exports) plus **by-concern
submodules** for the data model and the planning algorithm. **Strictly behavior-preserving** — pure
code movement, no planner-logic or output changes.

## Why (evidence, measured 2026-06-15)
- `src/overlay_plan.rs` = **4,102 LOC**, the 3rd-largest file, mixing four concerns in one flat file:
  1. **Input manifest schema** — `CacheOverlayManifest`, `OverlayRootOverrides`, `OverlayImports`,
     `ArchiveRef`, `RegionSpec`, `ArchiveMode`, `OverlayAllow`, `OverlayRoots` (lines ~44–198): the
     deserialized cacheoverlay manifest.
  2. **Output plan schema** — `OverlayPlanOutput` + ~15 serde structs (`OverlayPlanSelected`,
     `OverlayPlanArchiveGroups/Files`, `OverlayPlanImports`, `OverlayPlanDb`, `OverlaySemanticManifest`,
     `Rs3CacheManifest`, `OverlayWarning`, `OverlayBlockedIssue`, `OverlayPlanProof`,
     `OverlayProofSummary/Issue`, `OverlayPlanAudit`, `DependencyEdgeSample`) (lines ~199–369): the
     JSON the command emits.
  3. **Ref / dependency model** — `RefKind` (+impl), `SemanticRefKey` (+impl), `ArchiveDef`,
     `ConfigTarget`, `RawGroupTarget`, `SelectionMode`, `PendingRef`, `RootKind`,
     `RefGraphRepository` (+impl), `ConfigSemanticIndex` (+impl) (lines ~370–980).
  4. **The planning algorithm** — `PlanBuilder` (+ its large impl, lines ~981→~3400): `seed_imports`,
     `process_ref`, `process_map_groups`, `scan_refs_for_kind`, `scan_binary_multivar_dependencies`,
     `resolve_roots`, `finalize_plan`, `all_archives`, `config_target`, … + late helpers
     `CompareState`, `MultivarRefs`.
- Orchestration: `run_overlay_plan_command` (the pub entry, ~line 700) parses the manifest, drives a
  `PlanBuilder` through the phases, and emits.

## The API is SMALL — restructure freely
**Only two items are public:** `run_overlay_plan_command` and `OverlayPlanCommandOptions` (the cli
calls `overlay_plan::run_overlay_plan_command(OverlayPlanCommandOptions { .. })`). Everything else is
private/`pub(crate)`. So **there is no re-export gymnastics** — keep those two reachable from
`overlay_plan::` (define/re-export them in `mod.rs`) and you may move all internals freely. The real
work is wiring `use` correctly across the new submodules (the `PlanBuilder` algorithm references the
manifest, output, and ref types heavily) and keeping it compiling at each step.

## Target architecture
- `src/overlay_plan/manifest.rs` — the input schema (concern 1) + its `Deserialize` impls/helpers.
- `src/overlay_plan/plan_output.rs` — the output schema (concern 2) + its `Serialize` impls. (These
  serde structs define the emitted JSON shape — **do not change field names, order, `#[serde(...)]`
  attributes, or types**; that would change output bytes.)
- `src/overlay_plan/refs.rs` — the ref/dependency model (concern 3): `RefKind`, `SemanticRefKey`,
  `ArchiveDef`, `ConfigTarget`, `RawGroupTarget`, `SelectionMode`, `PendingRef`, `RootKind`,
  `RefGraphRepository`, `ConfigSemanticIndex` + their impls.
- `src/overlay_plan/builder.rs` — `PlanBuilder` + its full impl (concern 4, the algorithm) + the late
  `CompareState`/`MultivarRefs` helpers. **Keep `PlanBuilder` and its impl cohesive in one module** —
  its methods share `&mut self` state, so do NOT scatter the impl across modules.
- `src/overlay_plan/mod.rs` (thin) — `run_overlay_plan_command` (entry/orchestration),
  `OverlayPlanCommandOptions` (pub), `ProofState`, `overlay_plan_cache_path`, the `pub mod`
  declarations, and `pub use` so `overlay_plan::run_overlay_plan_command` /
  `overlay_plan::OverlayPlanCommandOptions` resolve unchanged.

Dependency direction: `manifest`/`plan_output`/`refs` are leaves; `builder` depends on all three;
`mod` depends on `builder`. Wire with `use super::...` / `use crate::overlay_plan::...`.

## Execution discipline — incremental, ALWAYS GREEN
Move **one concern at a time**, `cargo build --release` after each; never leave it non-compiling.
Suggested order (leaves first, so the builder still sees its types):
1. `manifest.rs` (input schema). 2. `plan_output.rs` (output schema). 3. `refs.rs` (dep model).
4. `builder.rs` (PlanBuilder + impl). 5. Thin `mod.rs` to entry + orchestration + re-exports.

## Behavior-preservation proof (this module is less oracle-covered than config/interface — be extra careful)
- **Byte-identical movement:** reconstruct the original `overlay_plan.rs` from `HEAD` (`git show
  HEAD:src/overlay_plan.rs`) and diff each moved block against the new files — the moved code must be
  byte-identical (only module doc-headers, `use` lines, and visibility widening to `pub(crate)` may
  differ). Report this diff result.
- **Tests:** the full suite (**518**) must pass after every step and at the end (it exercises the
  overlay path). Do NOT edit `tests/`.
- **Optional but encouraged smoke test:** if you can locate a ready cacheoverlay manifest + the inputs
  it needs (look under `server/` for an existing manifest and the `--cache-dir`/`--source-cache-tar`
  the `overlay-plan` command takes; AGENTS.md shows the invocation), run `overlay-plan` **before**
  (build from a clean `git stash` of your changes, or `git worktree`/`HEAD` binary) and **after**, and
  diff the emitted plan JSON — it must be identical. If the inputs aren't readily available, skip this
  and rely on byte-identity + the suite (state clearly that you skipped it and why).

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No planner-logic, serde (field names/order/attrs/types), or
  output-byte changes. The two public names/signatures unchanged.
- **clippy:** `cargo clippy --release --all-targets` stays at **0**.
- **Scope:** only `src/overlay_plan.rs` → `src/overlay_plan/**`. The directory module reaches via the
  existing `pub mod overlay_plan;` in `lib.rs` — you should NOT need to edit `lib.rs` (the `overlay_plan.rs`
  → `overlay_plan/mod.rs` move is a rename; use `git mv` first so the rename is tracked and the tree
  never has both). Do NOT touch `server/`, generated files, or anything else. The tree is clean.
- Downstream (`cli`/`commands`) calls only the two public items — they should need **no edits**.
- Use Edit/Write; `python3` to search file contents (a hook blocks grep/rg on repo files — grep works
  only on `/tmp`). Large verbatim block moves via a `python3` script writing the module file is fine —
  moved code must be byte-identical.
- Do **not** commit or push.

## Final verification (run all, report verbatim)
```bash
cd tools/rs3-cache-rs
cargo build --release                       # clean
cargo test --release                        # 518 passed / 0 failed
cargo clippy --release --all-targets        # 0
wc -l src/overlay_plan/*.rs ; git diff --stat | tail
# + the byte-identity diff result, and the overlay-plan smoke-diff result (or why skipped)
```

## Done criteria
- `overlay_plan.rs` → `overlay_plan/` = `mod.rs` (thin entry+orchestration+re-exports) + `manifest.rs`
  + `plan_output.rs` + `refs.rs` + `builder.rs`.
- Public API identical (`run_overlay_plan_command` + `OverlayPlanCommandOptions` resolve unchanged;
  no downstream edits). Build green, 518 tests pass, clippy 0, byte-identity confirmed.

## Budget valve
If you near context/budget limits before all concerns are extracted, **stop at a green state**
(compiles + all tests pass) and report what remains in `overlay_plan.rs`/`mod.rs`. Partial-but-green
is a success; a broken build is not.
