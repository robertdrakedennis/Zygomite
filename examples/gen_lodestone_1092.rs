//! Lodestone Network — graft the 948 map + the three post-910 lodestones onto
//! 910's NATIVE interface 1092, driven entirely by the live 948 cache layout
//! (zero hand-transcribed coordinates).
//!
//! 948 redrew the lodestone map (`graphic_35673`, 560x320 vs 910's 23194 at
//! 496x294) and added Fort Forinthry (27103), City of Um (31776) and
//! Wendlewick (35676). Its component numbering, however, shifts every
//! destination (948 com8..com40 vs 910 com9..com35), which would break 910's
//! scripts 6001/6002/11674 (hardcoded packed component ids) and the server's
//! click table. The 910 central-window host is also narrower than 948's
//! 576x360 dialog, so the 560x320 map cannot render 1:1 without clipping the
//! edge lodestones (Lunar Isle / Wendlewick) — live-verified. So instead of
//! splicing the donor group we keep 910's numbering AND its 496x294 map
//! viewport, scaling the 948 layout into it:
//!
//!   * com7 (map graphic) keeps its 496x294 rect but swaps the art to 948's
//!     `graphic_35673` (if3 graphics render scaled to the component rect),
//!   * every destination icon com9..com35 is REPOSITIONED to 948's (x,y) for
//!     the same icon graphic, scaled by 496/560 x 294/320 to match the
//!     downscaled map art,
//!   * append com60 (hidden filler layer keeping the file roster contiguous —
//!     the server treats component 60 as a close fallback, so it must never
//!     be interactive), and three new icons cloned from the native Menaphos
//!     icon (com23): com61 Fort Forinthry, com62 City of Um, com63 Wendlewick
//!     at 948's scaled positions.
//!
//! The matching sprite groups (27050/27103/31775/31776/35673/35675/35676) are
//! free ids in 910 and are spliced verbatim from the 948 donor; scripts
//! 6002/6003/11674 gain zero-shift appended cases (see
//! cache-patches/lodestone-948/).
//!
//! Every emitted component is re-encoded with the interface codec and verified
//! two ways (decode/encode idempotency + clean text-parse). Output blobs are
//! written as <component-id>.bin; build-merged-1092.ts overrides/appends them.
//!
//! Usage: cargo run --example `gen_lodestone_1092` -- <910-cache> <948-cache> <out-dir>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_INTERFACES;
use rs3_cache_rs::error::{CacheError, Result};
use rs3_cache_rs::interface::parse_component;
use rs3_cache_rs::interface_codec::{Body, Component, decode_raw, encode_raw};
use std::collections::BTreeMap;

const BUILD_910: u32 = 910;
const BUILD_948: u32 = 948;
const GROUP: u32 = 1092;

const MAP_GRAPHIC: u32 = 7; // 910 com7 — the map art graphic (rect stays 496x294)
const MAP_SPRITE_948: i32 = 35673;
// 948 lays icons out on its 560x320 map; 910's viewport is 496x294. Scale the
// 948 positions down so they track the downscaled map art.
const MAP_948_W: f64 = 560.0;
const MAP_948_H: f64 = 320.0;
const MAP_910_W: f64 = 496.0;
const MAP_910_H: f64 = 294.0;

fn scale_pos(x: i16, y: i16) -> (i16, i16) {
    let sx = (f64::from(x) * MAP_910_W / MAP_948_W).round();
    let sy = (f64::from(y) * MAP_910_H / MAP_948_H).round();
    #[allow(clippy::cast_possible_truncation)]
    (sx as i16, sy as i16)
}

/// 910 destination icon components (incl. the hidden J-Mod icon com35).
const ICON_COMPONENTS: [u32; 27] = [
    9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    33, 34, 35,
];
const ICON_TEMPLATE: u32 = 23; // Menaphos — graphic comp with both teleport ops
const FILLER_TEMPLATE: u32 = 37; // hidden 50x50 hover-ring layer
const FILLER_ID: u32 = 60;

/// (new component id, 948 icon graphic, name — for the log only)
const NEW_LODESTONES: [(u32, i32, &str); 3] = [
    (61, 27103, "Fort Forinthry"),
    (62, 31776, "City of Um"),
    (63, 35676, "Wendlewick"),
];

