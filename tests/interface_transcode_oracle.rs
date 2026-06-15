//! Oracle for the 948 → 910 interface-component **wire-version transcoder**
//! (`rs3_cache_rs::interface::transcode`). The faithful Rust mirror of the 910
//! client `Component.decode` (`rs3_cache_rs::interface::decode910`) IS the oracle:
//! it replaces "run the client and watch it crash".
//!
//! Committed donor artifacts (no external cache / pack dependency), embedded via
//! `include_bytes!`:
//!   * `server/cache-patches/relic-system-948/interfaces/691.dat` — the relic
//!     window. PROTECTED oracle dir (read-only here): every component uses a
//!     primitive type the 910 decoder already handles, so it baked fine on the
//!     live client. The transcode must be a **no-op-equivalent**.
//!   * `server/cache-patches/ritual-pedestal-948/interfaces/1224.dat` — the ritual
//!     selection window. Uses composite widget types (10 button, 12 check) the 910
//!     `Component.decode` has no body for; feeding the raw bytes to the mirror
//!     reproduces the live `ArrayIndexOutOfBoundsException` at
//!     `Component.decode:973`.
//!
//! Both groups are decoded WITHOUT an archive index via
//! `decode_interface_group_raw` (file count inferred from the chunk footer) — the
//! exact path `interface transcode --raw-dat` uses.

use std::collections::BTreeMap;

use rs3_cache_rs::interface::component::decode_interface_group_raw;
use rs3_cache_rs::interface::decode910::{Decode910Error, Decoded910, decode_component_910};
use rs3_cache_rs::interface::transcode::{
    Downcode, TargetType, transcode_and_pack, transcode_group,
};

/// Build the donor components decode at (the crate's 948/947 layout constant).
const BUILD: u32 = 947;

/// The committed relic interface-691 raw group (all primitive types).
const INTERFACE_691_DAT: &[u8] =
    include_bytes!("../../../server/cache-patches/relic-system-948/interfaces/691.dat");

/// The committed ritual interface-1224 raw group (button + check widgets).
const INTERFACE_1224_DAT: &[u8] =
    include_bytes!("../../../server/cache-patches/ritual-pedestal-948/interfaces/1224.dat");

/// Decode a committed raw group into its component file map.
fn files(dat: &[u8]) -> BTreeMap<u32, Vec<u8>> {
    decode_interface_group_raw(dat, BUILD)
        .expect("decode committed raw interface group")
        .files()
        .clone()
}

/// Compare two mirror decodes ignoring the version byte (downcoded to 9 by design)
/// and the end position (buffer length differs after a body rewrite).
fn same_meaning(a: &Decoded910, b: &Decoded910) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.version = 0;
    b.version = 0;
    a.end_pos = 0;
    b.end_pos = 0;
    a == b
}

// ── 1224: the bug reproduction ────────────────────────────────────────────────

/// The RAW (un-transcoded) 1224 widget components must crash the 910 mirror with
/// the exact live failure: an op-name index overrun (the `Component.decode:973`
/// AIOOBE). This is the bug the transcoder exists to fix.
#[test]
fn raw_1224_widgets_trip_the_910_opname_aioobe() {
    let files = files(INTERFACE_1224_DAT);
    let mut aioobe = 0usize;
    for bytes in files.values() {
        if let Err(Decode910Error::OpnameIndexOutOfBounds { index, .. }) =
            decode_component_910(bytes)
        {
            // The live crash reads a 255 index into a length-1 array.
            assert_eq!(index, 255, "expected the 255-index overrun the client hits");
            aioobe += 1;
        }
    }
    assert!(
        aioobe >= 1,
        "raw 1224 should reproduce the 910 op-name AIOOBE on at least one widget"
    );
}

