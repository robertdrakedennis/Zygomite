//! Branch/switch target renumbering for length-changing IR passes.
//!
//! Branch operands ([`Operand::Jump`]) and switch-case targets are *absolute*
//! instruction indices (the form the reversible asm encodes). Any pass that
//! inserts or removes instructions must remap every target through an old→new
//! index map, or branches land on the wrong instruction. This is the typed
//! equivalent of `build-ritual-scripts.py::_renumber` + the `old_to_new`
//! bookkeeping in `apply_common_rewrites`.
//!
//! Usage: drive a [`Rebuilder`] over the old instruction stream. Call
//! [`Rebuilder::begin`] at the start of each *old* instruction (recording that
//! its old index maps to the current output position), then [`Rebuilder::push`]
//! the rewritten instruction(s). [`Rebuilder::finish`] appends the one-past-end
//! sentinel and remaps every target.

use crate::port::ir::cs2::{Insn, Operand};

/// Accumulates rewritten instructions while tracking the old→new index map, then
/// remaps all branch/switch targets in [`Self::finish`].
#[derive(Debug, Default)]
pub struct Rebuilder {
    out: Vec<Insn>,
    /// `old_to_new[i]` = the output index the old instruction `i` begins at.
    old_to_new: Vec<usize>,
}

impl Rebuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the start of a new *old* instruction: its old index maps to the
    /// current output length. Mirrors `old_to_new.append(len(rebuilt))`.
    pub fn begin(&mut self) {
        self.old_to_new.push(self.out.len());
    }

    /// Append a rewritten instruction.
    pub fn push(&mut self, insn: Insn) {
        self.out.push(insn);
    }

    /// Peek the last-pushed instruction.
    #[must_use]
    pub fn last(&self) -> Option<&Insn> {
        self.out.last()
    }

    /// Overwrite the last-pushed instruction (e.g. negate a constant push for the
    /// `sub`→`add` zero-shift). Mirrors `rebuilt[-1] = …`.
    pub fn replace_last(&mut self, insn: Insn) {
        if let Some(last) = self.out.last_mut() {
            *last = insn;
        }
    }

    /// For the `tostring` radix-drop: the caller has already `begin`-ed the
    /// CURRENT old instruction (the `tostring`), so `old_to_new.last()` is the
    /// tostring's entry. Pop the previously-pushed instruction (the radix push)
    /// and set the tostring's entry to the new output length. Mirrors the Python
    /// `tostring` branch exactly: `rebuilt.pop(); old_to_new[-1] = len(rebuilt)`
    /// (where `old_to_new[-1]` is the entry appended for the tostring itself).
    pub fn drop_prev_remap_current(&mut self) {
        self.out.pop();
        if let Some(last) = self.old_to_new.last_mut() {
            *last = self.out.len();
        }
    }

    /// Finalise: append the one-past-end sentinel and remap every branch/switch
    /// target through the old→new map. Returns the rewritten instruction stream.
    #[must_use]
    pub fn finish(mut self) -> Vec<Insn> {
        // Sentinel for a target equal to len (one-past-end fall-through).
        self.old_to_new.push(self.out.len());
        remap_targets(&mut self.out, &self.old_to_new);
        self.out
    }
}

/// Remap every `Jump` / switch-case target through `old_to_new`. A target out of
/// range is left untouched (defensive; the rebuild guarantees in-range targets for
/// well-formed input).
///
/// Renumbers EVERY branch family — `branch_*`, `long_branch_*`, and switch cases.
///
/// The retired `build-ritual-scripts.py` renumbered targets with the regex
/// `^// @cs2 (branch\S*) …`, which matches `branch`/`branch_equals`/… but NOT the
/// `long_branch_*` family (those start with `long_branch`, not `branch`). That left a
/// latent off-by-one in donor long-comparison branches after any length-changing
/// rewrite (e.g. ritual `script20804`: a `tostring_localised` rewrite precedes a
/// `long_branch_greater_than`). The port layer is now the sole producer, so it emits
/// the CORRECT renumbering for all jump families — the first place the layer improves
/// on the hand-port rather than reproducing it byte-for-byte.
fn remap_targets(code: &mut [Insn], old_to_new: &[usize]) {
    let map = |t: i32| -> i32 {
        if t >= 0 && (t as usize) < old_to_new.len() {
            old_to_new[t as usize] as i32
        } else {
            t
        }
    };
    for insn in code.iter_mut() {
        match &mut insn.operand {
            Operand::Jump(t) => *t = map(*t),
            Operand::Switch(cases) => {
                for case in cases.iter_mut() {
                    case.target = map(case.target);
                }
            }
            _ => {}
        }
    }
}

/// Renumber a freshly-rebuilt instruction stream given a pre-built old→new map.
/// The map must already include the one-past-end sentinel. Used by passes that
/// build the map themselves (e.g. the surgical 17523 bypass).
pub fn renumber_with_map(code: &mut [Insn], old_to_new: &[usize]) {
    remap_targets(code, old_to_new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::SwitchCase;

    fn jump(t: i32) -> Insn {
        Insn {
            op: "branch".into(),
            operand: Operand::Jump(t),
        }
    }

    #[test]
    fn insertion_shifts_later_targets() {
        // old: [0 nop][1 nop][2 branch->0][3 branch->1]; rewrite instr 1 into TWO.
        let old = vec![Insn::bare("nop"), Insn::bare("nop"), jump(0), jump(1)];
        let mut rb = Rebuilder::new();
        for (i, insn) in old.iter().enumerate() {
            rb.begin();
            if i == 1 {
                rb.push(Insn::bare("a"));
                rb.push(Insn::bare("b"));
            } else {
                rb.push(insn.clone());
            }
        }
        let out = rb.finish();
        // [0 nop][1 a][2 b][3 branch->0][4 branch->1]
        assert_eq!(out.len(), 5);
        assert_eq!(out[3].operand, Operand::Jump(0)); // old 0 → new 0
        assert_eq!(out[4].operand, Operand::Jump(1)); // old 1 → new 1 (start of a/b)
    }

    #[test]
    fn switch_targets_are_remapped() {
        let old = vec![
            Insn::bare("x"),
            Insn {
                op: "switch".into(),
                operand: Operand::Switch(vec![
                    SwitchCase { value: 0, target: 3 },
                    SwitchCase { value: 1, target: 0 },
                ]),
            },
            Insn::bare("y"),
            Insn::bare("z"),
        ];
        let mut rb = Rebuilder::new();
        for (i, insn) in old.iter().enumerate() {
            rb.begin();
            if i == 0 {
                rb.push(Insn::bare("x"));
                rb.push(Insn::bare("x2")); // expand instr 0 → shifts everything +1
            } else {
                rb.push(insn.clone());
            }
        }
        let out = rb.finish();
        if let Operand::Switch(cases) = &out[2].operand {
            assert_eq!(cases[0].target, 4); // old 3 → new 4
            assert_eq!(cases[1].target, 0); // old 0 → new 0
        } else {
            panic!("expected switch");
        }
    }
}
