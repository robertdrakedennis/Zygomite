//! Regression-lock for `explain-loc`: the Mysterious Monolith relic chain.
//!
//! Cross-checked against the committed relic truth chain in
//! `server/cache-patches/relic-system-948/README.md` (NEVER edited here — it is the
//! oracle the tool must reproduce):
//!
//!   * Loc multivar parent **115416** switches on varbit **49357**:
//!     `0 → 115415 Interact`, `1 → 116440 Manage powers / Offer relic`,
//!     `2 → 119870`. OPLOC sends the PARENT id.
//!   * The option opens interface **691** (server-side `IF_OPENTOP`), whose
//!     onload/refresh script closure reads the relic unlock varps **9312/11743**
//!     (script 14603 `testbit`) and dbtable **94**.
//!
//! `explain-loc` cannot read the server-side open as a cache edge, so it
//! reverse-matches 691 by the gating varbit's feature varp window plus the loc's
//! op/name text overlap. This test pins: the multivar → child 116440 resolution +
//! the gating varbit 49357, and 691 as the TOP candidate carrying the 9312/11743 +
//! dbtable-94 evidence.
//!
//! Server-binding detection (plan 009): because no clientscript opens any of these
//! interfaces (the open is issued server-side on the OPLOC), every candidate is a
//! heuristic only — `explain-loc` must say so. So this suite also pins:
//!   * Both the monolith (115416) and the ritual pedestal (127375) are flagged
//!     `server_side_open` with a "no cache binding" banner.
//!   * Every candidate carries an honest confidence label — `low` here, since there
//!     is no cache open edge. 691 stays the monolith's TOP candidate *with* that
//!     honest label.
//!   * The ritual pedestal's 89-varp block surfaces only broad-block combat false
//!     positives (1319, 1430, …): they are flagged `low` + `generic_block_match`,
//!     and are NOT presented as a confident top hit.
//!
//! Runs against the donor 948 flat cache (where the relic scripts + interface 691
//! live); skips cleanly when that cache is absent (clean CI checkout).

use std::path::{Path, PathBuf};

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::explain_loc::{Confidence, ExplainLocOptions, ExplainedLoc, explain};

/// The donor 948 cache holds the Archaeology relic system (910 predates it).
const DONOR_948_CACHE: &str = "/Users/robert/projects/alerion/cache/unpacked/948";

/// Relic truth-chain ids (relic-system-948/README.md).
const MONOLITH_PARENT: u32 = 115416;
const MONOLITH_CHILD_MANAGE: u32 = 116440;
const GATING_VARBIT: u32 = 49357;
const RELIC_INTERFACE: u32 = 691;
const RELIC_UNLOCK_VARP_A: u32 = 9312;
const RELIC_UNLOCK_VARP_B: u32 = 11743;
const RELIC_DBTABLE: u32 = 94;

/// Ritual pedestal (Necromancy) multivar parent — the plan-009 road-test loc whose
/// open is pure server logic: an 89-varp gating block surfaces only broad-block
/// combat false positives. Its gating varbit is 53898.
const RITUAL_PEDESTAL_PARENT: u32 = 127375;
const RITUAL_GATING_VARBIT: u32 = 53898;
/// One of the action-bar / combat interfaces the broad block falsely surfaces (the
/// score-4201 #1 before plan 009). Used to assert it is now flagged low + generic.
const RITUAL_FALSE_POSITIVE_IFACE: u32 = 1319;

/// Resolve the 948 donor cache, or `None` (with a skip notice) when it is absent.
fn require_948() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RS3_DONOR_948_CACHE") {
        let dir = PathBuf::from(path);
        return dir.join("255").is_dir().then_some(dir);
    }
    let dir = PathBuf::from(DONOR_948_CACHE);
    if dir.join("255").is_dir() {
        Some(dir)
    } else {
        eprintln!("SKIP (no donor 948 cache at {})", dir.display());
        None
    }
}

/// Run `explain-loc` for a loc id against the 948 cache.
fn explain_loc_948(loc: u32) -> Option<ExplainedLoc> {
    let dir = require_948()?;
    let cache = FlatCache::open(&dir).expect("open donor 948 cache");
    // Every archive is already unpacked on disk, so the tar is never read; pass a
    // path inside the cache dir to satisfy the signature.
    let tar = dir.join("__unused_for_unpacked_cache.tar");
    let data_dir = Path::new("data");
    let explained = explain(
        &cache,
        &tar,
        data_dir,
        &ExplainLocOptions {
            loc,
            build: 948,
            subbuild: 1,
            max_candidates: 12,
            json: false,
        },
    )
    .expect("explain-loc must succeed against the 948 cache");
    Some(explained)
}