/// EVERY component of the transcoded 1224 group must decode cleanly through the
/// 910 mirror — no AIOOBE, no misalignment — and end exactly at end-of-buffer.
#[test]
fn transcoded_1224_decodes_clean_through_910_mirror() {
    let files = files(INTERFACE_1224_DAT);
    let transcoded = transcode_group(&files, BUILD).expect("transcode 1224");
    assert_eq!(transcoded.len(), 50, "1224 has 50 components");

    for (id, tc) in &transcoded {
        let decoded = decode_component_910(&tc.bytes)
            .unwrap_or_else(|e| panic!("910 mirror rejected transcoded com{id}: {e}"));
        assert_eq!(
            decoded.end_pos,
            tc.bytes.len(),
            "transcoded com{id} not exactly buffer-sized"
        );
    }
}

/// The transcode must rewrite exactly the unsupported widgets (the 2 buttons + 1
/// check) and keep everything else; the rewritten widgets must preserve their op
/// labels, op cursors and hook scripts (the interactivity lives in the common
/// tail, which is carried verbatim).
#[test]
fn transcoded_1224_rewrites_only_widgets_and_preserves_interactivity() {
    let files = files(INTERFACE_1224_DAT);
    let transcoded = transcode_group(&files, BUILD).expect("transcode 1224");

    let mut rewritten: Vec<u32> = Vec::new();
    let mut from_types: Vec<u8> = Vec::new();
    for (id, tc) in &transcoded {
        if let Downcode::Rewritten { from_type, .. } = &tc.downcode {
            rewritten.push(*id);
            from_types.push(*from_type);
        }
    }
    rewritten.sort_unstable();
    from_types.sort_unstable();
    // Exactly two type-10 buttons and one type-12 check.
    assert_eq!(from_types, vec![10, 10, 12], "only buttons+check are rewritten");
    assert_eq!(rewritten.len(), 3);

    // com46 (button → layer): op "Select", cursor 46, hooks 10642 (onload) +
    // 17787 (onop).
    let com46 = decode_component_910(&transcoded[&46].bytes).expect("decode com46");
    assert_eq!(com46.type_id, 0, "button downcodes to an interactive layer");
    assert_eq!(com46.ops, vec!["Select".to_string()]);
    assert_eq!(com46.op_cursors, vec![(0, 46)]);
    assert!(com46.scripts.contains(&10642) && com46.scripts.contains(&17787));

    // com49 (check → text): label "Show Locked" survives as a text component, op
    // "Select", hooks 10642 (onload) + 17789 (onbuttonclick).
    let com49 = decode_component_910(&transcoded[&49].bytes).expect("decode com49");
    assert_eq!(com49.type_id, 4, "labelled check downcodes to a text component");
    assert_eq!(com49.ops, vec!["Select".to_string()]);
    assert_eq!(com49.op_cursors, vec![(0, 46)]);
    assert!(com49.scripts.contains(&10642) && com49.scripts.contains(&17789));
    // The "Show Locked" label maps onto a real type-4 text body the 910 client
    // renders (the donor check uses the default font, so no explicit font ref).
    assert!(
        matches!(transcoded[&49].downcode, Downcode::Rewritten { target: TargetType::Text, .. })
    );
    assert_eq!(com49.text, "Show Locked", "the downcoded label text survives");
}

/// The window's load-bearing refs survive the transcode: the title host script
/// (8420, via com0 onload) and the section-header fonts (167) are intact after
/// re-decoding through the 910 mirror.
#[test]
fn transcoded_1224_preserves_title_and_header_refs() {
    let files = files(INTERFACE_1224_DAT);
    let transcoded = transcode_group(&files, BUILD).expect("transcode 1224");

    // com0 onload calls the shared window builder 8420 ("Ritual selection").
    let com0 = decode_component_910(&transcoded[&0].bytes).expect("decode com0");
    assert!(
        com0.scripts.contains(&8420),
        "com0 must still call the window-builder script 8420; got {:?}",
        com0.scripts
    );

    // The "Requirements"/"Input"/"Output" headers are type-4 text on font 167.
    let header_font_167 = transcoded
        .values()
        .filter_map(|tc| decode_component_910(&tc.bytes).ok())
        .filter(|d| d.type_id == 4 && d.fonts.contains(&167))
        .count();
    assert!(
        header_font_167 >= 3,
        "expected the three font-167 section headers to survive; found {header_font_167}"
    );
}

