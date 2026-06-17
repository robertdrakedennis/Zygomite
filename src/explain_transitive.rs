//! `explain-interface --transitive`: the full transitive clientscript closure of
//! an interface, its size, and the count missing from the 910 base (the splice
//! burden to port the interface onto the 910 client).
//!
//! The component-bound script set surfaced by [`crate::explain`] is only the
//! depth-1 layer (the scripts the interface's component hooks call directly). The
//! decisive scoping fact for a ritual splice — "how big is this port" — is the
//! FULL closure of those scripts under the script→script call graph, and how many
//! of those scripts the 910 base cache lacks. `dep-tree-interface` only walks one
//! script level deep, so it does not answer this either.
//!
//! ## The script-keying subtlety
//!
//! Interface→script and script→script edges carry a *raw* script id (the integer
//! pushed in the component hook / the `gosub_with_params` operand). The clientscripts
//! archive (archive 12) keys groups by id, and each shipped group holds exactly one
//! script (file 0 / its single min-file). So a raw id resolves to a script either
//! as a group id directly, or — for the rare packed form — as `(group<<16)|file`.
//! [`ScriptSource::resolve`] re-keys raw ids consistently to a canonical *group id*
//! (the form the 910-base roster is keyed by), mirroring
//! `load_script_call_target_from_cache` in the dep-tree CLI path.
//!
//! ## Two reachability sets over one graph
//!
//! [`transitive_script_closure`] reports both:
//!
//! * The **full closure** (`closure`): every script transitively reachable from
//!   the interface's depth-1 scripts, base scripts and their subtrees included.
//!   Its SIZE is the honest "how many scripts does this interface call" figure.
//! * The **splice burden** (`missing_from_910`): the actionable "how big is this
//!   port" number. A script already present in the 910 base needs no splicing —
//!   and neither does anything it transitively calls, because the base already
//!   ships that whole subtree intact. So this pass treats a 910-base script as a
//!   *sink*: it is not counted and the walk does not descend through it. The
//!   result is the set of donor-new scripts reachable from the interface's hooks
//!   without passing through a 910-base script — exactly the set a cache overlay
//!   must splice for the interface to mount. It is always a subset of the full
//!   closure.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::script::{CompiledScript, Operand, ScriptArgSignature};

/// A resolver from a *raw* clientscript id to its canonical group, plus the
/// script→script call edges of a group.
///
/// Implementors own any decoded-script cache; [`transitive_script_closure`] only
/// reads through this trait, so the same closure walk runs over a flat cache, a
/// single-file `.js5` pack, or a synthetic in-memory graph (the unit tests). The
/// edge accessor returns *raw* callee ids (re-keyed by the walk via
/// [`Self::resolve`]) so implementors need not surface a borrowed
/// [`CompiledScript`] across calls.
pub trait ScriptSource {
    /// Resolve a raw script id to its canonical *group* id when the source holds
    /// a script for it, applying the raw-id re-keying contract (try the raw id as
    /// a group directly, else the packed `(group<<16)|file` split). Returns
    /// `None` when no script in the source matches.
    fn resolve(&self, raw_id: i32) -> Option<u32>;

    /// The raw ids of the scripts a canonical group's script calls (every
    /// `gosub_with_params`-style [`Operand::Script`] operand, in code order).
    /// Empty when the group cannot be decoded or calls nothing.
    fn callees(&self, group: u32) -> Vec<i32>;

    /// The declared argument signature of a canonical group's script, when it can
    /// be read. Used to detect a donor↔910 proc-id collision (same id, different
    /// arg arity). `None` when the group is absent or its header cannot be read;
    /// the default returns `None` so existing in-memory sources need not change.
    fn signature(&self, _group: u32) -> Option<ScriptArgSignature> {
        None
    }
}

/// Collect the raw callee ids from a decoded script's [`Operand::Script`]
/// operands, in code order. Shared by every [`ScriptSource`] implementation.
#[must_use]
pub fn script_callees(script: &CompiledScript) -> Vec<i32> {
    script
        .code
        .iter()
        .filter_map(|instruction| match instruction.operand {
            Operand::Script(id) => Some(id),
            _ => None,
        })
        .collect()
}

