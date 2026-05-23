// Dependency reference maps for every config type the alerion cache overlay
// BFS dependency walker needs. Format: refs/{kind}.json → {id: {ref_kind: [ids]}}
use crate::cache::FlatCache;
use crate::config::{SeqEntry, SpotEntry, SpotOp};
use crate::constants::{
    ARCHIVE_CONFIG, ARCHIVE_ENUM_CONFIG, ARCHIVE_LOC_CONFIG, ARCHIVE_NPC_CONFIG, ARCHIVE_OBJ_CONFIG,
    ARCHIVE_SEQ_CONFIG, ARCHIVE_SPOT_CONFIG, ARCHIVE_STRUCT_CONFIG, CONFIG_GROUP_BAS,
    CONFIG_GROUP_DBROW, CONFIG_GROUP_DBTABLE, CONFIG_GROUP_PARAM, CONFIG_GROUP_VAR_BIT,
    CONFIG_GROUP_VAR_CLAN, CONFIG_GROUP_VAR_CLAN_SETTING, CONFIG_GROUP_VAR_CLIENT,
    CONFIG_GROUP_VAR_CONTROLLER, CONFIG_GROUP_VAR_GLOBAL, CONFIG_GROUP_VAR_NPC,
    CONFIG_GROUP_VAR_OBJECT, CONFIG_GROUP_VAR_PLAYER, CONFIG_GROUP_VAR_PLAYER_GROUP,
    CONFIG_GROUP_VAR_REGION, CONFIG_GROUP_VAR_WORLD,
};
use crate::js5;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

type RefMap = BTreeMap<String, BTreeSet<u32>>;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConfigRefGraph {
    pub obj: BTreeMap<u32, RefMap>,
    pub npc: BTreeMap<u32, RefMap>,
    pub loc: BTreeMap<u32, RefMap>,
    pub spot: BTreeMap<u32, RefMap>,
    pub seq: BTreeMap<u32, RefMap>,
    pub bas: BTreeMap<u32, RefMap>,
    pub r#enum: BTreeMap<u32, RefMap>,
    pub r#struct: BTreeMap<u32, RefMap>,
    pub dbtable: BTreeMap<u32, RefMap>,
    pub dbrow: BTreeMap<u32, RefMap>,
    pub varp: BTreeMap<u32, RefMap>,
    pub varbit: BTreeMap<u32, RefMap>,
    pub param: BTreeMap<u32, RefMap>,
}

// ── OpList key-based reference extraction ──

/// Maps opcode key names to the reference entity type they represent.
/// Only keys whose values are numeric IDs of other entities.
fn op_key_to_ref_kind(key: &str) -> Option<&'static str> {
    match key {
        // Model references
        "model" | "manwear" | "manwear2" | "manwear3" | "womanwear" | "womanwear2"
        | "womanwear3" | "manhead" | "manhead2" | "womanhead" | "womanhead2"
        | "covermarker" | "modela" | "modelb" => Some("model"),
        // Seq/animation references
        "anim" | "readyanim" | "walkanim" | "turnleftanim" | "turnrightanim"
        | "crawlanim" | "crawlanim_b" | "crawlanim_l" | "crawlanim_r" | "runanim"
        | "runanim_b" | "runanim_l" | "runanim_r" | "walkanim_b" | "walkanim_l"
        | "walkanim_r" | "readyanim_l" | "readyanim_r" | "crawlturn_l" | "crawlturn_r"
        | "runturn_l" | "runturn_r" | "walkturn_l" | "walkturn_r" | "randomreadyanim" => {
            Some("seq")
        }
        // Other typed references
        "bas" => Some("bas"),
        "msi" => Some("msi"),
        "vfx" => Some("vfx"),
        "quest" => Some("quest"),
        // Obj links
        "certlink" | "certtemplate" | "lentlink" | "lenttemplate" | "boughtlink"
        | "boughttemplate" | "shardlink" | "shardtemplate" | "placeholderlink"
        | "placeholdertemplate" => Some("obj"),
        // Cursors
        "cursor1" | "cursor2" | "cursor3" | "cursor4" | "cursor5" | "cursor6"
        | "icursor1" | "icursor2" | "icursor3" | "icursor4" | "icursor5"
        | "cursorattack" => Some("cursor"),
        // Multivariants
        "multiloc" => Some("loc"),
        "multinpc" => Some("npc"),
        "multimodel" => Some("model"),
        // Materials/textures
        "texture" | "bloomtexture" => Some("texture"),
        "material" => Some("material"),
        // General fallback — any key containing certain prefixes
        _ => None,
    }
}

