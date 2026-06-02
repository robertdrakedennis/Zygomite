//! Clone the 910 HUD "Magic Book" Powers-flyout ACTION WINDOW frame
//! (interface_1477 com261..com271) into a NEW Necromancy action-window frame
//! (com856..com866), driven entirely by the live 910 cache.
//!
//! Combat Style Modernisation — the Necromancy "purple book" action window in
//! the resizable ribbon Powers flyout. The flyout's button list is enum_7703
//! ([4,12,5,6,7,8] = Prayer/Familiar/MagicBook/Melee/Ranged/Defensive); each
//! entry is a dockable HUD window whose layout struct (enum_7716[windowId]) holds
//! its flyout icon (param_3495/96/97), frame component refs (param_3503..3513 ->
//! interface_1477:comNNN) and content interface (param_3514..3517).
//!
//! The "Magic Book" window (windowId 5, struct_21289) is the ideal clone template
//! for Necromancy because its content interface is interface_1459 — exactly the
//! page our necromancy book (interface_1207) was cloned from. Its frame is
//! interface_1477 com261 (host: 224x288, xmode=abs_right, ymode=abs_bottom,
//! layer=com50, hide=yes, onload=script8409(5)) with 10 child layers com262..271
//! (all `layer=com261`). We clone all 11 to com856..com866:
//!   - the root com856's onload arg 5 -> 1039 (the native-reserved Necromancy
//!     window id, already iterated by enum_7717),
//!   - every child's parent `layer` ref com261 -> com856 (the new root).
//! The host carries NO explicit x/y (it docks via xmode/ymode) so position is left
//! to the layout/position structs — unlike the earlier (wrong) action-bar clone.
//! Each emitted component is re-encoded with the interface codec and verified
//! (decode/encode idempotency).
//!
//! Output: <out-dir>/<component-id>.bin per NEW component (com856..com866).
//! These are packed into a full interface_1477 group-replace .dat by the
//! TypeScript builder (build-necro-action-window.ts).
//!
//! Usage: cargo run --example gen_necro_action_window -- <910-cache> <out-dir>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_INTERFACES;
use rs3_cache_rs::error::{CacheError, Result};
use rs3_cache_rs::interface_codec::{HookArg, decode_raw, encode_raw};
use std::collections::BTreeMap;

const BUILD: u32 = 910;
const IFACE_GROUP: u32 = 1477; // the resizable HUD top-level interface

// "Magic Book" action-window frame (the clone source).
const SRC_ROOT: u32 = 261; // com261: onload=script8409(5), 224x288 layer under com50
const SRC_CHILDREN: [u32; 10] = [262, 263, 264, 265, 266, 267, 268, 269, 270, 271];

// Necromancy action-window frame (the clone destination): the next free
// component ids in interface_1477 (highest native is com855).
const DST_ROOT: u32 = 856;
const DST_CHILDREN: [u32; 10] = [857, 858, 859, 860, 861, 862, 863, 864, 865, 866];

const SRC_WINDOW_ID: i32 = 5; // "Magic Book" (the onload arg on com261)
// NECROMANCY window id == its powers-book routing/bookId (13), mirroring the native combat books whose
// windowId == bookId (Magic Book=5, Melee=6, Ranged=7, Defensive=8). The page-init (script8423 ->
// script8411(bookId)) draws the window chrome for the bookId, so windowId==bookId makes the flyout-opened
// window and the page-init chrome the SAME window = ONE title bar (live-confirmed working). The docked
// content is the dedicated necro PRIMARY book interface_1211.
const DST_WINDOW_ID: i32 = 13;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: gen_necro_action_window <910-cache> <out-dir>");
        std::process::exit(2);
    }
    let cache_dir = &args[1];
    let out_dir = &args[2];
    std::fs::create_dir_all(out_dir)?;

    let cache = FlatCache::open(cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, IFACE_GROUP)?;
    if files.is_empty() {
        return Err(CacheError::message(format!(
            "source interface group {IFACE_GROUP} has no components"
        )));
    }

    // Map each source sub-id -> its destination sub-id.
    let mut remap: BTreeMap<u32, u32> = BTreeMap::new();
    remap.insert(SRC_ROOT, DST_ROOT);
    for (i, &c) in SRC_CHILDREN.iter().enumerate() {
        remap.insert(c, DST_CHILDREN[i]);
    }

    let mut produced: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let mut rewrote_onload = false;
    let mut reparented = 0usize;

    for (&src_sub, &dst_sub) in &remap {
        let bytes = files.get(&src_sub).ok_or_else(|| {
            CacheError::message(format!(
                "source component com{src_sub} missing in group {IFACE_GROUP}"
            ))
        })?;
        let mut component = decode_raw(bytes, BUILD)?;

        // Re-parent any layer ref that points at a cloned source component to its
        // destination (handles the root and any internal nesting generically).
        if component.layer >= 0 {
            if let Some(&dst) = remap.get(&(component.layer as u32)) {
                component.layer = i32::try_from(dst).expect("sub-id fits i32");
                reparented += 1;
            }
        }

        // On the root, retarget the onload window-id arg ("Magic Book" 5 -> NECRO
        // 1039). The root keeps its parent (com50) and abs_right/abs_bottom docking.
        if src_sub == SRC_ROOT {
            for hook in component.hooks.iter_mut().flatten() {
                if hook.script == 8409 {
                    for arg in &mut hook.args {
                        if let HookArg::Int(v) = arg {
                            if *v == SRC_WINDOW_ID {
                                *v = DST_WINDOW_ID;
                                rewrote_onload = true;
                            }
                        }
                    }
                }
            }
        }

        let encoded = encode_raw(&component, BUILD)?;
        let reparsed = decode_raw(&encoded, BUILD)?;
        if reparsed != component {
            return Err(CacheError::message(format!(
                "com{dst_sub} failed decode/encode idempotency after clone"
            )));
        }
        produced.insert(dst_sub, encoded);
    }

    if !rewrote_onload {
        return Err(CacheError::message(
            "expected to rewrite com261 onload window-id arg 5 -> 13 (1477 layout changed?)"
                .to_string(),
        ));
    }
    if reparented < SRC_CHILDREN.len() {
        return Err(CacheError::message(format!(
            "expected to re-parent {} child layers com261 -> com856 (got {reparented})",
            SRC_CHILDREN.len()
        )));
    }

    for (id, bytes) in &produced {
        std::fs::write(format!("{out_dir}/{id}.bin"), bytes)?;
    }
    println!(
        "cloned interface {IFACE_GROUP} \"Magic Book\" frame (com{SRC_ROOT}+{SRC_CHILDREN:?}) -> NECRO action window (com{DST_ROOT}+{DST_CHILDREN:?}): {} components (onload {SRC_WINDOW_ID} -> {DST_WINDOW_ID})",
        produced.len()
    );
    Ok(())
}