fn load_group(cache_dir: &str) -> Result<BTreeMap<u32, Vec<u8>>> {
    let cache = FlatCache::open(cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    cache.group_files_with_index(&index, ARCHIVE_INTERFACES, GROUP)
}

/// 948 icon graphic id -> (x, y) on the 560x320 map.
fn read_948_positions(files: &BTreeMap<u32, Vec<u8>>) -> BTreeMap<i32, (i16, i16)> {
    let mut positions = BTreeMap::new();
    for data in files.values() {
        if data.get(1).copied().unwrap_or(0) & 0x7F != 5 {
            continue;
        }
        let Ok(c) = decode_raw(data, BUILD_948) else {
            continue;
        };
        if let Body::Graphic(g) = &c.body
            && c.width == 40
            && c.height == 40
        {
            positions.insert(g.graphic, (c.x, c.y));
        }
    }
    positions
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cache_910 = args
        .next()
        .expect("usage: <910-cache> <948-cache> <out-dir>");
    let cache_948 = args
        .next()
        .expect("usage: <910-cache> <948-cache> <out-dir>");
    let out_dir = args
        .next()
        .expect("usage: <910-cache> <948-cache> <out-dir>");
    std::fs::create_dir_all(&out_dir)?;

    let files_910 = load_group(&cache_910)?;
    let files_948 = load_group(&cache_948)?;
    let positions_948 = read_948_positions(&files_948);

    let decode = |id: u32| -> Component {
        decode_raw(
            files_910
                .get(&id)
                .unwrap_or_else(|| panic!("910 com{id} missing")),
            BUILD_910,
        )
        .unwrap_or_else(|e| panic!("decode 910 com{id}: {e}"))
    };

    let mut produced: BTreeMap<u32, Vec<u8>> = BTreeMap::new();

    // ── A. Swap the map art to 948's redrawn sprite (rect stays 496x294) ──
    let mut map_graphic = decode(MAP_GRAPHIC);
    if let Body::Graphic(sprite) = &mut map_graphic.body {
        sprite.graphic = MAP_SPRITE_948;
    } else {
        return Err(CacheError::message("com7 is not a graphic component"));
    }
    emit(&mut produced, MAP_GRAPHIC, &map_graphic)?;

    // ── B. Reposition the 27 existing icons to 948's scaled (x,y), matched by icon ──
    for comp in ICON_COMPONENTS {
        let mut icon = decode(comp);
        let Body::Graphic(sprite) = &icon.body else {
            panic!("910 com{comp} is not a graphic component");
        };
        let &(x948, y948) = positions_948
            .get(&sprite.graphic)
            .unwrap_or_else(|| panic!("948 map has no icon with graphic {}", sprite.graphic));
        let (x, y) = scale_pos(x948, y948);
        if icon.x == x && icon.y == y {
            continue; // already aligned — nothing to override
        }
        icon.x = x;
        icon.y = y;
        emit(&mut produced, comp, &icon)?;
    }

    // ── C. com60 filler (hidden, non-interactive) + the three new lodestones ──
    let filler = decode(FILLER_TEMPLATE);
    emit(&mut produced, FILLER_ID, &filler)?;

    let icon_tpl = decode(ICON_TEMPLATE);
    for (comp, graphic, name) in NEW_LODESTONES {
        let &(x948, y948) = positions_948
            .get(&graphic)
            .unwrap_or_else(|| panic!("948 map has no icon with graphic {graphic}"));
        let (x, y) = scale_pos(x948, y948);
        let mut icon = icon_tpl.clone();
        icon.x = x;
        icon.y = y;
        if let Body::Graphic(sprite) = &mut icon.body {
            sprite.graphic = graphic;
        }
        emit(&mut produced, comp, &icon)?;
        println!("new lodestone \"{name}\": com{comp} at ({x},{y}) icon {graphic}");
    }

    let repositioned = produced.range(..FILLER_ID).count();
    for (id, bytes) in &produced {
        std::fs::write(format!("{out_dir}/{id}.bin"), bytes)?;
    }
    println!(
        "wrote {} blobs ({repositioned} map/icon overrides + 4 appended components) to {out_dir}",
        produced.len()
    );
    Ok(())
}

/// Encode, verify round-trip idempotency + clean text-parse, then store.
fn emit(out: &mut BTreeMap<u32, Vec<u8>>, id: u32, component: &Component) -> Result<()> {
    let bytes = encode_raw(component, BUILD_910)?;
    let reparsed = decode_raw(&bytes, BUILD_910)?;
    if &reparsed != component {
        return Err(CacheError::message(format!(
            "com{id} failed decode/encode idempotency"
        )));
    }
    parse_component(id, &bytes, BUILD_910)
        .map_err(|e| CacheError::message(format!("com{id} text-parse: {e}")))?;
    out.insert(id, bytes);
    Ok(())
}
