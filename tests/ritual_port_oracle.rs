//! BYTE-EXACT oracle for the semantic port layer (plan §9 steps 1–2 / §11).
//!
//! `cs2 port` (here driven directly via [`rs3_cache_rs::port::ritual`]) must
//! reproduce the committed `server/cache-patches/ritual-pedestal-948/scripts/
//! *.asm.ts` BYTE-FOR-BYTE — the regression oracle that gates retiring
//! `build-ritual-scripts.py`.
//!
//! The donor (948) scripts are decoded from the flat 948 cache
//! (`cache/unpacked/948`); the test SKIPS cleanly when that cache is absent (CI
//! without the donor corpus), exactly like the other cache-gated oracles.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_CLIENTSCRIPTS;
use rs3_cache_rs::port::book::BuildDescriptor;
use rs3_cache_rs::port::ritual::{self, PortedScript};
use rs3_cache_rs::port::{lodestone, material_storage, relic};
use rs3_cache_rs::script::OpcodeBook;

// Note: PortedScript is used by the shared `assert_family_byte_exact` helper.

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

fn committed_scripts_dir() -> PathBuf {
    family_scripts_dir("ritual-pedestal-948")
}

fn family_scripts_dir(family: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../server/cache-patches")
        .join(family)
        .join("scripts")
}

/// The flat 948 donor cache, or `None` (skip) when absent.
fn donor_948_cache() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RS3_CACHE_DIR_948") {
        let p = PathBuf::from(path);
        return p.join("255").is_dir().then_some(p);
    }
    let default = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/unpacked/948");
    default.join("255").is_dir().then_some(default)
}

/// The flat 910 base cache, or `None` (skip) when absent.
fn base_910_cache() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RS3_CACHE_DIR_910") {
        let p = PathBuf::from(path);
        return p.join("255").is_dir().then_some(p);
    }
    let default = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/unpacked/910");
    default.join("255").is_dir().then_some(default)
}

