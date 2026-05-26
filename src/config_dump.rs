// Text config dumps (legacy Java tool format; kept for human inspection via dump-configs).
// CacheOverlay.ts SemanticRepository consumes these for dependency scanning.
use crate::cache::FlatCache;
use crate::config::SpotOp;
use crate::constants::{
    ARCHIVE_CONFIG, ARCHIVE_ENUM_CONFIG, ARCHIVE_LOC_CONFIG, ARCHIVE_NPC_CONFIG,
    ARCHIVE_OBJ_CONFIG, ARCHIVE_SEQ_CONFIG, ARCHIVE_SPOT_CONFIG, ARCHIVE_STRUCT_CONFIG,
    CONFIG_GROUP_BAS, CONFIG_GROUP_LOC_LEGACY, CONFIG_GROUP_NPC_LEGACY, CONFIG_GROUP_OBJ_LEGACY,
    CONFIG_GROUP_PARAM, CONFIG_GROUP_SEQ, CONFIG_GROUP_SPOT, CONFIG_GROUP_STRUCT,
    CONFIG_GROUP_VAR_BIT, CONFIG_GROUP_VAR_CLAN, CONFIG_GROUP_VAR_CLAN_SETTING,
    CONFIG_GROUP_VAR_CLIENT, CONFIG_GROUP_VAR_CONTROLLER, CONFIG_GROUP_VAR_GLOBAL,
    CONFIG_GROUP_VAR_NPC, CONFIG_GROUP_VAR_OBJECT, CONFIG_GROUP_VAR_PLAYER,
    CONFIG_GROUP_VAR_PLAYER_GROUP, CONFIG_GROUP_VAR_REGION, CONFIG_GROUP_VAR_WORLD,
};
use crate::error::{Context, Result};
use crate::js5;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

pub fn dump_config_texts(cache: &FlatCache, out_dir: &Path, build: u32) -> Result<usize> {
    let dir = out_dir.join("config");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut wrote = 0usize;

    dump_split_ops(
        cache,
        &dir,
        "obj",
        ARCHIVE_OBJ_CONFIG,
        8,
        CONFIG_GROUP_OBJ_LEGACY,
        crate::config::parse_obj,
    )?;
    wrote += 1;
    dump_split_ops(
        cache,
        &dir,
        "npc",
        ARCHIVE_NPC_CONFIG,
        7,
        CONFIG_GROUP_NPC_LEGACY,
        crate::config::parse_npc,
    )?;
    wrote += 1;
    dump_split_ops(
        cache,
        &dir,
        "loc",
        ARCHIVE_LOC_CONFIG,
        8,
        CONFIG_GROUP_LOC_LEGACY,
        crate::config::parse_loc,
    )?;
    wrote += 1;
    dump_legacy_ops(cache, &dir, "bas", CONFIG_GROUP_BAS, |id, data| {
        crate::config::parse_bas(id, data, build)
    })?;
    wrote += 1;

    dump_seq(cache, &dir)?;
    wrote += 1;
    dump_spot(cache, &dir)?;
    wrote += 1;
    dump_struct(cache, &dir)?;
    wrote += 1;
    dump_enums(cache, &dir)?;
    wrote += 1;
    dump_varps(cache, &dir)?;
    wrote += 1;
    dump_varbits(cache, &dir)?;
    wrote += 1;
    dump_params(cache, &dir)?;
    wrote += 1;

    eprintln!("Wrote {wrote} text dumps to {}", dir.display());
    Ok(wrote)
}

fn create_writer(path: &Path) -> Result<BufWriter<fs::File>> {
    let file = fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    Ok(BufWriter::new(file))
}

fn split_config_id(group: u32, file: u32, bit_shift: u32) -> u32 {
    (group << bit_shift) | file
}

fn archive_index_if_present(cache: &FlatCache, archive: u32) -> Result<Option<js5::ArchiveIndex>> {
    if cache.get(255, archive)?.is_none() {
        return Ok(None);
    }
    Ok(Some(cache.archive_index(archive)?))
}