/// The 910-base script roster: which canonical group ids the base cache already
/// ships (and therefore need not be spliced).
pub trait BaseRoster {
    /// Whether the 910 base cache already contains a script for this group id.
    fn contains(&self, group: u32) -> bool;

    /// The declared argument signature of the 910-base script at this group id,
    /// when known. Compared against the donor's signature for the same id to
    /// detect proc-id collisions. `None` when absent or unreadable; the default
    /// returns `None` so a bare presence roster keeps the old (id-only) behavior.
    fn signature(&self, _group: u32) -> Option<ScriptArgSignature> {
        None
    }
}

/// A donor↔910 proc-id collision: a transitively-referenced script id that
/// exists in BOTH the donor (948) and the 910 base, but with DIFFERENT declared
/// argument signatures. The id is present in 910 — so the old id-only prune
/// would treat it as "already there" — but the 910 proc at that id is a
/// different procedure with a different arity. Keeping a `gosub_with_params <id>`
/// against it makes the 910 interpreter transfer the wrong arg count off the
/// stack (an under/overflow). Such an id must be REMAPPED, not pruned.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ScriptCollision {
    /// The colliding canonical script group id.
    pub group: u32,
    /// The donor (948) script's declared arg signature at this id.
    pub donor: ScriptArgSignature,
    /// The 910-base script's declared arg signature at this id.
    pub base: ScriptArgSignature,
}

/// The transitive clientscript closure of an interface and its splice burden.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TransitiveScripts {
    /// Every canonical script group reachable from the interface's depth-1
    /// scripts under the script→script call graph (base scripts INCLUDED, and
    /// their subtrees walked). This is the FULL closure; its length is the
    /// headline "transitive scripts" size.
    pub closure: BTreeSet<u32>,
    /// The splice burden: donor-new scripts (absent from the 910 base) reachable
    /// from the depth-1 set without passing *through* a 910-base script — because
    /// the base already ships every base script's whole subtree (see the module
    /// docs). This is a subset of [`Self::closure`]. Proc-id COLLISIONS (see
    /// [`Self::collisions`]) are included here too: they are not in the base as
    /// the *same* proc, so they must be spliced/remapped.
    pub missing_from_910: BTreeSet<u32>,
    /// Proc-id collisions: ids present in both donor and 910 base but with
    /// different arg signatures (see [`ScriptCollision`]). These are NOT pruned —
    /// they are folded into [`Self::missing_from_910`] and surfaced here with both
    /// signatures so the porter knows to remap them. Keyed by group id.
    pub collisions: BTreeMap<u32, ScriptCollision>,
    /// Raw script ids referenced (by the interface or a walked script) that no
    /// script in the source resolved — dangling references, reported for
    /// diagnostics. Keyed by the raw id as it appeared.
    pub unresolved: BTreeSet<u32>,
}

impl TransitiveScripts {
    /// Number of scripts in the full transitive closure.
    #[must_use]
    pub fn closure_len(&self) -> usize {
        self.closure.len()
    }

    /// Number of donor-new scripts the 910 base must be spliced with (collisions
    /// included).
    #[must_use]
    pub fn missing_len(&self) -> usize {
        self.missing_from_910.len()
    }

    /// Number of donor↔910 proc-id collisions detected.
    #[must_use]
    pub fn collision_len(&self) -> usize {
        self.collisions.len()
    }
}

