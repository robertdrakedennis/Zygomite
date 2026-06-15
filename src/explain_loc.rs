//! `explain-loc N` — the loc-first complement to `explain-interface`.
//!
//! Answers "give me this loc's interactions and which interfaces each option is
//! supposed to show, gated by which varps/varbits". It:
//!
//! 1. Decodes loc `N` and resolves its **multivar** surface (a parent loc that
//!    switches its model/ops by a gating varbit/varp into per-value child locs).
//!    Accepts either the parent id or a child id.
//! 2. Lists the per-variant ops (`op1..op5`, members ops) and, where the cache
//!    wires an op to a clientscript, follows it to its `IF_OPENTOP`/`IF_OPENSUB`
//!    targets + var/enum/dbtable reads (server-driven ops carry no cache script —
//!    expected; they fall through to step 3).
//! 3. **Reverse-matches candidate interfaces** by the gating varbit/varp. The
//!    loc→interface open is game-server logic (the server issues `IF_OPENTOP` on
//!    the `OPLOC`), so it is not a cache edge. The cache-derivable bridge is the
//!    gating varbit's **config neighbourhood**: the run of player varps used as
//!    varbit bases around the gate's base varp is the feature's varp block; the
//!    interface the loc opens is the one whose onload/hook script closure reads
//!    that same block. Candidates are seeded by op/name text overlap and ranked
//!    with a strong bonus for reading the gating block. Each candidate is
//!    summarised with its `explain-interface` closure so the match is legible.
//! 4. **Labels the server-binding case** so a confident-looking wrong answer can't
//!    masquerade as the truth. Because the open is server-side, no clientscript in
//!    the cache actually opens these interfaces — so every candidate is a heuristic
//!    (`confidence: low`) and the report carries a "server-side open: no cache
//!    binding" banner. A *broad* gating block (a large feature reservation, e.g.
//!    the ritual pedestal's 89 varps 11096..11260) includes a generic tail shared
//!    by a whole UI family; block reads are therefore specificity-weighted (a varp
//!    read by many interfaces is discounted) and a candidate matching only that
//!    generic tail is flagged a `generic_block_match` and down-ranked, so the
//!    action-bar/combat false positives no longer present as a confident #1 while a
//!    feature-specific match (the relic monolith's 691) still surfaces on top. When
//!    a real cache open edge *does* exist (a script opens the interface and reads
//!    the gate), that candidate is `confidence: high` and outranks every heuristic.
//!
//! Source of truth reused (not re-derived): the loc multivar parser
//! ([`crate::config::parse_loc`]), the interface component port +
//! `explain-interface` closure ([`crate::interface`] / [`crate::explain`]), and
//! the dep-tree catalog loader ([`crate::dep_tree::ResolverContext`]). The worked
//! example (monolith parent 115416 → child 116440 → interface 691) is the relic
//! system documented in `server/cache-patches/relic-system-948/README.md`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use crate::cache::FlatCache;
use crate::dep_tree::ResolverContext;
use crate::error::{Context, Result};
use crate::interface::component::explain_interface_group;
use crate::script::{CompiledScript, Operand, decode_script};
use crate::vars::VarDomain;

/// How far (in call edges) the bound-script closure is walked when checking which
/// gating-block varps an interface reads. The relic monolith's onload reaches its
/// state reader (script 14603, reading the relic varps) at depth 3; a small bound
/// keeps the closure feature-local instead of fanning out into shared library
/// scripts (which eventually read every varp and so discriminate nothing).
const CLOSURE_DEPTH: u32 = 4;

/// Half-width (in varp ids) of the gating var's feature window, centred on the
/// gate's base varp. A feature reserves a band of player varps for its state
/// (the relic system spans ~9238..9320 around the monolith gate's base 9239 — its
/// unlock/energy varps sit a few dozen ids above the gate's own base run). The
/// window is intersected with the varps actually used as varbit bases, so only
/// real feature varps are reported, and is paired with op/name text overlap so a
/// merely-adjacent feature is not mistaken for the gated one.
const BLOCK_WINDOW: u32 = 96;

/// Minimum token length for a loc op/name word to count as a search signal. Drops
/// articles/short connectives; "relic"/"powers"/"manage" survive.
const MIN_TOKEN_LEN: usize = 4;

/// Per-candidate score: each gating-block varp read by the interface's bounded
/// closure is worth this much. Set well above any plausible token-overlap count so
/// the interface that actually reads the gate's feature block always sorts first.
const BLOCK_VARP_WEIGHT: i64 = 100;

/// Above this many varps the gating var's feature block is "broad": large feature
/// reservations (the ritual-site block spans 89 varps, 11096..11260) include a
/// generic tail shared by a whole UI family (action-bar / combat interfaces), so a
/// candidate merely reading the block is not, on its own, evidence it is *the*
/// gated interface. At/under this width (the relic monolith's effective block is
/// tight) every block read is full-weight, matching prior behaviour. (Picked above
/// any single feature's own varp run but below the broad ritual reservation.)
const BROAD_BLOCK_THRESHOLD: usize = 30;

/// In a broad block, a block varp read by more than this many interface closures is
/// treated as generic shared chrome (the ritual tail varps are each read by ~15
/// interfaces; the relic-specific varps by 2–5). Generic block reads are
/// specificity-discounted (`BLOCK_VARP_WEIGHT / readers`) so a broad-aggregator
/// closure that incidentally covers the block cannot out-score a focused
/// feature-local reader. Feature-specific block varps (few readers) keep full
/// weight, so the relic monolith's interface 691 is unaffected.
const GENERIC_VARP_READERS: usize = 8;

/// The open-interface clientscript opcode in this opcode book
/// (`if_opensubclient(component, clientinterface)`; the clientinterface arg is the
/// interface opened). A loc→interface open is normally issued by the game server on
/// the `OPLOC`, so this op rarely appears in a cache-reachable script — when it
/// does, and the opening closure also reads the gate, that is a real cache edge and
/// the candidate is high-confidence rather than a heuristic domain match.
const OPEN_INTERFACE_OPCODE: &str = "if_opensubclient";

/// Generic English words and UI chrome that carry no feature signal.
const STOPWORDS: &[&str] = &[
    "with", "from", "this", "that", "your", "have", "into", "more", "menu", "none", "null", "click",
    "here", "back", "next", "page", "open", "close", "cancel", "select", "view", "examine", "press",
];