fn for_each_split_archive_file(
    cache: &FlatCache,
    archive: u32,
    bit_shift: u32,
    mut f: impl FnMut(u32, &[u8]) -> Result<()>,
) -> Result<bool> {
    let Some(index) = archive_index_if_present(cache, archive)? else {
        return Ok(false);
    };
    for &group in &index.group_id {
        let Some(data) = cache.get(archive, group)? else {
            continue;
        };
        for (file, bytes) in js5::unpack_group(&index, group, &data)? {
            f(split_config_id(group, file, bit_shift), &bytes)?;
        }
    }
    Ok(true)
}

fn for_each_legacy_config_file(
    cache: &FlatCache,
    group: u32,
    mut f: impl FnMut(u32, &[u8]) -> Result<()>,
) -> Result<()> {
    let Some(data) = cache.get(ARCHIVE_CONFIG, group)? else {
        return Ok(());
    };
    let index = cache.archive_index(ARCHIVE_CONFIG)?;
    for (id, bytes) in js5::unpack_group(&index, group, &data)? {
        f(id, &bytes)?;
    }
    Ok(())
}

fn for_each_split_or_legacy_file(
    cache: &FlatCache,
    archive: u32,
    bit_shift: u32,
    legacy_group: u32,
    mut f: impl FnMut(u32, &[u8]) -> Result<()>,
) -> Result<()> {
    if for_each_split_archive_file(cache, archive, bit_shift, &mut f)? {
        return Ok(());
    }
    for_each_legacy_config_file(cache, legacy_group, f)
}

fn dump_legacy_ops(
    cache: &FlatCache,
    dir: &Path,
    name: &str,
    group: u32,
    parse: impl Fn(u32, &[u8]) -> Result<crate::config::OpListEntry>,
) -> Result<()> {
    let path = dir.join(format!("dump.{name}"));
    let mut w = create_writer(&path)?;
    for_each_legacy_config_file(cache, group, |id, bytes| {
        write_oplist_entry(&mut w, name, id, &parse(id, bytes)?)
    })
}

fn dump_split_ops(
    cache: &FlatCache,
    dir: &Path,
    name: &str,
    archive: u32,
    bit_shift: u32,
    legacy_group: u32,
    parse: impl Fn(u32, &[u8]) -> Result<crate::config::OpListEntry>,
) -> Result<()> {
    let path = dir.join(format!("dump.{name}"));
    let mut w = create_writer(&path)?;
    for_each_split_or_legacy_file(cache, archive, bit_shift, legacy_group, |id, bytes| {
        write_oplist_entry(&mut w, name, id, &parse(id, bytes)?)
    })
}

fn write_oplist_entry(
    w: &mut impl Write,
    name: &str,
    id: u32,
    entry: &crate::config::OpListEntry,
) -> Result<()> {
    writeln!(w, "[{name}_{id}]")?;
    for op in &entry.ops {
        writeln!(w, "{}", rewrite_op_value(op))?;
    }
    writeln!(w)?;
    Ok(())
}

fn dump_seq(cache: &FlatCache, dir: &Path) -> Result<()> {
    let path = dir.join("dump.seq");
    let mut w = create_writer(&path)?;
    for_each_split_or_legacy_file(
        cache,
        ARCHIVE_SEQ_CONFIG,
        7,
        CONFIG_GROUP_SEQ,
        |id, bytes| {
            let entry = crate::config::parse_seq(id, bytes)?;
            writeln!(w, "[seq_{id}]")?;
            for frame in &entry.frames {
                if frame.anim_id != 0xFFFF {
                    writeln!(w, "frame=seq_{}", frame.anim_id)?;
                }
            }
            for frame in &entry.iframes {
                if frame.anim_id != 0xFFFF {
                    writeln!(w, "iframe=seq_{}", frame.anim_id)?;
                }
            }
            for &id in &entry.walkmerge {
                if id != 0xFFFF {
                    writeln!(w, "walkmerge=seq_{id}")?;
                }
            }
            if let Some(group) = entry.group {
                writeln!(w, "group=seqgroup_{group}")?;
            }
            for param in &entry.params {
                writeln!(w, "param=param_{}", param.param_id)?;
            }
            writeln!(w)?;
            Ok(())
        },
    )
}