fn insert_ref_id(refs: &mut RefMap, kind: &str, id: u32) {
    if id == 0xFFFF_FFFF {
        return;
    }
    refs.entry(kind.to_string()).or_default().insert(id);
}

/// `multivar=varbit:7`, `multivar=varp:5`, `condition=varbit:7,0,1`, etc.
fn scan_op_special_refs(refs: &mut RefMap, op: &str) {
    if let Some(rest) = op.strip_prefix("multivar=varbit:") {
        if let Ok(id) = rest.split([',', ' ']).next().unwrap_or(rest).parse::<u32>() {
            insert_ref_id(refs, "multivar_varbit", id);
        }
        return;
    }
    if let Some(rest) = op.strip_prefix("multivar=varp:") {
        if let Ok(id) = rest.split([',', ' ']).next().unwrap_or(rest).parse::<u32>() {
            insert_ref_id(refs, "multivar_varp", id);
        }
        return;
    }
    if let Some(rest) = op.strip_prefix("condition=varbit:") {
        if let Ok(id) = rest.split([',', ' ']).next().unwrap_or(rest).parse::<u32>() {
            insert_ref_id(refs, "varbit", id);
        }
    }
}

fn scan_oplist(ops: &[String]) -> RefMap {
    let mut refs = RefMap::new();
    for op in ops {
        scan_op_special_refs(&mut refs, op);
        if let Some((key, value)) = op.split_once('=') {
            // Truncate value at first comma or space (multi-value fields)
            let num_str = value
                .split([',', ' '])
                .next()
                .unwrap_or(value);
            if let Ok(id) = num_str.parse::<u32>() {
                if let Some(kind) = op_key_to_ref_kind(key) {
                    insert_ref_id(&mut refs, kind, id);
                } else if key.starts_with("head")
                    || key.starts_with("model")
                    || key.starts_with("cursor")
                    || key.starts_with("icursor")
                    || key.starts_with("man")
                    || key.starts_with("woman")
                {
                    // Heuristic: keys with model/cursor/etc. prefixes
                    if let Some(kind) = infer_ref_kind(key) {
                        insert_ref_id(&mut refs, kind, id);
                    }
                }
            }
        }
    }
    refs
}

fn infer_ref_kind(key: &str) -> Option<&'static str> {
    if key.contains("model") || key.contains("head") || key.contains("wear") {
        Some("model")
    } else if key.contains("cursor") {
        Some("cursor")
    } else if key.contains("anim") {
        Some("seq")
    } else {
        None
    }
}

fn scan_seq(entry: &SeqEntry) -> RefMap {
    let mut refs = RefMap::new();
    for f in &entry.frames {
        if f.anim_id != 0xFFFF {
            let id = u32::from(f.anim_id);
            insert_ref_id(&mut refs, "seq", id);
            insert_ref_id(&mut refs, "anim", id);
        }
    }
    for f in &entry.iframes {
        if f.anim_id != 0xFFFF {
            let id = u32::from(f.anim_id);
            insert_ref_id(&mut refs, "seq", id);
            insert_ref_id(&mut refs, "anim", id);
        }
    }
    for &w in &entry.walkmerge {
        if w != 0xFFFF {
            refs.entry("seq".into())
                .or_default()
                .insert(u32::from(w));
        }
    }
    if let Some(g) = entry.group {
        insert_ref_id(&mut refs, "seqgroup", u32::from(g));
    }
    if let Some(keyframeset) = entry.keyframeset {
        if keyframeset != 0xFFFF {
            insert_ref_id(&mut refs, "anim", u32::from(keyframeset));
        }
    }
    for p in &entry.params {
        refs.entry("param".into())
            .or_default()
            .insert(p.param_id);
    }
    refs
}

fn scan_spot(entry: &SpotEntry) -> RefMap {
    let mut refs = RefMap::new();
    for op in &entry.ops {
        match op {
            SpotOp::Model(id) if *id >= 0 => {
                refs.entry("model".into())
                    .or_default()
                    .insert(*id as u32);
            }
            SpotOp::Anim(id) if *id >= 0 => {
                refs.entry("seq".into())
                    .or_default()
                    .insert(*id as u32);
            }
            _ => {}
        }
    }
    refs
}

// ── Archive group iteration helpers ──

fn read_group_files(
    cache: &FlatCache,
    archive: u32,
    group: u32,
) -> Result<Vec<(u32, Vec<u8>)>> {
    let Some(data) = cache.get(archive, group)? else {
        return Ok(Vec::new());
    };
    let decoded_index = cache.archive_index_bytes(archive)?;
    let index = js5::ArchiveIndex::decode(&decoded_index)?;
    let files = js5::unpack_group(&index, group, &data)?;
    Ok(files.into_iter().collect())
}

