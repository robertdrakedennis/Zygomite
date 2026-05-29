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
//! - `Switch { expr, cases }`   → switch table; no-match falls through to the
//!   join, so a reconstructed switch must have no default body.
//! - `Break` / `Continue`       → unlabeled, so only the *innermost* loop's exit
//!   / header can be expressed; anything else falls back to `Goto`.
//!
//! Correctness is gated downstream: a script is only `editable_structured` if
//! its structured form recompiles byte-identically, so any shape this can't
//! structure faithfully simply falls back to `Goto` (and stays non-editable)
//! rather than producing a miscompiling "editable" script.

use super::cfg::{Block, assignment_target_from_recovered};
use super::expr_recovery::RecoveredStmt;
use super::structured::{StructuredStmt, SwitchCaseStmt};

/// Guards against pathological/irreducible graphs blowing the stack.
const MAX_DEPTH: usize = 400;

#[expect(
    clippy::similar_names,
    reason = "succ/pred are the conventional names for the two CFG adjacency lists"
)]
pub fn structure(blocks: &[Block]) -> Vec<StructuredStmt> {
    if blocks.is_empty() {
        return Vec::new();
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

    let mut s = Structurer {
        blocks,
        ipdom,
        loops,
        emitted: vec![false; n],
    };
    s.emit_region(0, None, &[], 0)
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
                    if !set.contains(&s) {
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

    fn goto(&self, target: usize) -> StructuredStmt {
        let label = self.blocks.get(target).map_or(target, |b| b.start);
        StructuredStmt::Goto { target: label }
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
            for stmt in &self.blocks[cur].statements {
                if let Some(s) = convert_stmt(stmt) {
                    out.push(s);
                }
            }
            let (mut term, next) = self.emit_terminator(cur, loops, depth);
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

        if let Some((discriminant, cases)) = switch_of(block) {
            return self.emit_switch(cur, &discriminant, &cases, loops, depth);
        }

        if block.is_conditional_branch
            && block.successors.len() >= 2
            && let Some(condition) = block.branch_condition.clone()
        {
            let true_target = block.successors[0];
            let false_target = block.successors[1];
            let join = self.ipdom_of(cur);
            let then_body = self.emit_arm(true_target, join, loops, depth);
            let else_body = self.emit_arm(false_target, join, loops, depth);
            let stmt = StructuredStmt::If {
                condition,
                then_body,
                else_body: Some(else_body),
            };
            return (vec![stmt], join);
        }

        // Unconditional / fall-through: hand the single successor back to the
        // linear walk (whose guards handle loop edges, boundaries, cross edges).
        (Vec::new(), block.successors.first().copied())
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
        let (mut term, next) = self.emit_terminator(header, &inner, depth);
        body.append(&mut term);
        if let Some(n) = next {
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
        let mut out_cases = Vec::with_capacity(cases.len());
        for &(value, target_instr) in cases {
            let Some(entry) = block_at_instr(self.blocks, target_instr) else {
                // Can't map the case target → leave it unstructured.
                return (vec![self.goto(cur)], None);
            };
            let body = self.emit_arm(entry, join, loops, depth);
            out_cases.push(SwitchCaseStmt { value, body });
        }
        let stmt = StructuredStmt::Switch {
            expr: discriminant.clone(),
            cases: out_cases,
        };
        (vec![stmt], join)
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
