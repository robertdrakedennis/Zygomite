//! BYTE-EXACT oracle for the semantic port layer's INTERFACE path (plan §9 step
//! 5). `interface port` (driven here via [`rs3_cache_rs::port::interface`]) must
//! reproduce the committed transcoded interface `.dat` BYTE-FOR-BYTE — the
//! regression oracle that gates routing `interface transcode` through the IR path.
//!
//! Oracle (NEVER edit — committed regression artifacts the layer reproduces):
//!   * `server/cache-patches/ritual-pedestal-948/interfaces/1224-910.dat` — the
//!     948 donor "Ritual selection" group (50 components: 47 primitive + 2 buttons
//!     + 1 check) TRANSCODED to the 910 component wire format. sha-pinned in
//!     `CacheOverlay.ts` (`3ce4c90b…`); the IR encode must reproduce its exact
//!     bytes (gzip container included — this `.dat` was produced by the crate's
//!     flate2 path, which is deterministic, unlike the Node-zlib config `.dat`s).
//!   * `1224.dat` — the donor INPUT the port reads.
//!   * `relic-system-948/interfaces/691.dat` — all-primitive group; the port is a
//!     no-op-equivalent (every component kept, byte-stable through the 910 mirror).
//!
//! Both committed `.dat`s are embedded with `include_bytes!` (no external cache).

use rs3_cache_rs::interface::component::decode_interface_group_raw;
use rs3_cache_rs::interface::decode910::decode_component_910;
use rs3_cache_rs::interface::transcode::TARGET_VERSION;
use rs3_cache_rs::port::book::BuildDescriptor;
use rs3_cache_rs::port::interface::{downcode_from, port_interface_group};
use rs3_cache_rs::port::ir::interface::ComponentKind;
use std::path::{Path, PathBuf};

/// Build the donor components decode at (the 948/947 layout constant).
const BUILD: u32 = 947;

/// The committed ritual interface-1224 donor + transcoded groups.
const INTERFACE_1224_DONOR: &[u8] =
    include_bytes!("../../../server/cache-patches/ritual-pedestal-948/interfaces/1224.dat");
const INTERFACE_1224_910: &[u8] =
    include_bytes!("../../../server/cache-patches/ritual-pedestal-948/interfaces/1224-910.dat");
/// The committed relic interface-691 raw group (all primitive types).
const INTERFACE_691_DONOR: &[u8] =
    include_bytes!("../../../server/cache-patches/relic-system-948/interfaces/691.dat");

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

fn target_910() -> BuildDescriptor {
    BuildDescriptor::load(&data_dir(), 910).expect("load 910 descriptor")
}

/// Decode a committed raw group into its dense component file map.
fn files(dat: &[u8]) -> std::collections::BTreeMap<u32, Vec<u8>> {
    decode_interface_group_raw(dat, BUILD)
        .expect("decode committed raw interface group")
        .files()
        .clone()
}

/// THE GATE: `interface port --group 1224` reproduces the committed `1224-910.dat`
/// byte-for-byte (the full gzip JS5 container, exactly as `CacheOverlay.ts` reads
/// + sha-pins it).
#[test]
fn interface_port_reproduces_committed_1224_910_byte_for_byte() {
    let donor = files(INTERFACE_1224_DONOR);
    let ported = port_interface_group(
        1224,
        &donor,
        BUILD,
        &target_910(),
        u16::from(TARGET_VERSION),
    )
    .expect("port interface 1224 through the IR layer");

    assert_eq!(
        ported.group.dat.len(),
        INTERFACE_1224_910.len(),
        "ported 1224 .dat length {} != committed {} bytes",
        ported.group.dat.len(),
        INTERFACE_1224_910.len()
    );
    assert_eq!(
        ported.group.dat, INTERFACE_1224_910,
        "ported 1224 .dat does not reproduce the committed 1224-910.dat byte-for-byte"
    );
}

/// The port downcodes exactly the unsupported widgets (2 buttons + 1 check), and
/// the surviving label ("Show Locked" on the check) lands as a text component.
#[test]
fn port_1224_downcodes_only_widgets() {
    let donor = files(INTERFACE_1224_DONOR);
    let ported =
        port_interface_group(1224, &donor, BUILD, &target_910(), 9).expect("port interface 1224");
    assert_eq!(ported.component_count, 50, "1224 has 50 components");

    let mut from_types: Vec<u8> = ported
        .rewritten()
        .into_iter()
        .filter_map(|(_, d)| downcode_from(d).map(ComponentKind::type_id))
        .collect();
    from_types.sort_unstable();
    assert_eq!(
        from_types,
        vec![10, 10, 12],
        "only the 2 buttons + 1 check are rewritten"
    );

    // com49 (check → text): the "Show Locked" label survives as a text component.
    let com49 = decode_component_910(&ported.group.components[&49]).expect("decode com49");
    assert_eq!(
        com49.type_id, 4,
        "labelled check downcodes to a text component"
    );
    assert_eq!(com49.text, "Show Locked", "the downcoded label survives");
    assert_eq!(
        com49.ops,
        vec!["Select".to_string()],
        "op preserved via the verbatim tail"
    );
    assert!(com49.scripts.contains(&10642), "onload hook preserved");
}

/// 691 (all primitive) is a no-op-equivalent: every component is kept, and the
/// re-packed group still decodes clean through the 910 mirror.
#[test]
fn port_691_is_noop_equivalent() {
    let donor = files(INTERFACE_691_DONOR);
    let ported =
        port_interface_group(691, &donor, BUILD, &target_910(), 9).expect("port interface 691");
    assert_eq!(ported.component_count, 225, "691 has 225 components");
    assert!(
        ported.rewritten().is_empty(),
        "no 691 component is a composite widget"
    );

    for (id, bytes) in &ported.group.components {
        let decoded = decode_component_910(bytes)
            .unwrap_or_else(|e| panic!("910 mirror rejected ported 691 com{id}: {e}"));
        assert_eq!(decoded.version, 9, "kept components written at version 9");
        assert_eq!(
            decoded.end_pos,
            bytes.len(),
            "ported 691 com{id} not buffer-sized"
        );
    }
}

/// The packed group re-decodes (indexless, like the overlay applier) to the same
/// per-component bytes — proving the IR encode's packer is byte-faithful.
#[test]
fn ported_1224_group_roundtrips() {
    let donor = files(INTERFACE_1224_DONOR);
    let ported =
        port_interface_group(1224, &donor, BUILD, &target_910(), 9).expect("port interface 1224");
    let reparsed = decode_interface_group_raw(&ported.group.dat, BUILD)
        .expect("re-decode ported group .dat")
        .files()
        .clone();
    assert_eq!(reparsed.len(), ported.group.components.len());
    for (id, bytes) in &ported.group.components {
        assert_eq!(
            &reparsed[id], bytes,
            "packed group re-decode changed com{id}"
        );
    }
}
