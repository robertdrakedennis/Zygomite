//! Clone the 910 HUD-ribbon DEFENSIVE flyout icon component set
//! (interface_1477 com72..com76) into a NEW Necromancy flyout icon set
//! (com856..com860), driven entirely by the live 910 cache.
//!
//! Combat Style Modernisation — the 6th (Necromancy) combat style in the HUD
//! ribbon Powers flyout. The flyout strip is built by the fully data-driven
//! [proc,script8781] (it iterates enum_7717, resolves each combatId -> layout
//! struct via script10405/enum_7716, and shows struct_param(struct, param_3503)
//! as the icon). combatId 1039 is a NATIVE reserved Necromancy slot in 910:
//! it is already iterated by enum_7717 (index 77) and already has position
//! storage in script8701/8709 + a fallback position struct in enum_7712
//! (1039 -> struct_16567). The ONLY missing pieces are (a) enum_7716 has no
//! 1039 -> struct mapping (so script10405 returns null and 8781 skips it), and
//! (b) the icon component it would point at does not exist.
//!
//! This example produces (b): a verbatim codec-clone of DEFENSIVE's flyout icon
//! component set. DEFENSIVE (1034) uses interface_1477:com72 (the icon root,
//! onload=script8409(1034), a 352x128 layer under com50) with child layers
//! com73/com74/com75/com76. We clone all five to com856..com860:
//!   - every child's parent `layer` ref com72 -> com856 (the new root),
//!   - the root com856's onload arg 1034 -> 1039 (necromancy),
//!   - the root com856 x/y -> a distinct on-screen seed position so the
//!     necro icon does not initialise exactly on top of DEFENSIVE's icon
//!     (script8707 reads the icon component's x/y to seed the position
//!     varclientbits; the user can drag it afterwards in the live client).
//! Each emitted component is re-encoded with the interface codec and verified
//! (decode/encode idempotency).
//!
//! Output: <out-dir>/<component-id>.bin per NEW component (com856..com860).
//! These are packed into a full interface_1477 group-replace .dat by the
//! TypeScript builder (build-necro-ribbon.ts).
//!
//! Usage: cargo run --example gen_necro_ribbon_icon -- <910-cache> <out-dir>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_INTERFACES;
use rs3_cache_rs::error::{CacheError, Result};
use rs3_cache_rs::interface_codec::{HookArg, decode_raw, encode_raw};
use std::collections::BTreeMap;

const BUILD: u32 = 910;
const IFACE_GROUP: u32 = 1477; // the HUD action-bar/ribbon interface

// DEFENSIVE flyout icon component set (the clone source).
const SRC_ROOT: u32 = 72; // com72: onload=script8409(1034), 352x128 layer under com50
const SRC_CHILDREN: [u32; 4] = [73, 74, 75, 76];

// Necromancy flyout icon component set (the clone destination): the next free
// component ids in interface_1477 (highest native is com855).
const DST_ROOT: u32 = 856;
const DST_CHILDREN: [u32; 4] = [857, 858, 859, 860];

const SRC_COMBAT_ID: i32 = 1034; // DEFENSIVE (the onload arg on com72)
const DST_COMBAT_ID: i32 = 1039; // NECROMANCY (reserved native ribbon slot)

// Distinct on-screen seed position for the necro icon root. DEFENSIVE's com72 is
// at x=124,y=121; PRAYER's com77 is at x=331,y=71. We seed necro slightly offset
// inside the same cluster so it is visible and not exactly overlapping. The icon
// is freely repositionable in-client (script8707 -> script8709 -> varclientbits).
const DST_ROOT_X: i16 = 160;
const DST_ROOT_Y: i16 = 160;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: gen_necro_ribbon_icon <910-cache> <out-dir>");
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

    // The codec stores the `layer` parent as a plain SUB-ID within the same
    // group (e.g. com73.layer == 72), not a full (group<<16)|sub component ref.
    let src_root_layer_ref = i32::try_from(SRC_ROOT).expect("sub-id fits i32");
    let dst_root_layer_ref = i32::try_from(DST_ROOT).expect("sub-id fits i32");

    let mut produced: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let mut rewrote_onload = false;
    let mut rewrote_parent = false;

    for (&src_sub, &dst_sub) in &remap {
        let bytes = files.get(&src_sub).ok_or_else(|| {
            CacheError::message(format!("source component com{src_sub} missing in group {IFACE_GROUP}"))
        })?;
        let mut component = decode_raw(bytes, BUILD)?;

        // Re-parent any child whose layer ref is the source root -> new root.
        if component.layer == src_root_layer_ref {
            component.layer = dst_root_layer_ref;
            rewrote_parent = true;
        }

        // On the root, retarget the onload combat-id arg (DEFENSIVE -> NECRO) and
        // set a distinct seed position. The root keeps its parent (com50) intact.
        if src_sub == SRC_ROOT {
            for hook in component.hooks.iter_mut().flatten() {
                if hook.script == 8409 {
                    for arg in &mut hook.args {
                        if let HookArg::Int(v) = arg {
                            if *v == SRC_COMBAT_ID {
                                *v = DST_COMBAT_ID;
                                rewrote_onload = true;
                            }
                        }
                    }
                }
            }
            component.x = DST_ROOT_X;
            component.y = DST_ROOT_Y;
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
            "expected to rewrite com72 onload combat-id arg 1034 -> 1039 (1477 layout changed?)".to_string(),
        ));
    }
    if !rewrote_parent {
        return Err(CacheError::message(
            "expected to re-parent at least one child layer com72 -> com856 (1477 layout changed?)".to_string(),
        ));
    }

    for (id, bytes) in &produced {
        std::fs::write(format!("{out_dir}/{id}.bin"), bytes)?;
    }
    println!(
        "cloned interface {IFACE_GROUP} DEFENSIVE icon (com{SRC_ROOT}+{SRC_CHILDREN:?}) -> NECRO icon (com{DST_ROOT}+{DST_CHILDREN:?}): {} components (onload 1034 -> 1039, seed x={DST_ROOT_X} y={DST_ROOT_Y})",
        produced.len()
    );
    Ok(())
}