// ───────────────────────────── output model ─────────────────────────────

/// One `value → child loc → ops` row of the multivar variant table.
#[derive(Clone, Debug, Serialize)]
pub struct Variant {
    /// The gating var value that selects this child (the multivar slot index), or
    /// `null` for the default slot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<u32>,
    /// `true` when this is the multivar default child.
    pub is_default: bool,
    /// The child loc id selected at this value.
    pub child_loc: u32,
    /// The child loc's name, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The child loc's ops in slot order (`op1..op5`, then members ops), absent
    /// slots elided. Each entry is `"<slot>: <label>"`.
    pub ops: Vec<OpEntry>,
}

/// One op slot of a (variant, loc) plus whatever cache edges it carries.
#[derive(Clone, Debug, Serialize)]
pub struct OpEntry {
    /// Op slot label, e.g. `op1`, `membersop2`.
    pub slot: String,
    /// The op text the player sees, e.g. `Manage powers`.
    pub label: String,
    /// Interface ids this op's bound clientscript opens (`IF_OPENTOP`/`IF_OPENSUB`),
    /// when the cache wires one. Empty for server-driven ops (the common case).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub opens_interfaces: Vec<u32>,
    /// `true` when no clientscript is bound to this op (server-driven open).
    pub server_driven: bool,
}

/// How much to trust a candidate interface as the loc's actual open target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// A real cache edge: a clientscript reachable from this interface's bound
    /// closure opens it (`if_opensubclient`) and the same closure reads the gating
    /// var. The cache itself ties the loc's gate to this interface.
    High,
    /// Heuristic domain match only — there is no cache open edge; the candidate was
    /// surfaced by op/name text overlap and/or reads of the gating var's feature
    /// block. The real loc→open is server-side; verify in the loc handler.
    Low,
}

/// A candidate interface the loc's option is likely to open, with the evidence.
#[derive(Clone, Debug, Serialize)]
pub struct CandidateInterface {
    /// Interface group id.
    pub interface: u32,
    /// Confidence that this is the loc's real open target. `low` unless a cache open
    /// edge was found (see [`Confidence`]); a `low` candidate is never the answer,
    /// only a domain suggestion.
    pub confidence: Confidence,
    /// `true` when this candidate's gating-block reads are dominated by generic
    /// varps shared across a whole UI family (the broad-block false-positive
    /// pattern), i.e. it reads the block but not a feature-specific subset. Drives
    /// the "generic block match" annotation; always `false` for a tight block.
    pub generic_block_match: bool,
    /// Total ranking score (token overlap + specificity-weighted gating-block varp
    /// bonus). In a broad block, generic (widely-read) block varps are discounted.
    pub score: i64,
    /// Loc op/name tokens found in this interface's component text/ops.
    pub matched_tokens: Vec<String>,
    /// Gating-block varps this interface's bounded bound-script closure reads — the
    /// cache bridge that ties it to the gating varbit's feature.
    pub gating_block_varps: Vec<u32>,
    /// Other gating-related varps the closure reads (same feature, outside the
    /// contiguous block), e.g. a sibling unlock bitset.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_varps: Vec<u32>,
    /// Enum ids in the interface's `explain-interface` closure (legibility).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enums: Vec<u32>,
    /// DbTable ids the closure reads (legibility).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dbtables: Vec<u32>,
    /// A short title pulled from the interface's largest/earliest static text, to
    /// make the candidate human-recognisable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The whole `explain-loc N` answer.
#[derive(Clone, Debug, Serialize)]
pub struct ExplainedLoc {
    /// The loc id the user asked about.
    pub queried_loc: u32,
    /// The multivar parent id (equals `queried_loc` when a parent was given, or the
    /// resolved parent when a child was given, or the loc itself when not multivar).
    pub parent_loc: u32,
    /// `true` when the queried loc is a multivar parent.
    pub is_multivar: bool,
    /// The gating varbit id, when the multivar switches on a varbit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gating_varbit: Option<u32>,
    /// The gating varp id, when the multivar switches on a varp directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gating_varp: Option<u32>,
    /// For a gating varbit: its base varp (the player varp it bit-packs into).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gating_base_varp: Option<u32>,
    /// The gating var's feature varp window — the varbit-base varps within a fixed
    /// band of the base. Empty for a direct gating varp.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gating_block: Vec<u32>,
    /// `true` when the loc→interface open is server-side: no clientscript reachable
    /// from the loc's ops (or from any candidate's closure) opens an interface tied
    /// to the gate, so no cache edge exists. The candidates below are then heuristic
    /// domain matches only and must be verified in the server loc handler.
    pub server_side_open: bool,
    /// Number of clientscripts in the cache that read the gating varbit/varp
    /// directly. `0` is the strongest signal that the gate is purely server-driven
    /// (no cache code consumes it); a non-zero count is reported as evidence but,
    /// absent an actual open edge, still leaves the open server-side.
    pub gate_script_readers: u32,
    /// The human banner for the server-side / no-cache-edge case, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_note: Option<String>,
    /// The variant table: per gating value → child loc → ops.
    pub variants: Vec<Variant>,
    /// Ranked candidate interfaces for the loc's options (best first).
    pub candidate_interfaces: Vec<CandidateInterface>,
}

// ───────────────────────────── options / entry ─────────────────────────────

/// Options for [`run`].
pub struct ExplainLocOptions {
    /// Loc id (a multivar parent or one of its children).
    pub loc: u32,
    /// Build the catalog decodes at.
    pub build: u32,
    /// Subbuild (opcode book selection).
    pub subbuild: u32,
    /// How many candidate interfaces to report.
    pub max_candidates: usize,
    /// Emit JSON instead of the human report.
    pub json: bool,
}

/// Decode + explain a loc against an already-open flat cache.
pub fn explain(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    opts: &ExplainLocOptions,
) -> Result<ExplainedLoc> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, opts.build, opts.subbuild)?;
    let index = LocIndex::build(&ctx);
    index.explain(opts.loc, opts.max_candidates)
}

/// Run the command: explain, then print JSON or the human report.
pub fn run(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    opts: &ExplainLocOptions,
) -> Result<()> {
    let explained = explain(cache, tar_path, data_dir, opts)?;
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&explained).context("encode explain-loc JSON")?
        );
    } else {
        print!("{}", render_human(&explained));
    }
    Ok(())
}

