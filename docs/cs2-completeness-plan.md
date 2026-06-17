# CS2 Decompiler / Compiler / Transpiler — Completeness Plan

> **STATUS (2026-06-15) — this grind is essentially done; read the baselines below as history.**
> Both reversible surfaces now recompile byte-identically across the full corpus on the current
> builds: the RuneScript byte gate (`RS3_RUNESCRIPT_GATE`) is **0 failures on 910 (14,313 editable
> scripts) and 948 (20,621)**, and the TypeScript surface was already byte-exact. `editable_structured`
> now holds for ~100% of the corpus — the 8.7% / 12.2% "editable" figures below are the historical
> 2026 starting point (947 / 910), kept for the record; §P1b already records the climb to 100%. The
> only open axis is **quality, not fidelity or editability**: ~69% structure to clean nested control
> flow, ~25% fall back to a still-byte-exact, still-editable linear-goto form, ~2% to stack pseudo-ops
> (the `branch:operand` / `switch:operand` tail). Current state + mechanisms: the
> `cs2-transpiler-real-state` memory and `plans/tooling/cs2-runescript-decompiler.md`. (Note: the donor
> build is now 948, not 947 — the 947 columns below are historical.)

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

**Current gated baseline (full corpus): 947 = 20577/20577 = 100.00%, 910 = 14313/14313 = 100.00%**
(up from the post-relooper 4.6%/5.9% - full corpus closure, all byte-identity gated).

**Current default high-TS control-flow baseline (measured 2026-05-31):** default
`transpile-scripts --all-scripts` now tries aggressive high-control-flow output first, retries the
previous conservative high-control-flow form if the byte gate rejects that output, then falls back to
reversible linear output only if both high forms fail. `--output-style reversible` forces the old
conservative form. New `high_ts_coverage` diagnostics show:
- **947:** `controlFlowMarkers` **12619 -> 5196**, `noVisibleLowLevelMarkers` **7634 -> 14154**,
  `blocked: 0`; fallback reasons `gate_mismatch: 5037`, `residual_goto: 158`, `stack_goto: 1`.
  Occurrence totals: `gotoCalls: 197150`, `labelCalls: 126280`.
- **910:** `controlFlowMarkers` **8904 -> 3675**, `noVisibleLowLevelMarkers` **5274 -> 9961**,
  `blocked: 0`; fallback reasons `gate_mismatch: 3570`, `residual_goto: 103`, `stack_goto: 2`.
  Occurrence totals: `gotoCalls: 159701`, `labelCalls: 100046`.
  `high_ts_coverage` also includes `fallbackGateBlockers`, preserving the primary high-form gate
  blockers even when reversible fallback succeeds, plus total marker occurrence counts so partial
  high-TS improvements can be tracked when per-script marker counts stay flat. Current dominant
  buckets are `branch:operand` (947 2908 / 910 2263) and `switch:operand` (947 1090 / 910 742).

**goto / shared-block support (linear fallback).** Irreducible control flow (shared return/join
blocks, jump tables) can't be nested into if/while/switch, so it stayed `residual_goto`-blocked.
Added a linear fallback: when nested structuring leaves a goto, re-emit the whole script
block-by-block in original order with jump targets labelled and branches as `goto`/`if (cond) goto`
(`StructuredStmt::Label`; goto/label render + parse; lowering of goto→branch, label→mark, and a
single conditional branch for `if(cond)goto`). Original order is preserved, so it recompiles
byte-identically; `residual_goto` is removed as a blocker category (the gate decides). **947 +852,
910 +500.** Caveat: `assemble-script`'s post-compile validator (`validate.rs`) has its own,
incomplete opcode stack model, so editable scripts using commands it doesn't model (e.g.
`map_members`) need `--no-verify` — a pre-existing validator gap, not a byte-fidelity issue
(the recompile gate confirms identity). Wiring the client-extracted table into `validate.rs` would
close it.

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
time, the `extract-stack-effects` subcommand parses every handler across the client clientscript
package (`ScriptRunner.java` + the `*Ops.java` classes) into
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
- **✅ `concat(...)` and 910 subtraction lowering.** The decompiler renders `join_string N` as
  `concat(a, b, ...)`; lower it back to `join_string` with a count operand. Also lower binary `-`
  to 910's legacy `quickchat_dynamic_command_add` when `sub` is absent. **+292 (947) / +958 (910)**;
  `reverse_unsupported_cause:other` collapsed from 2043→342 (947) and 2521→258 (910) before the UI
  pass.
