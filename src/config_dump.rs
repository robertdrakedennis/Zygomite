// Text-format config dumps matching the zwyz-rs3-cache output format.
// Consumable by CacheOverlay.ts SemanticRepository for dependency scanning.
use crate::cache::FlatCache;
use crate::constants::*;
use crate::config::SpotOp;
use crate::js5;
use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn dump_config_texts(cache: &FlatCache, out_dir: &Path, build: u32) -> Result<usize> {
    let dir = out_dir.join("config");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut wrote = 0usize;

    // ── OpList types: rewrite values with type prefixes ──
    macro_rules! dump_ops {
        ($name:expr, $archive:expr, $group:expr, $parse:expr) => {
            let path = dir.join(concat!("dump.", $name));
            let f = fs::File::create(&path)?;
            let mut w = std::io::BufWriter::new(f);
            if let Some(data) = cache.get($archive, $group)? {
                let index = cache.archive_index($archive)?;
                for (id, bytes) in js5::unpack_group(&index, $group, &data)? {
                    let entry = $parse(id, &bytes)?;
                    write!(w, "[{}_{id}]\n", $name)?;
                    for op in &entry.ops {
                        write!(w, "{}\n", rewrite_op_value(op))?;
                    }
                    write!(w, "\n")?;
                }
            }
            wrote += 1;
        };
    }

    dump_ops!("obj", ARCHIVE_OBJ_CONFIG, 0, |id, d| crate::config::parse_obj(id, d));
    dump_ops!("npc", ARCHIVE_NPC_CONFIG, 0, |id, d| crate::config::parse_npc(id, d));
    dump_ops!("loc", ARCHIVE_LOC_CONFIG, 0, |id, d| crate::config::parse_loc(id, d));
    dump_ops!("bas", ARCHIVE_CONFIG, CONFIG_GROUP_BAS, |id, d| crate::config::parse_bas(id, d, build));

    // Seq
    {
        let path = dir.join("dump.seq");
        let f = fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(f);
        if let Some(data) = cache.get(ARCHIVE_SEQ_CONFIG, 0)? {
            let index = cache.archive_index(ARCHIVE_SEQ_CONFIG)?;
            for (id, bytes) in js5::unpack_group(&index, 0, &data)? {
                let e = crate::config::parse_seq(id, &bytes)?;
                write!(w, "[seq_{id}]\n")?;
                for f in &e.frames { if f.anim_id != 0xFFFF { write!(w, "frame=seq_{}\n", f.anim_id)?; } }
                for f in &e.iframes { if f.anim_id != 0xFFFF { write!(w, "iframe=seq_{}\n", f.anim_id)?; } }
                for &id in &e.walkmerge { if id != 0xFFFF { write!(w, "walkmerge=seq_{id}\n")?; } }
                if let Some(g) = e.group { write!(w, "group=seqgroup_{g}\n")?; }
                for p in &e.params { write!(w, "param=param_{}\n", p.param_id)?; }
                write!(w, "\n")?;
            }
        }
        wrote += 1;
    }

    // Spot
    {
        let path = dir.join("dump.spot");
        let f = fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(f);
        if let Some(data) = cache.get(ARCHIVE_SPOT_CONFIG, 0)? {
            let index = cache.archive_index(ARCHIVE_SPOT_CONFIG)?;
            for (id, bytes) in js5::unpack_group(&index, 0, &data)? {
                let e = crate::config::parse_spot(id, &bytes)?;
                write!(w, "[spotanim_{id}]\n")?;
                for op in &e.ops {
                    match op {
                        SpotOp::Model(m) if *m >= 0 => write!(w, "model=model_{m}\n")?,
                        SpotOp::Anim(a) if *a >= 0 => write!(w, "anim=seq_{a}\n")?,
                        _ => {}
                    }
                }
                write!(w, "\n")?;
            }
        }
        wrote += 1;
    }

    // Struct
    {
        let path = dir.join("dump.struct");
        let f = fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(f);
        if let Some(data) = cache.get(ARCHIVE_STRUCT_CONFIG, 0)? {
            let index = cache.archive_index(ARCHIVE_STRUCT_CONFIG)?;
            for (id, bytes) in js5::unpack_group(&index, 0, &data)? {
                let e = crate::config::parse_struct(id, &bytes)?;
                write!(w, "[struct_{id}]\n")?;
                for p in &e.params { write!(w, "param=param_{}\n", p.param_id)?; }
                write!(w, "\n")?;
            }
        }
        wrote += 1;
    }

    // Enums — multi-group archive 17
    {
        let path = dir.join("dump.enum");
        let f = fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(f);
        let index = cache.archive_index(ARCHIVE_ENUM_CONFIG)?;
        for &group in &index.group_id {
            if let Some(data) = cache.get(ARCHIVE_ENUM_CONFIG, group)? {
                for (id, bytes) in js5::unpack_group(&index, group, &data)? {
                    let e = crate::config::parse_enum(id, &bytes)?;
                    write!(w, "[enum_{id}]\n")?;
                    if let Some(t) = e.input_type_id { write!(w, "inputtype=param_{t}\n")?; }
                    if let Some(t) = e.output_type_id { write!(w, "outputtype=param_{t}\n")?; }
                    write!(w, "\n")?;
                }
            }
        }
        wrote += 1;
    }

    // Varps — per-domain
    let varp_groups: &[(&str, u32, crate::vars::VarDomain)] = &[
        ("var_player", CONFIG_GROUP_VAR_PLAYER, crate::vars::VarDomain::Player),
        ("var_npc", CONFIG_GROUP_VAR_NPC, crate::vars::VarDomain::Npc),
        ("var_client", CONFIG_GROUP_VAR_CLIENT, crate::vars::VarDomain::Client),
        ("var_world", CONFIG_GROUP_VAR_WORLD, crate::vars::VarDomain::World),
        ("var_region", CONFIG_GROUP_VAR_REGION, crate::vars::VarDomain::Region),
        ("var_object", CONFIG_GROUP_VAR_OBJECT, crate::vars::VarDomain::Object),
        ("var_clan", CONFIG_GROUP_VAR_CLAN, crate::vars::VarDomain::Clan),
        ("var_clan_setting", CONFIG_GROUP_VAR_CLAN_SETTING, crate::vars::VarDomain::ClanSetting),
        ("var_controller", CONFIG_GROUP_VAR_CONTROLLER, crate::vars::VarDomain::Controller),
        ("var_global", CONFIG_GROUP_VAR_GLOBAL, crate::vars::VarDomain::Global),
        ("var_player_group", CONFIG_GROUP_VAR_PLAYER_GROUP, crate::vars::VarDomain::PlayerGroup),
    ];
    for (domain_name, group, domain) in varp_groups {
        let path = dir.join(format!("dump.{domain_name}"));
        let f = fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(f);
        if let Some(data) = cache.get(ARCHIVE_CONFIG, *group)? {
            let index = cache.archive_index(ARCHIVE_CONFIG)?;
            for (id, bytes) in js5::unpack_group(&index, *group, &data)? {
                let e = crate::vars::parse_var(*domain, id, &bytes)?;
                write!(w, "[{}_{id}]\n", domain_name)?;
                if let Some(t) = e.type_id { write!(w, "type=param_{t}\n")?; }
                write!(w, "\n")?;
            }
        }
        wrote += 1;
    }

    // Varbits
    {
        let path = dir.join("dump.varbit");
        let f = fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(f);
        if let Some(data) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_BIT)? {
            let index = cache.archive_index(ARCHIVE_CONFIG)?;
            for (id, bytes) in js5::unpack_group(&index, CONFIG_GROUP_VAR_BIT, &data)? {
                let e = crate::vars::parse_varbit(id, &bytes)?;
                write!(w, "[varbit_{id}]\n")?;
                if let Some(b) = e.base_var { write!(w, "basevar=var_player_{b}\n")?; }
                write!(w, "\n")?;
            }
        }
        wrote += 1;
    }

    // Params
    {
        let path = dir.join("dump.param");
        let f = fs::File::create(&path)?;
        let mut w = std::io::BufWriter::new(f);
        if let Some(data) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_PARAM)? {
            let index = cache.archive_index(ARCHIVE_CONFIG)?;
            for (id, bytes) in js5::unpack_group(&index, CONFIG_GROUP_PARAM, &data)? {
                let e = crate::config::parse_param(id, &bytes)?;
                write!(w, "[param_{id}]\n")?;
                if let Some(t) = e.type_id { write!(w, "typeid={t}\n")?; }
                write!(w, "\n")?;
            }
        }
        wrote += 1;
    }

    eprintln!("Wrote {wrote} text dumps to {}", dir.display());
    Ok(wrote)
}