// ───────────────────────────── core index ─────────────────────────────

/// Pre-computed views over the catalog needed by the reverse-join: clientscripts
/// re-keyed by their natural id (group; clientscript files are always file 0) with
/// per-script var-reads + call edges, the interface→bound-scripts map, and the
/// interface→text-tokens map. Built once per `explain-loc` invocation.
struct LocIndex<'ctx> {
    ctx: &'ctx ResolverContext,
    /// Natural script id → decoded script. (`ctx.decoded_scripts` is keyed by the
    /// packed `(group<<16)|file`; we re-key to the bare script id so it joins with
    /// the component-hook script ids, which are stored un-packed.)
    scripts: BTreeMap<u32, CompiledScript>,
    /// Script id → player-varp ids it reads directly.
    reads_varp: BTreeMap<u32, BTreeSet<u32>>,
    /// Script id → varbit ids it reads directly.
    reads_varbit: BTreeMap<u32, BTreeSet<u32>>,
    /// Script id → script ids it calls.
    calls: BTreeMap<u32, BTreeSet<u32>>,
    /// Interface group → script ids bound to its components (hooks + onload).
    iface_scripts: BTreeMap<u32, BTreeSet<u32>>,
    /// Interface group → lowercased text/op tokens of its components.
    iface_tokens: BTreeMap<u32, BTreeSet<String>>,
    /// Player-varp id → number of interface bound-closures that read it. Used to
    /// discount generic (widely-read) block varps so a broad-aggregator closure can
    /// not out-rank a feature-local reader on a large gating block.
    varp_readers: BTreeMap<u32, u32>,
    /// Script id → interface ids it opens via [`OPEN_INTERFACE_OPCODE`]
    /// (`clientinterface` arg). Empty in practice — loc opens are server-side — but
    /// when present it is the cache edge that promotes a candidate to high confidence.
    opens_interface: BTreeMap<u32, BTreeSet<u32>>,
}

impl<'ctx> LocIndex<'ctx> {
    fn build(ctx: &'ctx ResolverContext) -> Self {
        // Re-key scripts by their natural id and decode any the catalog left raw.
        let mut scripts: BTreeMap<u32, CompiledScript> = BTreeMap::new();
        for (&packed, script) in &ctx.decoded_scripts {
            scripts.insert(packed >> 16, script.clone());
        }
        for (&packed, bytes) in &ctx.scripts {
            let id = packed >> 16;
            if let std::collections::btree_map::Entry::Vacant(slot) = scripts.entry(id)
                && let Ok(script) = decode_script(bytes, &ctx.opcode_book, ctx.build)
            {
                slot.insert(script);
            }
        }

        let mut reads_varp: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut reads_varbit: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut calls: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut opens_interface: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for (&id, script) in &scripts {
            // Track the most recent push_constant_int literals so an `if_opensubclient`
            // can recover its `clientinterface` argument (the interface it opens).
            let mut int_literals: Vec<i32> = Vec::new();
            for instruction in &script.code {
                match &instruction.operand {
                    Operand::VarRef(var) if var.domain == VarDomain::Player => {
                        reads_varp.entry(id).or_default().insert(u32::from(var.id));
                    }
                    Operand::VarBitRef(var) => {
                        reads_varbit.entry(id).or_default().insert(u32::from(var.id));
                    }
                    Operand::Script(called) if *called >= 0 => {
                        calls.entry(id).or_default().insert(*called as u32);
                    }
                    Operand::Int(value) if instruction.command == "push_constant_int" => {
                        int_literals.push(*value);
                    }
                    _ => {}
                }
                if instruction.command == OPEN_INTERFACE_OPCODE {
                    // `if_opensubclient(component, clientinterface)`: the clientinterface
                    // (interface opened) is the last int pushed before the call.
                    if let Some(&iface) = int_literals.iter().rev().find(|&&v| v > 0) {
                        opens_interface.entry(id).or_default().insert(iface as u32);
                    }
                }
                // Any opcode other than a bare int push consumes/relocates the stack,
                // so the literal window only carries the immediately-preceding pushes.
                if !matches!(&instruction.operand, Operand::Int(_))
                    || instruction.command != "push_constant_int"
                {
                    int_literals.clear();
                }
            }
        }

        // Interface → bound scripts + text tokens, from the catalog's parsed
        // components (re-uses the same `parse_component_deps` the dep-tree does).
        let mut iface_scripts: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let mut iface_tokens: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
        for (&iface, comps) in &ctx.parsed_components {
            let scripts_set = iface_scripts.entry(iface).or_default();
            for deps in comps.values() {
                for &script in &deps.scripts {
                    scripts_set.insert(script);
                }
            }
        }
        // Tokens come from the explain projection (static text + op labels). Decode
        // each group's component files via the same path `explain-interface` uses.
        for (&iface, files) in &ctx.interfaces {
            if let Ok(explained) = explain_interface_group(iface, files, ctx.build) {
                let mut tokens = BTreeSet::new();
                for component in &explained.components {
                    if let Some(text) = &component.text {
                        collect_tokens(text, &mut tokens);
                    }
                    for op in &component.ops {
                        collect_tokens(op, &mut tokens);
                    }
                }
                if !tokens.is_empty() {
                    iface_tokens.insert(iface, tokens);
                }
            }
        }

        // Varp popularity: for every interface, walk its bounded bound-script
        // closure (same depth + call map the ranking uses) and tally how many
        // distinct interfaces read each player varp. A varp read by many interfaces
        // is generic UI chrome; one read by few is feature-specific. Computed once
        // here with a local closure walk over `calls` (the `bound_closure` method is
        // not yet available — `Self` is still being constructed).
        let walk = |bound: &BTreeSet<u32>| -> BTreeSet<u32> {
            let mut seen = BTreeSet::new();
            let mut queue: VecDeque<(u32, u32)> = bound.iter().map(|&s| (s, 0)).collect();
            while let Some((script, depth)) = queue.pop_front() {
                if !seen.insert(script) {
                    continue;
                }
                if depth < CLOSURE_DEPTH
                    && let Some(callees) = calls.get(&script)
                {
                    for &callee in callees {
                        queue.push_back((callee, depth + 1));
                    }
                }
            }
            seen
        };
        let mut varp_readers: BTreeMap<u32, u32> = BTreeMap::new();
        for bound in iface_scripts.values() {
            let closure = walk(bound);
            let mut read: BTreeSet<u32> = BTreeSet::new();
            for script in &closure {
                if let Some(varps) = reads_varp.get(script) {
                    read.extend(varps.iter().copied());
                }
            }
            for varp in read {
                *varp_readers.entry(varp).or_default() += 1;
            }
        }

        Self {
            ctx,
            scripts,
            reads_varp,
            reads_varbit,
            calls,
            iface_scripts,
            iface_tokens,
            varp_readers,
            opens_interface,
        }
    }