- **✅ Generic UI camel/snake inverse.** Recovered missing UI stack effects for setobject variants,
  button/grid helpers, scriptqueue clear, and related methods; lower `UI.SetobjectNonum`,
  `UI.SetparamInt`, `UI.GetmodelangleX`, etc. by matching the decompiler's `sanitize_camel` output
  and argument count back to the `cc_`/`if_` opcode. **+114 (947) / +94 (910)** on top of concat/sub;
  `reverse_unsupported_cause:ui_method` is now 66 (947) and 0 (910).
- **✅ Callback watcher/target lowering.** Lower rendered watcher names (`varplayerint_3814`,
  dynamic `local_int_*`, enum/component constants) and enum-rendered callback targets back to int ids.
  **+108 (947) / +107 (910)**; `callback_watcher` is now zero for both builds and
  `callback_target` is 1 (947) / 0 (910).
- **✅ Array define/sort recovery.** `define_array` now consumes and renders its size operand
  (`define_array_0(4)`) and `array_sort` has a stack effect so it remains in structured output.
  This mostly moves scripts from `reverse_unsupported` into the honest byte-fidelity gate (+1/+1
  editable), cutting `reverse_unsupported_cause:other` to 260 (947) / 178 (910).
- **✅ Switch/layout lowering.** Linear fallback switches with `case X: goto(N)` now lower the switch
  table directly to the original target labels instead of trampoline case bodies, and final switch
  cases fall through instead of emitting a dead `branch end`. **+2343 (947) / +1774 (910)** from the
  array baseline; `switch:operand` is down to 403 (947) / 202 (910).
- **✅ `branch_not` semantics.** `branch_not` is binary int `!=` (`data/stack-effects.txt` says
  `2 -> 0`), not unary false. Recovery now renders `x != y`, lowering emits `branch_not` directly,
  and validation pops two ints. **+1000 (947) / +844 (910)**; this also removed large
  local-vs-constant cascades.
- **✅ `pop()` stack-drain lowering.** Multi-return command drains now recompile through
  `local = pop()` stores and `pop()` arguments (existing stack values) instead of blocking on
  residual stack syntax. **+498 (947) / +376 (910)**; the explicit `residual_pop` blocker is gone,
  with remaining issues ranked by concrete recompile mismatch or reverse-unsupported cause.
- **✅ Operand-preserving UI calls + find/create modes.** `UI.find`, `UI.findInterface`, and
  `UI.create` now preserve nonzero opcode byte operands. Generic UI commands, getters, hooks,
  scriptqueue calls, and no-arg send/delete commands use a reversible `WithMode(..., mode)` suffix
  when the original `cc_`/`if_` operand is nonzero. **+229 (947) / +206 (910)** across the two UI
  mode passes; buckets such as `cc_param:operand`, `cc_getwidth:operand`, `cc_setposition:operand`,
  and hook operand mismatches are cleared.
- **✅ Multi-result call arguments + dynamic callback targets.** Multi-value command fields used as
  call arguments lower through a single producer command, `getminimenutarget()` coalesces like other
  multi-result drains, local/`pop()` callback targets lower as dynamic script ids, and raw hook
  descriptor fallbacks no longer block lowering. Along with varbit identity preservation
  (`_transmog` suffixes and exact raw ids), this removes the last reverse-only blockers:
  **`reverse_unsupported` is now zero on both 910 and 947.**
- **✅ CFG branch successor + loop-exit layout.** CFG now treats `branch` as unconditional (no
  fallthrough successor) and reads conditional+`branch` pairs as true target from the conditional
  operand, false target from the following `branch`. The structurer no longer emits the loop-exit
  block inside loop bodies, and the lowerer points single-statement `break`/`continue` arms directly
  at loop labels. This matches the original compact shape for `while (true) { if (...) continue;
  else break; }` loops. **+1992 (947) / +574 (910)** from the prior tracked baseline;
  `length:structured_longer` is gone and broad loop-exit `return->branch` fallout collapsed.
