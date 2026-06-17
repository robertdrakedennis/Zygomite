# `config.rs` decomposition — architecture refactor plan

**Goal:** decompose the 4,554-LOC flat module `src/config.rs` — **47 independent config-type parsers,
55 structs, ~10 op-enums** — into `src/config/mod.rs` (shared types + re-exports) plus one module per
config-type family under `src/config/`. **Strictly behavior-preserving**: pure code movement, no
parser-logic or byte-output changes.

## Why (evidence, measured 2026-06-15)
- `src/config.rs` = **4,554 LOC, 48 pub fns (47 of them `parse_*`/`decode_*`), 55 pub structs**, plus
  config-specific op-enums (`HitmarkMulti`, `ParticleEffectorOp`, `ParticleEmitterOp`, `IdkOp`,
  `SpotOp`, `QuestOp`, `SeqUnknown19/20`, `DbTypeBase`, …). Each `parse_X` + its struct(s) + op-enum(s)
  is an **independent** config type (loc, obj, npc, seq, struct, enum, dbtable, area, hunt, particle,
  …). There is **no internal dispatch** — config.rs is a flat library of parse fns, so there is
  nothing to untangle, only to relocate.
- Same shape as the just-completed `cli.rs` win (multi-X god-module → per-X modules), and lower risk:
  the parsers don't call each other much, and behavior is heavily gated (the config oracle tests +
  decode/encode round-trips).

## The one hard constraint: preserve the public API exactly
`config.rs` is a **widely-imported library** — ~20+ distinct `crate::config::<item>` are used across
the crate (`ScalarValue`, `parse_loc`, `parse_obj`, `parse_npc`, `parse_seq`, `parse_struct`,
`parse_dbtable`, `parse_param`, `OpListEntry`, `DbRowEntry`, `ParamEntry`, …) by `config_dump`,
`config_refs`, `config_transcode`, `port/config`, `dep_tree`, `explain*`, the CLI commands, etc.
**`src/config/mod.rs` MUST `pub use` every currently-public item** so every existing
`crate::config::X` path resolves unchanged. `cargo build` is the proof — downstream won't compile if
any export is dropped or renamed. **Do not change any public name or signature.**

## Target architecture
- `src/config/mod.rs` — the shared surface:
  - Shared types used across many parsers (notably `ScalarValue`, 28 downstream uses) and any common
    reader helpers that live in config.rs today (the binary reader itself is `crate::packet::Packet`,
    external — keep using it).
  - `mod <family>;` declarations + a block of `pub use <family>::*;` (or explicit re-exports)
    re-exporting the full public API.
- `src/config/<family>.rs` — one module per config-type family. Each holds its `parse_*` fn(s), the
  struct(s) they return, and the op-enums they own. Suggested grouping (finalize by actual cohesion):
  | module | types |
  |---|---|
  | `value.rs` (or keep in mod) | `ScalarValue` + shared value/reader helpers |
  | `loc.rs` / `obj.rs` / `npc.rs` | loc, obj, npc |
  | `seq.rs` | seq + seqgroup (+ `SeqUnknown19/20`) |
  | `spot.rs` | spotanim (+ `SpotOp`) · `idk.rs` identkit (+ `IdkOp`) |
  | `db.rs` | dbtable + dbrow + dbtype (+ `DbTypeBase`, `DbRowEntry`) |
  | `data.rs` | enum, struct, param (+ `ParamEntry`, `OpListEntry`) |
  | `world.rs` | area, hunt, category, controller, cursor, inv |
  | `particle.rs` | effector + emitter (+ `ParticleEffectorOp`/`ParticleEmitterOp`) · `hitmark.rs` (+ `HitmarkMulti`) |
  | `chat.rs` | quickchat (+ `QuickChatDynamicCommand`) · `quest.rs` (+ `QuestOp`) |
  | `vars.rs` | var_client_string, var_npc_bit, … |
  | `misc.rs` | mesanim, itemcode, gamelogevent, bugtemplate, … (small leftovers) |
  Aim for **~15–25 cohesive files**, not 47 one-liners and not 3 mega-files. A `parse_X`-only helper
  used by a single parser moves with it; a helper used by 2+ families goes to `mod.rs`.

## Execution discipline — incremental, ALWAYS GREEN
Move **one family at a time**, and after each, run `cargo build --release` and the relevant tests.
Never leave the tree non-compiling. Recommended order: `value`/`ScalarValue` first (it's the shared
dependency), then the heavily-imported entity parsers (loc/obj/npc/seq/struct/db/data), then the rest,
then thin `config.rs` → `config/mod.rs` to just shared types + re-exports.

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No parser-logic changes, no struct-field/serde changes, no byte-output
  changes. Public names + signatures unchanged.
- **Tests:** the full suite (**518**) must pass after every step and at the end — especially
  `tests/config_port_oracle.rs` and `tests/config_transcode_oracle.rs` (the byte-exact config gates).
  Do NOT edit anything under `tests/`.
- **clippy:** `cargo clippy --release --all-targets` stays at **0**.
- **Scope:** only touch `src/config.rs` → `src/config/**` and the `pub mod config;` line in
  `src/lib.rs`. Do NOT touch `server/cache-patches/`, generated files, or anything else.
- **An unrelated refactor is in flight** in `src/cli/` and `src/commands/` (uncommitted). Do **not**
  touch those files; your `git diff` will show them — ignore them and report only your own `config/`
  changes. Downstream files that `use crate::config::X` should need **no edits** if re-exports are
  complete (if one needs a path tweak, that's a sign an export was missed — fix the export, not the
  consumer, unless the consumer used a private item).
- Use the Edit/Write tools; use `python3` to search file contents (a hook blocks grep/rg on repo
  files — grep works only on `/tmp`). For large verbatim block moves, a `python3` script that writes
  the new module file is fine (the cli refactor did this) — the moved code must be byte-identical.
- Do **not** commit or push — the parent verifies and handles git.

## Final verification (run all, report verbatim)
```bash
cd tools/rs3-cache-rs
cargo build --release                       # clean
cargo test --release                        # 518 passed / 0 failed
cargo test --release --test config_port_oracle --test config_transcode_oracle  # config byte gates green
cargo clippy --release --all-targets        # 0
git status --short | grep -E 'config' ; wc -l src/config/*.rs ; git diff --stat | tail
```

## Done criteria
- `src/config.rs` removed; `src/config/` = `mod.rs` + ~15–25 per-family modules.
- **Public API identical** — the whole crate compiles with no downstream import edits (or only
  trivial ones where a consumer reached for a now-private helper).
- Build green, 518 tests pass, clippy 0, config oracle tests green.

## Budget valve
If you near context/budget limits before all families are extracted, **stop at a green state**
(compiles + all tests pass) and report which families remain in `config.rs`. Partial-but-green is a
success; a broken build is not.
