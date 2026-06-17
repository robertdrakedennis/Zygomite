//! Config-extraction commands: `interfaces`, `varps`, `varbits`, `configs`.
//!
//! Each reads the relevant cache archives, parses every entry, optionally writes
//! the JSON dump, and prints a summary. These are also re-used by `unpack`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;

use crate::cache::FlatCache;
use crate::cli::context::CommandContext;
use crate::cli::shared::{print_json, write_json, write_text};
use crate::cli::VarDomainArg;
use crate::config::{
    parse_achievement, parse_area, parse_bas, parse_billboard, parse_bugtemplate, parse_category,
    parse_controller, parse_cursor, parse_dbrow, parse_dbtable, parse_enum, parse_gamelogevent,
    parse_headbar, parse_hitmark, parse_hunt, parse_idk, parse_inv, parse_itemcode, parse_light,
    parse_loc, parse_material, parse_mel, parse_mesanim, parse_msi, parse_npc, parse_obj,
    parse_overlay, parse_param, parse_particle_effector, parse_particle_emitter, parse_quest,
    parse_quickchatcat, parse_quickchatphrase, parse_seq, parse_seqgroup, parse_skybox, parse_spot,
    parse_struct, parse_stylesheet, parse_texture, parse_underlay, parse_var_client_string,
    parse_var_npc_bit, parse_var_shared, parse_var_shared_string, parse_water, parse_worldarea,
};
use crate::constants::{
    ARCHIVE_ACHIEVEMENTS, ARCHIVE_BILLBOARDS, ARCHIVE_CONFIG, ARCHIVE_ENUM_CONFIG,
    ARCHIVE_INTERFACES, ARCHIVE_LOC_CONFIG, ARCHIVE_MATERIALS, ARCHIVE_NPC_CONFIG, ARCHIVE_OBJ_CONFIG,
    ARCHIVE_PARTICLES, ARCHIVE_QUICKCHAT_CONFIG, ARCHIVE_SEQ_CONFIG, ARCHIVE_SPOT_CONFIG,
    ARCHIVE_STRUCT_CONFIG, ARCHIVE_STYLESHEETS, ARCHIVE_TEXTURES,
    CONFIG_GROUP_ACHIEVEMENT_ARCHIVE57, CONFIG_GROUP_AREA, CONFIG_GROUP_BAS,
    CONFIG_GROUP_BILLBOARD_ARCHIVE29, CONFIG_GROUP_BUGTEMPLATE, CONFIG_GROUP_CATEGORY,
    CONFIG_GROUP_CONTROLLER, CONFIG_GROUP_CURSOR, CONFIG_GROUP_DBROW, CONFIG_GROUP_DBTABLE,
    CONFIG_GROUP_GAMELOGEVENT, CONFIG_GROUP_HEADBAR, CONFIG_GROUP_HITMARK, CONFIG_GROUP_HUNT,
    CONFIG_GROUP_IDK, CONFIG_GROUP_INV, CONFIG_GROUP_ITEMCODE, CONFIG_GROUP_LIGHT,
    CONFIG_GROUP_LOC_LEGACY, CONFIG_GROUP_MATERIAL_ARCHIVE26, CONFIG_GROUP_MEL, CONFIG_GROUP_MESANIM,
    CONFIG_GROUP_MSI, CONFIG_GROUP_NPC_LEGACY, CONFIG_GROUP_OBJ_LEGACY, CONFIG_GROUP_OVERLAY,
    CONFIG_GROUP_PARAM, CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27,
    CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27, CONFIG_GROUP_QUEST, CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24,
    CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24, CONFIG_GROUP_SEQ, CONFIG_GROUP_SEQGROUP,
    CONFIG_GROUP_SKYBOX, CONFIG_GROUP_SPOT, CONFIG_GROUP_STRUCT, CONFIG_GROUP_UNDERLAY,
    CONFIG_GROUP_VAR_BIT, CONFIG_GROUP_VAR_CLIENT_STRING, CONFIG_GROUP_VAR_NPC_BIT,
    CONFIG_GROUP_VAR_SHARED, CONFIG_GROUP_VAR_SHARED_STRING, CONFIG_GROUP_WATER,
    CONFIG_GROUP_WORLDAREA,
};
use crate::fixture::ensure_archive_complete;
use crate::interface::render_interface_group;
use crate::vars::{parse_var, parse_varbit};

