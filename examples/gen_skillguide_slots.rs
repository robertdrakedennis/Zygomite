//! Regenerate interface 1218's skill-guide grid for 29 skills, driven entirely
//! by the live 947 cache layout (zero hand-transcribed coordinates).
//!
//! 910 ships a 3-col x 9-row grid (40px spacing) for 27 skills — full. 947 fits
//! 29 skills by recompressing to 3 cols x 10 rows (36px spacing), all inside the
//! same 360px container. We replicate that exact layout:
//!
//!   * the 27 existing 910 slot buttons are REPOSITIONED to 947's (x,y) for the
//!     same skill id (only the button moves; its rectangle/states/icon are
//!     anchored `abs_centre` to it), and
//!   * two new slots (com251..com266) are cloned from 910's native Woodcutting
//!     slot template (com7..com14) and placed at 947's coords for skills 28
//!     (Archaeology) and 29 (Necromancy), with skill id, hover name and icon all
//!     read from 947.
//!
//! Every emitted component is re-encoded with the interface codec and verified
//! two ways (decode/encode idempotency + clean text-parse). Output blobs are
//! written as <component-id>.bin; build-merged-1218.ts overrides/appends them.
//!
//! Usage: cargo run --example `gen_skillguide_slots` -- <910-cache> <947-cache> <out-dir>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_INTERFACES;
use rs3_cache_rs::error::{CacheError, Result};
use rs3_cache_rs::interface::parse_component;
use rs3_cache_rs::interface_codec::{Body, Component, HookArg, decode_raw, encode_raw};
use std::collections::BTreeMap;

const BUILD_910: u32 = 910;
const BUILD_947: u32 = 947;
const GROUP: u32 = 1218;
const NEW_SKILLS: [(i32, u32); 2] = [(28, 251), (29, 259)]; // (skill id, first component id)

struct Grid {
    /// skill id -> grid button component id
    buttons: BTreeMap<i32, u32>,
    /// skill id -> (x, y)
    positions: BTreeMap<i32, (i16, i16)>,
    /// skill id -> hover name (from onmouserepeat string arg)
    names: BTreeMap<i32, String>,
    /// skill id -> icon graphic id (from the slot's graphic child)
    icons: BTreeMap<i32, i32>,
}