    /// Resolve the queried loc to its multivar parent + variant table, then rank
    /// candidate interfaces for its options.
    fn explain(&self, queried: u32, max_candidates: usize) -> Result<ExplainedLoc> {
        let queried_ops = self.loc_ops(queried).ok_or_else(|| {
            crate::error::CacheError::message(format!("loc {queried} not found in cache"))
        })?;

        // Decide the parent: the queried loc if it carries a multivar, else the
        // parent that lists it as a child, else itself (a plain loc).
        let (parent_loc, parent_ops) = if multivar_of(&queried_ops).is_some() {
            (queried, queried_ops.clone())
        } else if let Some((pid, pops)) = self.find_parent_of(queried) {
            (pid, pops)
        } else {
            (queried, queried_ops.clone())
        };

        let multivar = multivar_of(&parent_ops);
        let (gating_varbit, gating_varp) = match &multivar {
            Some(MultiVar::VarBit(id)) => (Some(*id), None),
            Some(MultiVar::VarP(id)) => (None, Some(*id)),
            None => (None, None),
        };

        // Variant table.
        let variants = self.build_variants(&parent_ops, &queried_ops, parent_loc, queried);

        // Gating-varbit base varp + varp block (the feature's varp neighbourhood).
        let gating_base_varp =
            gating_varbit.and_then(|vb| self.ctx.varbits.get(&vb).and_then(|e| e.base_var));
        let gating_block = gating_base_varp
            .map(|base| self.varp_block(base))
            .unwrap_or_default();

        // Candidate interfaces.
        let candidate_interfaces = self.rank_candidates(
            &variants,
            gating_varbit,
            gating_varp,
            &gating_block,
            max_candidates,
        );

        // Server-binding detection. A loc's options never carry their own cache
        // open edge (the open is issued server-side on the OPLOC), so the only way a
        // candidate is cache-derivable is the High path: a clientscript opens it and
        // reads the gate. Absent any such candidate, the open is server-side and the
        // candidates below are heuristic only.
        let gate_script_readers = self.gate_reader_count(gating_varbit, gating_varp);
        let has_cache_edge = candidate_interfaces
            .iter()
            .any(|c| c.confidence == Confidence::High);
        let server_side_open = !has_cache_edge;
        // The banner caveats the candidate list, so only emit it when there is a
        // list to caveat (and no cache edge). A plain loc with no candidates needs
        // no "heuristic only" note — the empty-candidates line already says it all.
        let binding_note = (server_side_open && !candidate_interfaces.is_empty()).then(|| {
            server_binding_banner(
                gating_varbit,
                gating_varp,
                gate_script_readers,
                &candidate_interfaces,
            )
        });

        Ok(ExplainedLoc {
            queried_loc: queried,
            parent_loc,
            is_multivar: multivar.is_some(),
            gating_varbit,
            gating_varp,
            gating_base_varp,
            gating_block,
            server_side_open,
            gate_script_readers,
            binding_note,
            variants,
            candidate_interfaces,
        })
    }

    /// How many cache clientscripts read the gating varbit (or varp) directly. `0`
    /// is the strongest cache-side signal that the gate is purely server-driven.
    fn gate_reader_count(&self, gating_varbit: Option<u32>, gating_varp: Option<u32>) -> u32 {
        let mut count = 0u32;
        if let Some(vb) = gating_varbit {
            count += self
                .reads_varbit
                .values()
                .filter(|set| set.contains(&vb))
                .count() as u32;
        }
        if let Some(vp) = gating_varp {
            count += self
                .reads_varp
                .values()
                .filter(|set| set.contains(&vp))
                .count() as u32;
        }
        count
    }

    /// The ops list for a loc id, or `None` when absent.
    fn loc_ops(&self, id: u32) -> Option<Vec<String>> {
        self.ctx.locs.get(&id).map(|entry| entry.ops.clone())
    }

    /// Find the multivar parent that lists `child` in one of its `multiloc` slots.
    fn find_parent_of(&self, child: u32) -> Option<(u32, Vec<String>)> {
        for (&id, entry) in &self.ctx.locs {
            if multivar_of(&entry.ops).is_none() {
                continue;
            }
            for child_id in multi_children(&entry.ops) {
                if child_id == child {
                    return Some((id, entry.ops.clone()));
                }
            }
        }
        None
    }

    /// Build the `value → child loc → ops` variant table from the parent's
    /// `multiloc` slots. When the parent is not actually a multivar (a plain loc
    /// queried directly), emit a single self-variant so its ops still show.
    fn build_variants(
        &self,
        parent_ops: &[String],
        queried_ops: &[String],
        parent_loc: u32,
        queried: u32,
    ) -> Vec<Variant> {
        let slots = multi_slots(parent_ops);
        if slots.is_empty() {
            // Plain loc: one variant for the loc itself.
            return vec![self.variant_for(None, false, queried, queried_ops)];
        }
        let mut variants = Vec::with_capacity(slots.len());
        for slot in slots {
            let child_ops = self.loc_ops(slot.child).unwrap_or_default();
            variants.push(self.variant_for(slot.value, slot.is_default, slot.child, &child_ops));
        }
        let _ = parent_loc;
        variants
    }

    /// Project one child loc into a [`Variant`], extracting its op slots and any
    /// bound-clientscript open targets.
    fn variant_for(
        &self,
        value: Option<u32>,
        is_default: bool,
        child: u32,
        child_ops: &[String],
    ) -> Variant {
        let name = child_ops
            .iter()
            .find_map(|op| op.strip_prefix("name=").map(ToOwned::to_owned));
        let mut ops = Vec::new();
        for op in child_ops {
            let Some((slot, label)) = parse_op_slot(op) else {
                continue;
            };
            let (opens_interfaces, server_driven) = self.op_open_targets(child, &slot);
            ops.push(OpEntry {
                slot,
                label,
                opens_interfaces,
                server_driven,
            });
        }
        Variant {
            value,
            is_default,
            child_loc: child,
            name,
            ops,
        }
    }