fn for_each_group_file(
    cache: &FlatCache,
    archive: u32,
    group: u32,
    mut f: impl FnMut(u32, &[u8]) -> Result<()>,
) -> Result<()> {
    for (id, bytes) in read_group_files(cache, archive, group)? {
        f(id, &bytes)?;
    }
    Ok(())
}

// ── Build each section ──

fn build_oplist_section(
    cache: &FlatCache,
    archive: u32,
    group: u32,
    parse: impl Fn(u32, &[u8]) -> Result<crate::config::OpListEntry>,
) -> Result<BTreeMap<u32, RefMap>> {
    let mut map = BTreeMap::new();
    for_each_group_file(cache, archive, group, |id, data| {
        let entry = parse(id, data)?;
        map.insert(id, scan_oplist(&entry.ops));
        Ok(())
    })?;
    Ok(map)
}

fn build_varp_section(cache: &FlatCache, group: u32, domain: crate::vars::VarDomain) -> Result<BTreeMap<u32, RefMap>> {
    let mut map = BTreeMap::new();
    for_each_group_file(cache, ARCHIVE_CONFIG, group, |id, data| {
        let entry = crate::vars::parse_var(domain, id, data)?;
        let mut refs = RefMap::new();
        if let Some(tid) = entry.type_id {
            refs.entry("param".into()).or_default().insert(u32::from(tid));
        }
        map.insert(id, refs);
        Ok(())
    })?;
    Ok(map)
}

#[allow(clippy::field_reassign_with_default)]
pub fn build_config_ref_graph(cache: &FlatCache, build: u32) -> Result<ConfigRefGraph> {
    let mut g = ConfigRefGraph::default();

    g.obj = build_oplist_section(cache, ARCHIVE_OBJ_CONFIG, 0, |id, d| {
        crate::config::parse_obj(id, d)
    })?;
    g.npc = build_oplist_section(cache, ARCHIVE_NPC_CONFIG, 0, |id, d| {
        crate::config::parse_npc(id, d)
    })?;
    g.loc = build_oplist_section(cache, ARCHIVE_LOC_CONFIG, 0, |id, d| {
        crate::config::parse_loc(id, d)
    })?;
    g.bas = build_oplist_section(cache, ARCHIVE_CONFIG, CONFIG_GROUP_BAS, |id, d| {
        crate::config::parse_bas(id, d, build)
    })?;

    // Seq — archive 20 (split)
    g.seq = {
        let mut map = BTreeMap::new();
        for_each_group_file(cache, ARCHIVE_SEQ_CONFIG, 0, |id, data| {
            let entry = crate::config::parse_seq(id, data)?;
            map.insert(id, scan_seq(&entry));
            Ok(())
        })?;
        map
    };

    // Spot — archive 21 (split)
    for_each_group_file(cache, ARCHIVE_SPOT_CONFIG, 0, |id, data| {
        let entry = crate::config::parse_spot(id, data)?;
        g.spot.insert(id, scan_spot(&entry));
        Ok(())
    })?;

    // Struct — archive 22 (split)
    for_each_group_file(cache, ARCHIVE_STRUCT_CONFIG, 0, |id, data| {
        let entry = crate::config::parse_struct(id, data)?;
        let mut refs = RefMap::new();
        for p in &entry.params {
            refs.entry("param".into()).or_default().insert(p.param_id);
        }
        g.r#struct.insert(id, refs);
        Ok(())
    })?;

    // DbTable
    for_each_group_file(cache, ARCHIVE_CONFIG, CONFIG_GROUP_DBTABLE, |id, data| {
        let entry = crate::config::parse_dbtable(id, data)?;
        let mut refs = RefMap::new();
        for col in &entry.columns {
            for &tid in &col.tuple_types {
                refs.entry("param".into()).or_default().insert(u32::from(tid));
            }
        }
        g.dbtable.insert(id, refs);
        Ok(())
    })?;

    // DbRow
    for_each_group_file(cache, ARCHIVE_CONFIG, CONFIG_GROUP_DBROW, |id, data| {
        let entry = crate::config::parse_dbrow(id, data)?;
        let mut refs = RefMap::new();
        if let Some(table_id) = entry.table {
            refs.entry("dbtable".into()).or_default().insert(table_id);
        }
        for col in &entry.columns {
            for &tid in &col.tuple_types {
                refs.entry("param".into()).or_default().insert(u32::from(tid));
            }
        }
        g.dbrow.insert(id, refs);
        Ok(())
    })?;

    // Param
    for_each_group_file(cache, ARCHIVE_CONFIG, CONFIG_GROUP_PARAM, |id, data| {
        let entry = crate::config::parse_param(id, data)?;
        let mut refs = RefMap::new();
        if let Some(tid) = entry.type_id {
            refs.entry("param".into()).or_default().insert(u32::from(tid));
        }
        g.param.insert(id, refs);
        Ok(())
    })?;

    // Varps (all domains)
    for (group, domain) in [
        (CONFIG_GROUP_VAR_PLAYER, crate::vars::VarDomain::Player),
        (CONFIG_GROUP_VAR_NPC, crate::vars::VarDomain::Npc),
        (CONFIG_GROUP_VAR_CLIENT, crate::vars::VarDomain::Client),
        (CONFIG_GROUP_VAR_WORLD, crate::vars::VarDomain::World),
        (CONFIG_GROUP_VAR_REGION, crate::vars::VarDomain::Region),
        (CONFIG_GROUP_VAR_OBJECT, crate::vars::VarDomain::Object),
        (CONFIG_GROUP_VAR_CLAN, crate::vars::VarDomain::Clan),
        (CONFIG_GROUP_VAR_CLAN_SETTING, crate::vars::VarDomain::ClanSetting),
        (CONFIG_GROUP_VAR_CONTROLLER, crate::vars::VarDomain::Controller),
        (CONFIG_GROUP_VAR_GLOBAL, crate::vars::VarDomain::Global),
        (CONFIG_GROUP_VAR_PLAYER_GROUP, crate::vars::VarDomain::PlayerGroup),
    ] {
        let section = build_varp_section(cache, group, domain)?;
        g.varp.extend(section);
    }

    // Varbits
    for_each_group_file(cache, ARCHIVE_CONFIG, CONFIG_GROUP_VAR_BIT, |id, data| {
        let _entry = crate::vars::parse_varbit(id, data)?;
        g.varbit.insert(id, RefMap::new());
        Ok(())
    })?;

    // Enums (archive 17)
    {
        let index = cache.archive_index(ARCHIVE_ENUM_CONFIG)?;
        for &group in &index.group_id {
            for_each_group_file(cache, ARCHIVE_ENUM_CONFIG, group, |id, data| {
                let entry = crate::config::parse_enum(id, data)?;
                let mut refs = RefMap::new();
                if let Some(tid) = entry.input_type_id {
                    refs.entry("param".into()).or_default().insert(u32::from(tid));
                }
                if let Some(tid) = entry.output_type_id {
                    refs.entry("param".into()).or_default().insert(u32::from(tid));
                }
                g.r#enum.insert(id, refs);
                Ok(())
            })?;
        }
    }

    Ok(g)
}