- **✅ Fixed-point cross-script return signatures.** Full `transpile-scripts --all-scripts` used a
  return-type-free catalog and inferred referenced script signatures lazily, so calls to later
  scripts could be treated as value-producing or unresolved while rendering earlier scripts. Seed
  the renderer and recompile gate with a fixed-point return-type map across all scripts. This
  restores missing helper calls such as `interface_inv_update_big(...)` and removes large cascades
  from wrong void/value call classification. **+1524 (947) / +1168 (910)**; this moved the
  then-current baseline to 81.69% (947) and 86.87% (910).
- **✅ Void command + missing stack-effect recovery.** Generic 0-pop/0-push commands now render as
  statements instead of disappearing, and recovery/validation knows stack effects for
  `db_filter_value`, `cam2_setlookatmode`, `cam2_setpositionmode`, `cam2_setpositionentity_player`,
  and `error`. This restores no-arg void calls (`notifications_init`), camera mode setters, DB filter
  calls, and error payloads. **+131 (947) / +117 (910)** before the dead-branch pass.
- **✅ Dead branch-only block preservation.** Predecessorless unreachable `branch` blocks emitted
  after returns now stay as `goto` labels instead of collapsing into the target. This preserves
  compiler padding such as `return; branch end; ...` and reduces branch target drift. Additional
  **+250 (947) / +173 (910)**.
- **✅ Interface option payload arities.** `if_setop*`/drag option handlers route through shared
  client helpers, so extracted stack effects saw only the component pop and recovery dropped option
  index/text/cursor payloads. Manual payload arities restore calls such as
  `UI.Setop(index, text, component)` and `UI.Setopbase(text, component)`. **+530 (947) / +397
  (910)**.
- **✅ Multi-value return preservation.** CS2 `return` consumes every live typed stack value; recovery
  previously kept only the top value, shortening branches and corrupting multi-return helpers.
  Structured TS now renders `return stack(a, b, ...)`, and lowering emits every value in order,
  including multi-result command prefixes such as `viewportgeteffectivesize().width/.height`.
  **+742 (947) / +473 (910)**.
- **✅ Build-specific DB find/filter arities.** Build 919+ `db_find`, `db_find_with_count`, and
  `db_find_refine` carry a third `basevartype` argument. Recovery/validation now model that shape,
  plus the `db_filter_*` stack effects. This restores table-id searches such as
  `dbfind(503808, key, 0)`. **+216 (947) / +0 (910)**.
- **✅ Build-specific string command arities.** Build 919+ `tostring` consumes `(value, radix)` while
  910 consumes only `value`; build 936+ `tostring_long` has the same extra radix argument. Recovery,
  return inference, CFG construction, and validator fallback now use build-aware opcode effects and
  the string command signature table, so 947 scripts such as `script47` preserve the radix without
  regressing 910's one-argument form.
- **✅ Source-backed and manual stack-effect arities.** Added command-signature coverage for
  inventory/config/core/quest/detail/interface/streaming families and targeted overrides for
  interface child creation/options, clan find, avatar base setters, camera helpers, misc unknowns,
  `inv_stockbase`, and `if_sethflip`/`if_setvflip`. This preserves payloads that client helper
  wrappers hid from static extraction. Net from this checkpoint and the literal/operand work below:
  **+954 (947) / +187 (910)**.
- **✅ Literal and operand provenance.** `push_constant_int` now round-trips through `intconst(...)`,
  typed long constants through `longconst(...)`, including `i64::MIN`; generic nonzero opcode byte
  operands lower with `WithMode(..., mode)`; var transmog refs keep `_transmog`; UI getters resolve
  by arg count before name fallback. These moves shifted reverse failures into honest byte-layout
  mismatches and removed typed literal parse blockers.
- **✅ Explicit empty discards.** Empty-stack discard opcodes now render as typed pseudo calls
  (`popintdiscard`, `popstringdiscard`, `poplongdiscard`) and lower back to `pop_*_discard` without
  adding an extra discard. This removed the `pop_int_discard->return` bucket and restored void
  helpers that discard multiple return slots.
- **✅ Switch default fallthrough bodies.** CFG now splits switch fallthrough only when it is a real
  non-branch default body (including empty-case switches), keeping branch-trampoline switches in the
  byte-faithful linear form. Structured TS can parse/render/lower `default:` bodies, and lowering
  places immediate default bytecode before case bodies to match RT7 layout. **+93 (947) / +57
  (910)**, clearing the `push_var->cam_reset` / `push_var->cam_smoothreset` default-camera family
  without regressing branch-trampoline switch tables.
