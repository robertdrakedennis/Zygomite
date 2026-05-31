//! Region-based control-flow structurer (decompile direction).
//!
//! Reconstructs structured TypeScript (`if`/`else`, `while`, `switch`) from the
//! basic-block CFG using dominator / immediate-post-dominator analysis plus
//! natural-loop detection — replacing the previous ad-hoc recursive descent
//! that fell back to `goto`/commented branches for everything beyond the
//! simplest diamond and back-edge.
//!
//! Output contract (must match `ts_lower`):
//! - `If { cond, then, else }`  → branch/branch-not + labels.
//! - `While { body }`           → an *infinite* loop (`continue:` body `branch
//!   continue; break:`); the body must `Break` to exit and never re-emits the
//!   back-edge.
//! - `Switch { expr, cases, default_body }` → switch table; no-match falls
//!   through to `default_body` when the bytecode has a real fallthrough block.
//! - `Break` / `Continue`       → unlabeled, so only the *innermost* loop's exit
//!   / header can be expressed; anything else falls back to `Goto`.
//!
//! Correctness is gated downstream: a script is only `editable_structured` if
//! its structured form recompiles byte-identically, so any shape this can't
//! structure faithfully simply falls back to `Goto` (and stays non-editable)
//! rather than producing a miscompiling "editable" script.

use super::cfg::{Block, assignment_target_from_recovered};
use super::expr_recovery::RecoveredStmt;
use super::structured::{StructuredStmt, SwitchCaseStmt, stmts_terminate};
use std::collections::BTreeSet;

/// Guards against pathological/irreducible graphs blowing the stack.
const MAX_DEPTH: usize = 400;

pub fn structure(blocks: &[Block]) -> Vec<StructuredStmt> {
    structure_with_report(blocks).statements
}

pub struct StructureResult {
    pub statements: Vec<StructuredStmt>,
    pub fallback_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StructureOptions {
    pub fold_branch_trampolines: bool,
}

impl StructureOptions {
    pub(crate) const AGGRESSIVE: Self = Self {
        fold_branch_trampolines: true,
    };
    pub(crate) const CONSERVATIVE: Self = Self {
        fold_branch_trampolines: false,
    };
}

pub fn structure_with_report(blocks: &[Block]) -> StructureResult {
    structure_with_options(blocks, StructureOptions::AGGRESSIVE)
}

#[expect(
    clippy::similar_names,
    reason = "succ/pred are the conventional names for the two CFG adjacency lists"
)]
pub(crate) fn structure_with_options(
    blocks: &[Block],
    options: StructureOptions,
) -> StructureResult {
    if blocks.is_empty() {
        return StructureResult {
            statements: Vec::new(),
            fallback_reason: None,
        };
    }
    let n = blocks.len();
    let succ: Vec<Vec<usize>> = blocks
        .iter()
        .map(|b| b.successors.iter().copied().filter(|&s| s < n).collect())
        .collect();
    let pred: Vec<Vec<usize>> = blocks
        .iter()
        .map(|b| b.predecessors.iter().copied().filter(|&p| p < n).collect())
        .collect();

    let idom = dominators(n, 0, &pred, &succ);
    let ipdom = post_dominators(n, &succ);
    let loops = LoopInfo::detect(&succ, &pred, &idom);

    if let Some(reason) = conservative_fallback_reason(blocks) {
        return StructureResult {
            statements: structure_linear(blocks),
            fallback_reason: Some(reason.to_string()),
        };
    }

    let statements = emit_structured_pass(blocks, ipdom, loops, &BTreeSet::new(), options);
    StructureResult {
        statements: statements.statements,
        fallback_reason: None,
    }
}

struct StructuredPass {
    statements: Vec<StructuredStmt>,
}

