# `interface/mod.rs` decomposition — architecture refactor plan

**Goal:** split the 2,129-LOC bloated module root `src/interface/mod.rs` into a thin `mod.rs`
(shared types + `pub mod` declarations + public-API re-exports) plus **three by-concern modules**.
**Strictly behavior-preserving**: pure code movement, no decode/render/byte-output changes.

## Why (evidence, measured 2026-06-15)
- `src/interface/mod.rs` = **2,129 LOC, 52 fns** in the module *root*, while sibling submodules
  (`component.rs` 462, `decode910.rs` 475, `transcode.rs` 921) already exist — the root is doing far
  too much. It mixes **three distinct concerns**:
  1. **Decoding** — `parse_component` (the dispatch) + ~16 per-component-type decoders
     (`decode_layer/rectangle/graphic/model/text/line/button/panel/check/input/grid/list/…`) +
     their helpers. The bulk (~lines 52→1400).
  2. **Rendering / naming** — `render_interface_group`, `component_uid`, `component_fallback_name`.
  3. **Dependency extraction** — `VarTransmitRef` (enum), `ComponentDeps` (struct),
     `parse_component_deps` (~lines 1428→end).
- Same low-risk pattern as the just-completed `config.rs`/`cli.rs` wins, and well-gated:
  `tests/interface_port_oracle.rs` + `tests/interface_transcode_oracle.rs` (byte-exact) + the
  component-codec unit tests.

## The hard constraint: preserve the public API exactly
`interface::*` is imported across the crate — notably `VarTransmitRef` (25 uses),
`parse_component_deps`, `ComponentDeps`, `parse_component`, `render_interface_group`, `component_uid`,
`component_fallback_name`, `ComponentKind`, `InterfaceIr`, plus the submodules. **`mod.rs` MUST
re-export every currently-public item** (`pub use <new_module>::*;`) so every existing
`crate::interface::X` / `interface::X` path resolves unchanged. `cargo build` is the proof.
**Do not change any public name or signature.** Do **not** touch the existing `component`,
`decode910`, or `transcode` submodules.

## Target architecture
- `src/interface/decode.rs` — `parse_component` + all `decode_*` per-type fns + their decode-only
  helpers. Keep the decoders **grouped here** (not one file per type): unlike config's independent
  parsers, they're tightly coupled through the shared component model + the `parse_component`
  dispatch, so one cohesive `decode` module is the right granularity.
- `src/interface/render.rs` — `render_interface_group`, `component_uid`, `component_fallback_name`
  (human-readable rendering + id/name helpers).
- `src/interface/deps.rs` — `VarTransmitRef`, `ComponentDeps`, `parse_component_deps`
  (dependency-graph extraction).
- `src/interface/mod.rs` (thin) — the `pub mod component/decode910/transcode/decode/render/deps;`
  declarations, any genuinely shared types that several concerns use (e.g. `ComponentKind`,
  `InterfaceIr` if they live in mod.rs today — keep the shared ones here, or in a small
  `interface/model.rs` if that reads better), and the `pub use` re-exports that preserve the API.

Cross-module calls are expected (e.g. `render` and `deps` may call into `decode`) — wire them with
`use`; the dependency direction is one-way (decode is the leaf).

## Execution discipline — incremental, ALWAYS GREEN
Move **one concern at a time**, `cargo build --release` + relevant tests after each; never leave the
tree non-compiling. Suggested order (smallest/most-bounded first):
1. `deps.rs` (cleanly bounded block at the end of the file).
2. `render.rs`.
3. `decode.rs` (the bulk).
4. Thin `mod.rs` to shared types + `pub mod` + re-exports.

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No decode/render-logic, struct/serde, or byte-output changes. Public
  names + signatures unchanged.
- **Tests:** the full suite (**518**) must pass after every step and at the end — especially
  `tests/interface_port_oracle.rs` and `tests/interface_transcode_oracle.rs` (byte-exact gates). Do
  NOT edit anything under `tests/`.
- **clippy:** `cargo clippy --release --all-targets` stays at **0**.
- **Scope:** only touch `src/interface/mod.rs` → new `src/interface/{decode,render,deps}.rs` (+ an
  optional `model.rs`). Do NOT modify the existing `component.rs`/`decode910.rs`/`transcode.rs`, do
  NOT touch `server/cache-patches/`, generated files, or anything else. The working tree is clean
  (recent refactors are committed) — your `git diff` should show only your `interface/` changes.
- Downstream files that `use crate::interface::X` should need **no edits** if re-exports are complete
  (if one breaks, you missed a re-export; fix the export, not the consumer, unless it used a private
  item).
- Use Edit/Write; use `python3` to search file contents (a hook blocks grep/rg on repo files — grep
  works only on `/tmp`). Large verbatim block moves via a `python3` script that writes the module is
  fine — the moved code must be byte-identical.
- Do **not** commit or push.

## Final verification (run all, report verbatim)
```bash
cd tools/rs3-cache-rs
cargo build --release                       # clean
cargo test --release                        # 518 passed / 0 failed
cargo test --release --test interface_port_oracle --test interface_transcode_oracle  # byte gates green
cargo clippy --release --all-targets        # 0
wc -l src/interface/*.rs ; git diff --stat | tail
```

## Done criteria
- `interface/mod.rs` reduced to shared types + `pub mod` declarations + re-exports; the three concern
  modules (`decode`/`render`/`deps`) hold the logic.
- **Public API identical** — the whole crate compiles with no downstream import edits.
- Build green, 518 tests pass, clippy 0, both interface oracle tests green.

## Budget valve
If you near context/budget limits before all three concerns are extracted, **stop at a green state**
(compiles + all tests pass) and report what remains in `mod.rs`. Partial-but-green is a success; a
broken build is not.
