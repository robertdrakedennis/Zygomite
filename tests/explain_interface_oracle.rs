//! Regression-lock for `explain-interface`: the interface-691 dependency closure
//! the tool computes must reproduce the relic-system-948 overlay's font / sprite
//! / script splice sets exactly.
//!
//! Oracle (NEVER edit — committed regression artifacts the tool must reproduce):
//!   * `server/cache-patches/relic-system-948/interfaces/691.dat`
//!     — the patched relic interface group (a JS5 raw group: container + 2-byte
//!       version trailer), embedded with `include_bytes!` so the test runs with
//!       no external cache/pack dependency.
//!   * The `RELIC_SYSTEM_948_{FONT,SPRITE,SCRIPT}_GROUP_PATCHES` lists in
//!     `server/src/lostcity/tools/cacheoverlay/CacheOverlay.ts` — the ids the
//!     overlay splices in because interface 691 references them. Their relevant
//!     members are mirrored below as the contract.
//!
//! The closure is decoded WITHOUT an archive index via
//! `decode_interface_group_raw` (file count inferred from the self-consistent
//! chunk footer), exercising the exact path `explain-interface --raw-dat` uses.

use rs3_cache_rs::constants::BUILD;
use rs3_cache_rs::interface::component::decode_interface_group_raw;

/// The committed relic interface-691 raw group.
const INTERFACE_691_DAT: &[u8] =
    include_bytes!("../../../server/cache-patches/relic-system-948/interfaces/691.dat");

/// `RELIC_SYSTEM_948_FONT_GROUP_PATCHES` group ids — the 8 fonts interface 691
/// references (the plan's validation target).
const RELIC_FONT_GROUPS: [u32; 8] = [26, 28, 29, 32, 56, 57, 206, 207];

/// The members of `RELIC_SYSTEM_948_SPRITE_GROUP_PATCHES` that interface 691
/// references (i.e. were spliced because 691 needs them). 23835 is a base-cache
/// sprite that did not need patching, so it is not in the patch list.
const RELIC_SPRITE_GROUPS_USED_BY_691: [u32; 4] = [10270, 10314, 10316, 10319];

/// The members of `RELIC_SYSTEM_948_SCRIPT_PATCHES` that interface 691's
/// component hooks call (the donor-authored relic scripts spliced for 691).
const RELIC_SCRIPTS_USED_BY_691: [u32; 9] = [
    14459, 14847, 14848, 14849, 14854, 14859, 14867, 19821, 19822,
];

/// Decode the embedded relic 691 group and return its closure.
fn closure() -> rs3_cache_rs::interface::component::InterfaceClosure {
    let group = decode_interface_group_raw(INTERFACE_691_DAT, BUILD)
        .expect("decode relic interface-691 raw group");
    group
        .explain(691)
        .expect("explain interface 691 closure")
        .requires
}

/// The closure's font set must be EXACTLY the relic font-patch groups — the
/// plan's hard requirement (26/28/29/32/56/57/206/207).
#[test]
fn interface_691_requires_exactly_the_eight_relic_fonts() {
    let req = closure();
    let fonts: Vec<u32> = req.fonts.iter().copied().collect();
    assert_eq!(
        fonts, RELIC_FONT_GROUPS,
        "interface 691 font closure != RELIC_SYSTEM_948_FONT_GROUP_PATCHES groups"
    );
}

/// Every sprite the relic overlay spliced for interface 691 must appear in the
/// closure's sprite set (the closure may also include base-cache sprites).
#[test]
fn interface_691_requires_the_spliced_relic_sprites() {
    let req = closure();
    for id in RELIC_SPRITE_GROUPS_USED_BY_691 {
        assert!(
            req.sprites.contains(&id),
            "interface 691 sprite closure missing relic-spliced sprite {id}; got {:?}",
            req.sprites
        );
    }
    // The known base-cache sprite is referenced too (sanity on completeness).
    assert!(
        req.sprites.contains(&23835),
        "interface 691 sprite closure missing base sprite 23835; got {:?}",
        req.sprites
    );
}

/// Every relic script the overlay spliced for interface 691 must appear in the
/// closure's script set (the closure also includes base-cache script calls).
#[test]
fn interface_691_requires_the_spliced_relic_scripts() {
    let req = closure();
    for id in RELIC_SCRIPTS_USED_BY_691 {
        assert!(
            req.scripts.contains(&id),
            "interface 691 script closure missing relic-spliced script {id}; got {:?}",
            req.scripts
        );
    }
}