    /// Cache-wired open targets for a loc op. Locs do not carry per-op clientscript
    /// bindings in the cache (the op→script→`IF_OPEN*` wiring lives on the server),
    /// so this is server-driven in practice; the hook is kept for completeness and
    /// returns any `IF_OPENTOP`/`IF_OPENSUB` targets a future bound script exposes.
    fn op_open_targets(&self, _loc: u32, _slot: &str) -> (Vec<u32>, bool) {
        (Vec::new(), true)
    }

    /// The gating var's feature varp window: every player varp used as a varbit
    /// base within [`BLOCK_WINDOW`] of `base`. A feature reserves a band of player
    /// varps for its state, so the gate's base anchors that band; intersecting the
    /// window with actual varbit-base varps keeps the reported set to real feature
    /// varps. Discrimination from neighbouring features is provided by the op/name
    /// text overlap requirement in [`Self::rank_candidates`].
    fn varp_block(&self, base: u32) -> Vec<u32> {
        let lo = base.saturating_sub(BLOCK_WINDOW);
        let hi = base.saturating_add(BLOCK_WINDOW);
        let mut block: BTreeSet<u32> = self
            .ctx
            .varbits
            .values()
            .filter(|e| e.domain == Some(VarDomain::Player))
            .filter_map(|e| e.base_var)
            .filter(|&v| v >= lo && v <= hi)
            .collect();
        block.insert(base);
        block.into_iter().collect()
    }

    /// Score and rank candidate interfaces for the loc's options.
    fn rank_candidates(
        &self,
        variants: &[Variant],
        gating_varbit: Option<u32>,
        gating_varp: Option<u32>,
        gating_block: &[u32],
        max_candidates: usize,
    ) -> Vec<CandidateInterface> {
        // Signal tokens from every variant's op labels + names.
        let mut signal: BTreeSet<String> = BTreeSet::new();
        for variant in variants {
            if let Some(name) = &variant.name {
                collect_tokens(name, &mut signal);
            }
            for op in &variant.ops {
                collect_tokens(&op.label, &mut signal);
            }
        }

        // The gating block: the varbit feature window, plus the gate's own varp
        // when the multivar switches on a varp directly (a single-varp "block").
        let mut block_set: BTreeSet<u32> = gating_block.iter().copied().collect();
        if let Some(varp) = gating_varp {
            block_set.insert(varp);
        }

        // Seed candidates: any interface sharing a signal token, plus any interface
        // whose bound closure reads the gate directly (belt and braces — the relic
        // gate has no script readers, but a future loc's gate might).
        let mut candidate_ids: BTreeSet<u32> = BTreeSet::new();
        for (&iface, tokens) in &self.iface_tokens {
            if signal.iter().any(|t| tokens.contains(t)) {
                candidate_ids.insert(iface);
            }
        }
        for (&iface, bound) in &self.iface_scripts {
            let closure = self.bound_closure(bound);
            let reads_gate_varbit = gating_varbit.is_some_and(|vb| {
                closure
                    .iter()
                    .any(|s| self.reads_varbit.get(s).is_some_and(|v| v.contains(&vb)))
            });
            let reads_gate_varp = gating_varp.is_some_and(|vp| {
                closure
                    .iter()
                    .any(|s| self.reads_varp.get(s).is_some_and(|v| v.contains(&vp)))
            });
            if reads_gate_varbit || reads_gate_varp {
                candidate_ids.insert(iface);
            }
        }

        // A broad block carries a generic tail shared by a whole UI family, so block
        // reads alone are weak evidence and must be specificity-weighted.
        let broad_block = block_set.len() > BROAD_BLOCK_THRESHOLD;

        let mut candidates = Vec::new();
        for iface in candidate_ids {
            let tokens = self.iface_tokens.get(&iface);
            let matched: Vec<String> = tokens
                .map(|t| signal.iter().filter(|s| t.contains(*s)).cloned().collect())
                .unwrap_or_default();

            // Bounded bound-script closure → varps read.
            let closure = self
                .iface_scripts
                .get(&iface)
                .map(|bound| self.bound_closure(bound))
                .unwrap_or_default();
            let read_varps: BTreeSet<u32> = closure
                .iter()
                .filter_map(|s| self.reads_varp.get(s))
                .flatten()
                .copied()
                .collect();

            let gating_block_varps: Vec<u32> =
                block_set.iter().filter(|v| read_varps.contains(v)).copied().collect();
            // Related varps: those read by a closure script that ALSO reads a block
            // varp (same feature reader), but which sit outside the contiguous block
            // — e.g. a sibling unlock bitset packed far from the gate's base.
            let related_varps = self.related_feature_varps(&closure, &block_set, &read_varps);

            // Specificity-weighted block contribution. In a broad block, a block varp
            // read by more than GENERIC_VARP_READERS interfaces is generic chrome and
            // is discounted (weight / readers); feature-specific block varps keep full
            // weight. In a tight block every block read is full-weight (prior
            // behaviour), so the relic monolith's 691 is unchanged.
            let block_score: i64 = gating_block_varps
                .iter()
                .map(|&v| self.block_varp_weight(v, broad_block))
                .sum();
            // A broad-block match is "generic" when it reads block varps but none of
            // them is feature-specific (all are widely shared) — the combat / action-
            // bar false-positive pattern. Feature-specific reads (few readers) mark a
            // candidate as a real domain match, not generic chrome.
            let has_specific_block_varp = gating_block_varps
                .iter()
                .any(|&v| !self.is_generic_varp(v));
            let generic_block_match =
                broad_block && !gating_block_varps.is_empty() && !has_specific_block_varp;

            let score = matched.len() as i64 + block_score;

            // Skip pure noise (no token AND no gating-block read).
            if score == 0 {
                continue;
            }

            // Confidence: High only when a cache open edge ties this interface to the
            // gate — a clientscript in its closure opens it AND the closure reads the
            // gating var. Otherwise the open is server-side and this is a heuristic.
            let confidence = if self.has_cache_open_edge(iface, &closure, gating_varbit, gating_varp)
            {
                Confidence::High
            } else {
                Confidence::Low
            };

            let (enums, dbtables, title) = self.interface_summary(iface);

            candidates.push(CandidateInterface {
                interface: iface,
                confidence,
                generic_block_match,
                score,
                matched_tokens: matched,
                gating_block_varps,
                related_varps,
                enums,
                dbtables,
                title,
            });
        }

        // Rank: cache-edge (High) candidates first, then by score, then a generic
        // broad-block match loses to an equal-scoring specific one, then by id. This
        // keeps a real binding (when one exists) above every heuristic and never lets
        // a generic block match present as a more confident hit than a specific one.
        candidates.sort_by(|a, b| {
            confidence_rank(a.confidence)
                .cmp(&confidence_rank(b.confidence))
                .then(b.score.cmp(&a.score))
                .then(a.generic_block_match.cmp(&b.generic_block_match))
                .then(b.gating_block_varps.len().cmp(&a.gating_block_varps.len()))
                .then(a.interface.cmp(&b.interface))
        });
        candidates.truncate(max_candidates);
        candidates
    }

