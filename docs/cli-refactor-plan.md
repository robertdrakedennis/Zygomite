# `cli.rs` decomposition — architecture refactor plan

**Goal:** decompose the 9,666-LOC god-module `src/cli.rs` into (a) the clap surface, (b) a thin
dispatch, and (c) per-command domain-module handlers `run(ctx, opts)` sharing a `CommandContext`.
**Strictly behavior-preserving** — pure structure, no logic or output changes.

## Why (evidence, measured 2026-06-15)
- `src/cli.rs` = **9,666 LOC, 168 fns, 31 `run_*` handlers, 6 clap derive blocks, one ~700-line
  `pub fn run()` dispatch** (≈1315→2020+). Nearly 2× the next-largest file.
- Shared environment is threaded by hand as positional args across every handler
  (`FlatCache` ×67, `subbuild` ×89, `data_dir` ×109). Worst case `run_migrate_check` takes **14
  positional args**, the first ~4 of which are pure context boilerplate.
- **The clean pattern already exists in this file.** The newer commands delegate correctly:
  `Command::OverlayPlan => overlay_plan::run_overlay_plan_command(OverlayPlanCommandOptions {..})`,
  `Command::ExtractCs2Registry => cs2_registry::run(Cs2RegistryOpts {..})`,
  `extract-protocol`/`generate-protocol` similarly. clap args → a named `Options` struct → a
  **domain-module `run()`**; cli.rs only maps. The older ~28 commands never got migrated and keep
  their logic inline as god-handlers. **This refactor finishes the existing pattern — it does not
  invent a new one.**

## Target architecture

### 1. `CommandContext` — resolve the shared env once
Create `src/cli/context.rs` (or `src/command_context.rs`) with a `CommandContext` that bundles what
the handlers currently receive piecemeal. Derive the exact fields from the resolution block in the
current `run()` and the common handler params — at minimum:
- `cache: FlatCache`
- `tar_path: PathBuf`
- `version: RuntimeVersion` (build + subbuild)
- `data_dir: PathBuf`
- lazily-built `OpcodeBook` and `ReverseCompileContext` — today these are rebuilt inline in `run()`
  or per-handler; expose them as memoized accessors (`OnceCell`/`once_cell`-style method:
  `ctx.opcode_book()`, `ctx.reverse_ctx()`), built on first use so cheap commands don't pay for them.

Build it once in the dispatch for commands that need the cache; pass `&CommandContext` to handlers.

### 2. Per-command handlers move to their domain module
Each god-handler `fn run_X(cache, tar, data_dir, version, <args…>)` becomes
`pub fn run(ctx: &CommandContext, opts: XOpts) -> Result<()>` living in the module that owns its
domain. `XOpts` is a named struct of the *command-specific* args only (context comes from `ctx`).

**Destination rule:**
- If a domain module already exists, move the handler into it: `migrate` (migrate.rs, 2.6k),
  `dep_tree` (dep_tree.rs, 1.8k), `validate` (validate.rs, 3.2k), `transpile`, `config`,
  `interface`, `cs2`/`cs2_registry`/`cs2_coverage`/`cs2_datagen`/`cs2_javagen`, `protocol_registry`,
  `font`, `explain`/`explain_loc`/`explain_transitive`, `overlay_plan` (already done). Name the
  entry `run_<verb>` (e.g. `migrate::run_check`, `migrate::run_script`, `dep_tree::run_interface`).
- If there is no obvious home (e.g. `unpack`, `models`, `assemble-script`, `verify-map-archive`,
  `dump-raw-flat`, `dump-refs`, `prepare-overlay`, `build-collision`), create `src/commands/<name>.rs`.
- **Helpers:** determine each private helper's callers by searching. A helper used by **one**
  command moves **with** that command. A helper used by **2+** commands moves to `src/cli/shared.rs`
  (or the most relevant domain module if clearly owned there). Do not duplicate helpers.

### 3. Thin the dispatch
`src/cli.rs` (or `src/cli/mod.rs`) keeps only: the clap definitions (optionally split into
`src/cli/args.rs`), context construction, and a flat route table mapping each clap variant → its
`XOpts` → the module `run`. Replace the stacked early-returns + 700-line match with a uniform router
that resolves `ctx` only for commands that need the cache. **Target: cli surface + dispatch under
~1.5k LOC**, everything else redistributed.