fn emit_structured_pass(
    blocks: &[Block],
    ipdom: Vec<Option<usize>>,
    loops: LoopInfo,
    label_targets: &BTreeSet<usize>,
    options: StructureOptions,
) -> StructuredPass {
    let n = blocks.len();
    let mut s = Structurer {
        blocks,
        ipdom,
        loops,
        emitted: vec![false; n],
        label_targets,
        goto_targets: BTreeSet::new(),
        options,
    };
    let mut out = s.emit_region(0, None, &[], 0);

    // Emit any blocks the reachable region never visited, in original order.
    // The RT7 compiler appends an unreachable default-return epilogue
    // (`push <default>; return`) after a script's real return; dropping it makes
    // the recompile shorter than the original (the length:structured_shorter /
    // branch:operand mismatch family). Re-emitting these unreachable tails keeps
    // the structured form byte-faithful. They are genuinely dead (an editable
    // script has no residual goto reaching them), so appending them as trailing
    // statements changes no behaviour.
    for b in 0..n {
        if !s.emitted[b] {
            out.extend(s.emit_region(b, None, &[], 0));
        }
    }

    if label_targets.is_empty() && contains_goto(&out) && !s.goto_targets.is_empty() {
        let goto_targets = s.goto_targets.clone();
        return emit_structured_pass(blocks, s.ipdom, s.loops, &goto_targets, s.options);
    }

    StructuredPass { statements: out }
}

fn conservative_fallback_reason(blocks: &[Block]) -> Option<&'static str> {
    if has_stack_goto(blocks) {
        Some("stack_goto")
    } else {
        None
    }
}

fn contains_goto(stmts: &[StructuredStmt]) -> bool {
    stmts.iter().any(|s| match s {
        StructuredStmt::Goto { .. } | StructuredStmt::StackGoto { .. } => true,
        StructuredStmt::While { body } => contains_goto(body),
        StructuredStmt::If {
            then_body,
            else_body,
            ..
        } => contains_goto(then_body) || else_body.as_deref().is_some_and(contains_goto),
        StructuredStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            cases.iter().any(|c| contains_goto(&c.body))
                || default_body.as_deref().is_some_and(contains_goto)
        }
        _ => false,
    })
}

fn has_stack_goto(blocks: &[Block]) -> bool {
    blocks.iter().any(|block| {
        block
            .statements
            .iter()
            .any(|stmt| matches!(stmt, RecoveredStmt::GotoStack { .. }))
    })
}

/// Linear control-flow emission: transcribe the CFG block-by-block in original
/// order, labelling jump targets and emitting each terminator as a `goto` /
/// `if (cond) goto` / `switch`/`return`. Order-faithful, so it recompiles
/// byte-identically; used only as the fallback for control flow that cannot be
/// reduced to nested `if`/`while`/`switch`.
pub(crate) fn structure_linear(blocks: &[Block]) -> Vec<StructuredStmt> {
    let n = blocks.len();
    let mut needs_label = vec![false; n];
    let mut mark = |instr: usize| {
        if let Some(bi) = block_at_instr(blocks, instr) {
            needs_label[bi] = true;
        }
    };
    for b in blocks {
        for stmt in &b.statements {
            match stmt {
                RecoveredStmt::Goto(t)
                | RecoveredStmt::GotoStack { target: t, .. }
                | RecoveredStmt::Branch { target: t, .. }
                | RecoveredStmt::BranchBinary { target: t, .. } => mark(*t),
                RecoveredStmt::Switch { cases, .. } => {
                    for (_, t) in cases {
                        mark(*t);
                    }
                }
                _ => {}
            }
        }
    }

    let label_start =
        |instr: usize| block_at_instr(blocks, instr).map_or(instr, |bi| blocks[bi].start);
    let mut out = Vec::new();
    for (bi, b) in blocks.iter().enumerate() {
        if needs_label[bi] {
            out.push(StructuredStmt::Label { target: b.start });
        }
        for stmt in &b.statements {
            match stmt {
                RecoveredStmt::Expression(expr) => {
                    out.push(StructuredStmt::Expr { expr: expr.clone() });
                }
                RecoveredStmt::Assignment { target, value, .. } => {
                    out.push(StructuredStmt::Assignment {
                        target: assignment_target_from_recovered(target),
                        value: value.clone(),
                    });
                }
                RecoveredStmt::Comment(text) => out.push(StructuredStmt::Comment(text.clone())),
                RecoveredStmt::Return(value) => {
                    out.push(StructuredStmt::Return {
                        value: value.clone(),
                    });
                }
                RecoveredStmt::Goto(target) => out.push(StructuredStmt::Goto {
                    target: label_start(*target),
                }),
                RecoveredStmt::GotoStack { target, values } => {
                    out.push(StructuredStmt::StackGoto {
                        target: label_start(*target),
                        values: values.clone(),
                    });
                }
                RecoveredStmt::Branch { target, .. }
                | RecoveredStmt::BranchBinary { target, .. } => {
                    if let Some(condition) = super::cfg::branch_condition_expr(stmt) {
                        out.push(StructuredStmt::If {
                            condition,
                            then_body: vec![StructuredStmt::Goto {
                                target: label_start(*target),
                            }],
                            else_body: None,
                        });
                    } else {
                        out.push(StructuredStmt::Goto {
                            target: label_start(*target),
                        });
                    }
                }
                RecoveredStmt::Switch {
                    discriminant,
                    cases,
                } => {
                    let cases = cases
                        .iter()
                        .map(|(value, target)| SwitchCaseStmt {
                            value: *value,
                            body: vec![StructuredStmt::Goto {
                                target: label_start(*target),
                            }],
                            fallthrough: false,
                            break_after: true,
                        })
                        .collect();
                    out.push(StructuredStmt::Switch {
                        expr: discriminant.clone(),
                        cases,
                        default_body: None,
                    });
                }
            }
        }
    }
    out
}