/// The parent loc resolves its multivar → child 116440 at value 1, on varbit 49357.
#[test]
fn monolith_parent_resolves_multivar_to_child_116440() {
    let Some(explained) = explain_loc_948(MONOLITH_PARENT) else {
        return;
    };

    assert!(explained.is_multivar, "115416 must be a multivar parent");
    assert_eq!(explained.parent_loc, MONOLITH_PARENT);
    assert_eq!(
        explained.gating_varbit,
        Some(GATING_VARBIT),
        "the monolith must gate on varbit 49357"
    );

    // value 1 → child 116440 with the two relic ops.
    let value_one = explained
        .variants
        .iter()
        .find(|v| v.value == Some(1))
        .expect("variant table must have a value-1 slot");
    assert_eq!(
        value_one.child_loc, MONOLITH_CHILD_MANAGE,
        "value 1 must select child loc 116440"
    );
    let op_labels: Vec<&str> = value_one.ops.iter().map(|o| o.label.as_str()).collect();
    assert!(
        op_labels.contains(&"Manage powers"),
        "child 116440 must expose op 'Manage powers'; got {op_labels:?}"
    );
    assert!(
        op_labels.contains(&"Offer relic"),
        "child 116440 must expose op 'Offer relic'; got {op_labels:?}"
    );
}

/// The gating varbit's base varp anchors the relic feature window, and the loc's
/// options are server-driven (no cache clientscript binds the OPLOC).
#[test]
fn monolith_gate_has_base_varp_and_server_driven_ops() {
    let Some(explained) = explain_loc_948(MONOLITH_PARENT) else {
        return;
    };
    assert!(
        explained.gating_base_varp.is_some(),
        "varbit 49357 must resolve to a base varp"
    );
    assert!(
        !explained.gating_block.is_empty(),
        "the gating varbit must yield a feature varp window"
    );
    // The relic opens are issued by the server, so every op is reported as such.
    let all_server_driven = explained
        .variants
        .iter()
        .flat_map(|v| &v.ops)
        .all(|op| op.server_driven && op.opens_interfaces.is_empty());
    assert!(
        all_server_driven,
        "loc ops carry no cache open edge — they must be reported server-driven"
    );
}

/// Interface 691 is the TOP candidate, and its summary carries the relic evidence:
/// it reads relic varps 9312/11743 and dbtable 94.
#[test]
fn monolith_top_candidate_is_interface_691_with_relic_evidence() {
    let Some(explained) = explain_loc_948(MONOLITH_PARENT) else {
        return;
    };

    let top = explained
        .candidate_interfaces
        .first()
        .expect("there must be at least one candidate interface");
    assert_eq!(
        top.interface, RELIC_INTERFACE,
        "interface 691 must be the top candidate; full ranking: {:?}",
        explained
            .candidate_interfaces
            .iter()
            .map(|c| (c.interface, c.score))
            .collect::<Vec<_>>()
    );

    // 691 reads relic varp 9312 in its gating-block reads, and 11743 either in the
    // block or as a related feature varp (it sits just outside the contiguous run).
    assert!(
        top.gating_block_varps.contains(&RELIC_UNLOCK_VARP_A),
        "691 must read relic unlock varp 9312 in the gating block; got {:?}",
        top.gating_block_varps
    );
    let reads_b = top.gating_block_varps.contains(&RELIC_UNLOCK_VARP_B)
        || top.related_varps.contains(&RELIC_UNLOCK_VARP_B);
    assert!(
        reads_b,
        "691 must read relic unlock varp 11743; block={:?} related={:?}",
        top.gating_block_varps, top.related_varps
    );

    // dbtable 94 (the relic powers table) must appear in 691's closure summary.
    assert!(
        top.dbtables.contains(&RELIC_DBTABLE),
        "691's summary must surface dbtable 94; got {:?}",
        top.dbtables
    );

    // Honest confidence: no clientscript opens 691 (the open is server-side), so the
    // top candidate is a heuristic — it must be labelled `low`, not presented as a
    // confident cache-derived answer. It reads relic-SPECIFIC varps, so it is not a
    // generic broad-block match (unlike the ritual combat false positives below).
    assert_eq!(
        top.confidence,
        Confidence::Low,
        "691 has no cache open edge — it must carry an honest `low` confidence label"
    );
    assert!(
        !top.generic_block_match,
        "691 reads feature-specific relic varps — it is not a generic block match"
    );
}