/// Walk the script→script call graph from `seed_raw_ids` (the interface's
/// depth-1 component-bound scripts, as *raw* ids) to a fixpoint.
///
/// Computes two reachability sets over the same graph (see the module docs):
///  * `closure` — the FULL closure: descend through every reachable script
///    (base scripts and their subtrees included). Its size answers "how many
///    scripts does this interface transitively call".
///  * `missing_from_910` — the splice burden: donor-new scripts reachable
///    without descending *through* a 910-base script (base scripts are sinks,
///    because the base already ships their subtrees).
///
/// Re-keys raw ids consistently via [`ScriptSource::resolve`]; dangling refs go
/// to `unresolved`.
pub fn transitive_script_closure(
    seed_raw_ids: impl IntoIterator<Item = i32>,
    source: &dyn ScriptSource,
    base: &dyn BaseRoster,
) -> TransitiveScripts {
    let mut result = TransitiveScripts::default();
    let seeds: Vec<i32> = seed_raw_ids.into_iter().collect();

    // Pass 1 — FULL closure: descend through every reachable script. Unresolved
    // refs are recorded here (the splice-burden pass sees a subset of edges).
    let mut full_stack: Vec<u32> = Vec::new();
    let visit_full = |result: &mut TransitiveScripts, stack: &mut Vec<u32>, raw: i32| match source
        .resolve(raw)
    {
        Some(group) => {
            if result.closure.insert(group) {
                stack.push(group);
            }
        }
        None => {
            if raw >= 0 {
                result.unresolved.insert(raw as u32);
            }
        }
    };
    for &raw in &seeds {
        visit_full(&mut result, &mut full_stack, raw);
    }
    while let Some(group) = full_stack.pop() {
        for called in source.callees(group) {
            visit_full(&mut result, &mut full_stack, called);
        }
    }

    // Pass 2 — splice burden: descend through donor-new scripts AND proc-id
    // collisions; a 910-base script that is the SAME proc (matching arg
    // signature) is a sink (the base already ships it and its subtree).
    //
    // A base-present id is pruned ONLY when the donor and 910-base scripts at that
    // id have the same declared arg signature. If their signatures differ, it is a
    // COLLISION — the 910 proc at that id is a different procedure with a
    // different arity, so a `gosub_with_params <id>` kept against it would
    // transfer the wrong arg count off the operand stack. Such an id is recorded
    // in `collisions`, folded into the missing/remap set, and descended through
    // like a donor-new script (its donor subtree is needed). Signatures are only
    // compared when BOTH sides are readable; an unreadable signature falls back to
    // the conservative id-only prune (no false collision).
    let mut burden_stack: Vec<u32> = Vec::new();
    let visit_burden = |result: &mut TransitiveScripts, stack: &mut Vec<u32>, raw: i32| {
        let Some(group) = source.resolve(raw) else {
            return;
        };
        if base.contains(group) {
            // Present in 910: prune unless the signatures prove a collision.
            if let (Some(donor_sig), Some(base_sig)) =
                (source.signature(group), base.signature(group))
                && donor_sig != base_sig
            {
                // Collision: same id, different proc. Remap required.
                result.collisions.entry(group).or_insert(ScriptCollision {
                    group,
                    donor: donor_sig,
                    base: base_sig,
                });
                if result.missing_from_910.insert(group) {
                    stack.push(group);
                }
            }
            // Matching (or unreadable) signature: genuine base proc — sink it.
            return;
        }
        // Donor-new: not in the 910 base at all.
        if result.missing_from_910.insert(group) {
            stack.push(group);
        }
    };
    for &raw in &seeds {
        visit_burden(&mut result, &mut burden_stack, raw);
    }
    while let Some(group) = burden_stack.pop() {
        for called in source.callees(group) {
            visit_burden(&mut result, &mut burden_stack, called);
        }
    }

    result
}

/// A [`ScriptSource`] backed by a present-group roster and lazy decoders.
///
/// Used by the CLI to walk the donor (948) clientscripts archive: the full group
/// roster is captured up front (so [`Self::resolve`] can re-key raw ids against
/// the real archive), and each group's call edges (and arg signature) are decoded
/// on first visit and memoised. The walk needs callee ids and, for the collision
/// check, the declared arg signature, so the two decoders return those directly —
/// no [`CompiledScript`] is borrowed across calls.
pub struct MapScriptSource<F, G>
where
    F: Fn(u32) -> Vec<i32>,
    G: Fn(u32) -> Option<ScriptArgSignature>,
{
    /// Canonical group ids the source holds a script for (the full archive
    /// roster). Used for raw-id re-keying.
    present: BTreeSet<u32>,
    /// Decoder invoked the first time a group's edges are needed; returns the
    /// group's raw callee ids. Its result is cached.
    decode_callees: F,
    /// Decoder invoked the first time a group's signature is needed; returns the
    /// group's declared arg signature (`None` if unreadable). Its result is cached.
    decode_signature: G,
    /// Lazily filled callee cache, keyed by canonical group id.
    cache: std::cell::RefCell<BTreeMap<u32, Vec<i32>>>,
    /// Lazily filled signature cache, keyed by canonical group id.
    sig_cache: std::cell::RefCell<BTreeMap<u32, Option<ScriptArgSignature>>>,
}