fn dump_spot(cache: &FlatCache, dir: &Path) -> Result<()> {
    let path = dir.join("dump.spot");
    let mut w = create_writer(&path)?;
    for_each_split_or_legacy_file(
        cache,
        ARCHIVE_SPOT_CONFIG,
        8,
        CONFIG_GROUP_SPOT,
        |id, bytes| {
            let entry = crate::config::parse_spot(id, bytes)?;
            writeln!(w, "[spotanim_{id}]")?;
            for op in &entry.ops {
                match op {
                    SpotOp::Model(model) if *model >= 0 => writeln!(w, "model=model_{model}")?,
                    SpotOp::Anim(anim) if *anim >= 0 => writeln!(w, "anim=seq_{anim}")?,
                    _ => {}
                }
            }
            writeln!(w)?;
            Ok(())
        },
    )
}

fn dump_struct(cache: &FlatCache, dir: &Path) -> Result<()> {
    let path = dir.join("dump.struct");
    let mut w = create_writer(&path)?;
    for_each_split_or_legacy_file(
        cache,
        ARCHIVE_STRUCT_CONFIG,
        5,
        CONFIG_GROUP_STRUCT,
        |id, bytes| {
            let entry = crate::config::parse_struct(id, bytes)?;
            writeln!(w, "[struct_{id}]")?;
            for param in &entry.params {
                writeln!(w, "param=param_{}", param.param_id)?;
            }
            writeln!(w)?;
            Ok(())
        },
    )
}

fn dump_enums(cache: &FlatCache, dir: &Path) -> Result<()> {
    let path = dir.join("dump.enum");
    let mut w = create_writer(&path)?;
    let Some(index) = archive_index_if_present(cache, ARCHIVE_ENUM_CONFIG)? else {
        return Ok(());
    };
    for &group in &index.group_id {
        let Some(data) = cache.get(ARCHIVE_ENUM_CONFIG, group)? else {
            continue;
        };
        for (file, bytes) in js5::unpack_group(&index, group, &data)? {
            let id = split_config_id(group, file, 8);
            let entry = crate::config::parse_enum(id, &bytes)?;
            writeln!(w, "[enum_{id}]")?;
            if let Some(type_id) = entry.input_type_id {
                writeln!(w, "inputtype=param_{type_id}")?;
            }
            if let Some(type_id) = entry.output_type_id {
                writeln!(w, "outputtype=param_{type_id}")?;
            }
            writeln!(w)?;
        }
    }
    Ok(())
}

fn dump_varps(cache: &FlatCache, dir: &Path) -> Result<()> {
    let varp_groups: &[(&str, u32, crate::vars::VarDomain)] = &[
        (
            "var_player",
            CONFIG_GROUP_VAR_PLAYER,
            crate::vars::VarDomain::Player,
        ),
        ("var_npc", CONFIG_GROUP_VAR_NPC, crate::vars::VarDomain::Npc),
        (
            "var_client",
            CONFIG_GROUP_VAR_CLIENT,
            crate::vars::VarDomain::Client,
        ),
        (
            "var_world",
            CONFIG_GROUP_VAR_WORLD,
            crate::vars::VarDomain::World,
        ),
        (
            "var_region",
            CONFIG_GROUP_VAR_REGION,
            crate::vars::VarDomain::Region,
        ),
        (
            "var_object",
            CONFIG_GROUP_VAR_OBJECT,
            crate::vars::VarDomain::Object,
        ),
        (
            "var_clan",
            CONFIG_GROUP_VAR_CLAN,
            crate::vars::VarDomain::Clan,
        ),
        (
            "var_clan_setting",
            CONFIG_GROUP_VAR_CLAN_SETTING,
            crate::vars::VarDomain::ClanSetting,
        ),
        (
            "var_controller",
            CONFIG_GROUP_VAR_CONTROLLER,
            crate::vars::VarDomain::Controller,
        ),
        (
            "var_global",
            CONFIG_GROUP_VAR_GLOBAL,
            crate::vars::VarDomain::Global,
        ),
        (
            "var_player_group",
            CONFIG_GROUP_VAR_PLAYER_GROUP,
            crate::vars::VarDomain::PlayerGroup,
        ),
    ];
    for (domain_name, group, domain) in varp_groups {
        let path = dir.join(format!("dump.{domain_name}"));
        let mut w = create_writer(&path)?;
        for_each_legacy_config_file(cache, *group, |id, bytes| {
            let entry = crate::vars::parse_var(*domain, id, bytes)?;
            writeln!(w, "[{domain_name}_{id}]")?;
            if let Some(type_id) = entry.type_id {
                writeln!(w, "type=param_{type_id}")?;
            }
            writeln!(w)?;
            Ok(())
        })?;
    }
    Ok(())
}