fn load_group(cache_dir: &str) -> Result<BTreeMap<u32, Vec<u8>>> {
    let cache = FlatCache::open(cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    cache.group_files_with_index(&index, ARCHIVE_INTERFACES, GROUP)
}

/// `onop = skillguide_setskill(<id>)` — the only hook with exactly one int arg.
fn button_skill(c: &Component) -> Option<i32> {
    c.hooks
        .iter()
        .flatten()
        .find_map(|h| match h.args.as_slice() {
            [HookArg::Int(id)] => Some(*id),
            _ => None,
        })
}

/// `onmouserepeat = script8799("<name>", event_com, -1)` — leading string arg.
fn button_name(c: &Component) -> Option<String> {
    c.hooks.iter().flatten().find_map(|h| match h.args.first() {
        Some(HookArg::Str(name)) => Some(name.clone()),
        _ => None,
    })
}

fn read_grid(files: &BTreeMap<u32, Vec<u8>>, build: u32) -> Result<Grid> {
    // icon graphic (type 5) keyed by its parent layer (the button component id).
    let mut icon_by_parent: BTreeMap<i32, i32> = BTreeMap::new();
    for data in files.values() {
        if data.get(1).copied().unwrap_or(0) & 0x7F == 5
            && let Ok(c) = decode_raw(data, build)
            && let Body::Graphic(g) = &c.body
        {
            icon_by_parent.insert(c.layer, g.graphic);
        }
    }

    let mut grid = Grid {
        buttons: BTreeMap::new(),
        positions: BTreeMap::new(),
        names: BTreeMap::new(),
        icons: BTreeMap::new(),
    };
    for (comp, data) in files {
        if data.get(1).copied().unwrap_or(0) & 0x7F != 0 {
            continue;
        }
        let Ok(c) = decode_raw(data, build) else {
            continue;
        };
        let Some(skill) = button_skill(&c) else {
            continue;
        };
        if !(1..=29).contains(&skill) {
            continue; // skip non-slot single-int hooks (e.g. event_com sentinels)
        }
        grid.buttons.insert(skill, *comp);
        grid.positions.insert(skill, (c.x, c.y));
        if let Some(name) = button_name(&c) {
            grid.names.insert(skill, name);
        }
        if let Some(icon) = icon_by_parent.get(&i32::try_from(*comp)?) {
            grid.icons.insert(skill, *icon);
        }
    }
    Ok(grid)
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cache_910 = args
        .next()
        .expect("usage: <910-cache> <947-cache> <out-dir>");
    let cache_947 = args
        .next()
        .expect("usage: <910-cache> <947-cache> <out-dir>");
    let out_dir = args
        .next()
        .expect("usage: <910-cache> <947-cache> <out-dir>");
    std::fs::create_dir_all(&out_dir)?;

    let files_910 = load_group(&cache_910)?;
    let files_947 = load_group(&cache_947)?;
    let grid_947 = read_grid(&files_947, BUILD_947)?;
    let grid_910 = read_grid(&files_910, BUILD_910)?;

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

    // ── A. Reposition the 27 existing slot buttons to 947's coordinates ──
    for (&skill, &button_comp) in &grid_910.buttons {
        let &(x, y) = grid_947
            .positions
            .get(&skill)
            .unwrap_or_else(|| panic!("947 grid has no position for skill {skill}"));
        let mut button = decode(button_comp);
        if button.x == x && button.y == y {
            continue; // already aligned — nothing to override
        }
        button.x = x;
        button.y = y;
        emit(&mut produced, button_comp, &button)?;
    }

    // ── B. Two new slots cloned from the native template, placed per 947 ──
    let button_tpl = decode(7);
    let rect_tpl = decode(8);
    let state_tpls: Vec<Component> = (9..=13).map(decode).collect();
    let graphic_tpl = decode(14);

    for (skill, base_id) in NEW_SKILLS {
        let &(x, y) = grid_947
            .positions
            .get(&skill)
            .unwrap_or_else(|| panic!("947 grid has no position for new skill {skill}"));
        let name = grid_947
            .names
            .get(&skill)
            .unwrap_or_else(|| panic!("947 grid has no name for skill {skill}"))
            .clone();
        let icon = *grid_947
            .icons
            .get(&skill)
            .unwrap_or_else(|| panic!("947 grid has no icon for skill {skill}"));

        let mut button = button_tpl.clone();
        button.x = x;
        button.y = y;
        for hook in button.hooks.iter_mut().flatten() {
            match hook.args.as_mut_slice() {
                [HookArg::Int(id)] => *id = skill,
                [HookArg::Str(n), ..] => n.clone_from(&name),
                _ => {}
            }
        }
        emit(&mut produced, base_id, &button)?;

        let mut rect = rect_tpl.clone();
        rect.layer = i32::try_from(base_id)?;
        // The selection-highlight rectangle (com8 clone) is visible by default in the
        // template; the native skillguide_setskill (5683) is a hardcoded switch over
        // skills 1-27 that hides every slot's rectangle then unhides the selected one,
        // and it never references our new component ids. Default this rectangle to
        // hidden so it isn't stuck-on; the server (InterfaceManager) toggles it per
        // selected skill for ids 28/29.
        rect.flags |= 1; // hide=yes
        emit(&mut produced, base_id + 1, &rect)?;

        for (i, tpl) in state_tpls.iter().enumerate() {
            let mut state = tpl.clone();
            state.layer = i32::try_from(base_id)?;
            emit(&mut produced, base_id + 2 + u32::try_from(i)?, &state)?;
        }

        let mut graphic = graphic_tpl.clone();
        graphic.layer = i32::try_from(base_id)?;
        if let Body::Graphic(sprite) = &mut graphic.body {
            sprite.graphic = icon;
        }
        emit(&mut produced, base_id + 7, &graphic)?;

        println!(
            "new slot skill {skill} \"{name}\": button com{base_id} at ({x},{y}) icon {icon} -> com{base_id}..com{}",
            base_id + 7
        );
    }

    let repositioned = produced.range(..251).count();
    for (id, bytes) in &produced {
        std::fs::write(format!("{out_dir}/{id}.bin"), bytes)?;
    }
    println!(
        "wrote {} blobs ({repositioned} repositioned existing buttons + 16 new components) to {out_dir}",
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