impl<F, G> MapScriptSource<F, G>
where
    F: Fn(u32) -> Vec<i32>,
    G: Fn(u32) -> Option<ScriptArgSignature>,
{
    /// Build a source over the present group roster, decoding callees and arg
    /// signatures lazily.
    pub fn new(present: BTreeSet<u32>, decode_callees: F, decode_signature: G) -> Self {
        Self {
            present,
            decode_callees,
            decode_signature,
            cache: std::cell::RefCell::new(BTreeMap::new()),
            sig_cache: std::cell::RefCell::new(BTreeMap::new()),
        }
    }
}

impl<F, G> ScriptSource for MapScriptSource<F, G>
where
    F: Fn(u32) -> Vec<i32>,
    G: Fn(u32) -> Option<ScriptArgSignature>,
{
    fn resolve(&self, raw_id: i32) -> Option<u32> {
        if raw_id < 0 {
            return None;
        }
        let raw = raw_id as u32;
        // A raw id is a group id directly for every shipped script (single-file
        // groups). The packed `(group<<16)|file` split is the documented fallback
        // for the rare multi-file case; it collapses to the same group when file
        // is 0, so trying the raw id first is correct and cheap.
        if self.present.contains(&raw) {
            return Some(raw);
        }
        let group = raw >> 16;
        if self.present.contains(&group) {
            return Some(group);
        }
        None
    }

    fn callees(&self, group: u32) -> Vec<i32> {
        if let Some(cached) = self.cache.borrow().get(&group) {
            return cached.clone();
        }
        let callees = (self.decode_callees)(group);
        self.cache.borrow_mut().insert(group, callees.clone());
        callees
    }

    fn signature(&self, group: u32) -> Option<ScriptArgSignature> {
        if let Some(cached) = self.sig_cache.borrow().get(&group) {
            return *cached;
        }
        let sig = (self.decode_signature)(group);
        self.sig_cache.borrow_mut().insert(group, sig);
        sig
    }
}

/// A [`BaseRoster`] over the 910-base canonical group ids, optionally carrying
/// each present group's declared arg signature (for collision detection).
pub struct SetRoster {
    /// Present group ids, plus each one's arg signature when known. A `None`
    /// signature means the group is present but its header could not be read, so
    /// the collision check conservatively treats it as a non-collision sink.
    groups: BTreeMap<u32, Option<ScriptArgSignature>>,
}

impl SetRoster {
    /// Build a presence-only roster from the base cache's script group ids (no
    /// signatures — the collision check then falls back to the id-only prune).
    pub fn new(groups: BTreeSet<u32>) -> Self {
        Self {
            groups: groups.into_iter().map(|g| (g, None)).collect(),
        }
    }

    /// Build a roster that also carries each present group's declared arg
    /// signature, enabling donor↔910 proc-id collision detection.
    pub fn with_signatures(groups: BTreeMap<u32, Option<ScriptArgSignature>>) -> Self {
        Self { groups }
    }

    /// The number of script groups in the base roster.
    #[must_use]
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether the base roster is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

impl BaseRoster for SetRoster {
    fn contains(&self, group: u32) -> bool {
        self.groups.contains_key(&group)
    }