// ── Dominator analysis (Cooper-Harvey-Kennedy) ────────────────────────────

/// Reverse postorder of a graph rooted at `entry` over the given successor
/// lists. Unreachable nodes are omitted.
fn reverse_postorder(n: usize, entry: usize, succ: &[Vec<usize>]) -> Vec<usize> {
    let mut visited = vec![false; n];
    let mut post = Vec::with_capacity(n);
    let mut stack: Vec<(usize, usize)> = vec![(entry, 0)];
    visited[entry] = true;
    while let Some(&(node, ci)) = stack.last() {
        if ci < succ[node].len() {
            stack.last_mut().expect("non-empty").1 += 1;
            let next = succ[node][ci];
            if !visited[next] {
                visited[next] = true;
                stack.push((next, 0));
            }
        } else {
            post.push(node);
            stack.pop();
        }
    }
    post.reverse();
    post
}

/// Immediate dominators over `succ` rooted at `entry`. `idom[entry] == entry`;
/// unreachable nodes stay `None`.
fn dominators(
    n: usize,
    entry: usize,
    pred: &[Vec<usize>],
    succ: &[Vec<usize>],
) -> Vec<Option<usize>> {
    let rpo = reverse_postorder(n, entry, succ);
    let mut rpo_num = vec![usize::MAX; n];
    for (i, &b) in rpo.iter().enumerate() {
        rpo_num[b] = i;
    }
    let mut idom = vec![None; n];
    idom[entry] = Some(entry);

    let intersect = |mut a: usize, mut b: usize, idom: &[Option<usize>]| -> usize {
        while a != b {
            while rpo_num[a] > rpo_num[b] {
                a = idom[a].expect("processed node has idom");
            }
            while rpo_num[b] > rpo_num[a] {
                b = idom[b].expect("processed node has idom");
            }
        }
        a
    };

    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo {
            if b == entry {
                continue;
            }
            let mut new_idom: Option<usize> = None;
            for &p in &pred[b] {
                if idom[p].is_some() {
                    new_idom = Some(match new_idom {
                        None => p,
                        Some(cur) => intersect(p, cur, &idom),
                    });
                }
            }
            if new_idom.is_some() && idom[b] != new_idom {
                idom[b] = new_idom;
                changed = true;
            }
        }
    }
    idom
}

/// Immediate post-dominators: dominators on the reverse graph rooted at a
/// virtual exit (node index `n`) that every exit block (no successors) points
/// to. `ipdom[b] == Some(n)` means "no in-function join — paths reach the
/// function end", which callers treat as `None`.
fn post_dominators(n: usize, succ: &[Vec<usize>]) -> Vec<Option<usize>> {
    let exit = n;
    let size = n + 1;
    // Reverse graph: edge a->b becomes b->a; every block with no successors
    // gets an edge from the virtual exit.
    let mut rsucc: Vec<Vec<usize>> = vec![Vec::new(); size];
    let mut rpred: Vec<Vec<usize>> = vec![Vec::new(); size];
    for b in 0..n {
        for &s in &succ[b] {
            rsucc[s].push(b);
            rpred[b].push(s);
        }
        if succ[b].is_empty() {
            rsucc[exit].push(b);
            rpred[b].push(exit);
        }
    }
    let idom = dominators(size, exit, &rpred, &rsucc);
    // Drop the virtual-exit slot; keep ipdom per real block.
    idom[..n].to_vec()
}

fn dominates(idom: &[Option<usize>], a: usize, b: usize) -> bool {
    let mut x = b;
    loop {
        if x == a {
            return true;
        }
        match idom[x] {
            Some(d) if d != x => x = d,
            _ => return false,
        }
    }
}

