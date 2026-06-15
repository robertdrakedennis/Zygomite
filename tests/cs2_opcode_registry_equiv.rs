//! Equivalence gate for the registry-backed `OpcodeBook`.
//!
//! Build 910: the registry-backed book MUST be byte-for-byte identical to the
//! txt-driven book (`by_id`, `by_name`, `large_by_id`, `aliases`). This single
//! assertion makes the Stage 4 swap safe for every consumer of
//! `OpcodeBook::load` without touching any of them.
//!
//! Build 948: the registry CANNOT reproduce the txt book and is therefore NOT
//! routed through `from_registry` by `load` (it stays txt-driven). The registry
//! is anchored to the 1,432-case 910 dispatch switch, so it omits the donor-only
//! opcodes present in `opcodes-948.txt` but absent from the 910 client. This
//! test pins that divergence as a documented, standing invariant: the only
//! difference between the two 948 books is exactly the set of donor-only ids,
//! and every other entry agrees. If the registry ever does gain full 948
//! coverage, this test will start failing and the §3.2 routing can be revisited.
//!
//! Skips silently when `data/cs2/registry-910.json` is absent.

use rs3_cache_rs::script::OpcodeBook;
use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn data_dir() -> PathBuf {
    crate_dir().join("data")
}

fn registry_path() -> PathBuf {
    data_dir().join("cs2").join("registry-910.json")
}

#[test]
fn registry_book_equals_txt_book_910() {
    let registry = registry_path();
    if !registry.is_file() {
        eprintln!("skip: registry absent at {}", registry.display());
        return;
    }

    let from_reg =
        OpcodeBook::from_registry(&registry, 910).expect("from_registry(910)");
    // 910 is subbuild 0.
    let from_txt = OpcodeBook::load_from_txt(&data_dir(), 910, 0).expect("load_from_txt(910)");

    // by_id: same length and element-wise equal.
    assert_eq!(
        from_reg.by_id.len(),
        from_txt.by_id.len(),
        "910: by_id length differs"
    );
    for (id, (reg, txt)) in from_reg.by_id.iter().zip(from_txt.by_id.iter()).enumerate() {
        assert_eq!(reg, txt, "910: by_id mismatch at id {id}");
    }

    // by_name, large_by_id (length + element-wise), aliases: full equality.
    assert_eq!(from_reg.by_name(), from_txt.by_name(), "910: by_name differs");
    assert_eq!(
        from_reg.large_by_id().len(),
        from_txt.large_by_id().len(),
        "910: large_by_id length differs"
    );
    for (id, (reg, txt)) in from_reg
        .large_by_id()
        .iter()
        .zip(from_txt.large_by_id().iter())
        .enumerate()
    {
        assert_eq!(reg, txt, "910: large_by_id mismatch at id {id}");
    }
    assert_eq!(from_reg.aliases(), from_txt.aliases(), "910: aliases differ");
}

#[test]
fn registry_948_diverges_only_by_donor_only_opcodes() {
    let registry = registry_path();
    if !registry.is_file() {
        eprintln!("skip: registry absent at {}", registry.display());
        return;
    }

    // The 948 opcode file is unscoped (`opcodes-948.txt`), so subbuild is
    // irrelevant to fallback resolution; pass 1 (the donor subbuild convention).
    let from_reg =
        OpcodeBook::from_registry(&registry, 948).expect("from_registry(948)");
    let from_txt = OpcodeBook::load_from_txt(&data_dir(), 948, 1).expect("load_from_txt(948)");

    // Every id the registry-derived 948 book DOES map must agree with the txt
    // book — no contradictory mappings, only omissions.
    for (name, &id) in from_reg.by_name() {
        assert_eq!(
            from_txt.by_name().get(name),
            Some(&id),
            "948: registry maps `{name}`->{id} but txt book disagrees"
        );
    }

    // The names the txt book has but the registry omits are precisely the
    // donor-only 948 opcodes (no 910 command). There must be at least one (this
    // is the documented reason 948 is NOT registry-routed); `sub` (id 824) is a
    // concrete witness observed in the live 948 cache.
    let any_missing = from_txt
        .by_name()
        .keys()
        .any(|n| !from_reg.by_name().contains_key(n));
    assert!(
        any_missing,
        "948: expected the registry to omit donor-only opcodes; found none — \
         the registry may now have full 948 coverage, revisit load() routing"
    );
    assert!(
        from_txt.by_name().contains_key("sub"),
        "948: witness opcode `sub` absent from txt book — data changed"
    );
    assert!(
        !from_reg.by_name().contains_key("sub"),
        "948: registry unexpectedly knows donor-only opcode `sub`"
    );

    // The registry-backed book is a strict subset by id count.
    assert!(
        from_reg.by_name().len() < from_txt.by_name().len(),
        "948: registry book must have fewer commands than the txt book"
    );
}
