//! Regression-lock for the Round-2 Rust CLI polish (plan 011):
//!   A. `decode --format auto` inference + helpful errors for the archives the
//!      ritual road-test hit (npc 18, obj 19) plus dbtable/dbrow (40/41).
//!   B. `--pack-root` donor auto-fallback shared across `decode` /
//!      `explain-interface` / (font) — a group absent from the runtime overlay
//!      pack is served from the donor pack (`cache/rs3-cache/948-all/pack`) with
//!      a note, and "neither pack has it" errors quote the exact `--pack-root`.
//!
//! Two tiers, mirroring the other oracles:
//!   * Hermetic (always runs): the `auto` inference table and the
//!     genuine-unknown error text — no cache/pack dependency.
//!   * Live (pack-gated, skips cleanly when the packs are absent): the actual
//!     decode + donor fallback against the in-repo runtime overlay + donor packs.

use std::path::{Path, PathBuf};

use rs3_cache_rs::decode::{self, DecodeOptions, FORMAT_NAMES, Format};

// ─────────────────────────────── hermetic ───────────────────────────────

/// The `auto` inference table MUST map the road-test archives (and the obvious
/// config rest) to a concrete format. This is the heart of gap A — locked here
/// without any I/O so it can never silently regress to "cannot infer".
#[test]
fn auto_inference_table_covers_road_test_archives() {
    let cases = [
        (18u32, Format::Npc),  // road-test: npc config
        (19, Format::Obj),     // road-test: obj/item config
        (40, Format::DbTable), // dbtables (flat archive 40)
        (41, Format::DbRow),   // dbrows  (flat archive 41)
        (3, Format::Interface),
        (8, Format::Sprite),
        (13, Format::FontMetrics),
        (17, Format::Enum),
        (22, Format::Struct),
        (58, Format::FontMetrics2),
        (59, Format::Ttf),
    ];
    for (archive, want) in cases {
        assert_eq!(
            Format::auto_for_archive(archive),
            Some(want),
            "decode --format auto must infer {want:?} for archive {archive}"
        );
    }
    // A genuinely unmapped archive infers nothing (so `auto` falls to the
    // helpful-error path rather than guessing).
    assert_eq!(Format::auto_for_archive(39), None);
    assert_eq!(Format::auto_for_archive(12345), None);
}

/// Every concrete format name round-trips through `FromStr`, and the published
/// `FORMAT_NAMES` list (used in help + error suggestions) parses cleanly. Locks
/// that the suggestion list can't drift out of sync with the parser.
#[test]
fn every_published_format_name_parses() {
    for &name in FORMAT_NAMES {
        assert!(
            name.parse::<Format>().is_ok(),
            "published format name {name:?} does not parse"
        );
    }
    // The two road-test additions specifically.
    assert_eq!("npc".parse::<Format>().ok(), Some(Format::Npc));
    assert_eq!("obj".parse::<Format>().ok(), Some(Format::Obj));
    // `item` is accepted as an alias of `obj`.
    assert_eq!("item".parse::<Format>().ok(), Some(Format::Obj));
}

/// On a genuine-unknown `auto`, the error must (a) name the archive, and (b)
/// list the valid `--format` values to pass — NOT a bare "cannot infer". This
/// is the second half of gap A (helpful errors). Driven through the public
/// `decode()` against a non-existent pack root so it resolves the format error
/// before any pack I/O.
#[test]
fn auto_unknown_archive_lists_formats() {
    let nowhere = PathBuf::from("/no/such/pack-root");
    let err = decode::decode(&DecodeOptions {
        archive: 39,
        group: 0,
        format: Format::Auto,
        pack_root: &nowhere,
        json: true,
    })
    .expect_err("auto on unknown archive 39 must error");
    let msg = err.to_string();
    assert!(
        msg.contains("archive 39"),
        "auto error must name the archive; got: {msg}"
    );
    // Lists concrete formats (e.g. npc/obj/dbtable) to pass.
    assert!(
        msg.contains("npc") && msg.contains("dbtable") && msg.contains("--format"),
        "auto error must list valid --format values; got: {msg}"
    );
}

// ──────────────────────────────── live ─────────────────────────────────

/// Crate-relative runtime overlay pack root (the default `--pack-root`).
fn runtime_pack_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../server/data/pack-910-base-948-overlay")
}

/// Crate-relative donor (948-all) pack root — the auto-fallback target.
fn donor_pack_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rs3_cache_rs::pack_root::DONOR_PACK_ROOT)
}

/// `true` when both packs (runtime overlay interfaces + donor interfaces) are
/// present, so the live fallback tests can run. Skips otherwise.
fn live_packs_present() -> bool {
    runtime_pack_root().join("client.interfaces.js5").is_file()
        && donor_pack_root().join("client.interfaces.js5").is_file()
}