// ── Loop detection ────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum LoopExit {
    /// No edge leaves the loop (it returns out, or is infinite).
    None,
    /// Exactly one block outside the loop is targeted.
    Single(usize),
    /// Multiple distinct exit targets — not expressible with unlabeled `break`.
    Multi,
}

#[derive(Clone)]
struct LoopInfo {
    headers: Vec<bool>,
    exit: std::collections::HashMap<usize, LoopExit>,
}

impl LoopInfo {
    fn detect(succ: &[Vec<usize>], pred: &[Vec<usize>], idom: &[Option<usize>]) -> Self {
        let n = succ.len();
        let mut headers = vec![false; n];
        let mut body: std::collections::HashMap<usize, std::collections::HashSet<usize>> =
            std::collections::HashMap::new();
        for (u, succs) in succ.iter().enumerate() {
            for &h in succs {
                // Back edge: u -> h where h dominates u.
                if dominates(idom, h, u) {
                    headers[h] = true;
                    let nat = natural_loop_body(pred, h, u);
                    body.entry(h).or_default().extend(nat);
                }
            }
        }
        let mut exit = std::collections::HashMap::new();
        for (&h, set) in &body {
            let mut exits = std::collections::HashSet::new();
            for &b in set {
                for &s in &succ[b] {
                    // A terminal successor (no successors of its own — a `return`
                    // / function-end block) reached from inside the loop is an
                    // inline return, not a loop exit needing an (inexpressible)
                    // labelled break. Excluding it lets search loops like
                    // `while(true){ if(done) return; ...; if(found) return; i++ }`
                    // — whose only out-edges are returns — structure as `while`
                    // with inline returns instead of falling back to goto.
                    if !set.contains(&s) && !succ[s].is_empty() {
                        exits.insert(s);
                    }
                }
            }
            let classified = match exits.len() {
                0 => LoopExit::None,
                1 => LoopExit::Single(exits.into_iter().next().expect("len 1")),
                _ => LoopExit::Multi,
            };
            exit.insert(h, classified);
        }
        Self { headers, exit }
    }
}

fn natural_loop_body(
    pred: &[Vec<usize>],
    header: usize,
    back_source: usize,
) -> std::collections::HashSet<usize> {
    let mut set = std::collections::HashSet::new();
    set.insert(header);
    set.insert(back_source);
    let mut stack = vec![back_source];
    while let Some(b) = stack.pop() {
        if b == header {
            continue;
        }
        for &p in &pred[b] {
            if set.insert(p) {
                stack.push(p);
            }
        }
    }
    set
}

// ── Region emitter ──────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct LoopCtx {
    header: usize,
    exit: Option<usize>,
}

struct Structurer<'a> {
    blocks: &'a [Block],
    ipdom: Vec<Option<usize>>,
    loops: LoopInfo,
    emitted: Vec<bool>,
    label_targets: &'a BTreeSet<usize>,
    goto_targets: BTreeSet<usize>,
    options: StructureOptions,
}