    /// Score contribution of one gating-block varp for a candidate. Full
    /// [`BLOCK_VARP_WEIGHT`] for a feature-specific varp (or any varp in a tight
    /// block); discounted by reader count for a generic varp in a broad block.
    fn block_varp_weight(&self, varp: u32, broad_block: bool) -> i64 {
        if broad_block && self.is_generic_varp(varp) {
            let readers = i64::from(self.varp_readers.get(&varp).copied().unwrap_or(1).max(1));
            (BLOCK_VARP_WEIGHT / readers).max(1)
        } else {
            BLOCK_VARP_WEIGHT
        }
    }

    /// Whether a player varp is generic UI chrome (read by more than
    /// [`GENERIC_VARP_READERS`] interface closures) rather than feature-specific.
    fn is_generic_varp(&self, varp: u32) -> bool {
        self.varp_readers.get(&varp).copied().unwrap_or(0) > GENERIC_VARP_READERS as u32
    }

    /// A real cache open edge: some script in the interface's bound closure opens
    /// THIS interface via [`OPEN_INTERFACE_OPCODE`], and the same closure reads the
    /// gating var (so the cache, not the server, ties the gate to this open).
    fn has_cache_open_edge(
        &self,
        iface: u32,
        closure: &BTreeSet<u32>,
        gating_varbit: Option<u32>,
        gating_varp: Option<u32>,
    ) -> bool {
        let opens_this = closure
            .iter()
            .any(|s| self.opens_interface.get(s).is_some_and(|set| set.contains(&iface)));
        if !opens_this {
            return false;
        }
        // The same closure must also read the gate, so the cache (not the server)
        // ties this gated open to the gating var.
        gating_varbit.is_some_and(|vb| {
            closure
                .iter()
                .any(|s| self.reads_varbit.get(s).is_some_and(|v| v.contains(&vb)))
        }) || gating_varp.is_some_and(|vp| {
            closure
                .iter()
                .any(|s| self.reads_varp.get(s).is_some_and(|v| v.contains(&vp)))
        })
    }

    /// Varps read by a closure script that also reads a gating-block varp, minus the
    /// block itself — the rest of the feature's varp set (e.g. unlock bitsets that
    /// live outside the contiguous base run).
    fn related_feature_varps(
        &self,
        closure: &BTreeSet<u32>,
        block: &BTreeSet<u32>,
        read_varps: &BTreeSet<u32>,
    ) -> Vec<u32> {
        if block.is_empty() {
            return Vec::new();
        }
        // Only varps read by a script that ALSO reads a block varp in the SAME
        // script (a feature state reader, e.g. the relic unlock testbit script that
        // reads 9312 and the sibling bitset 11743 together) — this keeps the set
        // feature-local instead of dragging in every varp a big window-reader
        // touches.
        let mut related = BTreeSet::new();
        for &script in closure {
            let Some(reads) = self.reads_varp.get(&script) else {
                continue;
            };
            let block_hits = reads.iter().filter(|v| block.contains(v)).count();
            // Require the script to be a focused feature reader (touches a block
            // varp but is not a broad multi-feature aggregator).
            if block_hits == 0 || reads.len() > 8 {
                continue;
            }
            for &v in reads {
                if !block.contains(&v) && read_varps.contains(&v) {
                    related.insert(v);
                }
            }
        }
        related.into_iter().take(8).collect()
    }

    /// The bounded ([`CLOSURE_DEPTH`]) call-closure of a set of bound scripts.
    fn bound_closure(&self, bound: &BTreeSet<u32>) -> BTreeSet<u32> {
        let mut seen = BTreeSet::new();
        let mut queue: VecDeque<(u32, u32)> = bound.iter().map(|&s| (s, 0)).collect();
        while let Some((script, depth)) = queue.pop_front() {
            if !seen.insert(script) {
                continue;
            }
            if depth < CLOSURE_DEPTH
                && let Some(callees) = self.calls.get(&script)
            {
                for &callee in callees {
                    queue.push_back((callee, depth + 1));
                }
            }
        }
        seen
    }

    /// `(enums, dbtables, title)` for an interface's `explain-interface` closure +
    /// the dbtable ids its bound-script closure reads.
    fn interface_summary(&self, iface: u32) -> (Vec<u32>, Vec<u32>, Option<String>) {
        let explained = self
            .ctx
            .interfaces
            .get(&iface)
            .and_then(|files| explain_interface_group(iface, files, self.ctx.build).ok());
        let enums = explained
            .as_ref()
            .map(|e| e.requires.enums.iter().copied().collect())
            .unwrap_or_default();
        let title = explained.as_ref().and_then(pick_title);
        let dbtables = self
            .iface_scripts
            .get(&iface)
            .map(|bound| {
                let closure = self.bound_closure(bound);
                self.dbtables_in_closure(&closure)
            })
            .unwrap_or_default();
        (enums, dbtables, title)
    }

