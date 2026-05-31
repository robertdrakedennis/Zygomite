//! Clone the 910 magic ability-book page (interface 1459) into a new interface
//! group 1207 (the Necromancy page), driven entirely by the live 910 cache.
//!
//! Phase 3a of the Combat Style Modernisation (server/cache-patches/
//! combat-modernisation). 947 already ships `interface_1207` as the Necromancy
//! page — and it is BYTE-IDENTICAL to 910's magic page 1459 except for exactly
//! two things on com6's `onload` hook:
//!   1. the three component-ref args point at 1207's own coms (not 1459's), and
//!   2. the trailing style-id arg is the powers-book routing case.
//!
//! 947 used routing case 4 (shifting Defence to 5); we use a FREE case (13) so
//! the native melee/ranged/magic/defence routing is not renumbered.
//!
//! So instead of splicing the donor whole-group (which would also drag the
//! renumbered 947 component layout + its deeper call graph — the donor
//! whole-group hazard the skills-29 work documents), we CLONE 910's own 1459:
//! every component is copied verbatim, and only com6's onload hook is rewritten
//! (1459-> 1207 component refs, style arg -> 13). Each emitted component is
//! re-encoded with the interface codec and verified (decode/encode idempotency).
//!
//! Output: <out-dir>/<component-id>.bin per component (the necro page group).
//!
//! Usage: cargo run --example `gen_necro_page` -- <910-cache> <out-dir>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_INTERFACES;
use rs3_cache_rs::error::{CacheError, Result};
use rs3_cache_rs::interface_codec::{HookArg, decode_raw, encode_raw};
use std::collections::BTreeMap;

const BUILD: u32 = 910;
const SRC_GROUP: u32 = 1459; // 910 magic ability-book page
const DST_GROUP: u32 = 1207; // necromancy page (clone)
const NECRO_ROUTING_STYLE: i32 = 13; // free powers-book routing case for necromancy

fn comp_ref(group: u32, sub: u32) -> i32 {
    i32::try_from((group << 16) | sub).expect("component ref fits i32")
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: gen_necro_page <910-cache> <out-dir>");
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

    // Pre-compute the component-ref remaps for com6's onload args.
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

        // Any hook that references a SRC_GROUP component must be retargeted to
        // DST_GROUP, and the magic style-id arg (3) on com6's onload becomes the
        // necromancy routing case (13). 1459 has exactly one hook (com6 onload =
        // script8422(:com7,:com8,:com11, 3)); we assert we hit it.
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
                        // the magic page's style-id arg -> necromancy routing case
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