#[test]
fn cs2_port_reproduces_committed_ritual_scripts_byte_for_byte() {
    let Some(cache_dir) = donor_948_cache() else {
        eprintln!("skipping ritual port oracle: flat 948 cache absent (set RS3_CACHE_DIR_948)");
        return;
    };

    let cache = FlatCache::open(&cache_dir).expect("open 948 cache");
    let index = cache
        .archive_index(ARCHIVE_CLIENTSCRIPTS)
        .expect("clientscripts index");
    let book_948 = OpcodeBook::load(&data_dir(), 948, 1).expect("948 opcode book");
    let d948 = BuildDescriptor::load(&data_dir(), 948).expect("948 descriptor");
    let d910 = BuildDescriptor::load(&data_dir(), 910).expect("910 descriptor");

    let source = ritual::cache_source(&cache, &index, &book_948);
    let ported = ritual::port_ritual_scripts(&source, &d948, &d910)
        .expect("port ritual scripts through the layer");

    // Map out_id → committed file text, asserting each ported listing matches.
    let mut mismatches = Vec::new();
    let mut produced: BTreeMap<i32, String> = BTreeMap::new();
    for p in &ported {
        produced.insert(p.out_id, p.text.clone());
        let committed_path = committed_scripts_dir().join(format!("script{}.asm.ts", p.out_id));
        let committed = std::fs::read_to_string(&committed_path).unwrap_or_else(|_| {
            panic!(
                "committed oracle missing: {} (the layer produced script{} which is not committed)",
                committed_path.display(),
                p.out_id
            )
        });
        if committed != p.text {
            mismatches.push((p.out_id, first_diff_line(&committed, &p.text)));
        }
    }

    // Every committed `.asm.ts` must be reproduced (no committed file left out).
    let committed_ids = committed_script_ids();
    let produced_ids: std::collections::BTreeSet<i32> = produced.keys().copied().collect();
    let missing: Vec<i32> = committed_ids
        .iter()
        .copied()
        .filter(|id| !produced_ids.contains(id))
        .collect();

    assert!(
        mismatches.is_empty(),
        "{} of {} ritual listings differ from the committed oracle (NOT byte-exact):\n{}",
        mismatches.len(),
        ported.len(),
        mismatches
            .iter()
            .map(|(id, d)| format!("  script{id}.asm.ts: {d}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert!(
        missing.is_empty(),
        "the layer did not reproduce these committed listings: {missing:?}",
    );
    assert_eq!(
        produced_ids, committed_ids,
        "produced id set differs from committed id set",
    );
}

#[test]
fn cs2_port_reproduces_committed_relic_scripts_byte_for_byte() {
    let Some(cache_dir) = donor_948_cache() else {
        eprintln!("skipping relic port oracle: flat 948 cache absent (set RS3_CACHE_DIR_948)");
        return;
    };
    let cache = FlatCache::open(&cache_dir).expect("open 948 cache");
    let index = cache
        .archive_index(ARCHIVE_CLIENTSCRIPTS)
        .expect("clientscripts index");
    let book_948 = OpcodeBook::load(&data_dir(), 948, 1).expect("948 opcode book");
    let d948 = BuildDescriptor::load(&data_dir(), 948).expect("948 descriptor");
    let d910 = BuildDescriptor::load(&data_dir(), 910).expect("910 descriptor");

    let source = ritual::cache_source(&cache, &index, &book_948);
    let ported =
        relic::port_relic_scripts(&source, &d948, &d910).expect("port relic scripts through layer");
    assert_family_byte_exact(&ported, "relic-system-948");
}

#[test]
fn cs2_port_reproduces_committed_material_storage_scripts_byte_for_byte() {
    let (Some(cache_948), Some(cache_910)) = (donor_948_cache(), base_910_cache()) else {
        eprintln!(
            "skipping material-storage port oracle: 948 or 910 flat cache absent \
             (set RS3_CACHE_DIR_948 / RS3_CACHE_DIR_910)"
        );
        return;
    };
    let c948 = FlatCache::open(&cache_948).expect("open 948 cache");
    let c910 = FlatCache::open(&cache_910).expect("open 910 cache");
    let i948 = c948.archive_index(ARCHIVE_CLIENTSCRIPTS).expect("948 idx");
    let i910 = c910.archive_index(ARCHIVE_CLIENTSCRIPTS).expect("910 idx");
    let book_948 = OpcodeBook::load(&data_dir(), 948, 1).expect("948 book");
    let book_910 = OpcodeBook::load(&data_dir(), 910, 0).expect("910 book");
    let d948 = BuildDescriptor::load(&data_dir(), 948).expect("948 descriptor");
    let d910 = BuildDescriptor::load(&data_dir(), 910).expect("910 descriptor");

    let donor_source = ritual::flat_cache_source(&c948, &i948, &book_948, 948);
    let base_source = ritual::flat_cache_source(&c910, &i910, &book_910, 910);
    let ported = material_storage::port_material_storage_scripts(
        &donor_source,
        &base_source,
        &d948,
        &d910,
    )
    .expect("port material-storage scripts through layer");
    assert_family_byte_exact(&ported, "material-storage-948");
}

#[test]
fn cs2_port_reproduces_committed_lodestone_scripts_byte_for_byte() {
    let Some(cache_910) = base_910_cache() else {
        eprintln!("skipping lodestone port oracle: 910 flat cache absent (set RS3_CACHE_DIR_910)");
        return;
    };
    let c910 = FlatCache::open(&cache_910).expect("open 910 cache");
    let i910 = c910.archive_index(ARCHIVE_CLIENTSCRIPTS).expect("910 idx");
    let book_910 = OpcodeBook::load(&data_dir(), 910, 0).expect("910 book");
    let d910 = BuildDescriptor::load(&data_dir(), 910).expect("910 descriptor");

    let base_source = ritual::flat_cache_source(&c910, &i910, &book_910, 910);
    let ported = lodestone::port_lodestone_scripts(&base_source, &d910)
        .expect("port lodestone scripts through layer");
    assert_family_byte_exact(&ported, "lodestone-948");
}

/// Assert every ported listing matches its committed `.asm.ts` byte-for-byte AND
/// that the produced id set equals the committed id set for the family.
fn assert_family_byte_exact(ported: &[PortedScript], family: &str) {
    let dir = family_scripts_dir(family);
    let mut mismatches = Vec::new();
    let mut produced_ids = std::collections::BTreeSet::new();
    for p in ported {
        produced_ids.insert(p.out_id);
        let path = dir.join(format!("script{}.asm.ts", p.out_id));
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "committed oracle missing: {} (layer produced script{} which is not committed)",
                path.display(),
                p.out_id
            )
        });
        if committed != p.text {
            mismatches.push((p.out_id, first_diff_line(&committed, &p.text)));
        }
    }
    let committed_ids: std::collections::BTreeSet<i32> = std::fs::read_dir(&dir)
        .expect("read committed scripts dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".asm.ts")?
                .strip_prefix("script")?
                .parse::<i32>()
                .ok()
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} of {} {family} listings differ from the committed oracle (NOT byte-exact):\n{}",
        mismatches.len(),
        ported.len(),
        mismatches
            .iter()
            .map(|(id, d)| format!("  script{id}.asm.ts: {d}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert_eq!(
        produced_ids, committed_ids,
        "{family}: produced id set differs from committed id set",
    );
}

/// The set of script ids committed under the ritual scripts dir.
fn committed_script_ids() -> std::collections::BTreeSet<i32> {
    std::fs::read_dir(committed_scripts_dir())
        .expect("read committed scripts dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".asm.ts")?;
            stem.strip_prefix("script")?.parse::<i32>().ok()
        })
        .collect()
}

/// The first differing line (1-based) between two texts, for a compact failure.
fn first_diff_line(a: &str, b: &str) -> String {
    for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            return format!("line {} differs\n    committed: {la:?}\n    produced:  {lb:?}", i + 1);
        }
    }
    let (na, nb) = (a.lines().count(), b.lines().count());
    if na == nb {
        "trailing/whitespace difference".to_string()
    } else {
        format!("line count differs (committed {na}, produced {nb})")
    }
}