impl Structurer<'_> {
    fn ipdom_of(&self, b: usize) -> Option<usize> {
        match self.ipdom[b] {
            Some(j) if j < self.blocks.len() => Some(j),
            _ => None,
        }
    }

    /// If `target` is the innermost loop's header (back edge) or single exit,
    /// produce the corresponding `Continue`/`Break`. Unlabeled, so only the
    /// innermost loop is expressible.
    fn loop_edge(&self, target: usize, loops: &[LoopCtx]) -> Option<StructuredStmt> {
        let inner = loops.last()?;
        if target == inner.header {
            Some(StructuredStmt::Continue)
        } else if Some(target) == inner.exit {
            Some(StructuredStmt::Break)
        } else {
            None
        }
    }

    fn goto(&mut self, target: usize) -> StructuredStmt {
        let label = self.blocks.get(target).map_or(target, |b| b.start);
        self.goto_targets.insert(label);
        StructuredStmt::Goto { target: label }
    }

    fn goto_instr(&mut self, target: usize) -> StructuredStmt {
        let label = block_at_instr(self.blocks, target).map_or(target, |b| self.blocks[b].start);
        self.goto_targets.insert(label);
        StructuredStmt::Goto { target: label }
    }

    fn stack_goto_instr(
        &mut self,
        target: usize,
        values: Vec<super::ast::Expression>,
    ) -> StructuredStmt {
        let label = block_at_instr(self.blocks, target).map_or(target, |b| self.blocks[b].start);
        self.goto_targets.insert(label);
        StructuredStmt::StackGoto {
            target: label,
            values,
        }
    }

    /// Structure a linear region beginning at `entry`, stopping at `stop`
    /// (boundary, emitted by the caller) or any already-emitted block.
    fn emit_region(
        &mut self,
        entry: usize,
        stop: Option<usize>,
        loops: &[LoopCtx],
        depth: usize,
    ) -> Vec<StructuredStmt> {
        let mut out = Vec::new();
        let mut cur = entry;
        if depth > MAX_DEPTH {
            out.push(self.goto(cur));
            return out;
        }
        loop {
            if Some(cur) == stop || cur >= self.blocks.len() {
                break;
            }
            if let Some(stmt) = self.loop_edge(cur, loops) {
                out.push(stmt);
                break;
            }
            if self.emitted[cur] {
                out.push(self.goto(cur));
                break;
            }
            // A loop header reached from outside its own context: structure it.
            let in_own_loop = loops.last().is_some_and(|l| l.header == cur);
            if self.loops.headers[cur] && !in_own_loop {
                if let Some((stmt, next)) = self.emit_loop(cur, loops, depth) {
                    out.push(stmt);
                    match next {
                        Some(n) => {
                            cur = n;
                            continue;
                        }
                        None => break,
                    }
                }
                out.push(self.goto(cur));
                break;
            }

            self.emitted[cur] = true;
            if self.label_targets.contains(&self.blocks[cur].start) {
                out.push(StructuredStmt::Label {
                    target: self.blocks[cur].start,
                });
            }
            for stmt in &self.blocks[cur].statements {
                if let Some(s) = convert_stmt(stmt) {
                    out.push(s);
                }
            }
            if self.blocks[cur].predecessors.is_empty()
                && let [RecoveredStmt::Goto(target)] = self.blocks[cur].statements.as_slice()
            {
                out.push(self.goto_instr(*target));
                break;
            }
            if self.blocks[cur].predecessors.is_empty()
                && let [RecoveredStmt::GotoStack { target, values }] =
                    self.blocks[cur].statements.as_slice()
            {
                out.push(self.stack_goto_instr(*target, values.clone()));
                break;
            }
            let (mut term, next) = self.emit_terminator(cur, stop, loops, depth);
            out.append(&mut term);
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        out
    }

    /// Emit a conditional/switch/return/jump terminator. Returns the structured
    /// statements plus the block to continue linear emission at (`None` = the
    /// region ends here).
    fn emit_terminator(
        &mut self,
        cur: usize,
        stop: Option<usize>,
        loops: &[LoopCtx],
        depth: usize,
    ) -> (Vec<StructuredStmt>, Option<usize>) {
        let block = &self.blocks[cur];
        if let Some(value) = block.statements.iter().find_map(|s| match s {
            RecoveredStmt::Return(v) => Some(v.clone()),
            _ => None,
        }) {
            return (vec![StructuredStmt::Return { value }], None);
        }

        if let Some((target, values)) = block.statements.iter().find_map(|s| match s {
            RecoveredStmt::GotoStack { target, values } => Some((*target, values.clone())),
            _ => None,
        }) {
            return (vec![self.stack_goto_instr(target, values)], None);
        }

        if let Some((discriminant, cases)) = switch_of(block) {
            return self.emit_switch(cur, &discriminant, &cases, loops, depth);
        }

        if block.is_conditional_branch
            && block.successors.len() >= 2
            && let Some(condition) = block.branch_condition.clone()
        {
            let true_target = block.successors[0];
            let raw_false_target = block.successors[1];
            let false_target = if self.options.fold_branch_trampolines {
                self.branch_trampoline_target(raw_false_target)
                    .unwrap_or(raw_false_target)
            } else {
                raw_false_target
            };
            if self.is_shared_forward_continuation(cur, true_target, false_target, loops) {
                if self.options.fold_branch_trampolines && raw_false_target != false_target {
                    self.emitted[raw_false_target] = true;
                }
                let then_body = self.emit_arm(true_target, Some(false_target), loops, depth);
                let stmt = StructuredStmt::If {
                    condition,
                    then_body,
                    else_body: None,
                };
                return (vec![stmt], Some(false_target));
            }
            let join = stop
                .filter(|stop| block.successors.contains(stop))
                .or_else(|| self.ipdom_of(cur));
            let then_body = self.emit_arm(true_target, join, loops, depth);
            let else_body = self.emit_arm(false_target, join, loops, depth);
            // An empty else arm must be `None`, not `Some(vec![])`: a simple
            // `if (cond) { then }` in the original has no else branch, and
            // `Some(empty)` makes lower_if emit a spurious `branch end` + else
            // label, breaking byte-identity.
            let stmt = StructuredStmt::If {
                condition,
                then_body,
                else_body: if else_body.is_empty() {
                    None
                } else {
                    Some(else_body)
                },
            };
            return (vec![stmt], join);
        }

        // Unconditional / fall-through: hand the single successor back to the
        // linear walk (whose guards handle loop edges, boundaries, cross edges).
        (Vec::new(), block.successors.first().copied())
    }

    fn is_shared_forward_continuation(
        &self,
        cur: usize,
        true_target: usize,
        false_target: usize,
        loops: &[LoopCtx],
    ) -> bool {
        let Some(false_block) = self.blocks.get(false_target) else {
            return false;
        };
        let Some(true_block) = self.blocks.get(true_target) else {
            return false;
        };
        false_target > cur
            && true_block.start < false_block.start
            && !self.emitted[false_target]
            && self.loop_edge(false_target, loops).is_none()
    }

    fn branch_trampoline_target(&self, block_index: usize) -> Option<usize> {
        let [RecoveredStmt::Goto(target)] = self.blocks.get(block_index)?.statements.as_slice()
        else {
            return None;
        };
        block_at_instr(self.blocks, *target)
    }

    /// Structure one arm of an `if`, bounded by the join (immediate
    /// post-dominator). A loop-edge target becomes `break`/`continue`; the join
    /// itself is an empty arm.
    fn emit_arm(
        &mut self,
        target: usize,
        join: Option<usize>,
        loops: &[LoopCtx],
        depth: usize,
    ) -> Vec<StructuredStmt> {
        if let Some(stmt) = self.loop_edge(target, loops) {
            return vec![stmt];
        }
        if Some(target) == join {
            return Vec::new();
        }
        if target >= self.blocks.len() || self.emitted[target] {
            return vec![self.goto(target)];
        }
        self.emit_region(target, join, loops, depth + 1)
    }

    fn emit_loop(
        &mut self,
        header: usize,
        loops: &[LoopCtx],
        depth: usize,
    ) -> Option<(StructuredStmt, Option<usize>)> {
        let exit = match self.loops.exit.get(&header) {
            Some(LoopExit::Single(e)) => Some(*e),
            Some(LoopExit::None) => None,
            // Multi-exit loops need labeled break — not expressible.
            Some(LoopExit::Multi) | None => return None,
        };
        let mut inner = loops.to_vec();
        inner.push(LoopCtx { header, exit });

        self.emitted[header] = true;
        let mut body = Vec::new();
        for stmt in &self.blocks[header].statements {
            if let Some(s) = convert_stmt(stmt) {
                body.push(s);
            }
        }
        let (mut term, next) = self.emit_terminator(header, None, &inner, depth);
        body.append(&mut term);
        if let Some(n) = next
            && Some(n) != exit
        {
            body.extend(self.emit_region(n, None, &inner, depth + 1));
        }
        Some((StructuredStmt::While { body }, exit))
    }

    fn emit_switch(
        &mut self,
        cur: usize,
        discriminant: &super::ast::Expression,
        cases: &[(i32, usize)],
        loops: &[LoopCtx],
        depth: usize,
    ) -> (Vec<StructuredStmt>, Option<usize>) {
        let join = self.ipdom_of(cur);
        let default_body = block_at_instr(self.blocks, self.blocks[cur].end)
            .filter(|&entry| Some(entry) != join)
            .filter(|&entry| {
                !cases.iter().any(|(_, target_instr)| {
                    block_at_instr(self.blocks, *target_instr) == Some(entry)
                })
            })
            .map(|entry| self.emit_arm(entry, join, loops, depth));
        let mut out_cases = Vec::with_capacity(cases.len());
        let entries = cases
            .iter()
            .map(|&(value, target_instr)| {
                block_at_instr(self.blocks, target_instr).map(|entry| (value, entry))
            })
            .collect::<Option<Vec<_>>>();
        let Some(entries) = entries else {
            // Can't map a case target -> leave it unstructured.
            return (vec![self.goto(cur)], None);
        };
        for (index, &(value, entry)) in entries.iter().enumerate() {
            if entries
                .get(index + 1)
                .is_some_and(|&(_, next_entry)| next_entry == entry)
            {
                out_cases.push(SwitchCaseStmt {
                    value,
                    body: Vec::new(),
                    fallthrough: true,
                    break_after: false,
                });
                continue;
            }
            let body = self.emit_arm(entry, join, loops, depth);
            let has_following_case_or_default = index + 1 < entries.len() || default_body.is_some();
            let break_after = (!stmts_terminate(&body) && has_following_case_or_default)
                || self.dead_break_after_case_body(entry, join);
            out_cases.push(SwitchCaseStmt {
                value,
                body,
                fallthrough: false,
                break_after,
            });
        }
        let stmt = StructuredStmt::Switch {
            expr: discriminant.clone(),
            cases: out_cases,
            default_body,
        };
        (vec![stmt], join)
    }

    fn dead_break_after_case_body(&self, entry: usize, join: Option<usize>) -> bool {
        let mut cur = entry;
        let mut seen = BTreeSet::new();
        while seen.insert(cur) {
            let Some(block) = self.blocks.get(cur) else {
                return false;
            };
            let mut saw_return = false;
            for stmt in &block.statements {
                if saw_return && let RecoveredStmt::Goto(target) = stmt {
                    return self.dead_break_target_matches(*target, join);
                }
                if matches!(stmt, RecoveredStmt::Return(_)) {
                    saw_return = true;
                }
            }
            if block
                .statements
                .iter()
                .any(|stmt| matches!(stmt, RecoveredStmt::Return(_)))
            {
                if let [target] = block.successors.as_slice() {
                    return join.is_none_or(|join| *target == join);
                }
                let Some(next_block) = block_at_instr(self.blocks, block.end) else {
                    return false;
                };
                let [RecoveredStmt::Goto(target)] = self.blocks[next_block].statements.as_slice()
                else {
                    return false;
                };
                return self.dead_break_target_matches(*target, join);
            }
            let [next] = block.successors.as_slice() else {
                return false;
            };
            cur = *next;
        }
        false
    }

    fn dead_break_target_matches(&self, target: usize, join: Option<usize>) -> bool {
        join.is_none_or(|join| block_at_instr(self.blocks, target) == Some(join))
    }
}