// ── JSON serializer ──

pub fn write_refs_json(graph: &ConfigRefGraph, out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;

    let mut wrote = 0usize;
    macro_rules! write_ref_file {
        ($field:ident, $name:expr) => {
            if !graph.$field.is_empty() {
                let path = out_dir.join(concat!($name, ".json"));
                fs::write(&path, serde_json::to_string_pretty(&graph.$field)?)
                    .with_context(|| format!("writing {}", path.display()))?;
                wrote += 1;
            }
        };
    }
    write_ref_file!(obj, "obj");
    write_ref_file!(npc, "npc");
    write_ref_file!(loc, "loc");
    write_ref_file!(spot, "spot");
    write_ref_file!(seq, "seq");
    write_ref_file!(bas, "bas");
    write_ref_file!(r#enum, "enum");
    write_ref_file!(r#struct, "struct");
    write_ref_file!(dbtable, "dbtable");
    write_ref_file!(dbrow, "dbrow");
    write_ref_file!(varp, "varp");
    write_ref_file!(varbit, "varbit");
    write_ref_file!(param, "param");
    eprintln!("Wrote {wrote} ref files to {}", out_dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::scan_oplist;

    #[test]
    fn scan_oplist_extracts_multivar_varbit_and_varp() {
        let refs = scan_oplist(&[
            "multivar=varbit:7".to_string(),
            "multivar=varp:5".to_string(),
            "model=42".to_string(),
        ]);
        assert_eq!(refs.get("multivar_varbit").map(|s| s.contains(&7)), Some(true));
        assert_eq!(refs.get("multivar_varp").map(|s| s.contains(&5)), Some(true));
        assert_eq!(refs.get("model").map(|s| s.contains(&42)), Some(true));
    }

    #[test]
    fn scan_oplist_extracts_condition_varbit() {
        let refs = scan_oplist(&["condition=varbit:9,0,1".to_string()]);
        assert_eq!(refs.get("varbit").map(|s| s.contains(&9)), Some(true));
    }
}
