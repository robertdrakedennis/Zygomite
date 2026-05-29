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

**Current gated baseline (full corpus): 947 = 3678/20577 = 17.87%, 910 = 2791/14313 = 19.50%**
(up from the post-relooper 4.6%/5.9% — a 3.9x / 3.3x session gain, all byte-identity gated).
`recompile_mismatch` remains the dominant blocker (947 ~6600, 910 ~4400), now followed by
`reverse_unsupported` (947 2770, 910 2123). Both are data-driven via `recompile_mismatch_cause:*`
and `reverse_unsupported_cause:*` histograms.

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

### Key finding: a large slice of the residual is corpus dead code, not a tool gap
Investigating the dominant `recompile_mismatch` causes showed many are **degenerate / dead-code
scripts**, not fixable by better structuring:
- `branch_equals:operand` (~2540) is overwhelmingly **no-op forward branches** (target == next
  instruction) compiled ahead of an early `return`, leaving the rest of the script **unreachable**
  (`bool_to_int`, `meslayer_mode1-4`, `script48`). The CFG-adjacency handling renders these as
  empty-then `if (cond) {} else {body}`, but the original bytes contain the no-op branch + dead body
  that clean structuring correctly omits — so byte-identity is impossible and the ASM-trailer
  fallback is the right answer (they stay non-editable). A sampled 200: ~42% empty-no-else
  (degenerate), ~52% empty-then-with-else (also degenerate no-op-branch), ~5% genuine.
  An attempted skip-if-false lowering for these netted **0** (correctly) and was reverted.
- The structurer already handles **genuine** if/else (including both-arms-return) correctly; those
  are in the editable set. So the practical ceiling for *clean-structured + byte-exact* on this
  corpus is much nearer the current ~18-20% than the raw cause counts imply.

Next levers (genuine capability, not corpus artifacts):
- [ ] **Recover result types for generic commands** so value-producing command calls (the ~912
  residual "unsupported call expression") lower with the right discard/assignment type instead of
  `Void`. Needs per-opcode push type/count in the lowerer.
- [ ] **`ui_hook` (947)**: the SETON-hook lowering (`emit_ui_hook_call`) bails for hook variants not
  in its table; extend it / make it data-driven like the `if_*`/`cc_*` set methods.
- [ ] **`structured_parse` (567)**: the decompiler emits TypeScript that fails oxc re-parse — a
  generation-correctness bug. Capture the oxc diagnostics and fix the malformed output.
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
