# CS2 Decompiler / Compiler / Transpiler — Completeness Plan

Goal: move the toolchain from "lossless byte round-trip + best-effort structured
decompile" to "high-coverage, editable, recompilable structured TypeScript across
the full corpus on both builds (910 + 947)".

## Baseline (measured 2026 on 947, full corpus)

`transpile-scripts --all-scripts` on 947 (20,577 scripts, 0 transpile errors):

| Metric | Value |
|---|---|
| Decompile→**editable structured TS** (recompilable) | **1,800 / 20,577 = 8.7%** |
| Falls back to embedded **ASM trailer** (lossless but not editable) | 18,777 = 91.3% |
| Byte-perfect decode→encode→decode roundtrip (sampled 100/build, 910+947) | passing |

Top blockers preventing structured recovery (script-occurrences; avg 1.55/script):

| Blocker | Scripts | % | Owner |
|---|---|---|---|
| `residual_goto` | 12,773 | 62% | control-flow recovery (`cfg.rs`) |
| `commented_branch` | 10,068 | 49% | control-flow recovery (`cfg.rs`) |
| `reverse_unsupported` | 5,270 | 26% | lowering / recovery (`ts_lower.rs`/`expr_recovery.rs`) |
| `residual_pop` | 3,699 | 18% | expression recovery (`expr_recovery.rs`) |

**Read:** byte-fidelity is essentially done; the real completeness gap is the
**decompile/recovery direction**, dominated by control-flow recovery (goto →
structured if/while/switch). Two blockers (`residual_goto`, `commented_branch`)
gate the majority of the corpus.

Definitions of "complete":
- **Lossless**: every script round-trips byte-identically (already true via ASM
  trailer; needs full-corpus proof + CI gate).
- **Editable**: a high fraction decompile to structured TS a human can edit.
- **Recompilable**: editable TS recompiles to byte-identical CS2 (closed loop).

Status legend: `[ ]` todo · `[~]` in progress · `[x]` done

---

## P0 — Make coverage a tracked, repeatable metric (do first) ✅ DONE
- [x] **P0.1** `transpile-scripts` now emits a canonical `transpile_coverage` event (editable %,
  blocker histogram, totals) and a `coverage` block in `transpile-diagnostics.json`.
- [x] **P0.2 / P0.3** Baselines (full corpus, `--all-scripts`):
  - **947: 1800/20577 = 8.7% editable** — blockers: residual_goto 12773, commented_branch 10068,
    reverse_unsupported 5270, residual_pop 3699.
  - **910: 1750/14313 = 12.2% editable** — blockers: residual_goto 9070, commented_branch 7292,
    reverse_unsupported 3076, residual_pop 2234.
  - Re-measure after each P1/P2 change via the `transpile_coverage` event.

## Progress log
- **P0 ✅** coverage metric + baselines (947 8.7%, 910 12.2%). Committed.
- **P4 ✅** decompile audit (findings above) + fixed `escape_string` `\r` recompile bug. Committed.
- **P2 ✅ (partial)** `mod`→`modulo` recovery + `pop_varbit` store fix. Committed. **Empirical
  finding:** these correct real bugs but move coverage ~0 (8.7%→8.6%) because almost every blocked
  script *also* carries a control-flow blocker — a script needs ALL blockers cleared to become
  editable. The `pop_varbit` fix even slightly lowered editable% by converting ~25 *false*-editable
  scripts (whose dropped stores would recompile wrong) into honestly-blocked ones. **Confirms the
  lever is P1.** Remaining P2 (model `random`/`addpercent`/`setbit`/… and categorize
  `reverse_unsupported`) needs authoritative opcode arities (derive from the client `ScriptRunner`)
  and is gated behind P1 for coverage impact.
- **P3 ✅ (done right).** `editable_structured` now requires the structured form to **recompile
  byte-identically** to the original (encode(lower(structured)) == encode(original), original = the
  embedded canonical ASM trailer). Mismatches → `recompile_mismatch` blocker. **This corrected a
  ~2-3x overstatement**: the *honest* gated baselines are **947 = 3.0% (620/20577)** and **910 =
  5.9% (849/14313)** vs the previously-claimed 8.7% / 12.2%. Editable is now provably-recompilable
  by construction; a dev harness (`/tmp/p1validate.sh`: assemble default-vs-`--strict-structured`,
  compare) confirmed 79/79 editable scripts recompile, 0 mismatch. Committed.