/// Querying the child loc 116440 directly resolves up to the same parent + gate and
/// lists the two relic ops, surfacing 691 as the top candidate.
#[test]
fn querying_child_116440_resolves_parent_and_ops() {
    let Some(explained) = explain_loc_948(MONOLITH_CHILD_MANAGE) else {
        return;
    };
    assert_eq!(
        explained.parent_loc, MONOLITH_PARENT,
        "child 116440 must resolve to multivar parent 115416"
    );
    assert_eq!(explained.gating_varbit, Some(GATING_VARBIT));

    // The child's own ops show in the variant table.
    let child_ops: Vec<&str> = explained
        .variants
        .iter()
        .find(|v| v.child_loc == MONOLITH_CHILD_MANAGE)
        .expect("child variant present")
        .ops
        .iter()
        .map(|o| o.label.as_str())
        .collect();
    assert!(child_ops.contains(&"Manage powers"));
    assert!(child_ops.contains(&"Offer relic"));

    assert_eq!(
        explained.candidate_interfaces.first().map(|c| c.interface),
        Some(RELIC_INTERFACE),
        "child query must still rank 691 first"
    );
}

/// The monolith open is server-side (no clientscript opens 691, and the gating
/// varbit 49357 has zero clientscript readers), so `explain-loc` must mark it
/// `server_side_open` with a "no cache binding" banner and zero gate readers.
#[test]
fn monolith_flagged_server_side_open_with_banner() {
    let Some(explained) = explain_loc_948(MONOLITH_PARENT) else {
        return;
    };
    assert!(
        explained.server_side_open,
        "the monolith open is server-side — no candidate has a cache open edge"
    );
    assert_eq!(
        explained.gate_script_readers, 0,
        "gating varbit 49357 has no clientscript readers"
    );
    let banner = explained
        .binding_note
        .as_deref()
        .expect("server-side case must carry a binding banner");
    assert!(
        banner.contains("no cache binding") && banner.contains("heuristic"),
        "banner must name the no-cache-binding / heuristic-only case; got {banner:?}"
    );
}

/// The ritual pedestal (plan-009 road-test loc) resolves its multivar on varbit
/// 53898 and is flagged server-side: its 89-varp gating block surfaces only
/// broad-block combat false positives, which must be down-ranked and labelled.
#[test]
fn ritual_pedestal_server_side_and_false_positives_demoted() {
    let Some(explained) = explain_loc_948(RITUAL_PEDESTAL_PARENT) else {
        return;
    };

    assert!(explained.is_multivar, "127375 must be a multivar parent");
    assert_eq!(
        explained.gating_varbit,
        Some(RITUAL_GATING_VARBIT),
        "the ritual pedestal must gate on varbit 53898"
    );

    // Server-side: no clientscript opens the ritual UI, so the banner fires and the
    // block is broad enough that the surfaced candidates are generic.
    assert!(
        explained.server_side_open,
        "the ritual pedestal open is server-side — no cache open edge exists"
    );
    let banner = explained
        .binding_note
        .as_deref()
        .expect("ritual case must carry a binding banner");
    assert!(
        banner.contains("no cache binding"),
        "ritual banner must flag the no-cache-binding case; got {banner:?}"
    );

    // The block must be broad (the false-positive trigger): this is the 89-varp
    // reservation the road-test exposed.
    assert!(
        explained.gating_block.len() > 30,
        "the ritual gating block must be broad (>30 varps); got {}",
        explained.gating_block.len()
    );

    // Every candidate is a heuristic — none may be high confidence, since no cache
    // open edge exists. In particular, no candidate is a confident top hit.
    assert!(
        explained
            .candidate_interfaces
            .iter()
            .all(|c| c.confidence == Confidence::Low),
        "no ritual candidate may be high confidence (the open is server-side)"
    );

    // The combat / action-bar false positive (interface 1319 — the score-4201 #1
    // before plan 009) must now be flagged low AND a generic broad-block match.
    let fp = explained
        .candidate_interfaces
        .iter()
        .find(|c| c.interface == RITUAL_FALSE_POSITIVE_IFACE)
        .expect("interface 1319 must still be surfaced as a (now-flagged) candidate");
    assert_eq!(
        fp.confidence,
        Confidence::Low,
        "the combat false positive must be flagged low confidence"
    );
    assert!(
        fp.generic_block_match,
        "the combat false positive must be flagged a generic broad-block match"
    );

    // The plan's core regression: it must NOT look like the confident score-4201 #1
    // it used to be. The specificity discount must have collapsed its score well
    // below the old flat block bonus.
    assert!(
        fp.score < 4201,
        "the combat false positive's score must be specificity-discounted, \
         not the old flat 4201; got {}",
        fp.score
    );

    // Whatever ranks #1 is itself a low-confidence generic block match here (there is
    // no real answer to surface), so the tool is not presenting a confident hit.
    let top = explained
        .candidate_interfaces
        .first()
        .expect("there is at least one (heuristic) candidate");
    assert_eq!(
        top.confidence,
        Confidence::Low,
        "the ritual #1 must be honestly low confidence, not a confident answer"
    );
}