    fn signature(&self, group: u32) -> Option<ScriptArgSignature> {
        self.groups.get(&group).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::Instruction;

    /// A tiny synthetic [`ScriptSource`] built from a `group → callee groups`
    /// adjacency map. Each entry's callees stand in for `gosub_with_params`
    /// operands (the only edge kind the closure follows), so the closure walk is
    /// exercised without any cache or pack dependency. Optional per-group arg
    /// signatures drive the collision tests.
    struct AdjSource {
        edges: BTreeMap<u32, Vec<i32>>,
        signatures: BTreeMap<u32, ScriptArgSignature>,
    }

    impl AdjSource {
        fn new(edges: &[(u32, &[i32])]) -> Self {
            let mut edge_map = BTreeMap::new();
            for (group, callees) in edges {
                // Round-trip through a CompiledScript so `script_callees` (the
                // shared extractor the real sources use) is exercised too.
                let code = callees
                    .iter()
                    .map(|&c| Instruction {
                        opcode: 0,
                        command: "gosub_with_params".to_string(),
                        operand: Operand::Script(c),
                    })
                    .collect();
                let script = CompiledScript {
                    name: None,
                    local_count_int: 0,
                    local_count_object: 0,
                    local_count_long: 0,
                    argument_count_int: 0,
                    argument_count_object: 0,
                    argument_count_long: 0,
                    code,
                };
                edge_map.insert(*group, script_callees(&script));
            }
            Self {
                edges: edge_map,
                signatures: BTreeMap::new(),
            }
        }

        /// Attach a donor arg signature `(int, obj, long)` to a group.
        fn with_signature(mut self, group: u32, int: u16, obj: u16, long: u16) -> Self {
            self.signatures
                .insert(group, ScriptArgSignature { int, obj, long });
            self
        }
    }

    impl ScriptSource for AdjSource {
        fn resolve(&self, raw_id: i32) -> Option<u32> {
            if raw_id < 0 {
                return None;
            }
            let raw = raw_id as u32;
            if self.edges.contains_key(&raw) {
                return Some(raw);
            }
            let group = raw >> 16;
            if self.edges.contains_key(&group) {
                return Some(group);
            }
            None
        }

        fn callees(&self, group: u32) -> Vec<i32> {
            self.edges.get(&group).cloned().unwrap_or_default()
        }

        fn signature(&self, group: u32) -> Option<ScriptArgSignature> {
            self.signatures.get(&group).copied()
        }
    }

    /// Build a base roster carrying explicit `(group → (int,obj,long))` arg
    /// signatures for the collision tests.
    fn base_with_sigs(sigs: &[(u32, u16, u16, u16)]) -> SetRoster {
        let map: BTreeMap<u32, Option<ScriptArgSignature>> = sigs
            .iter()
            .map(|&(g, int, obj, long)| (g, Some(ScriptArgSignature { int, obj, long })))
            .collect();
        SetRoster::with_signatures(map)
    }

    #[test]
    fn closure_walks_to_fixpoint() {
        // 100 -> 101 -> 102 ; 100 -> 103 ; 102 -> 103 (shared) ; 103 leaf.
        let source =
            AdjSource::new(&[(100, &[101, 103]), (101, &[102]), (102, &[103]), (103, &[])]);
        let base = SetRoster::new(BTreeSet::new()); // nothing in base
        let result = transitive_script_closure([100], &source, &base);
        assert_eq!(
            result.closure,
            BTreeSet::from([100, 101, 102, 103]),
            "closure must reach every transitively-called script once"
        );
        // With an empty base, every closure member is donor-new.
        assert_eq!(
            result.missing_from_910,
            BTreeSet::from([100, 101, 102, 103])
        );
        assert!(result.unresolved.is_empty());
    }

    #[test]
    fn base_scripts_shield_their_subtree_from_the_splice_burden() {
        // 100(donor) -> 200(base) -> 300(donor-only, only reachable via 200).
        // 200 is in the base, so its subtree (300) must NOT be pulled into the
        // splice burden — the base already ships 200 and everything it calls.
        let source = AdjSource::new(&[(100, &[200]), (200, &[300]), (300, &[])]);
        let base = SetRoster::new(BTreeSet::from([200]));
        let result = transitive_script_closure([100], &source, &base);
        // The FULL closure still reaches everything (200's subtree included),
        // because the full-closure size is the honest "how many scripts" figure.
        assert_eq!(result.closure, BTreeSet::from([100, 200, 300]));
        // Only 100 is a donor-new script that must be spliced; 300 is shielded by
        // the base-shipped 200, and 200 itself is already in the base.
        assert_eq!(result.missing_from_910, BTreeSet::from([100]));
    }

    #[test]
    fn handles_cycles_without_looping() {
        // 100 <-> 101 cycle, plus 101 -> 102 leaf.
        let source = AdjSource::new(&[(100, &[101]), (101, &[100, 102]), (102, &[])]);
        let base = SetRoster::new(BTreeSet::new());
        let result = transitive_script_closure([100], &source, &base);
        assert_eq!(result.closure, BTreeSet::from([100, 101, 102]));
        assert_eq!(result.missing_from_910, BTreeSet::from([100, 101, 102]));
    }

    #[test]
    fn re_keys_packed_ids_to_canonical_group() {
        // The seed is given as a packed (group<<16)|file id; it must re-key to the
        // canonical group 102 and resolve there.
        let source = AdjSource::new(&[(102, &[])]);
        let base = SetRoster::new(BTreeSet::new());
        let packed = 102_i32 << 16; // (group 102, file 0) packed form
        let result = transitive_script_closure([packed], &source, &base);
        assert_eq!(result.closure, BTreeSet::from([102]));
    }

    #[test]
    fn records_unresolved_dangling_refs() {
        // 100 calls 999 which no script in the source provides.
        let source = AdjSource::new(&[(100, &[999])]);
        let base = SetRoster::new(BTreeSet::new());
        let result = transitive_script_closure([100], &source, &base);
        assert_eq!(result.closure, BTreeSet::from([100]));
        assert_eq!(result.unresolved, BTreeSet::from([999]));
    }

    #[test]
    fn missing_is_subset_of_closure() {
        // Mixed base/donor graph; the splice burden must always be a subset of the
        // full closure.
        let source = AdjSource::new(&[(1, &[2, 3]), (2, &[4]), (3, &[5]), (4, &[]), (5, &[])]);
        let base = SetRoster::new(BTreeSet::from([3, 4]));
        let result = transitive_script_closure([1], &source, &base);
        assert!(
            result.missing_from_910.is_subset(&result.closure),
            "splice burden must be a subset of the full closure"
        );
        // 1,2 are donor-new and reachable; 3 is a base frontier (4 via 2 is donor
        // but reachable independently, 5 is shielded behind base 3).
        assert!(result.missing_from_910.contains(&1));
        assert!(result.missing_from_910.contains(&2));
        assert!(!result.missing_from_910.contains(&3));
        assert!(!result.missing_from_910.contains(&5));
    }

    #[test]
    fn map_script_source_decodes_lazily_and_resolves() {
        // Two scripts: 10 -> 11. Decode counter proves laziness + memoisation.
        use std::cell::Cell;
        let calls = Cell::new(0_usize);
        let present = BTreeSet::from([10_u32, 11]);
        let source = MapScriptSource::new(
            present,
            |g| {
                calls.set(calls.get() + 1);
                if g == 10 { vec![11] } else { Vec::new() }
            },
            |_g| None, // signatures unused in this test
        );
        assert_eq!(source.resolve(10), Some(10));
        assert_eq!(source.resolve(11), Some(11));
        let base = SetRoster::new(BTreeSet::new());
        let result = transitive_script_closure([10], &source, &base);
        assert_eq!(result.closure, BTreeSet::from([10, 11]));
        // Each group decoded exactly once despite repeated visits.
        assert_eq!(calls.get(), 2, "callee decode must be memoised per group");
    }

    // --- proc-id collisions (Bug B) ------------------------------------------

    #[test]
    fn collision_when_signatures_differ_is_flagged_not_pruned() {
        // 100(donor) -> 5360. 5360 exists in BOTH donor and 910 base, but the
        // donor declares 2 int args and 910 declares 5 — a COLLISION. The id must
        // NOT be pruned as "present in 910": it belongs in the missing/remap set
        // and the collision detail, and the walk descends through its donor
        // subtree (5360 -> 700, a donor-only callee).
        let source = AdjSource::new(&[(100, &[5360]), (5360, &[700]), (700, &[])])
            .with_signature(100, 0, 0, 0)
            .with_signature(5360, 2, 0, 0) // donor: 2 int args
            .with_signature(700, 0, 0, 0);
        let base = base_with_sigs(&[(5360, 5, 0, 0)]); // 910: 5 int args, different proc
        let result = transitive_script_closure([100], &source, &base);

        // 5360 is flagged as a collision with both signatures recorded.
        assert_eq!(result.collision_len(), 1);
        let c = result.collisions.get(&5360).expect("5360 collision");
        assert_eq!(
            c.donor,
            ScriptArgSignature {
                int: 2,
                obj: 0,
                long: 0
            }
        );
        assert_eq!(
            c.base,
            ScriptArgSignature {
                int: 5,
                obj: 0,
                long: 0
            }
        );
        // It is in the splice/remap burden, NOT pruned …
        assert!(result.missing_from_910.contains(&5360));
        // … and the walk descended through it to reach the donor-only 700.
        assert!(result.missing_from_910.contains(&700));
        assert!(result.missing_from_910.contains(&100));
    }

    #[test]
    fn matching_signature_is_pruned_as_genuine_base_proc() {
        // Same id 5360 in both, but identical signatures (2 int args): it is the
        // SAME proc, so it is pruned (a sink) and its subtree shielded — no
        // collision, exactly the pre-existing behavior.
        let source = AdjSource::new(&[(100, &[5360]), (5360, &[700]), (700, &[])])
            .with_signature(100, 0, 0, 0)
            .with_signature(5360, 2, 0, 0)
            .with_signature(700, 0, 0, 0);
        let base = base_with_sigs(&[(5360, 2, 0, 0)]); // identical signature
        let result = transitive_script_closure([100], &source, &base);

        assert_eq!(
            result.collision_len(),
            0,
            "identical signature is no collision"
        );
        assert!(
            !result.missing_from_910.contains(&5360),
            "matching base proc is pruned"
        );
        assert!(
            !result.missing_from_910.contains(&700),
            "its subtree is shielded"
        );
        assert_eq!(result.missing_from_910, BTreeSet::from([100]));
        // The full closure still reaches everything.
        assert_eq!(result.closure, BTreeSet::from([100, 5360, 700]));
    }

    #[test]
    fn unreadable_base_signature_falls_back_to_id_only_prune() {
        // 5360 present in base but with an UNKNOWN signature (None): the
        // collision check must stay conservative and prune by id (no false
        // collision), matching the old behavior for a presence-only roster.
        let source = AdjSource::new(&[(100, &[5360]), (5360, &[700]), (700, &[])])
            .with_signature(5360, 2, 0, 0);
        let base = SetRoster::with_signatures(BTreeMap::from([(5360_u32, None)]));
        let result = transitive_script_closure([100], &source, &base);

        assert_eq!(
            result.collision_len(),
            0,
            "unknown base sig must not collide"
        );
        assert!(!result.missing_from_910.contains(&5360));
        assert!(!result.missing_from_910.contains(&700));
    }

    #[test]
    fn presence_only_roster_keeps_legacy_behavior() {
        // A bare SetRoster::new (no signatures) must behave exactly as before:
        // base ids prune by id, no collisions ever surface.
        let source = AdjSource::new(&[(100, &[200]), (200, &[300]), (300, &[])])
            .with_signature(200, 1, 0, 0);
        let base = SetRoster::new(BTreeSet::from([200]));
        let result = transitive_script_closure([100], &source, &base);
        assert_eq!(result.collision_len(), 0);
        assert_eq!(result.missing_from_910, BTreeSet::from([100]));
    }

    #[test]
    fn multiple_collisions_all_flagged() {
        // Two independent colliding ids reachable from the seed; both must be
        // flagged and remapped, proving the collision set accumulates.
        let source = AdjSource::new(&[(1, &[5360, 7853]), (5360, &[]), (7853, &[])])
            .with_signature(1, 0, 0, 0)
            .with_signature(5360, 2, 0, 0)
            .with_signature(7853, 1, 1, 0);
        let base = base_with_sigs(&[(5360, 5, 0, 0), (7853, 3, 0, 0)]);
        let result = transitive_script_closure([1], &source, &base);
        assert_eq!(result.collision_len(), 2);
        assert!(result.collisions.contains_key(&5360));
        assert!(result.collisions.contains_key(&7853));
        assert!(result.missing_from_910.contains(&5360));
        assert!(result.missing_from_910.contains(&7853));
    }
}