#[derive(Debug, Serialize)]
struct InterfacesSummary {
    archive: u32,
    groups: usize,
    files: usize,
    parsed_groups: usize,
}

#[derive(Debug, Serialize)]
struct VarSummary {
    groups: usize,
    entries: usize,
}

#[derive(Debug, Serialize)]
struct ConfigSummary {
    params: usize,
    enums: usize,
    dbtables: usize,
    dbrows: usize,
    idks: usize,
    locs: usize,
    npcs: usize,
    objs: usize,
    seqs: usize,
    spots: usize,
    bass: usize,
    quests: usize,
    mels: usize,
    waters: usize,
    achievements: usize,
    materials: usize,
    invs: usize,
    cursors: usize,
    seqgroups: usize,
    structs: usize,
    controllers: usize,
    categories: usize,
    areas: usize,
    hunts: usize,
    mesanims: usize,
    itemcodes: usize,
    gamelogevents: usize,
    bugtemplates: usize,
    varcstrs: usize,
    varnbits: usize,
    vars: usize,
    varsstrs: usize,
    underlays: usize,
    overlays: usize,
    msis: usize,
    skyboxes: usize,
    worldareas: usize,
    quickchatcats: usize,
    headbars: usize,
    hitmarks: usize,
    lights: usize,
    quickchatphrases: usize,
    billboards: usize,
    particleeffectors: usize,
    particleemitters: usize,
    textures: usize,
    stylesheets: usize,
}

/// Options for `interfaces`.
#[derive(Clone, Debug, Default)]
pub struct InterfacesOpts {
    pub out_dir: Option<PathBuf>,
}

/// Options for `varps`.
#[derive(Clone, Debug)]
pub struct VarpsOpts {
    pub out_file: Option<PathBuf>,
    pub domain: VarDomainArg,
}

/// Options for `varbits`.
#[derive(Clone, Debug, Default)]
pub struct VarbitsOpts {
    pub out_file: Option<PathBuf>,
}

/// Options for `configs`.
#[derive(Clone, Debug, Default)]
pub struct ConfigsOpts {
    pub out_dir: Option<PathBuf>,
}

/// `interfaces` — render every interface group, optionally writing each to disk.
pub fn run_interfaces(ctx: &CommandContext, opts: InterfacesOpts) -> Result<()> {
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let build = ctx.build();
    let InterfacesOpts { out_dir } = opts;
    let out_dir = out_dir.as_deref();

    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_INTERFACES)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let group_counts = index
        .group_id
        .par_iter()
        .map(|group| -> Result<usize> {
            let group_files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, *group)?;
            let file_count = group_files.len();
            let rendered = render_interface_group(*group, &group_files, build);
            if let Some(out) = out_dir {
                let is_scripted = group_files
                    .values()
                    .any(|bytes| bytes.first().copied() == Some(u8::MAX));
                let extension = if is_scripted { "if3" } else { "if" };
                let path = out.join(format!("interface_{group}.{extension}"));
                write_text(&path, &rendered.join("\n"))?;
            }
            Ok(file_count)
        })
        .collect::<Vec<_>>();

    let mut files = 0_usize;
    for count in group_counts {
        files += count?;
    }

    print_json(&InterfacesSummary {
        archive: ARCHIVE_INTERFACES,
        groups: index.group_count,
        files,
        parsed_groups: index.group_id.len(),
    })
}

/// `varps` — dump variable-player configs for the requested domain(s).
pub fn run_varps(ctx: &CommandContext, opts: VarpsOpts) -> Result<()> {
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let VarpsOpts { out_file, domain } = opts;
    let out_file = out_file.as_deref();

    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CONFIG)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_CONFIG)?;

    let mut out = Vec::new();
    for (group_id, var_domain) in domain.groups() {
        let group_payload = cache
            .get(ARCHIVE_CONFIG, *group_id)?
            .with_context(|| format!("missing group {group_id} in archive 2"))?;
        let vars = crate::js5::unpack_group(&index, *group_id, &group_payload)?;
        for (id, bytes) in vars {
            out.push(parse_var(*var_domain, id, &bytes)?);
        }
    }

    if let Some(path) = out_file {
        write_json(path, &out)?;
    }
    print_json(&VarSummary {
        groups: domain.groups().len(),
        entries: out.len(),
    })
}