### Before / after (the migrate-check handler — the worst offender)
```rust
// before — logic + 14 positional args (4 boilerplate) live in cli.rs
fn run_migrate_check(cache, tar_path, data_dir, interface_group, out_file, audit_dir,
                     target_version, source_cache_tar, source_build, source_subbuild,
                     enable_remap, remap_buffer, validate_target, allow_heuristic_sites) { .. }

// after — logic in migrate.rs; cli.rs just maps clap → opts
migrate::run_check(&ctx, MigrateCheckOpts {
    interface_group, out_file, audit_dir,
    source: MigrateSource { tar: source_cache_tar, build: source_build, subbuild: source_subbuild },
    remap: RemapOpts { enabled: enable_remap, buffer: remap_buffer },
    validate_target, allow_heuristic_sites,
})
```

## Execution discipline — incremental, ALWAYS GREEN
Do it in this order, and **after every command moved, run `cargo build --release` and the relevant
tests; never leave the tree non-compiling or a test failing between steps.**
1. **Step 1 — template.** Add `CommandContext`; convert **2–3 cache-using handlers** (e.g.
   `transpile-scripts`, `migrate-check`, one `dep-tree-*`) to `(&ctx, Opts)`. Verify green. This is
   the template every other command copies.
2. **Step 2 — peel groups.** Migrate the remaining commands group by group (dep-tree family,
   cs2 family, config family, models/unpack/assemble, validate, prepare-overlay, etc.). One coherent
   commit-sized chunk at a time, each compiling + tests green.
3. **Step 3 — thin dispatch.** Once handlers are out, collapse the early-returns + mega-match into
   the uniform router; split clap into `src/cli/args.rs` if it helps.

## Hard guardrails (do not violate)
- **Behavior-preserving only.** No logic changes, no changed CLI output, no changed byte output.
- **Tests:** the full suite (currently **518 tests**) must pass after every step and at the end.
  Do **not** edit anything under `tests/` to make them pass — identical behavior is the requirement.
- **Byte-exact gates stay green:** the oracle tests (`ritual_port_oracle`, `font_oracle`,
  `interface_port_oracle`, `explain_loc_oracle`, …) and the RuneScript byte gate
  (`RS3_RUNESCRIPT_GATE`, 0 failures on 910 + 948).
- **clippy:** `cargo clippy --release --all-targets` stays at **0**. The `#[allow(clippy::too_many_arguments)]`
  currently on `run_cs2_port` should be **removed** once it takes `(&ctx, opts)` (the whole point);
  keep a *justified* `#[allow]` only if a handler genuinely still needs many distinct args.
- Do **not** touch `server/cache-patches/`, generated files, or anything outside
  `tools/rs3-cache-rs/src/`. Use the Edit tool; use `python3` to search file contents (a hook blocks
  grep/rg on repo files; grep works on `/tmp`).
- Do **not** commit or push — the parent verifies and handles that.

## Final verification (run all, report results)
```bash
cd tools/rs3-cache-rs
cargo build --release
cargo test --release            # expect 518 passed / 0 failed
cargo clippy --release --all-targets   # expect 0 errors/warnings
# RuneScript byte gate, both builds — expect 0 failures each
RS3_RUNESCRIPT_GATE=1 RS3_CACHE_DIR=../../cache/unpacked/948 target/release/rs3-cache-rs \
  --cache-dir ../../cache/unpacked/948 --data-dir data --build 948 --subbuild 1 \
  transpile-scripts --out-dir /tmp/948-rsgate --all-scripts 2>&1 | grep -oE '"runescript_gate":[0-9]+|"editable":[0-9]+'
RS3_RUNESCRIPT_GATE=1 RS3_CACHE_DIR=../../cache/unpacked/910 target/release/rs3-cache-rs \
  --cache-dir ../../cache/unpacked/910 --data-dir data --build 910 --subbuild 1 \
  transpile-scripts --out-dir /tmp/910-rsgate --all-scripts 2>&1 | grep -oE '"runescript_gate":[0-9]+|"editable":[0-9]+'
git -C . diff --stat | tail -5
```

## Done criteria
- `cli.rs`/`cli/mod.rs` (the surface + dispatch) under ~1.5k LOC; all 31 command bodies live in
  domain/`commands` modules as `run(ctx, opts)`.
- `CommandContext` threaded; positional-arg explosion gone (no handler >~6 params).
- Build green, 518 tests pass, clippy 0, RuneScript gate 0 on both builds.

## Budget valve
If you approach context/budget limits before all 31 are migrated, **stop at a green state** (compiles,
all tests pass) and report exactly which commands remain inline. A partial-but-green migration is a
success; a broken build is not.