/// `decode --format auto` against the live runtime pack infers + decodes npc
/// (18), obj (19) and dbtable (40) — the road-test archives that used to error.
#[test]
fn live_auto_decodes_npc_obj_dbtable() {
    if !live_packs_present() {
        eprintln!("SKIP live_auto_decodes_npc_obj_dbtable: packs absent");
        return;
    }
    let root = runtime_pack_root();

    // npc archive 18, group 0 → 128 npc files, format inferred as npc.
    let npc = decode::decode(&DecodeOptions {
        archive: 18,
        group: 0,
        format: Format::Auto,
        pack_root: &root,
        json: true,
    })
    .expect("auto-decode npc archive 18 group 0");
    assert_eq!(npc["format"], "npc");
    assert!(
        npc["file_count"].as_u64().unwrap_or(0) >= 100,
        "npc group 0 should hold a full split block; got {}",
        npc["file_count"]
    );

    // obj archive 19, group 0 → 256 obj files, format inferred as obj.
    let obj = decode::decode(&DecodeOptions {
        archive: 19,
        group: 0,
        format: Format::Auto,
        pack_root: &root,
        json: true,
    })
    .expect("auto-decode obj archive 19 group 0");
    assert_eq!(obj["format"], "obj");
    assert!(obj["file_count"].as_u64().unwrap_or(0) >= 100);

    // dbtable archive 40: format inferred as dbtable, read from the canonical
    // config group 40 regardless of the `--group` passed (here a bogus 999).
    let dbt = decode::decode(&DecodeOptions {
        archive: 40,
        group: 999,
        format: Format::Auto,
        pack_root: &root,
        json: true,
    })
    .expect("auto-decode dbtable archive 40");
    assert_eq!(dbt["format"], "dbtable");
    assert_eq!(
        dbt["group"], 40,
        "dbtable must read config group 40 even when --group is bogus"
    );
    assert!(dbt["file_count"].as_u64().unwrap_or(0) > 0);
}

/// The donor auto-fallback: a group ABSENT from the runtime overlay pack but
/// present in the donor pack is decoded from the donor, and the result records
/// which pack served it. Interface group 643 is donor-only (verified by the
/// pack-diff probe); if a future overlay splices it in, this picks another
/// donor-only group, or skips if none remain.
#[test]
fn live_decode_falls_back_to_donor_for_donor_only_group() {
    if !live_packs_present() {
        eprintln!("SKIP live_decode_falls_back_to_donor_for_donor_only_group: packs absent");
        return;
    }
    let runtime = runtime_pack_root();
    let donor = donor_pack_root();

    let Some(group) = donor_only_interface_group(&runtime, &donor) else {
        eprintln!("SKIP: no donor-only interface group found (overlay fully covers donor)");
        return;
    };

    let out = decode::decode_with_note(&DecodeOptions {
        archive: 3,
        group,
        format: Format::Interface,
        pack_root: &runtime,
        json: true,
    })
    .unwrap_or_else(|e| panic!("decode donor-only interface {group} via fallback: {e}"));

    // The decode succeeded against the donor pack, with a fallback note that
    // names the donor pack and the group.
    assert_eq!(out.value["format"], "interface");
    assert!(out.value["file_count"].as_u64().unwrap_or(0) > 0);
    let pack = out.value["pack"].as_str().unwrap_or_default();
    assert!(
        pack.contains("948-all"),
        "donor-only group {group} should be served from the donor pack; pack={pack}"
    );
    let note = out.pack_note.unwrap_or_default();
    assert!(
        note.contains("donor pack") && note.contains(&group.to_string()),
        "fallback note must name the donor pack + group; got: {note}"
    );
}

/// When NEITHER pack has the group, the error must quote the exact donor
/// `--pack-root` to pass (the precise-error half of gap B).
#[test]
fn live_neither_pack_error_names_donor_root() {
    if !live_packs_present() {
        eprintln!("SKIP live_neither_pack_error_names_donor_root: packs absent");
        return;
    }
    let root = runtime_pack_root();
    let err = decode::decode(&DecodeOptions {
        archive: 3,
        group: 999_999,
        format: Format::Interface,
        pack_root: &root,
        json: true,
    })
    .expect_err("a group in neither pack must error");
    let msg = err.to_string();
    assert!(
        msg.contains("--pack-root") && msg.contains("948-all"),
        "neither-pack error must quote the donor --pack-root; got: {msg}"
    );
}

/// `explain-interface` shares the same fallback: a donor-only interface explains
/// without an explicit `--pack-root`, with the fallback note recorded. Locks
/// gap B's "apply consistently across font, explain-interface, and decode".
#[test]
fn live_explain_interface_falls_back_to_donor() {
    use rs3_cache_rs::explain::{ExplainInterfaceOptions, InterfaceSource, explain_with_note};

    if !live_packs_present() {
        eprintln!("SKIP live_explain_interface_falls_back_to_donor: packs absent");
        return;
    }
    let runtime = runtime_pack_root();
    let donor = donor_pack_root();
    let Some(group) = donor_only_interface_group(&runtime, &donor) else {
        eprintln!("SKIP: no donor-only interface group found");
        return;
    };

    let (explained, note) = explain_with_note(&ExplainInterfaceOptions {
        interface: group,
        build: rs3_cache_rs::constants::BUILD,
        source: InterfaceSource::Pack(&runtime),
        json: false,
        transitive: None,
        data_closure: false,
    })
    .unwrap_or_else(|e| panic!("explain donor-only interface {group} via fallback: {e}"));

    assert_eq!(explained.interface, group);
    assert!(
        explained.component_count > 0,
        "donor-only interface {group} should decode some components"
    );
    let note = note.unwrap_or_default();
    assert!(
        note.contains("donor pack"),
        "explain-interface fallback must note the donor pack; got: {note}"
    );
}

/// Find an interface group present in the donor pack but absent from the runtime
/// overlay pack (the donor-fallback case). Returns `None` when the overlay
/// already covers every donor interface.
fn donor_only_interface_group(runtime: &Path, donor: &Path) -> Option<u32> {
    use rs3_cache_rs::js5pack::PackArchive;
    let rt = PackArchive::open(&runtime.join("client.interfaces.js5")).ok()?;
    let dn = PackArchive::open(&donor.join("client.interfaces.js5")).ok()?;
    dn.group_ids().find(|&g| !rt.has_group(g))
}