/// The indexless raw decode must recover the same component count the runtime
/// pack reports for group 691 (225 components), proving the file-count
/// inference from the chunk footer is correct.
#[test]
fn interface_691_raw_decode_recovers_all_components() {
    let group = decode_interface_group_raw(INTERFACE_691_DAT, BUILD)
        .expect("decode relic interface-691 raw group");
    assert_eq!(
        group.files().len(),
        225,
        "indexless raw decode recovered the wrong component count"
    );
    // Component ids are the dense range 0..225.
    let ids: Vec<u32> = group.files().keys().copied().collect();
    assert_eq!(ids.first(), Some(&0));
    assert_eq!(ids.last(), Some(&224));

    // The explain projection must surface the relic "Loadout N" text rows with
    // their font 26 — a concrete check that per-component projection works.
    let explained = group.explain(691).expect("explain");
    let loadout_text_rows = explained
        .components
        .iter()
        .filter(|c| c.text.as_deref() == Some("Loadout 1") && c.textfont == Some(26))
        .count();
    assert!(
        loadout_text_rows >= 1,
        "expected at least one 'Loadout 1' text component with font 26"
    );
}

// ─────────────────────────── transitive closure ───────────────────────────
//
// `explain-interface --transitive` walks the donor script call graph from the
// interface's depth-1 component-bound scripts and reports the FULL closure size
// plus the count MISSING from the 910 base (the splice burden). The two tests
// below lock (a) the splice-burden frontier semantics, hermetically, against the
// committed relic-691 depth-1 seed, and (b) the live numbers for 691 and 1224
// against the donor (948) cache + 910-base roster when those are present.

use rs3_cache_rs::explain_transitive::{BaseRoster, ScriptSource, transitive_script_closure};
use std::collections::BTreeSet;

/// The relic 691 depth-1 component-bound script set the tool decodes from the
/// committed `.dat` MUST contain every relic-spliced script — these are the seed
/// of the transitive walk. (This is the same set the depth-1 oracle locks, here
/// asserted as the closure seed.)
#[test]
fn interface_691_depth1_seed_contains_relic_scripts() {
    let req = closure();
    for id in RELIC_SCRIPTS_USED_BY_691 {
        assert!(
            req.scripts.contains(&id),
            "relic-691 transitive seed (depth-1 scripts) missing relic script {id}"
        );
    }
}

/// A synthetic donor graph: the 9 relic scripts are donor-new and each calls a
/// shared base script (1000) that in turn calls a deep base-only chain. The base
/// scripts must shield their subtree from the splice burden, so the burden is
/// EXACTLY the donor-new relic seed — proving the frontier rule on a relic-shaped
/// graph with no cache dependency.
#[test]
fn transitive_burden_stops_at_base_frontier_relic_shaped() {
    struct Graph;
    impl ScriptSource for Graph {
        fn resolve(&self, raw: i32) -> Option<u32> {
            if raw < 0 {
                return None;
            }
            let raw = raw as u32;
            // Every id in our universe is a valid group (single-file convention).
            let known: BTreeSet<u32> = RELIC_SCRIPTS_USED_BY_691
                .iter()
                .copied()
                .chain([1000, 1001, 1002])
                .collect();
            if known.contains(&raw) {
                Some(raw)
            } else {
                let g = raw >> 16;
                known.contains(&g).then_some(g)
            }
        }
        fn callees(&self, group: u32) -> Vec<i32> {
            // Each relic script calls the shared base hub 1000; the base hub runs
            // a base-only chain 1000 -> 1001 -> 1002.
            if RELIC_SCRIPTS_USED_BY_691.contains(&group) {
                vec![1000]
            } else if group == 1000 {
                vec![1001]
            } else if group == 1001 {
                vec![1002]
            } else {
                Vec::new()
            }
        }
    }
    struct Base;
    impl BaseRoster for Base {
        fn contains(&self, group: u32) -> bool {
            // The hub and its chain are 910-base; the relic scripts are donor-new.
            matches!(group, 1000..=1002)
        }
    }

    let seeds = RELIC_SCRIPTS_USED_BY_691.iter().map(|&id| id as i32);
    let result = transitive_script_closure(seeds, &Graph, &Base);

    // FULL closure reaches the base chain too (honest "how many scripts" size).
    assert!(
        result.closure.contains(&1000) && result.closure.contains(&1002),
        "full closure must include the reachable base chain"
    );
    // Splice burden is EXACTLY the donor-new relic seed — the base hub shields
    // its 1001/1002 subtree.
    let expected: BTreeSet<u32> = RELIC_SCRIPTS_USED_BY_691.iter().copied().collect();
    assert_eq!(
        result.missing_from_910, expected,
        "splice burden must be exactly the donor-new relic scripts (base subtree shielded)"
    );
    assert!(
        result.missing_from_910.is_subset(&result.closure),
        "splice burden must be a subset of the full closure"
    );
}

