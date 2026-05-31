//! Print the skill-guide (interface 1218) grid: for each slot button
//! (`onop = skillguide_setskill(id)`) show its skill id, x, y, parent layer,
//! and the icon graphic of the slot's `graphic` child.
//!
//! Usage: cargo run --example `skillguide_grid` -- <cache-dir> <build>

use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::constants::ARCHIVE_INTERFACES;
use rs3_cache_rs::error::Result;
use rs3_cache_rs::interface_codec::{Body, HookArg, decode_raw};
use std::collections::BTreeMap;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cache_dir = args.next().expect("usage: <cache-dir> <build>");
    let build: u32 = args
        .next()
        .expect("usage: <cache-dir> <build>")
        .parse()
        .unwrap();

    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, 1218)?;

    // First pass: index every graphic component by parent layer so we can find
    // each slot's icon.
    let mut icon_by_parent: BTreeMap<i32, i32> = BTreeMap::default();
    for data in files.values() {
        if data.get(1).copied().unwrap_or(0) & 0x7F != 5 {
            continue;
        }
        if let Ok(c) = decode_raw(data, build)
            && let Body::Graphic(g) = &c.body
        {
            icon_by_parent.insert(c.layer, g.graphic);
        }
    }

    println!("build {build}: {} components in 1218", files.len());
    println!(
        "{:>4} {:>5} {:>4} {:>4} {:>6} {:>8}",
        "com", "skill", "x", "y", "parent", "icon"
    );
    let mut rows = Vec::new();
    for (comp, data) in &files {
        if data.get(1).copied().unwrap_or(0) & 0x7F != 0 {
            continue;
        }
        let Ok(c) = decode_raw(data, build) else {
            continue;
        };
        // The onop hook holds skillguide_setskill(id); find a layer whose onop
        // has a single int arg (the skill id).
        let mut skill = None;
        for hook in c.hooks.iter().flatten() {
            if let [HookArg::Int(id)] = hook.args.as_slice() {
                // setskill hooks pass exactly one int (the skill id).
                skill = Some(*id);
            }
        }
        let Some(skill) = skill else { continue };
        let icon = icon_by_parent
            .get(&i32::try_from(*comp)?)
            .copied()
            .unwrap_or(-2);
        rows.push((skill, *comp, c.x, c.y, c.layer, icon));
    }
    rows.sort_by_key(|r| r.0);
    for (skill, comp, x, y, parent, icon) in rows {
        println!("{comp:>4} {skill:>5} {x:>4} {y:>4} com{parent:<4} {icon:>8}");
    }
    Ok(())
}