- **✅ Shared return branch preservation.** If original bytecode has an explicit unconditional
  `branch` into a return-only block, the structurer now chooses the existing byte-faithful linear
  form instead of collapsing that edge to an inline `return`. This removes the `branch->return`
  bucket and cuts nearby branch target drift. **+45 (947) / +19 (910)**.
- **✅ Typed return signatures.** Return-type inference now preserves homogeneous string/bigint
  returns in both CFG and pre-CFG recovered paths, uses the fixed-point script signature map for
  calls inside return expressions, and ignores void helper calls embedded in `stack(...)` returns.
  This fixes string-result call statements that previously recompiled with `pop_int_discard`.
  **+5 (947) / +4 (910)**.
- **✅ Value-producing scriptqueue calls.** `cc_scriptqueue_add` / `if_scriptqueue_add` now recover as
  stack values consumed by the following long discard instead of emitting an immediate statement plus
  a second explicit `poplongdiscard()`. This clears the scriptqueue long-discard tail. **+11 (947) /
  +0 (910)**.
- **✅ Long branch opcodes.** Recovery now treats `long_branch_*` as binary branch conditions, and
  lowering chooses the long branch opcode when either comparison operand is `bigint`. This removes
  the `long_branch_*:operand` buckets without adding new blocked scripts. **+41 (947) / +2 (910)**.
- **✅ Stack assignment groups.** Consecutive local pops now recover as explicit `stackassign_N(...)`
  pseudo calls, so lowering preserves original push-push-pop-pop byte layout instead of expanding
  each assignment to push-pop. Multi-result property/index drains stay in their named assignment
  form. **+108 (947) / +98 (910)**.
- **✅ Custom interface/NPC arities.** Source-backed overrides now correct stack effects for
  `if_npc_setcustom*`, `cc_npc_setcustom*`, and custom body/head recol/retex commands whose
  extracted handlers only exposed component pops. **+11 (947) / +4 (910)**.
- **✅ Store lookup multi-result recovery.** `store_lookup(pos, currency)` now recovers as a
  thirteen-slot indexed result and lowers back to one opcode plus typed stores. **+2 (947) / +0
  (910)**.
- **✅ Residual opcode arities and UI ambiguity.** `UI.ListAddentry` now selects `if_list_addentry`
  when its third argument is a component constant, `field6563` is modelled as a 910 int producer, and
  camera axis commands pop their four int payloads. **+2 (947) / +3 (910)**.
- **✅ Delayed stack value preservation.** `stackpush_then(value, sideEffect())` now preserves byte
  order when the VM pushes return/discard values before later zero-stack side effects, covering the
  autosetup ultra and worldmap camera-reset cases. **+2 (947) / +1 (910)**.
- **✅ Stack-carrying control-flow and assignment preservation.** Added source-backed command
  families for file-system, wiki, minimenu, and interface-misc ops; recovered minimenu multi-result
  helpers; fixed `if_setopkey`'s four-int payload; and preserved live stack values through
  `goto`, `switch`, void `gosub`, and local assignments using `stackpush_then(..., goto(...))` and
  `stackpush_then(..., stackassign_1(...))`. Lowering now groups multi-result values across
  `stackassign_N`, so generated `pop()` placeholders in property/index drains recompile instead of
  becoming reverse blockers. **+376 (947) / +188 (910)**.
- **✅ Duplicate enum constants + targeted interface arities.** Enum exports, decompiler enum
  lookup, and reverse lowering now share duplicate-safe member names, so repeated labels such as
  `RESERVED` lower to the exact key instead of the last duplicate. Targeted interface overrides also
  restore source-backed arities for `if_set2dangle`, `if_setnpcmodel`, and
  `if_grid_setlayoutparams`, preserving payload values hidden by helper-based stack extraction.
  **+159 (947) / +106 (910)**.
- **✅ Residual command arities + branch stack preservation.** Added exact effects for
  marketing/camera/world-map/highlight/detail/resume/bounding-box residual commands; preserved live
  stack values before branch operands; recovered value-producing gosubs feeding shared return blocks
  as `stackpush_then(call, return pop())`; and kept private forward-tail branches in byte-faithful
  linear form. **+54 (947) / +24 (910)**.