/// Rewrite an op string to include type-prefixed values for dependency scanning.
/// e.g., "model=2595" → "model=model_2595"
fn rewrite_op_value(op: &str) -> String {
    if let Some((key, value)) = op.split_once('=') {
        if let Some(prefix) = key_to_ref_prefix(key) {
            let num = value.split([',', ' ']).next().unwrap_or(value);
            if num.parse::<i32>().is_ok() {
                return format!("{key}={prefix}_{num}{}", &value[num.len()..]);
            }
        }
    }
    op.to_string()
}

fn key_to_ref_prefix(key: &str) -> Option<&'static str> {
    match key {
        "model" | "manwear" | "manwear2" | "manwear3" | "womanwear" | "womanwear2"
        | "womanwear3" | "manhead" | "manhead2" | "womanhead" | "womanhead2"
        | "covermarker" | "modela" | "modelb" | "model1" | "model2" | "model3"
        | "model4" | "model5" | "model6" | "model7" | "model8" => Some("model"),
        "anim" | "readyanim" | "walkanim" | "turnleftanim" | "turnrightanim"
        | "crawlanim" | "crawlanim_b" | "crawlanim_l" | "crawlanim_r" | "runanim"
        | "runanim_b" | "runanim_l" | "runanim_r" | "walkanim_b" | "walkanim_l"
        | "walkanim_r" | "readyanim_l" | "readyanim_r" | "crawlturn_l" | "crawlturn_r"
        | "runturn_l" | "runturn_r" | "walkturn_l" | "walkturn_r" => Some("seq"),
        "bas" => Some("bas"),
        "msi" => Some("msi"),
        "vfx" => Some("vfx"),
        "quest" => Some("quest"),
        "cursor1" | "cursor2" | "cursor3" | "cursor4" | "cursor5" | "cursor6"
        | "icursor1" | "icursor2" | "icursor3" | "icursor4" | "icursor5"
        | "cursorattack" => Some("cursor"),
        "texture" | "bloomtexture" => Some("texture"),
        "material" => Some("material"),
        "certlink" | "certtemplate" | "lentlink" | "lenttemplate" | "boughtlink"
        | "boughttemplate" | "shardlink" | "shardtemplate" | "placeholderlink"
        | "placeholdertemplate" => Some("obj"),
        "multiloc" => Some("loc"),
        "multinpc" => Some("npc"),
        "multimodel" => Some("model"),
        _ => None,
    }
}