    /// DbTable ids referenced by `db_*` opcodes anywhere in a script closure.
    ///
    /// A `db_*` operand is one of two forms pushed just before the opcode: a raw
    /// table id (`db_find` / `db_listall`) or a **dbcolumn** constant
    /// `tableId << 12 | columnId` (`db_getfield` / `db_getfieldcount`). Both are
    /// `push_constant_*` literals, so we track the recent literals and, at each
    /// `db_*` site, accept any whose raw value or whose `>> 12` is a known dbtable.
    fn dbtables_in_closure(&self, closure: &BTreeSet<u32>) -> Vec<u32> {
        let mut tables = BTreeSet::new();
        for &script_id in closure {
            let Some(script) = self.scripts.get(&script_id) else {
                continue;
            };
            let mut literals: Vec<i32> = Vec::new();
            for instruction in &script.code {
                match &instruction.operand {
                    // dbcolumn/table constants ride either typed push (the string
                    // push carries an int payload for the packed column ref).
                    Operand::Int(value)
                        if matches!(
                            instruction.command.as_str(),
                            "push_constant_int" | "push_constant_string"
                        ) =>
                    {
                        literals.push(*value);
                    }
                    _ => {
                        if instruction.command.starts_with("db_") {
                            for &value in &literals {
                                if value <= 0 {
                                    continue;
                                }
                                let raw = value as u32;
                                let packed = raw >> 12;
                                if self.ctx.dbtables.contains_key(&raw) {
                                    tables.insert(raw);
                                }
                                if packed > 0 && self.ctx.dbtables.contains_key(&packed) {
                                    tables.insert(packed);
                                }
                            }
                        }
                        literals.clear();
                    }
                }
            }
        }
        tables.into_iter().collect()
    }
}

// ───────────────────────────── multivar helpers ─────────────────────────────

/// The gating var a multivar loc switches on.
enum MultiVar {
    VarBit(u32),
    VarP(u32),
}

/// One resolved multivar slot: `value → child`, or the default slot.
struct MultiSlot {
    value: Option<u32>,
    is_default: bool,
    child: u32,
}

/// The gating var of a loc's ops, if it is a multivar.
fn multivar_of(ops: &[String]) -> Option<MultiVar> {
    for op in ops {
        if let Some(rest) = op.strip_prefix("multivar=varbit:")
            && let Ok(id) = rest.parse()
        {
            return Some(MultiVar::VarBit(id));
        }
        if let Some(rest) = op.strip_prefix("multivar=varp:")
            && let Ok(id) = rest.parse()
        {
            return Some(MultiVar::VarP(id));
        }
    }
    None
}

/// All `multiloc` child slots of a loc's ops (value slots + default).
fn multi_slots(ops: &[String]) -> Vec<MultiSlot> {
    let mut slots = Vec::new();
    for op in ops {
        let Some(rest) = op.strip_prefix("multiloc=") else {
            continue;
        };
        let Some((key, child)) = rest.split_once(',') else {
            continue;
        };
        let Ok(child) = child.parse::<u32>() else {
            continue;
        };
        if key == "default" {
            slots.push(MultiSlot {
                value: None,
                is_default: true,
                child,
            });
        } else if let Ok(value) = key.parse::<u32>() {
            slots.push(MultiSlot {
                value: Some(value),
                is_default: false,
                child,
            });
        }
    }
    slots
}

/// Just the child loc ids of a multivar loc (any slot).
fn multi_children(ops: &[String]) -> Vec<u32> {
    multi_slots(ops).into_iter().map(|s| s.child).collect()
}

/// Parse an op line into `(slot, label)` for the 10 loc op slots, or `None`.
fn parse_op_slot(op: &str) -> Option<(String, String)> {
    let (key, value) = op.split_once('=')?;
    let is_op = (key.starts_with("op") && key[2..].parse::<u32>().is_ok())
        || (key.starts_with("membersop") && key[9..].parse::<u32>().is_ok());
    if is_op {
        Some((key.to_string(), value.to_string()))
    } else {
        None
    }
}

// ───────────────────────────── token helpers ─────────────────────────────

/// Split a label into lowercase alphanumeric tokens of at least [`MIN_TOKEN_LEN`]
/// characters, dropping stopwords, into `out`.
fn collect_tokens(text: &str, out: &mut BTreeSet<String>) {
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.len() < MIN_TOKEN_LEN {
            continue;
        }
        let token = raw.to_ascii_lowercase();
        if STOPWORDS.contains(&token.as_str()) {
            continue;
        }
        out.insert(token);
    }
}

/// Pick a human title for a candidate: the longest static text of a `text`
/// component, capped, falling back to `None`.
fn pick_title(explained: &crate::interface::component::ExplainedInterface) -> Option<String> {
    explained
        .components
        .iter()
        .filter_map(|c| c.text.as_ref())
        .map(|t| t.trim())
        .filter(|t| t.len() >= MIN_TOKEN_LEN)
        .max_by_key(|t| t.len())
        .map(|t| {
            let cleaned = t.replace('\n', " ");
            if cleaned.chars().count() > 48 {
                let short: String = cleaned.chars().take(47).collect();
                format!("{short}…")
            } else {
                cleaned
            }
        })
}

/// Sort key for confidence (lower sorts first): High before Low.
fn confidence_rank(confidence: Confidence) -> u8 {
    match confidence {
        Confidence::High => 0,
        Confidence::Low => 1,
    }
}

/// Build the server-side / no-cache-edge banner. Names the gate, its (lack of)
/// cache readers, and what the candidates below are worth.
fn server_binding_banner(
    gating_varbit: Option<u32>,
    gating_varp: Option<u32>,
    gate_script_readers: u32,
    candidates: &[CandidateInterface],
) -> String {
    let gate = if let Some(vb) = gating_varbit {
        format!("gating varbit {vb}")
    } else if let Some(vp) = gating_varp {
        format!("gating varp {vp}")
    } else {
        "this loc".to_string()
    };
    let readers = match gate_script_readers {
        0 => format!("{gate} has no clientscript readers"),
        1 => format!("{gate} has 1 clientscript reader, none opening an interface"),
        n => format!("{gate} has {n} clientscript readers, none opening an interface"),
    };
    let any_generic = candidates.iter().any(|c| c.generic_block_match);
    let tail = if any_generic {
        "candidates below are heuristic domain matches only (some are generic \
         broad-block matches) — verify in the server loc handler"
    } else {
        "candidates below are heuristic domain matches only — verify in the server \
         loc handler"
    };
    format!("server-side open: no cache binding ({readers}); {tail}.")
}

// ───────────────────────────── human render ─────────────────────────────