/// Convert a non-control statement; returns `None` for control flow the
/// structurer handles itself (`Branch`/`BranchBinary`/`Switch`/`Goto`/`Return`).
fn convert_stmt(stmt: &RecoveredStmt) -> Option<StructuredStmt> {
    match stmt {
        RecoveredStmt::Expression(expr) => Some(StructuredStmt::Expr { expr: expr.clone() }),
        RecoveredStmt::Assignment { target, value, .. } => Some(StructuredStmt::Assignment {
            target: assignment_target_from_recovered(target),
            value: value.clone(),
        }),
        RecoveredStmt::Comment(text) => Some(StructuredStmt::Comment(text.clone())),
        RecoveredStmt::Branch { .. }
        | RecoveredStmt::BranchBinary { .. }
        | RecoveredStmt::Switch { .. }
        | RecoveredStmt::Goto(_)
        | RecoveredStmt::GotoStack { .. }
        | RecoveredStmt::Return(_) => None,
    }
}

fn switch_of(block: &Block) -> Option<(super::ast::Expression, Vec<(i32, usize)>)> {
    block.statements.iter().find_map(|s| match s {
        RecoveredStmt::Switch {
            discriminant,
            cases,
        } => Some((discriminant.clone(), cases.clone())),
        _ => None,
    })
}