// ── 691: the no-op-equivalent ─────────────────────────────────────────────────

/// 691 uses only primitive types, so the transcode keeps every component and the
/// 910 mirror must decode each transcoded component IDENTICALLY to the original
/// (modulo the version byte downcoded 11 → 9). This is the plan's "691 →
/// no-op-equivalent" gate.
#[test]
fn transcoded_691_is_noop_equivalent_through_910_mirror() {
    let files = files(INTERFACE_691_DAT);
    let transcoded = transcode_group(&files, BUILD).expect("transcode 691");
    assert_eq!(transcoded.len(), 225, "691 has 225 components");

    for (id, tc) in &transcoded {
        // Nothing in 691 is a composite widget → all kept.
        assert!(
            matches!(tc.downcode, Downcode::Kept { .. }),
            "691 com{id} should be kept (primitive type), not rewritten"
        );
        let before = decode_component_910(&files[id])
            .unwrap_or_else(|e| panic!("original 691 com{id} should decode through mirror: {e}"));
        let after = decode_component_910(&tc.bytes)
            .unwrap_or_else(|e| panic!("transcoded 691 com{id} should decode through mirror: {e}"));
        assert!(
            same_meaning(&before, &after),
            "691 com{id} decode changed across transcode (not a no-op)"
        );
        // The downcoded wire version is exactly 9.
        assert_eq!(after.version, 9, "kept components are written at version 9");
    }
}

// ── group re-pack round-trip ──────────────────────────────────────────────────

/// The re-packed group `.dat` must round-trip: re-decoding it (indexless, like the
/// live overlay applier) recovers the same per-component transcoded bytes, and the
/// whole group still decodes through the 910 mirror. Proves the packer is
/// byte-faithful end to end.
#[test]
fn repacked_1224_group_roundtrips_and_decodes() {
    let files = files(INTERFACE_1224_DAT);
    let group = transcode_and_pack(&files, BUILD, 9).expect("transcode + pack 1224");
    assert_eq!(group.roster, (0..50).collect::<Vec<u32>>(), "dense 0..50 roster");

    // Re-decode the packed .dat without an index, as the overlay applier would.
    let reparsed = decode_interface_group_raw(&group.dat, BUILD)
        .expect("re-decode packed group .dat")
        .files()
        .clone();
    assert_eq!(reparsed.len(), group.components.len());
    for (id, tc) in &group.components {
        assert_eq!(
            &reparsed[id], &tc.bytes,
            "packed group re-decode changed com{id} bytes"
        );
        // And it still passes the mirror after the pack/unpack round-trip.
        decode_component_910(&reparsed[id])
            .unwrap_or_else(|e| panic!("repacked com{id} failed the 910 mirror: {e}"));
    }
}

/// 691 packs and round-trips the same way (the no-op-equivalent group is still a
/// valid, mirror-clean group).
#[test]
fn repacked_691_group_roundtrips_and_decodes() {
    let files = files(INTERFACE_691_DAT);
    let group = transcode_and_pack(&files, BUILD, 9).expect("transcode + pack 691");
    let reparsed = decode_interface_group_raw(&group.dat, BUILD)
        .expect("re-decode packed 691 .dat")
        .files()
        .clone();
    assert_eq!(reparsed.len(), 225);
    for (id, tc) in &group.components {
        assert_eq!(&reparsed[id], &tc.bytes, "packed 691 re-decode changed com{id}");
        decode_component_910(&reparsed[id])
            .unwrap_or_else(|e| panic!("repacked 691 com{id} failed the mirror: {e}"));
    }
}