/// Render the human report.
#[must_use]
pub fn render_human(explained: &ExplainedLoc) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "loc {} — interaction tracer", explained.queried_loc);
    if explained.is_multivar {
        let _ = writeln!(
            out,
            "  multivar parent {} switches on {}",
            explained.parent_loc,
            gate_label(explained)
        );
        if let Some(base) = explained.gating_base_varp {
            let _ = write!(out, "  gating varbit base varp {base}");
            if !explained.gating_block.is_empty() {
                let _ = write!(
                    out,
                    " — feature varp block {}..{} ({} varps)",
                    explained.gating_block.first().copied().unwrap_or(base),
                    explained.gating_block.last().copied().unwrap_or(base),
                    explained.gating_block.len()
                );
            }
            let _ = writeln!(out);
        }
    } else if explained.parent_loc != explained.queried_loc {
        let _ = writeln!(
            out,
            "  child of multivar parent {} (gate {})",
            explained.parent_loc,
            gate_label(explained)
        );
    } else {
        let _ = writeln!(out, "  (not a multivar loc)");
    }

    if let Some(note) = &explained.binding_note {
        let _ = writeln!(out, "  ⚠ {note}");
    }

    out.push_str("variants:\n");
    for variant in &explained.variants {
        let value = match (variant.is_default, variant.value) {
            (true, _) => "default".to_string(),
            (false, Some(v)) => v.to_string(),
            (false, None) => "-".to_string(),
        };
        let name = variant.name.as_deref().unwrap_or("");
        let _ = writeln!(
            out,
            "  value {value:>7} → loc {} {}",
            variant.child_loc,
            if name.is_empty() {
                String::new()
            } else {
                format!("\"{name}\"")
            }
        );
        for op in &variant.ops {
            let edge = if !op.opens_interfaces.is_empty() {
                format!(
                    " → opens {}",
                    op.opens_interfaces
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else if op.server_driven {
                " (server-driven open)".to_string()
            } else {
                String::new()
            };
            let _ = writeln!(out, "      {:<11} {}{edge}", op.slot, op.label);
        }
    }

    out.push_str("candidate interfaces (best first):\n");
    if explained.candidate_interfaces.is_empty() {
        out.push_str("  (none — no cache signal links this loc to an interface)\n");
    }
    for (rank, candidate) in explained.candidate_interfaces.iter().enumerate() {
        let title = candidate
            .title
            .as_deref()
            .map(|t| format!(" \"{t}\""))
            .unwrap_or_default();
        let label = match candidate.confidence {
            Confidence::High => "high",
            Confidence::Low => "low",
        };
        let kind = if candidate.generic_block_match {
            ", generic block match"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  #{} [{label}] interface {} (score {}{kind}){title}",
            rank + 1,
            candidate.interface,
            candidate.score
        );
        if !candidate.matched_tokens.is_empty() {
            let _ = writeln!(out, "       tokens: {}", candidate.matched_tokens.join(", "));
        }
        if !candidate.gating_block_varps.is_empty() {
            let _ = writeln!(
                out,
                "       reads gating-block varps: {}",
                join_ids(&candidate.gating_block_varps)
            );
        }
        if !candidate.related_varps.is_empty() {
            let _ = writeln!(
                out,
                "       related feature varps: {}",
                join_ids(&candidate.related_varps)
            );
        }
        if !candidate.dbtables.is_empty() {
            let _ = writeln!(out, "       dbtables: {}", join_ids(&candidate.dbtables));
        }
        if !candidate.enums.is_empty() {
            let _ = writeln!(out, "       enums: {}", join_ids(&candidate.enums));
        }
    }

    out
}

/// Label for the gating var (varbit/varp/none).
fn gate_label(explained: &ExplainedLoc) -> String {
    if let Some(vb) = explained.gating_varbit {
        format!("varbit {vb}")
    } else if let Some(vp) = explained.gating_varp {
        format!("varp {vp}")
    } else {
        "(no gate)".to_string()
    }
}

/// Comma-join ids.
fn join_ids(ids: &[u32]) -> String {
    ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multivar_parse_and_slots() {
        let ops = vec![
            "multivar=varbit:49357".to_string(),
            "multiloc=0,115415".to_string(),
            "multiloc=1,116440".to_string(),
            "multiloc=2,119870".to_string(),
        ];
        match multivar_of(&ops) {
            Some(MultiVar::VarBit(id)) => assert_eq!(id, 49357),
            _ => panic!("expected varbit gate"),
        }
        let slots = multi_slots(&ops);
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[1].value, Some(1));
        assert_eq!(slots[1].child, 116_440);
        assert_eq!(multi_children(&ops), vec![115_415, 116_440, 119_870]);
    }

    #[test]
    fn op_slot_and_tokens() {
        assert_eq!(
            parse_op_slot("op1=Manage powers"),
            Some(("op1".to_string(), "Manage powers".to_string()))
        );
        assert_eq!(
            parse_op_slot("membersop2=Offer relic"),
            Some(("membersop2".to_string(), "Offer relic".to_string()))
        );
        assert!(parse_op_slot("name=Mysterious monolith").is_none());

        let mut tokens = BTreeSet::new();
        collect_tokens("Manage powers", &mut tokens);
        collect_tokens("Offer relic", &mut tokens);
        // 4+ char, non-stopword tokens only.
        assert!(tokens.contains("manage"));
        assert!(tokens.contains("powers"));
        assert!(tokens.contains("relic"));
        assert!(tokens.contains("offer"));
    }

    #[test]
    fn confidence_rank_orders_high_before_low() {
        assert!(confidence_rank(Confidence::High) < confidence_rank(Confidence::Low));
    }

    #[test]
    fn banner_names_gate_readers_and_heuristic() {
        // Zero readers (the monolith gate): banner says so and flags heuristic-only.
        let none = server_binding_banner(Some(49357), None, 0, &[]);
        assert!(none.contains("gating varbit 49357 has no clientscript readers"), "{none}");
        assert!(none.contains("no cache binding") && none.contains("heuristic"), "{none}");

        // A reader that opens nothing (the ritual gate, 1 reader) is still reported,
        // and the generic-match phrasing kicks in when a candidate is generic.
        let generic = CandidateInterface {
            interface: 1319,
            confidence: Confidence::Low,
            generic_block_match: true,
            score: 253,
            matched_tokens: vec![],
            gating_block_varps: vec![],
            related_varps: vec![],
            enums: vec![],
            dbtables: vec![],
            title: None,
        };
        let one = server_binding_banner(Some(53898), None, 1, std::slice::from_ref(&generic));
        assert!(
            one.contains("1 clientscript reader, none opening an interface"),
            "{one}"
        );
        assert!(one.contains("generic broad-block matches"), "{one}");
    }
}