/// `varbits` — dump variable-bit configs.
pub fn run_varbits(ctx: &CommandContext, opts: VarbitsOpts) -> Result<()> {
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let VarbitsOpts { out_file } = opts;
    let out_file = out_file.as_deref();

    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CONFIG)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_CONFIG)?;
    let varbit_group_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_BIT)?
        .context("missing varbit group data 2/69.dat")?;
    let varbits = crate::js5::unpack_group(&index, CONFIG_GROUP_VAR_BIT, &varbit_group_payload)?;
    let mut out = Vec::with_capacity(varbits.len());
    for (id, bytes) in varbits {
        out.push(parse_varbit(id, &bytes)?);
    }
    if let Some(path) = out_file {
        write_json(path, &out)?;
    }
    print_json(&VarSummary {
        groups: 1,
        entries: out.len(),
    })
}

/// `configs` — parse and dump every config family.
pub fn run_configs(ctx: &CommandContext, opts: ConfigsOpts) -> Result<()> {
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let build = ctx.build();
    let ConfigsOpts { out_dir } = opts;
    let out_dir = out_dir.as_deref();

    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CONFIG)?;
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_ENUM_CONFIG)?;
    let struct_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_STRUCT_CONFIG).is_ok();
    let quickchat_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_QUICKCHAT_CONFIG).is_ok();
    let loc_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_LOC_CONFIG).is_ok();
    let npc_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_NPC_CONFIG).is_ok();
    let obj_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_OBJ_CONFIG).is_ok();
    let seq_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_SEQ_CONFIG).is_ok();
    let spot_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_SPOT_CONFIG).is_ok();
    let particle_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_PARTICLES).is_ok();
    let billboard_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_BILLBOARDS).is_ok();
    let texture_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_TEXTURES).is_ok();
    let materials_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MATERIALS).is_ok();
    let achievements_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_ACHIEVEMENTS).is_ok();
    let stylesheet_archive_available =
        ensure_archive_complete(cache.root(), tar_path, ARCHIVE_STYLESHEETS).is_ok();
    let cache = FlatCache::open(cache.root())?;

    let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
    let enum_index = cache.archive_index(ARCHIVE_ENUM_CONFIG)?;

    let param_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_PARAM)?
        .with_context(|| format!("missing group {CONFIG_GROUP_PARAM} in archive 2"))?;
    let param_files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_PARAM, &param_payload)?;
    let mut params = Vec::with_capacity(param_files.len());
    for (id, data) in param_files {
        params.push(parse_param(id, &data).with_context(|| format!("parse_param id {id}"))?);
    }

    let dbtable_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_DBTABLE)?
        .with_context(|| format!("missing group {CONFIG_GROUP_DBTABLE} in archive 2"))?;
    let dbtable_files =
        crate::js5::unpack_group(&config_index, CONFIG_GROUP_DBTABLE, &dbtable_payload)?;
    let mut dbtables = Vec::with_capacity(dbtable_files.len());
    for (id, data) in dbtable_files {
        dbtables.push(parse_dbtable(id, &data).with_context(|| format!("parse_dbtable id {id}"))?);
    }

    let dbrow_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_DBROW)?
        .with_context(|| format!("missing group {CONFIG_GROUP_DBROW} in archive 2"))?;
    let dbrow_files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_DBROW, &dbrow_payload)?;
    let mut dbrows = Vec::with_capacity(dbrow_files.len());
    for (id, data) in dbrow_files {
        dbrows.push(parse_dbrow(id, &data).with_context(|| format!("parse_dbrow id {id}"))?);
    }

    let mut idks = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_IDK)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_IDK, &payload)?;
        idks.reserve(files.len());
        for (id, data) in files {
            idks.push(parse_idk(id, &data).with_context(|| format!("parse_idk id {id}"))?);
        }
    }

    let mut locs = Vec::new();
    if loc_archive_available {
        let loc_index = cache.archive_index(ARCHIVE_LOC_CONFIG)?;
        for group in &loc_index.group_id {
            let files = cache.group_files_with_index(&loc_index, ARCHIVE_LOC_CONFIG, *group)?;
            for (file, data) in files {
                let loc_id = (group << 8) | file;
                locs.push(
                    parse_loc(loc_id, &data).with_context(|| format!("parse_loc id {loc_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_LOC_LEGACY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_LOC_LEGACY, &payload)?;
        locs.reserve(files.len());
        for (id, data) in files {
            locs.push(parse_loc(id, &data).with_context(|| format!("parse_loc id {id}"))?);
        }
    }

    let mut npcs = Vec::new();
    if npc_archive_available {
        let npc_index = cache.archive_index(ARCHIVE_NPC_CONFIG)?;
        for group in &npc_index.group_id {
            let files = cache.group_files_with_index(&npc_index, ARCHIVE_NPC_CONFIG, *group)?;
            for (file, data) in files {
                let npc_id = (group << 7) | file;
                npcs.push(
                    parse_npc(npc_id, &data).with_context(|| format!("parse_npc id {npc_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_NPC_LEGACY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_NPC_LEGACY, &payload)?;
        npcs.reserve(files.len());
        for (id, data) in files {
            npcs.push(parse_npc(id, &data).with_context(|| format!("parse_npc id {id}"))?);
        }
    }

    let mut objs = Vec::new();
    if obj_archive_available {
        let obj_index = cache.archive_index(ARCHIVE_OBJ_CONFIG)?;
        for group in &obj_index.group_id {
            let files = cache.group_files_with_index(&obj_index, ARCHIVE_OBJ_CONFIG, *group)?;
            for (file, data) in files {
                let obj_id = (group << 8) | file;
                objs.push(
                    parse_obj(obj_id, &data).with_context(|| format!("parse_obj id {obj_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_OBJ_LEGACY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_OBJ_LEGACY, &payload)?;
        objs.reserve(files.len());
        for (id, data) in files {
            objs.push(parse_obj(id, &data).with_context(|| format!("parse_obj id {id}"))?);
        }
    }

    let mut seqs = Vec::new();
    if seq_archive_available {
        let seq_index = cache.archive_index(ARCHIVE_SEQ_CONFIG)?;
        for group in &seq_index.group_id {
            let files = cache.group_files_with_index(&seq_index, ARCHIVE_SEQ_CONFIG, *group)?;
            for (file, data) in files {
                let seq_id = (group << 7) | file;
                seqs.push(
                    parse_seq(seq_id, &data).with_context(|| format!("parse_seq id {seq_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SEQ)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SEQ, &payload)?;
        seqs.reserve(files.len());
        for (id, data) in files {
            seqs.push(parse_seq(id, &data).with_context(|| format!("parse_seq id {id}"))?);
        }
    }

    let mut spots = Vec::new();
    if spot_archive_available {
        let spot_index = cache.archive_index(ARCHIVE_SPOT_CONFIG)?;
        for group in &spot_index.group_id {
            let files = cache.group_files_with_index(&spot_index, ARCHIVE_SPOT_CONFIG, *group)?;
            for (file, data) in files {
                let spot_id = (group << 8) | file;
                spots.push(
                    parse_spot(spot_id, &data)
                        .with_context(|| format!("parse_spot id {spot_id}"))?,
                );
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SPOT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SPOT, &payload)?;
        spots.reserve(files.len());
        for (id, data) in files {
            spots.push(parse_spot(id, &data).with_context(|| format!("parse_spot id {id}"))?);
        }
    }

    let mut bass = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_BAS)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_BAS, &payload)?;
        bass.reserve(files.len());
        for (id, data) in files {
            bass.push(parse_bas(id, &data, build).with_context(|| format!("parse_bas id {id}"))?);
        }
    }

    let mut quests = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_QUEST)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_QUEST, &payload)?;
        quests.reserve(files.len());
        for (id, data) in files {
            quests.push(parse_quest(id, &data)?);
        }
    }

    let mut mels = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_MEL)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_MEL, &payload)?;
        mels.reserve(files.len());
        for (id, data) in files {
            mels.push(parse_mel(id, &data)?);
        }
    }

    let mut waters = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_WATER)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_WATER, &payload)?;
        waters.reserve(files.len());
        for (id, data) in files {
            waters.push(parse_water(id, &data)?);
        }
    }

    let mut achievements = Vec::new();
    if achievements_archive_available {
        let achievements_index = cache.archive_index(ARCHIVE_ACHIEVEMENTS)?;
        for group in &achievements_index.group_id {
            let files =
                cache.group_files_with_index(&achievements_index, ARCHIVE_ACHIEVEMENTS, *group)?;
            for (file, data) in files {
                let achievement_id = (group << CONFIG_GROUP_ACHIEVEMENT_ARCHIVE57) | file;
                achievements.push(parse_achievement(achievement_id, &data)?);
            }
        }
    }

    let mut materials = Vec::new();
    if materials_archive_available {
        let materials_index = cache.archive_index(ARCHIVE_MATERIALS)?;
        if let Some(payload) = cache.get(ARCHIVE_MATERIALS, CONFIG_GROUP_MATERIAL_ARCHIVE26)? {
            let files = crate::js5::unpack_group(
                &materials_index,
                CONFIG_GROUP_MATERIAL_ARCHIVE26,
                &payload,
            )?;
            materials.reserve(files.len());
            for (id, data) in files {
                materials.push(parse_material(id, &data)?);
            }
        } else {
            for group in &materials_index.group_id {
                let files =
                    cache.group_files_with_index(&materials_index, ARCHIVE_MATERIALS, *group)?;
                for (file, data) in files {
                    materials.push(parse_material(group + file, &data)?);
                }
            }
        }
    }

    let mut invs = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_INV)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_INV, &payload)?;
        invs.reserve(files.len());
        for (id, data) in files {
            invs.push(parse_inv(id, &data)?);
        }
    }

    let mut cursors = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_CURSOR)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_CURSOR, &payload)?;
        cursors.reserve(files.len());
        for (id, data) in files {
            cursors.push(parse_cursor(id, &data)?);
        }
    }

    let mut seqgroups = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SEQGROUP)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SEQGROUP, &payload)?;
        seqgroups.reserve(files.len());
        for (id, data) in files {
            seqgroups.push(parse_seqgroup(id, &data)?);
        }
    }

    let mut categories = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_CATEGORY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_CATEGORY, &payload)?;
        categories.reserve(files.len());
        for (id, data) in files {
            categories.push(parse_category(id, &data)?);
        }
    }

    let mut controllers = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_CONTROLLER)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_CONTROLLER, &payload)?;
        controllers.reserve(files.len());
        for (id, data) in files {
            controllers.push(parse_controller(id, &data)?);
        }
    }

    let mut areas = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_AREA)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_AREA, &payload)?;
        areas.reserve(files.len());
        for (id, data) in files {
            areas.push(parse_area(id, &data)?);
        }
    }

    let mut hunts = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_HUNT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_HUNT, &payload)?;
        hunts.reserve(files.len());
        for (id, data) in files {
            hunts.push(parse_hunt(id, &data)?);
        }
    }

    let mut mesanims = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_MESANIM)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_MESANIM, &payload)?;
        mesanims.reserve(files.len());
        for (id, data) in files {
            mesanims.push(parse_mesanim(id, &data)?);
        }
    }

    let mut itemcodes = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_ITEMCODE)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_ITEMCODE, &payload)?;
        itemcodes.reserve(files.len());
        for (id, data) in files {
            itemcodes.push(parse_itemcode(id, &data)?);
        }
    }

    let mut gamelogevents = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_GAMELOGEVENT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_GAMELOGEVENT, &payload)?;
        gamelogevents.reserve(files.len());
        for (id, data) in files {
            gamelogevents.push(parse_gamelogevent(id, &data)?);
        }
    }

    let mut bugtemplates = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_BUGTEMPLATE)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_BUGTEMPLATE, &payload)?;
        bugtemplates.reserve(files.len());
        for (id, data) in files {
            bugtemplates.push(parse_bugtemplate(id, &data)?);
        }
    }

    let mut var_client_strings = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_CLIENT_STRING)? {
        let files =
            crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_CLIENT_STRING, &payload)?;
        var_client_strings.reserve(files.len());
        for (id, data) in files {
            var_client_strings.push(parse_var_client_string(id, &data)?);
        }
    }

    let mut varnbits = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_NPC_BIT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_NPC_BIT, &payload)?;
        varnbits.reserve(files.len());
        for (id, data) in files {
            varnbits.push(parse_var_npc_bit(id, &data)?);
        }
    }

    let mut vars = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_SHARED)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_SHARED, &payload)?;
        vars.reserve(files.len());
        for (id, data) in files {
            vars.push(parse_var_shared(id, &data)?);
        }
    }

    let mut var_shared_strings = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_SHARED_STRING)? {
        let files =
            crate::js5::unpack_group(&config_index, CONFIG_GROUP_VAR_SHARED_STRING, &payload)?;
        var_shared_strings.reserve(files.len());
        for (id, data) in files {
            var_shared_strings.push(parse_var_shared_string(id, &data)?);
        }
    }

    let mut underlays = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_UNDERLAY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_UNDERLAY, &payload)?;
        underlays.reserve(files.len());
        for (id, data) in files {
            underlays.push(parse_underlay(id, &data)?);
        }
    }

    let mut overlays = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_OVERLAY)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_OVERLAY, &payload)?;
        overlays.reserve(files.len());
        for (id, data) in files {
            overlays.push(parse_overlay(id, &data)?);
        }
    }

    let mut msis = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_MSI)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_MSI, &payload)?;
        msis.reserve(files.len());
        for (id, data) in files {
            msis.push(parse_msi(id, &data)?);
        }
    }

    let mut skyboxes = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SKYBOX)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_SKYBOX, &payload)?;
        skyboxes.reserve(files.len());
        for (id, data) in files {
            skyboxes.push(parse_skybox(id, &data)?);
        }
    }

    let mut worldareas = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_WORLDAREA)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_WORLDAREA, &payload)?;
        worldareas.reserve(files.len());
        for (id, data) in files {
            worldareas.push(parse_worldarea(id, &data)?);
        }
    }

    let mut quickchat_categories = Vec::new();
    if quickchat_archive_available
        && let Some(payload) = cache.get(
            ARCHIVE_QUICKCHAT_CONFIG,
            CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24,
        )?
    {
        let quickchat_index = cache.archive_index(ARCHIVE_QUICKCHAT_CONFIG)?;
        let files = crate::js5::unpack_group(
            &quickchat_index,
            CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24,
            &payload,
        )?;
        quickchat_categories.reserve(files.len());
        for (id, data) in files {
            quickchat_categories.push(parse_quickchatcat(id, &data)?);
        }
    }

    let mut headbars = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_HEADBAR)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_HEADBAR, &payload)?;
        headbars.reserve(files.len());
        for (id, data) in files {
            headbars.push(parse_headbar(id, &data)?);
        }
    }

    let mut hitmarks = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_HITMARK)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_HITMARK, &payload)?;
        hitmarks.reserve(files.len());
        for (id, data) in files {
            hitmarks.push(parse_hitmark(id, &data)?);
        }
    }

    let mut lights = Vec::new();
    if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_LIGHT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_LIGHT, &payload)?;
        lights.reserve(files.len());
        for (id, data) in files {
            lights.push(parse_light(id, &data)?);
        }
    }

    let mut quickchat_phrases = Vec::new();
    if quickchat_archive_available
        && let Some(payload) = cache.get(
            ARCHIVE_QUICKCHAT_CONFIG,
            CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24,
        )?
    {
        let quickchat_index = cache.archive_index(ARCHIVE_QUICKCHAT_CONFIG)?;
        let files = crate::js5::unpack_group(
            &quickchat_index,
            CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24,
            &payload,
        )?;
        quickchat_phrases.reserve(files.len());
        for (id, data) in files {
            quickchat_phrases.push(parse_quickchatphrase(id, &data)?);
        }
    }

    let mut billboards = Vec::new();
    if billboard_archive_available
        && let Some(payload) = cache.get(ARCHIVE_BILLBOARDS, CONFIG_GROUP_BILLBOARD_ARCHIVE29)?
    {
        let billboard_index = cache.archive_index(ARCHIVE_BILLBOARDS)?;
        let files =
            crate::js5::unpack_group(&billboard_index, CONFIG_GROUP_BILLBOARD_ARCHIVE29, &payload)?;
        billboards.reserve(files.len());
        for (id, data) in files {
            billboards.push(parse_billboard(id, &data)?);
        }
    }

    let mut particleeffectors = Vec::new();
    let mut particleemitters = Vec::new();
    if particle_archive_available {
        let particle_index = cache.archive_index(ARCHIVE_PARTICLES)?;
        if let Some(payload) =
            cache.get(ARCHIVE_PARTICLES, CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27)?
        {
            let files = crate::js5::unpack_group(
                &particle_index,
                CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27,
                &payload,
            )?;
            particleemitters.reserve(files.len());
            for (id, data) in files {
                particleemitters.push(parse_particle_emitter(id, &data)?);
            }
        }
        if let Some(payload) =
            cache.get(ARCHIVE_PARTICLES, CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27)?
        {
            let files = crate::js5::unpack_group(
                &particle_index,
                CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27,
                &payload,
            )?;
            particleeffectors.reserve(files.len());
            for (id, data) in files {
                particleeffectors.push(parse_particle_effector(id, &data)?);
            }
        }
    }

    let mut textures = Vec::new();
    if texture_archive_available {
        let texture_index = cache.archive_index(ARCHIVE_TEXTURES)?;
        for group in &texture_index.group_id {
            let files = cache.group_files_with_index(&texture_index, ARCHIVE_TEXTURES, *group)?;
            for (file, data) in files {
                textures.push(parse_texture(group + file, &data)?);
            }
        }
    }

    let mut stylesheets = Vec::new();
    if stylesheet_archive_available {
        let stylesheet_index = cache.archive_index(ARCHIVE_STYLESHEETS)?;
        for group in &stylesheet_index.group_id {
            let files =
                cache.group_files_with_index(&stylesheet_index, ARCHIVE_STYLESHEETS, *group)?;
            for (file, data) in files {
                stylesheets.push(parse_stylesheet(group + file, &data)?);
            }
        }
    }

    let mut structs = Vec::new();
    if struct_archive_available {
        let struct_index = cache.archive_index(ARCHIVE_STRUCT_CONFIG)?;
        for group in &struct_index.group_id {
            let files =
                cache.group_files_with_index(&struct_index, ARCHIVE_STRUCT_CONFIG, *group)?;
            for (file, data) in files {
                structs.push(parse_struct((group << 5) | file, &data)?);
            }
        }
    } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_STRUCT)? {
        let files = crate::js5::unpack_group(&config_index, CONFIG_GROUP_STRUCT, &payload)?;
        structs.reserve(files.len());
        for (id, data) in files {
            structs.push(parse_struct(id, &data)?);
        }
    }

    let mut enums = Vec::new();
    for group in &enum_index.group_id {
        let files = cache.group_files_with_index(&enum_index, ARCHIVE_ENUM_CONFIG, *group)?;
        for (file, data) in files {
            let enum_id = (group << 8) | file;
            enums.push(parse_enum(enum_id, &data)?);
        }
    }

    if let Some(dir) = out_dir {
        write_json(&dir.join("params.json"), &params)?;
        write_json(&dir.join("enums.json"), &enums)?;
        write_json(&dir.join("dbtables.json"), &dbtables)?;
        write_json(&dir.join("dbrows.json"), &dbrows)?;
        write_json(&dir.join("idks.json"), &idks)?;
        write_json(&dir.join("locs.json"), &locs)?;
        write_json(&dir.join("npcs.json"), &npcs)?;
        write_json(&dir.join("objs.json"), &objs)?;
        write_json(&dir.join("seqs.json"), &seqs)?;
        write_json(&dir.join("spots.json"), &spots)?;
        write_json(&dir.join("bass.json"), &bass)?;
        write_json(&dir.join("quests.json"), &quests)?;
        write_json(&dir.join("mels.json"), &mels)?;
        write_json(&dir.join("waters.json"), &waters)?;
        write_json(&dir.join("achievements.json"), &achievements)?;
        write_json(&dir.join("materials.json"), &materials)?;
        write_json(&dir.join("invs.json"), &invs)?;
        write_json(&dir.join("cursors.json"), &cursors)?;
        write_json(&dir.join("seqgroups.json"), &seqgroups)?;
        write_json(&dir.join("structs.json"), &structs)?;
        write_json(&dir.join("controllers.json"), &controllers)?;
        write_json(&dir.join("categories.json"), &categories)?;
        write_json(&dir.join("areas.json"), &areas)?;
        write_json(&dir.join("hunts.json"), &hunts)?;
        write_json(&dir.join("mesanims.json"), &mesanims)?;
        write_json(&dir.join("itemcodes.json"), &itemcodes)?;
        write_json(&dir.join("gamelogevents.json"), &gamelogevents)?;
        write_json(&dir.join("bugtemplates.json"), &bugtemplates)?;
        write_json(&dir.join("varcstrs.json"), &var_client_strings)?;
        write_json(&dir.join("varnbits.json"), &varnbits)?;
        write_json(&dir.join("vars.json"), &vars)?;
        write_json(&dir.join("varsstrs.json"), &var_shared_strings)?;
        write_json(&dir.join("underlays.json"), &underlays)?;
        write_json(&dir.join("overlays.json"), &overlays)?;
        write_json(&dir.join("msis.json"), &msis)?;
        write_json(&dir.join("skyboxes.json"), &skyboxes)?;
        write_json(&dir.join("worldareas.json"), &worldareas)?;
        write_json(&dir.join("quickchatcats.json"), &quickchat_categories)?;
        write_json(&dir.join("headbars.json"), &headbars)?;
        write_json(&dir.join("hitmarks.json"), &hitmarks)?;
        write_json(&dir.join("lights.json"), &lights)?;
        write_json(&dir.join("quickchatphrases.json"), &quickchat_phrases)?;
        write_json(&dir.join("billboards.json"), &billboards)?;
        write_json(&dir.join("particleeffectors.json"), &particleeffectors)?;
        write_json(&dir.join("particleemitters.json"), &particleemitters)?;
        write_json(&dir.join("textures.json"), &textures)?;
        write_json(&dir.join("stylesheets.json"), &stylesheets)?;
    }

    print_json(&ConfigSummary {
        params: params.len(),
        enums: enums.len(),
        dbtables: dbtables.len(),
        dbrows: dbrows.len(),
        idks: idks.len(),
        locs: locs.len(),
        npcs: npcs.len(),
        objs: objs.len(),
        seqs: seqs.len(),
        spots: spots.len(),
        bass: bass.len(),
        quests: quests.len(),
        mels: mels.len(),
        waters: waters.len(),
        achievements: achievements.len(),
        materials: materials.len(),
        invs: invs.len(),
        cursors: cursors.len(),
        seqgroups: seqgroups.len(),
        structs: structs.len(),
        controllers: controllers.len(),
        categories: categories.len(),
        areas: areas.len(),
        hunts: hunts.len(),
        mesanims: mesanims.len(),
        itemcodes: itemcodes.len(),
        gamelogevents: gamelogevents.len(),
        bugtemplates: bugtemplates.len(),
        varcstrs: var_client_strings.len(),
        varnbits: varnbits.len(),
        vars: vars.len(),
        varsstrs: var_shared_strings.len(),
        underlays: underlays.len(),
        overlays: overlays.len(),
        msis: msis.len(),
        skyboxes: skyboxes.len(),
        worldareas: worldareas.len(),
        quickchatcats: quickchat_categories.len(),
        headbars: headbars.len(),
        hitmarks: hitmarks.len(),
        lights: lights.len(),
        quickchatphrases: quickchat_phrases.len(),
        billboards: billboards.len(),
        particleeffectors: particleeffectors.len(),
        particleemitters: particleemitters.len(),
        textures: textures.len(),
        stylesheets: stylesheets.len(),
    })
}