/// Live numbers for 691 and 1224 against the donor (948) flat cache + the
/// pristine 910-base scripts pack. Skips cleanly when those artifacts are absent
/// (clean checkout / CI), mirroring the `real_cache` suite's skip convention.
///
/// Asserts the plan's validation contract: 1224's port is large (sane lower
/// bound on the full closure + splice burden), 691's splice burden is small, and
/// 1224's burden exceeds 691's. Also locks that each interface's own donor seed
/// scripts land in its burden, and that the burden is a subset of the closure.
#[test]
fn transitive_closure_691_and_1224_against_donor_cache() {
    use rs3_cache_rs::explain::{TransitiveOptions, compute_transitive};
    use rs3_cache_rs::interface::component::explain_interface_group;
    use rs3_cache_rs::js5pack::PackArchive;
    use std::path::Path;

    let donor_cache = Path::new("../../cache/unpacked/948");
    let base_pack_root = Path::new("../../server/data/pack-910-base");
    let interfaces_pack =
        Path::new("../../server/data/pack-910-base-948-overlay/client.interfaces.js5");
    if !donor_cache.join("255").is_dir()
        || !base_pack_root.join("client.scripts.js5").is_file()
        || !interfaces_pack.is_file()
    {
        eprintln!("SKIP transitive live test (donor cache / base pack / interfaces pack absent)");
        return;
    }

    let opts = TransitiveOptions {
        scripts_cache: donor_cache,
        scripts_build: 948,
        scripts_subbuild: 1,
        data_dir: Path::new("data"),
        base_pack_root,
    };

    // Depth-1 component-bound scripts come from the runtime interfaces pack.
    let pack = PackArchive::open(interfaces_pack).expect("open interfaces pack");
    let depth1 = |iface: u32| -> BTreeSet<u32> {
        let files = pack
            .group_files(iface)
            .expect("unpack interface group")
            .unwrap_or_else(|| panic!("interface {iface} absent from pack"));
        explain_interface_group(iface, &files, 947)
            .expect("explain interface")
            .requires
            .scripts
    };

    let d691 = depth1(691);
    let d1224 = depth1(1224);
    let t691 = compute_transitive(&d691, &opts).expect("transitive 691");
    let t1224 = compute_transitive(&d1224, &opts).expect("transitive 1224");

    // Invariants: burden ⊆ closure; closure is the larger, full set.
    assert!(t691.missing_from_910.is_subset(&t691.closure));
    assert!(t1224.missing_from_910.is_subset(&t1224.closure));
    assert!(t691.closure_len() >= t691.missing_len() && t1224.closure_len() >= t1224.missing_len());

    // 1224 is a large port: the road-test put its full closure in the hundreds
    // and its splice burden near ~193. Assert sane lower bounds (the closure is
    // well into the hundreds; the burden is a substantial donor set).
    assert!(
        t1224.closure_len() >= 500,
        "interface 1224 full transitive closure unexpectedly small: {} (expected >= 500)",
        t1224.closure_len()
    );
    assert!(
        t1224.missing_len() >= 50,
        "interface 1224 splice burden unexpectedly small: {} (expected >= 50)",
        t1224.missing_len()
    );

    // 691 was a tidy splice: its donor burden is small (an order of magnitude
    // under the full closure, and well under 1224's burden).
    assert!(
        t691.missing_len() <= 150,
        "interface 691 splice burden unexpectedly large: {} (expected small, <= 150)",
        t691.missing_len()
    );
    assert!(
        t1224.missing_len() > t691.missing_len(),
        "1224 (big ritual port) burden {} should exceed 691 (tidy relic splice) burden {}",
        t1224.missing_len(),
        t691.missing_len()
    );

    // Each interface's own donor seed scripts must appear in its splice burden.
    for id in RELIC_SCRIPTS_USED_BY_691 {
        assert!(
            t691.missing_from_910.contains(&id),
            "relic script {id} missing from 691 splice burden"
        );
    }
    // The ritual 1224 depth-1 donor scripts (17784, 17786-17790, 16387) are
    // donor-new and must be in 1224's burden.
    for id in [16387_u32, 17784, 17786, 17787, 17788, 17789, 17790] {
        assert!(
            t1224.missing_from_910.contains(&id),
            "ritual script {id} missing from 1224 splice burden"
        );
    }
}
