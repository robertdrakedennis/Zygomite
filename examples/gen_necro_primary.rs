//! Clone the 910 magic ability-book page (interface 1459) into a SECOND Necromancy
//! interface group 1211 — the Necromancy "PRIMARY" book for the ribbon docked action
//! window, distinct from `interface_1207` (the Necromancy "MENU" book used by the Powers
//! WINDOW tab).
//!
//! Why two interfaces: the native combat styles each have a PRIMARY (docked action
//! window) AND a MENU (Powers-window tab) interface — e.g. Magic = 1461 PRIMARY / 1459
//! MENU, Ranged = 1452 PRIMARY / 1456 MENU. An interface instance can only be open in
//! one place at a time, so a single necro book (1207) cannot be in both the docked
//! window and the Powers tab simultaneously (opening Powers steals it, leaving the
//! docked window empty). 1211 gives the docked window its own book so both coexist.
//!
//! 1211 is a byte-identical clone of 910's magic page 1459 (== 1461) with com6's onload
//! retargeted to 1211's own coms + the necromancy powers-book routing case (13) — exactly
//! like `interface_1207` (`gen_necro_page.rs`). The necro book-routing scripts (script8423
//! page-init, script11435 first-show render, script8433 sub-tab click) gain parallel
//! cases for 1211:com7 (packed 79363079) so the necro ability grid renders in 1211 too.
//!
//! Output: <out-dir>/<component-id>.bin per component (the necro PRIMARY group).
//! Usage: cargo run --example `gen_necro_primary` -- <910-cache> <out-dir>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_INTERFACES;
use rs3_cache_rs::error::{CacheError, Result};
use rs3_cache_rs::interface_codec::{HookArg, decode_raw, encode_raw};
use std::collections::BTreeMap;

const BUILD: u32 = 910;
const SRC_GROUP: u32 = 1459; // 910 magic ability-book page (== 1461 PRIMARY)
const DST_GROUP: u32 = 1211; // necromancy PRIMARY page (docked action window)
const NECRO_ROUTING_STYLE: i32 = 13; // necromancy powers-book routing case (same as 1207)

fn comp_ref(group: u32, sub: u32) -> i32 {
    i32::try_from((group << 16) | sub).expect("component ref fits i32")
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: gen_necro_primary <910-cache> <out-dir>");
        std::process::exit(2);
    }
    let cache_dir = &args[1];
    let out_dir = &args[2];
    std::fs::create_dir_all(out_dir)?;

    let cache = FlatCache::open(cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, SRC_GROUP)?;
    if files.is_empty() {
        return Err(CacheError::message(format!(
            "source interface group {SRC_GROUP} has no components"
        )));
    }

    let src_com7 = comp_ref(SRC_GROUP, 7);
    let src_com8 = comp_ref(SRC_GROUP, 8);
    let src_com11 = comp_ref(SRC_GROUP, 11);
    let dst_com7 = comp_ref(DST_GROUP, 7);
    let dst_com8 = comp_ref(DST_GROUP, 8);
    let dst_com11 = comp_ref(DST_GROUP, 11);

    let mut produced: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let mut rewrote_onload = false;

    for (&sub, bytes) in &files {
        let mut component = decode_raw(bytes, BUILD)?;
        for hook in component.hooks.iter_mut().flatten() {
            for arg in &mut hook.args {
                if let HookArg::Int(v) = arg {
                    if *v == src_com7 {
                        *v = dst_com7;
                        rewrote_onload = true;
                    } else if *v == src_com8 {
                        *v = dst_com8;
                    } else if *v == src_com11 {
                        *v = dst_com11;
                    } else if *v == 3 && sub == 6 {
                        *v = NECRO_ROUTING_STYLE;
                    }
                }
            }
        }
        let encoded = encode_raw(&component, BUILD)?;
        let reparsed = decode_raw(&encoded, BUILD)?;
        if reparsed != component {
            return Err(CacheError::message(format!(
                "com{sub} failed decode/encode idempotency after clone"
            )));
        }
        produced.insert(sub, encoded);
    }

    if !rewrote_onload {
        return Err(CacheError::message(
            "expected to rewrite com6 onload component refs (1459 layout changed?)".to_string(),
        ));
    }

    for (id, bytes) in &produced {
        std::fs::write(format!("{out_dir}/{id}.bin"), bytes)?;
    }
    println!(
        "cloned interface {SRC_GROUP} -> {DST_GROUP}: {} components written to {out_dir} (onload style arg 3 -> {NECRO_ROUTING_STYLE})",
        produced.len()
    );
    Ok(())
}