fn block_at_instr(blocks: &[Block], instr: usize) -> Option<usize> {
    blocks
        .iter()
        .position(|b| instr >= b.start && instr < b.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transpile::ast::{BooleanLiteral, Expression};

    fn bool_expr(value: bool) -> Expression {
        Expression::BooleanLiteral(BooleanLiteral { value })
    }

    fn block(
        index: usize,
        start: usize,
        end: usize,
        statements: Vec<RecoveredStmt>,
        successors: Vec<usize>,
        predecessors: Vec<usize>,
    ) -> Block {
        Block {
            index,
            start,
            end,
            statements,
            successors,
            predecessors,
            is_loop_header: false,
            loop_target: None,
            is_conditional_branch: false,
            branch_condition: None,
        }
    }

    #[test]
    fn residual_goto_inserts_target_label_without_conservative_fallback() {
        let blocks = vec![
            block(0, 0, 1, vec![RecoveredStmt::Goto(4)], vec![2], vec![]),
            block(1, 2, 3, vec![RecoveredStmt::Return(None)], vec![], vec![]),
            block(
                2,
                4,
                5,
                vec![
                    RecoveredStmt::Expression(bool_expr(true)),
                    RecoveredStmt::Return(None),
                ],
                vec![],
                vec![0, 1],
            ),
        ];

        let structured = structure_with_report(&blocks);

        assert_eq!(None, structured.fallback_reason);
        assert!(
            structured
                .statements
                .iter()
                .any(|stmt| matches!(stmt, StructuredStmt::Goto { target } if *target == 4))
        );
        assert!(
            structured
                .statements
                .iter()
                .any(|stmt| matches!(stmt, StructuredStmt::Label { target } if *target == 4))
        );
    }

    #[test]
    fn shared_forward_continuation_structures_as_guard_clause() {
        let mut blocks = vec![
            block(
                0,
                0,
                4,
                vec![RecoveredStmt::Branch {
                    condition: bool_expr(true),
                    target: 5,
                    negated: false,
                }],
                vec![2, 1],
                vec![],
            ),
            block(1, 4, 5, vec![RecoveredStmt::Goto(11)], vec![5], vec![0]),
            block(
                2,
                5,
                8,
                vec![RecoveredStmt::Branch {
                    condition: bool_expr(false),
                    target: 9,
                    negated: false,
                }],
                vec![4, 3],
                vec![0],
            ),
            block(3, 8, 9, vec![RecoveredStmt::Goto(11)], vec![5], vec![2]),
            block(4, 9, 11, vec![RecoveredStmt::Return(None)], vec![], vec![2]),
            block(
                5,
                11,
                15,
                vec![RecoveredStmt::Return(None)],
                vec![],
                vec![1, 3],
            ),
        ];
        blocks[0].is_conditional_branch = true;
        blocks[0].branch_condition = Some(bool_expr(true));
        blocks[2].is_conditional_branch = true;
        blocks[2].branch_condition = Some(bool_expr(false));

        let structured = structure_with_report(&blocks);

        let [
            StructuredStmt::If {
                then_body,
                else_body: None,
                ..
            },
            StructuredStmt::Return { .. },
        ] = structured.statements.as_slice()
        else {
            panic!(
                "expected top-level guard clause: {:#?}",
                structured.statements
            );
        };
        assert!(
            matches!(
                then_body.as_slice(),
                [StructuredStmt::If {
                    else_body: None,
                    ..
                }]
            ),
            "expected nested guard clause: {then_body:#?}"
        );
        assert!(!contains_goto(&structured.statements));
        assert!(
            !structured
                .statements
                .iter()
                .any(|stmt| matches!(stmt, StructuredStmt::Label { .. })),
            "guard clause should not need labels: {:#?}",
            structured.statements
        );
    }

    #[test]
    fn conservative_shared_forward_continuation_keeps_trampoline_goto() {
        let mut blocks = vec![
            block(
                0,
                0,
                4,
                vec![RecoveredStmt::Branch {
                    condition: bool_expr(true),
                    target: 5,
                    negated: false,
                }],
                vec![2, 1],
                vec![],
            ),
            block(1, 4, 5, vec![RecoveredStmt::Goto(11)], vec![5], vec![0]),
            block(
                2,
                5,
                8,
                vec![RecoveredStmt::Branch {
                    condition: bool_expr(false),
                    target: 9,
                    negated: false,
                }],
                vec![4, 3],
                vec![0],
            ),
            block(3, 8, 9, vec![RecoveredStmt::Goto(11)], vec![5], vec![2]),
            block(4, 9, 11, vec![RecoveredStmt::Return(None)], vec![], vec![2]),
            block(
                5,
                11,
                15,
                vec![RecoveredStmt::Return(None)],
                vec![],
                vec![1, 3],
            ),
        ];
        blocks[0].is_conditional_branch = true;
        blocks[0].branch_condition = Some(bool_expr(true));
        blocks[2].is_conditional_branch = true;
        blocks[2].branch_condition = Some(bool_expr(false));

        let structured = structure_with_options(&blocks, StructureOptions::CONSERVATIVE);

        assert_eq!(None, structured.fallback_reason);
        assert!(contains_goto(&structured.statements));
    }
}