- **✅ Multi-return script slots + varbit stackassign.** Script signatures now preserve inferred
  return slot counts, so calls like `time_to_string(script4705(...), ...)` count the nested helper
  as three VM int slots without stealing older string stack values. Consecutive varbit stores now
  reuse the same `stackassign_N` byte-order-preserving form as local stores. **+4 (947) / +3
  (910)**.
- **✅ Gated linear fallback + tail arities.** If high-level structured TS fails byte identity, the
  exporter now retries the order-faithful linear CFG form and accepts it only after the same
  recompile gate passes. Added exact effects for camera screen FOV, build-specific `tostring`, two
  result `*_getcharposatindex`, and interface boolean payloads such as `if_settiling` /
  `if_setlinedirection`.
- **✅ Callback/UI stack and branch-target value fidelity.** Callback watcher counts now survive
  enum-named literals; generic UI setters preserve pending VM stack values; `if_getcharindexatpos`
  uses corpus-backed value arity; and value calls crossing branch-target labels materialize as
  explicit `push(...)` in original byte order. **+5 (947) / +2 (910)** from prior baseline.
- **✅ Final tail closure.** Build-aware stack effects now cover the final command arities
  (`tostring`, `tostring_long`, and `lobby_enterlobby_social_network`), param getters discard by
  string-vs-int param type, `stackpush_then(...)` statements preserve delayed VM values, and
  leave-index array reads round-trip through an explicit
  `push_array_int_leave_index_on_stack_N(index)` pseudo call instead of guessing from
  `array[i] = array[i] + x` syntax.
- **Current full-corpus gated baseline (measured 2026-05-31):** **20577/20577 = 100.00% (947)**
  and **14313/14313 = 100.00% (910)** from `transpile-scripts --all-scripts` release runs
  (`/tmp/rs3-review-947`, `/tmp/rs3-review-910`). Both reports have
  `blocked: 0`; no `recompile_mismatch` or `reverse_unsupported` blockers remain in either measured
  corpus.

### Correction to an earlier "dead code" claim
A prior pass concluded the dominant `branch_equals:operand` residual was corpus dead code (no-op
branches + unreachable bodies in `bool_to_int`/`meslayer_mode1-4`/`script48`). **That was wrong** —
it was the symptom of the off-by-one above. Those are genuine guard clauses (`if (cond) return;
<body>`); with the corrected targets they structure correctly and recompile byte-identically. Lesson:
byte round-trip alone cannot validate control-flow interpretation — cross-check the client VM.

Maintenance levers after full closure:
- [x] **Recompile layout fidelity**: branch/switch/operand mismatch buckets are clear in both full
  corpora.
- [x] **Operand/expression order fidelity**: local/constant drift and typed discard drift are clear
  in both full corpora.
- [x] **Residual reverse blockers**: callback watcher, `pop()` stack drains, dynamic callback
  targets, raw hook descriptors, property/array multi-result forms, and generic call arity/type
  ambiguity are solved; no `reverse_unsupported` blockers remain on 910 or 947.
- [ ] **Regression harness hardening**: keep 947/910 full-corpus gates cheap to rerun and add focused
  fixtures for pseudo-stack forms (`stackpush_then`, `stackassign_N`, leave-index arrays).
- [ ] Remove the now-dead `StructuredEmitter` from `cfg.rs` (the relooper replaced it).

## P1 — Control-flow recovery (the dominant lever: ~62%+49% of corpus)
Target `cfg.rs` (build_cfg / emit_structured) + the branch/goto handling.
- [ ] **P1.1** Eliminate `residual_goto`: reconstruct structured loops (`while`/`do`/early-exit) and
  nested `if/else` from the branch graph so no goto remains. Biggest single win (62%).
- [ ] **P1.2** Eliminate `commented_branch`: fold the branches currently emitted as comments into
  real structured conditions (49%). Likely the same CFG work as P1.1.
- [ ] **P1.3** Add structured-recovery regression tests over a representative script set; assert the
  editable % rises and these blockers fall toward 0.

## P2 — Expression recovery (`reverse_unsupported` + expression-order mismatch tail)
Target `expr_recovery.rs` + `ts_lower.rs`.
- [x] **P2.1** Fold `residual_pop`: leftover stack pops not absorbed into expressions/statements →
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