fn dump_varbits(cache: &FlatCache, dir: &Path) -> Result<()> {
    let path = dir.join("dump.varbit");
    let mut w = create_writer(&path)?;
    for_each_legacy_config_file(cache, CONFIG_GROUP_VAR_BIT, |id, bytes| {
        let entry = crate::vars::parse_varbit(id, bytes)?;
        writeln!(w, "[varbit_{id}]")?;
        if let (Some(domain), Some(base_var)) = (entry.domain, entry.base_var) {
            writeln!(w, "basevar=var_{}_{base_var}", domain.as_label())?;
        }
        writeln!(w)?;
        Ok(())
    })
}

fn dump_params(cache: &FlatCache, dir: &Path) -> Result<()> {
    let path = dir.join("dump.param");
    let mut w = create_writer(&path)?;
    for_each_legacy_config_file(cache, CONFIG_GROUP_PARAM, |id, bytes| {
        let entry = crate::config::parse_param(id, bytes)?;
        writeln!(w, "[param_{id}]")?;
        if let Some(type_id) = entry.type_id {
            writeln!(w, "typeid={type_id}")?;
        }
        writeln!(w)?;
        Ok(())
    })
}

/// Rewrite op string to include type-prefixed values for dependency scanning.
/// Example: `model=2595` becomes `model=model_2595`.
fn rewrite_op_value(op: &str) -> String {
    if let Some((key, value)) = op.split_once('=')
        && let Some(prefix) = key_to_ref_prefix(key)
    {
        let num = value.split([',', ' ']).next().unwrap_or(value);
        if num.parse::<i32>().is_ok() {
            return format!("{key}={prefix}_{num}{}", &value[num.len()..]);
        }
    }
    op.to_string()
}

fn key_to_ref_prefix(key: &str) -> Option<&'static str> {
    match key {
        "model" | "manwear" | "manwear2" | "manwear3" | "womanwear" | "womanwear2"
        | "womanwear3" | "manhead" | "manhead2" | "womanhead" | "womanhead2" | "covermarker"
        | "modela" | "modelb" | "model1" | "model2" | "model3" | "model4" | "model5" | "model6"
        | "model7" | "model8" => Some("model"),
        "anim" | "readyanim" | "walkanim" | "turnleftanim" | "turnrightanim" | "crawlanim"
        | "crawlanim_b" | "crawlanim_l" | "crawlanim_r" | "runanim" | "runanim_b" | "runanim_l"
        | "runanim_r" | "walkanim_b" | "walkanim_l" | "walkanim_r" | "readyanim_l"
        | "readyanim_r" | "crawlturn_l" | "crawlturn_r" | "runturn_l" | "runturn_r"
        | "walkturn_l" | "walkturn_r" => Some("seq"),
        "bas" => Some("bas"),
        "msi" => Some("msi"),
        "vfx" => Some("vfx"),
        "quest" => Some("quest"),
        "cursor1" | "cursor2" | "cursor3" | "cursor4" | "cursor5" | "cursor6" | "icursor1"
        | "icursor2" | "icursor3" | "icursor4" | "icursor5" | "cursorattack" => Some("cursor"),
        "texture" | "bloomtexture" => Some("texture"),
        "material" => Some("material"),
        "certlink"
        | "certtemplate"
        | "lentlink"
        | "lenttemplate"
        | "boughtlink"
        | "boughttemplate"
        | "shardlink"
        | "shardtemplate"
        | "placeholderlink"
        | "placeholdertemplate" => Some("obj"),
        "multiloc" => Some("loc"),
        "multinpc" => Some("npc"),
        "multimodel" => Some("model"),
        _ => None,
    }
}