- **P1 ✅ (relooper landed; structural blockers solved).** Replaced the ad-hoc emitter with a
  dominator/immediate-post-dominator **region structurer** (`structurer.rs`): if/else joined at the
  ipdom, natural-loop `while`+break/continue, switch case-body regions, conservative goto fallback,
  depth-guarded. Gate-protected. Results:
  - 947 gated editable **3.0% → 4.6%** (620 → 949, +53%); 910 flat at 5.9% (no regression).
  - Original dominant blockers collapsed on both: `commented_branch` 10068/7292 **→ 0**;
    `residual_goto` 12773 **→ 3816** (947), 9070 **→ 2626** (910).
  - **The bottleneck moved downstream** to round-trip byte fidelity: blockers now dominated by
    `reverse_unsupported` (947 8444 / 910 5788 — ts_lower can't re-lower some structured forms) and
    `recompile_mismatch` (947 4436 / 910 3030 — structurer output recompiles to slightly-different
    bytes, e.g. branch polarity / empty-else layout). These gate-block the now-structured scripts.

## P1b — round-trip byte fidelity (the new frontier; gated, safe)
The relooper structures the control flow; the remaining editable gain is locked behind making that
structure recompile **byte-identically**. Gate-protected, measured via `transpile_coverage`.

**Current gated baseline (full corpus): 947 = 7689/20577 = 37.37%, 910 = 5698/14313 = 39.81%**
(up from the post-relooper 4.6%/5.9% — an 8.1x / 6.7x session gain, all byte-identity gated).

**Highest-blast instruction-order fix: emit the dead-return epilogue.** The RT7 compiler appends an
unreachable `push <default>; return` after a script's real return. The structurer walked only
reachable blocks, dropping it — so every such recompile was shorter than the original and branch
targets shifted (`length:structured_shorter` + a large share of `branch:operand`). Emitting unvisited
blocks in original order (with return-type inference taught to stop at unreachable code) reproduces
the tail byte-for-byte. **947 +1473, 910 +736; length:structured_shorter eliminated, branch:operand
3091->2033.** (Earlier this pass: terminating-then jump omission, +271/+208.)

Remaining instruction-order tail is heterogeneous and lower-blast: `branch:operand` cascades from
assorted small order/length diffs, operand-evaluation-order swaps (`push_int_local` vs
`push_constant_string`, partly recovery reorderings / variant-flag opcode arity like `db_find`),
`switch:operand`. With recompile_mismatch down to 4858, **`residual_goto` (5291) is now the single
largest blocker** — control-flow structuring, a different bucket.

**Foundation: client-extracted opcode stack-effect table.** Rather than hand-model opcodes one at a
time, `scripts/extract-stack-effects.py` parses every handler in the client `ScriptRunner` into
`data/stack-effects.txt` (1,097 commands, build independent: pops, pushes, pushed type). The recovery
(`stack_effect`) consults it for pop/push counts after the hand-verified arms; the lowerer types
generic command results from it. **947 +531, 910 +449; residual_pop 2314->1529 (947), ~halved (910).**
This replaces the long tail of unmodeled opcodes with a single regenerable source of truth.
`recompile_mismatch` is still the dominant blocker, now led by `branch:operand` (947 3003) and
`length:structured_shorter` (1096); `residual_pop` was roughly halved and `reverse_unsupported`
keeps shrinking. All data-driven via the `recompile_mismatch_cause:*` / `reverse_unsupported_cause:*`
histograms.

Deep-work pass on the three dominant buckets (each gate-verified):
- **✅ residual_pop (unmodeled opcodes).** Extracted exact stack effects from the client ScriptRunner
  for the component getters (`if_getwidth`/`cc_getheight`/`getx`/`gety`/`gethide`/…) and value ops
  (`tostring`/`max`/`min`/`string_length`/`oc_name`/`scale`/`testbit`/`append`/`movecoord`/
  `clientclock`); made the CC/IF recovery arm push value-producing results; added a getter lowering
  arm (arg-count cc/if + result type). **947 +671, 910 +566; residual_pop ~halved.**
- **✅ residual_goto (control flow).** In-loop `return`s were miscounted as loop exits, making search
  loops `LoopExit::Multi` → goto fallback; treat terminal successors as inline returns so they
  structure as `while`. **947 +8, 910 +30** (mostly readability — many move to recompile_mismatch).
- **✅ branch:operand (layout fidelity).** `lower_if` emitted a stray `branch -> end` after a
  terminating then-body that the original compiler omits, shifting every downstream target. Skip it
  when the then-body returns/breaks/continues. **947 +271, 910 +208; branch:operand 3758->3003.**

Done this session (each gate-verified, byte-identity preserved):
- **✅ Rank `recompile_mismatch` by cause.** `recompile_fidelity_check` classifies the first
  divergence into a low-cardinality `recompile_mismatch_cause:<orig>-><emitted>` bucket. Turned the
  opaque blocker into a ranked histogram — every fix below was chosen from it.
- **✅ Typed-constant int encoding.** The RT7 corpus encodes int constants as the typed-constant
  `push_constant_string` (int tag), not `push_constant_int`. Switched the `NumberLiteral` lowering;
  fixed `validate.rs` to resolve the typed-constant stack effect from its operand tag (was a latent
  false `StackUnderflow`). Cleared the #1 cause (2527 on 947). +81/+77 editable.
- **✅ Void-call return-type inference in the gate.** The bulk `--all-scripts` path built its catalog
  `.without_return_types()`, so the gate's reverse context treated every void sub-proc call as
  int-returning and emitted a spurious `pop_*_discard` (the #2 cause, return->pop_int_discard, 1532).
  Now mirrors the renderer's lazily-inferred signatures into the reverse context. **+1673 (947) /
  +1101 (910) editable** — by far the largest win; the spurious discard also cascaded into length
  mismatches, so fixing it unblocked more than its first-divergence count.
- **✅ UI `if_*` set-method lowering.** The decompiler renders generic interface set-methods via
  `sanitize_camel` (capital-first, `if_sethide`->`UI.Sethide`), distinct from the explicit
  lowercase-first `cc_*` names. The lowering lowercased and always picked `cc_<lower>`, so
  `UI.Sethide(component, flag)` recompiled to `cc_sethide`. Now picks `if_<lower>` for capital-first
  methods backed by an `if_*` opcode. **+710 (947) / +592 (910)** — the whole `if_*` set family.
- **✅ Centralized int-constant encoding.** Routed every plain int-constant emit site (boolean/id/
  enum/component/negated, not just NumberLiteral) through one `emit_int_constant` helper that uses
  the typed-constant opcode. Cut `push_constant_string->push_constant_int` 1173->24; **+223 (947) /
  +154 (910)**, zero regression.
- **✅ Rank `reverse_unsupported` by cause.** `<blocker>_cause:*` histogram now covers both blockers.
  Top: "unsupported call expression" (1701), `ui_hook` (947), `structured_parse` (567).
- **✅ Generic CS2 command lowering.** The lowerer mapped only gosub + UI calls; every other command
  the decompiler renders as `command(args)` bailed. Added `ReverseCompileContext::resolve_command`
  (deterministic inverse of `sanitize_command`) and lower the call to its opcode. **+38 (947) /
  +16 (910)**; "unsupported call expression" 1701->912. Reports `Void` (void statements round-trip;
  value-producing commands stay gate-blocked pending result-type recovery).
- **✅ Reserved-word escaping.** The `enum` opcode (and any reserved-word name) rendered as a bare
  `enum(...)` call → invalid TS → oxc re-parse failure (`structured_parse`, 567). Escape reserved
  words in `sanitize_ts_ident` (round-trip-safe). A correctness fix first (valid TS for ~565
  scripts); `structured_parse` 567->2, editable +15/+14.
- **✅ Generalize SETON-hook lowering.** `emit_ui_hook_call` hardcoded 4 hooks; the rest of the
  `cc_seton*`/`if_seton*` family bailed (`ui_hook`, 947 — largest remaining lowering gap). Derive the
  cc_/if_ pair from `UI.Seton<suffix>` (arg-count split) + route the hook's own constant pushes
  (callback id, watcher ids/count) through the typed-constant `emit_int_constant`. **+281 (947) /
  +218 (910)**; `ui_hook` 947->4.

- **✅ Branch/switch jump-target off-by-one (the big one).** `decode_operand` computed branch
  targets as `index + offset`, but the client jumps to `index + offset + 1` (ScriptRunner does
  `pc += operand` then the dispatch loop pre-increments `instructions[++pc]`; switch is the same) —
  verified three ways against the client source. The bug was invisible to byte round-trip (decode
  and encode shared the wrong convention) but shifted every CFG branch target one instruction early,
  so genuine forward branches looked like no-ops and live guard-clause bodies looked like unreachable
  dead code. Fixed with +1 in decode / -1 in encode for branches and switches. **+761 (947) / +716
  (910) editable** — the largest single fix, and it corrects the decompiled control flow for every
  branching script (readability), not just the gated ones.

### Correction to an earlier "dead code" claim
A prior pass concluded the dominant `branch_equals:operand` residual was corpus dead code (no-op
branches + unreachable bodies in `bool_to_int`/`meslayer_mode1-4`/`script48`). **That was wrong** —
it was the symptom of the off-by-one above. Those are genuine guard clauses (`if (cond) return;
<body>`); with the corrected targets they structure correctly and recompile byte-identically. Lesson:
byte round-trip alone cannot validate control-flow interpretation — cross-check the client VM.

Next levers (genuine capability, not corpus artifacts):
- [ ] **Recover result types for generic commands** so value-producing command calls (the ~912
  residual "unsupported call expression") lower with the right discard/assignment type instead of
  `Void`. Needs a *typed* per-opcode effect (int/obj/long push) in the lowerer — reuse the typed
  model in `validate.rs::stack_effect_for` rather than the count-only `expr_recovery::stack_effect`.
- [ ] **`ui_method` (128), `callback_watcher` (104)**: smaller lowering gaps surfaced by the
  `reverse_unsupported_cause:*` histogram.
- [ ] Remove the now-dead `StructuredEmitter` from `cfg.rs` (the relooper replaced it).

## P1 — Control-flow recovery (the dominant lever: ~62%+49% of corpus)
Target `cfg.rs` (build_cfg / emit_structured) + the branch/goto handling.
- [ ] **P1.1** Eliminate `residual_goto`: reconstruct structured loops (`while`/`do`/early-exit) and
  nested `if/else` from the branch graph so no goto remains. Biggest single win (62%).
- [ ] **P1.2** Eliminate `commented_branch`: fold the branches currently emitted as comments into
  real structured conditions (49%). Likely the same CFG work as P1.1.
- [ ] **P1.3** Add structured-recovery regression tests over a representative script set; assert the
  editable % rises and these blockers fall toward 0.

## P2 — Expression recovery (`residual_pop` 18%, `reverse_unsupported` 26%)
Target `expr_recovery.rs` + `ts_lower.rs`.
- [ ] **P2.1** Fold `residual_pop`: leftover stack pops not absorbed into expressions/statements →
  recover into assignments/discards so no residual pop remains.
- [ ] **P2.2** Enumerate the distinct `reverse_unsupported` causes (instrument the diagnostic to
  carry the specific construct/opcode), then implement the top offenders. Currently a catch-all —
  break it down before building.
- [ ] **P2.3** Lowerer feature gaps surfaced by the compiler audit that also block recompile:
  subtraction on 910 (synthesize `add(a, neg(b))` or document as engine-unsupported), `goto`,
  string arrays — implement or give precise unsupported diagnostics.

## P3 — Close the recompile loop (prove editable == recompilable)
- [ ] **P3.1** Add a **structured-recompile roundtrip** test: for every `editableStructured` script,
  decode → structured TS → `assemble-script --strict-structured` → assert byte-identical to source.
  (Today only decode→encode is proven; the structured path is only spot-checked by `reversible_ts`.)
- [ ] **P3.2** Run it over the full editable set on both builds; fix any structured-recompile
  mismatch. As P1/P2 raise the editable set, this gate grows with it.

## P4 — Adversarial audit of the decompile direction ✅ DONE (findings below)
Audited `cfg.rs`, `expr_recovery.rs`, `codegen.rs`, `writer.rs`, `structured_writer.rs`,
`structured.rs`. Key findings (drive P1/P2):
- **cfg.rs structurer** recognizes only a single forward if/else diamond + single back-edge while;
  no post-dominator/join analysis, **switch cases emitted with EMPTY bodies** (cfg.rs:687 → targets
  strand as goto), multi-exit loops unhandled. Targeted wins: **switch-body reconstruction** (cheap,
  big), **flush-unvisited-blocks guard** (fixes a *silent block-drop* miscompile). Full fix = a
  Relooper-style pass (post-dominator if/else join + back-edge loops) — larger follow-up.
- **expr_recovery.rs** `residual_pop` dominated by **unmodeled arithmetic/bit opcodes** (`random`,
  `randominc`, `interpolate`, `addpercent`, `setbit`/`clearbit`/`testbit`, `pow`, `invpow`) → 0/0
  stack effect strands operands; **`mod` vs `modulo`** name mismatch; **`pop_varbit` decoded as a
  push** (store value stranded + lost). `reverse_unsupported` is an opaque catch-all — the real
  error is captured then discarded; categorize it.
- **codegen/writers**: `writer.rs` (whole `Writer`) + `codegen.rs::{generate_program,
  format_instruction,format_operand_raw,format_command_name,sanitize_ts_ident,escape_ts_string}` are
  **dead**; live path is `structured.rs::render`. `structured.rs::escape_string` **omits `\r`** (and
  `\t`/U+2028-9) → a CS2 string constant with CR breaks oxc re-parse on recompile (correctness bug).
- Panic surface low across all (the `unreachable!`s are guarded; convert to soft fallbacks as
  defense-in-depth since this runs over untrusted cache bytes).

## P5 — Full-corpus & cross-build byte fidelity (lossless guarantee)
- [ ] **P5.1** Run the byte-perfect roundtrip with `RS3_ROUNDTRIP_LIMIT=0` on the full 20,577 scripts
  for **both** 910 and 947; fix any non-byte-identical script.
- [ ] **P5.2** Promote a bounded full-corpus (or large-sample) roundtrip to a CI-runnable gate
  (env-gated on cache availability, like the existing tests).

## P6 — Generative / fuzz testing for the parsers
- [ ] **P6.1** `proptest` roundtrip for `js5` group pack/unpack and `script` encode/decode:
  `decode(encode(x)) == x` over arbitrary valid structures.
- [ ] **P6.2** Fuzz the decoder against malformed bytes (the C4–C9 bails should make it fail cleanly,
  never panic) — confirms robustness now that silent fallbacks are gone.

## Sequencing & exit criteria
1. **P0** first (can't manage what you don't measure; cheap).
2. **P1** is the highest-leverage work — it alone should move editable % from ~9% toward a majority.
3. **P2** follows / overlaps P1 (shared recovery code).
4. **P3** continuously, as the editable set grows.
5. **P4/P5/P6** harden and prove.

Exit criteria for "complete enough":
- Editable structured-recovery rate ≥ ~90% on both builds (P1+P2), with remaining cases carrying
  precise, enumerated unsupported diagnostics (not catch-alls).
- 100% of `editableStructured` scripts recompile byte-identically (P3).
- Full-corpus decode→encode byte-identical on both builds, gated in CI (P5).
- Decompile direction audited (P4); parsers fuzzed (P6).

## Notes
- Prior work: [cs2-compiler-audit.md](cs2-compiler-audit.md) hardened the **compile** direction
  (verification, no silent corruption, accurate filtered signatures). This plan targets the
  **decompile/recovery** direction, which is where the coverage gap lives.
- The ASM trailer fallback means the tool is already **lossless** today; this plan is about making
  the output **editable + recompilable**, i.e. fulfilling the "transpiler" promise.
