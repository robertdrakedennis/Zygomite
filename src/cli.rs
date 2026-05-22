use crate::animator::decode as decode_animator_controller;
use crate::audio::{AudioKind, inspect_audio_file};
use crate::cache::FlatCache;
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
    ARCHIVE_ACHIEVEMENTS, ARCHIVE_ANIMATOR, ARCHIVE_BILLBOARDS, ARCHIVE_BINARY,
    ARCHIVE_CLIENTSCRIPTS, ARCHIVE_CONFIG, ARCHIVE_CUTSCENE2D, ARCHIVE_DEFAULTS,
    ARCHIVE_ENUM_CONFIG, ARCHIVE_FONTMETRICS, ARCHIVE_INTERFACES, ARCHIVE_LOC_CONFIG,
    ARCHIVE_MAPSQUARES, ARCHIVE_MATERIALS, ARCHIVE_MODELS_RT7, ARCHIVE_NPC_CONFIG,
    ARCHIVE_OBJ_CONFIG, ARCHIVE_PARTICLES, ARCHIVE_QUICKCHAT_CONFIG, ARCHIVE_SEQ_CONFIG,
    ARCHIVE_SPOT_CONFIG, ARCHIVE_STRUCT_CONFIG, ARCHIVE_STYLESHEETS, ARCHIVE_TEXTURES, ARCHIVE_TTF,
    ARCHIVE_UI_ANIM, ARCHIVE_VFX, ARCHIVE_WORLDMAP, AUDIO_ARCHIVES, BUILD,
    CONFIG_GROUP_ACHIEVEMENT_ARCHIVE57, CONFIG_GROUP_AREA, CONFIG_GROUP_BAS,
    CONFIG_GROUP_BILLBOARD_ARCHIVE29, CONFIG_GROUP_BUGTEMPLATE, CONFIG_GROUP_CATEGORY,
    CONFIG_GROUP_CONTROLLER, CONFIG_GROUP_CURSOR, CONFIG_GROUP_DBROW, CONFIG_GROUP_DBTABLE,
    CONFIG_GROUP_GAMELOGEVENT, CONFIG_GROUP_HEADBAR, CONFIG_GROUP_HITMARK, CONFIG_GROUP_HUNT,
    CONFIG_GROUP_IDK, CONFIG_GROUP_INV, CONFIG_GROUP_ITEMCODE, CONFIG_GROUP_LIGHT,
    CONFIG_GROUP_LOC_LEGACY, CONFIG_GROUP_MATERIAL_ARCHIVE26, CONFIG_GROUP_MEL,
    CONFIG_GROUP_MESANIM, CONFIG_GROUP_MSI, CONFIG_GROUP_NPC_LEGACY, CONFIG_GROUP_OBJ_LEGACY,
    CONFIG_GROUP_OVERLAY, CONFIG_GROUP_PARAM, CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27,
    CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27, CONFIG_GROUP_QUEST,
    CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24, CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24, CONFIG_GROUP_SEQ,
    CONFIG_GROUP_SEQGROUP, CONFIG_GROUP_SKYBOX, CONFIG_GROUP_SPOT, CONFIG_GROUP_STRUCT,
    CONFIG_GROUP_UNDERLAY, CONFIG_GROUP_VAR_BIT, CONFIG_GROUP_VAR_CLAN,
    CONFIG_GROUP_VAR_CLAN_SETTING, CONFIG_GROUP_VAR_CLIENT, CONFIG_GROUP_VAR_CLIENT_STRING,
    CONFIG_GROUP_VAR_CONTROLLER, CONFIG_GROUP_VAR_GLOBAL, CONFIG_GROUP_VAR_NPC,
    CONFIG_GROUP_VAR_NPC_BIT, CONFIG_GROUP_VAR_OBJECT, CONFIG_GROUP_VAR_PLAYER,
    CONFIG_GROUP_VAR_PLAYER_GROUP, CONFIG_GROUP_VAR_REGION, CONFIG_GROUP_VAR_SHARED,
    CONFIG_GROUP_VAR_SHARED_STRING, CONFIG_GROUP_VAR_WORLD, CONFIG_GROUP_WATER,
    CONFIG_GROUP_WORLDAREA, DEFAULTS_GROUP_AUDIO, DEFAULTS_GROUP_GRAPHICS, DEFAULTS_GROUP_TITLE,
    DEFAULTS_GROUP_WEARPOS, DEFAULTS_GROUP_WORLDMAP, SUBBUILD,
};
use crate::cutscene2d::decode as decode_cutscene2d;
use crate::dep_tree::{EntityRef, EntityType, ResolverContext, build_tree};
use crate::fixture::{default_tar_path, ensure_archive_complete, open_cache};
use crate::interface::render_interface_group;
use crate::map::decode_map_square;
use crate::model::Model;
use crate::script::{CompiledScript, Instruction, OpcodeBook, Operand, decode_script};
use crate::transpile::Transpiler;
use crate::vars::{VarDomain, parse_var, parse_varbit};
use crate::vfx::decode as decode_vfx;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use image::{ImageBuffer, Rgb};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(name = "rs3-cache-rs")]
#[command(about = "Rust CLI for RS3 cache extraction and parsing")]
pub struct Cli {
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long)]
    pub cache_tar: Option<PathBuf>,
    #[arg(long, default_value = "../rs3-cache/data")]
    pub data_dir: PathBuf,
    #[arg(long, default_value_t = BUILD)]
    pub build: u32,
    #[arg(long, default_value_t = SUBBUILD)]
    pub subbuild: u32,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Interfaces {
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    Varps {
        #[arg(long)]
        out_file: Option<PathBuf>,
        #[arg(long, default_value = "all")]
        domain: VarDomainArg,
    },
    Varbits {
        #[arg(long)]
        out_file: Option<PathBuf>,
    },
    Configs {
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    Cs2 {
        #[arg(long)]
        out_file: Option<PathBuf>,
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    Models {
        #[arg(long)]
        out_file: Option<PathBuf>,
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long)]
        sample_only: bool,
    },
    Audio {
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long)]
        max_files: Option<usize>,
    },
    Unpack {
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        sample_models: bool,
        #[arg(long)]
        skip_audio: bool,
        #[arg(long)]
        max_audio_files: Option<usize>,
    },
    DepTreeInterface {
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeScript {
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeVarp {
        #[arg(long)]
        id: u32,
        #[arg(long)]
        domain: VarDomainArg,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeVarbit {
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    DepTreeConfig {
        #[arg(long)]
        kind: ConfigKindArg,
        #[arg(long)]
        id: u32,
        #[arg(long, default_value_t = 50)]
        max_depth: u32,
        #[arg(long)]
        out_file: PathBuf,
    },
    TsExport {
        #[arg(long)]
        out_dir: PathBuf,
    },
    TranspileScripts {
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long)]
        filter_script: Option<String>,
        #[arg(long, default_value_t = 100)]
        max_scripts: usize,
    },
    MigrateCheck {
        #[arg(long)]
        interface_group: u32,
        #[arg(long)]
        out_file: PathBuf,
        #[arg(long)]
        source_cache_tar: Option<PathBuf>,
        #[arg(long, default_value_t = 947)]
        source_build: u32,
        #[arg(long, default_value_t = 1)]
        source_subbuild: u32,
        /// Enable ID remap planning for conflicted entities.
        #[arg(long)]
        remap: bool,
        /// Buffer above target's max ID for allocating free IDs (default 10000).
        #[arg(long, default_value_t = 10000)]
        remap_buffer: u32,
    },
    MigrateScript {
        #[arg(long)]
        script_id: u32,
        #[arg(long)]
        out_file: PathBuf,
        #[arg(long)]
        source_cache_tar: Option<PathBuf>,
        #[arg(long, default_value_t = 947)]
        source_build: u32,
        #[arg(long, default_value_t = 1)]
        source_subbuild: u32,
        #[arg(long)]
        remap: bool,
        #[arg(long, default_value_t = 10000)]
        remap_buffer: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum VarDomainArg {
    All,
    Player,
    Npc,
    Client,
    World,
    Region,
    Object,
    Clan,
    ClanSetting,
    Controller,
    Global,
    PlayerGroup,
}

impl VarDomainArg {
    fn groups(self) -> &'static [(u32, VarDomain)] {
        const ALL: &[(u32, VarDomain)] = &[
            (CONFIG_GROUP_VAR_PLAYER, VarDomain::Player),
            (CONFIG_GROUP_VAR_NPC, VarDomain::Npc),
            (CONFIG_GROUP_VAR_CLIENT, VarDomain::Client),
            (CONFIG_GROUP_VAR_WORLD, VarDomain::World),
            (CONFIG_GROUP_VAR_REGION, VarDomain::Region),
            (CONFIG_GROUP_VAR_OBJECT, VarDomain::Object),
            (CONFIG_GROUP_VAR_CLAN, VarDomain::Clan),
            (CONFIG_GROUP_VAR_CLAN_SETTING, VarDomain::ClanSetting),
            (CONFIG_GROUP_VAR_CONTROLLER, VarDomain::Controller),
            (CONFIG_GROUP_VAR_GLOBAL, VarDomain::Global),
            (CONFIG_GROUP_VAR_PLAYER_GROUP, VarDomain::PlayerGroup),
        ];
        const PLAYER: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_PLAYER, VarDomain::Player)];
        const NPC: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_NPC, VarDomain::Npc)];
        const CLIENT: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_CLIENT, VarDomain::Client)];
        const WORLD: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_WORLD, VarDomain::World)];
        const REGION: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_REGION, VarDomain::Region)];
        const OBJECT: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_OBJECT, VarDomain::Object)];
        const CLAN: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_CLAN, VarDomain::Clan)];
        const CLAN_SETTING: &[(u32, VarDomain)] =
            &[(CONFIG_GROUP_VAR_CLAN_SETTING, VarDomain::ClanSetting)];
        const CONTROLLER: &[(u32, VarDomain)] =
            &[(CONFIG_GROUP_VAR_CONTROLLER, VarDomain::Controller)];
        const GLOBAL: &[(u32, VarDomain)] = &[(CONFIG_GROUP_VAR_GLOBAL, VarDomain::Global)];
        const PLAYER_GROUP: &[(u32, VarDomain)] =
            &[(CONFIG_GROUP_VAR_PLAYER_GROUP, VarDomain::PlayerGroup)];

        match self {
            Self::All => ALL,
            Self::Player => PLAYER,
            Self::Npc => NPC,
            Self::Client => CLIENT,
            Self::World => WORLD,
            Self::Region => REGION,
            Self::Object => OBJECT,
            Self::Clan => CLAN,
            Self::ClanSetting => CLAN_SETTING,
            Self::Controller => CONTROLLER,
            Self::Global => GLOBAL,
            Self::PlayerGroup => PLAYER_GROUP,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ConfigKindArg {
    Param,
    Enum,
    DbTable,
    DbRow,
    Loc,
    Npc,
    Obj,
    Seq,
    Spot,
    Struct,
    Inv,
    Cursor,
    Idk,
    Bas,
    Mel,
    Water,
    Achievement,
    Material,
    Quest,
    SeqGroup,
    Headbar,
    Hitmark,
    Light,
    SkyBox,
    WorldArea,
    Billboard,
    ParticleEmitter,
    ParticleEffector,
    Texture,
    Stylesheet,
    Controller,
    Category,
    Area,
    Hunt,
    MesAnim,
    ItemCode,
    GameLogEvent,
    BugTemplate,
    QuickChatCat,
    QuickChatPhrase,
    Underlay,
    Overlay,
    Msi,
}

impl ConfigKindArg {
    fn entity_type(self) -> EntityType {
        match self {
            Self::Param => EntityType::Param,
            Self::Enum => EntityType::Enum,
            Self::DbTable => EntityType::DbTable,
            Self::DbRow => EntityType::DbRow,
            Self::Loc => EntityType::Loc,
            Self::Npc => EntityType::Npc,
            Self::Obj => EntityType::Obj,
            Self::Seq => EntityType::Seq,
            Self::Spot => EntityType::Spot,
            Self::Struct => EntityType::Struct,
            Self::Inv => EntityType::Inv,
            Self::Cursor => EntityType::Cursor,
            Self::Idk => EntityType::Idk,
            Self::Bas => EntityType::Bas,
            Self::Mel => EntityType::Mel,
            Self::Water => EntityType::Water,
            Self::Achievement => EntityType::Achievement,
            Self::Material => EntityType::Material,
            Self::Quest => EntityType::Quest,
            Self::SeqGroup => EntityType::SeqGroup,
            Self::Headbar => EntityType::Headbar,
            Self::Hitmark => EntityType::Hitmark,
            Self::Light => EntityType::Light,
            Self::SkyBox => EntityType::SkyBox,
            Self::WorldArea => EntityType::WorldArea,
            Self::Billboard => EntityType::Billboard,
            Self::ParticleEmitter => EntityType::ParticleEmitter,
            Self::ParticleEffector => EntityType::ParticleEffector,
            Self::Texture => EntityType::Texture,
            Self::Stylesheet => EntityType::Stylesheet,
            Self::Controller => EntityType::ControllerConfig,
            Self::Category => EntityType::Category,
            Self::Area => EntityType::Area,
            Self::Hunt => EntityType::Hunt,
            Self::MesAnim => EntityType::MesAnim,
            Self::ItemCode => EntityType::ItemCode,
            Self::GameLogEvent => EntityType::GameLogEvent,
            Self::BugTemplate => EntityType::BugTemplate,
            Self::QuickChatCat => EntityType::QuickChatCat,
            Self::QuickChatPhrase => EntityType::QuickChatPhrase,
            Self::Underlay => EntityType::Underlay,
            Self::Overlay => EntityType::Overlay,
            Self::Msi => EntityType::Msi,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Param => "param",
            Self::Enum => "enum",
            Self::DbTable => "dbtable",
            Self::DbRow => "dbrow",
            Self::Loc => "loc",
            Self::Npc => "npc",
            Self::Obj => "obj",
            Self::Seq => "seq",
            Self::Spot => "spot",
            Self::Struct => "struct",
            Self::Inv => "inv",
            Self::Cursor => "cursor",
            Self::Idk => "idk",
            Self::Bas => "bas",
            Self::Mel => "mel",
            Self::Water => "water",
            Self::Achievement => "achievement",
            Self::Material => "material",
            Self::Quest => "quest",
            Self::SeqGroup => "seqgroup",
            Self::Headbar => "headbar",
            Self::Hitmark => "hitmark",
            Self::Light => "light",
            Self::SkyBox => "skybox",
            Self::WorldArea => "worldarea",
            Self::Billboard => "billboard",
            Self::ParticleEmitter => "particle_emitter",
            Self::ParticleEffector => "particle_effector",
            Self::Texture => "texture",
            Self::Stylesheet => "stylesheet",
            Self::Controller => "controller",
            Self::Category => "category",
            Self::Area => "area",
            Self::Hunt => "hunt",
            Self::MesAnim => "mesanim",
            Self::ItemCode => "itemcode",
            Self::GameLogEvent => "gamelogevent",
            Self::BugTemplate => "bugtemplate",
            Self::QuickChatCat => "quickchatcat",
            Self::QuickChatPhrase => "quickchatphrase",
            Self::Underlay => "underlay",
            Self::Overlay => "overlay",
            Self::Msi => "msi",
        }
    }
}

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

#[derive(Debug, Serialize)]
struct Cs2Summary {
    scripts: usize,
    instructions: usize,
    unique_opcodes: usize,
}

#[derive(Debug, Serialize)]
struct ModelsSummary {
    groups_parsed: usize,
    parse_errors: usize,
}

#[derive(Debug, Serialize)]
struct AudioSummary {
    archives: BTreeMap<u32, usize>,
    kinds: BTreeMap<String, usize>,
    extracted_embedded_ogg: usize,
    manifest_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct AudioManifestEntry {
    archive: u32,
    group: u32,
    file: u32,
    size: usize,
    kind: String,
    raw_extension: String,
    embedded_ogg_offset: Option<usize>,
    extracted_ogg: bool,
}

#[derive(Clone, Copy, Debug)]
struct RuntimeVersion {
    build: u32,
    subbuild: u32,
}

#[derive(Clone, Copy, Debug)]
struct UnpackRunOptions {
    sample_models: bool,
    skip_audio: bool,
    max_audio_files: Option<usize>,
}

pub fn run(cli: Cli) -> Result<()> {
    let tar_path = cli.cache_tar.unwrap_or_else(default_tar_path);
    let cache = open_cache(cli.cache_dir.as_deref())?;
    let version = RuntimeVersion {
        build: cli.build,
        subbuild: cli.subbuild,
    };

    match cli.command {
        Command::Interfaces { out_dir } => {
            run_interfaces(&cache, &tar_path, out_dir.as_deref(), version.build)
        }
        Command::Varps { out_file, domain } => {
            run_varps(&cache, &tar_path, out_file.as_deref(), domain)
        }
        Command::Varbits { out_file } => run_varbits(&cache, &tar_path, out_file.as_deref()),
        Command::Configs { out_dir } => {
            run_configs(&cache, &tar_path, out_dir.as_deref(), version.build)
        }
        Command::Cs2 { out_file, out_dir } => run_cs2(
            &cache,
            &tar_path,
            &cli.data_dir,
            out_file.as_deref(),
            out_dir.as_deref(),
            version,
        ),
        Command::Models {
            out_file,
            out_dir,
            sample_only,
        } => run_models(
            &cache,
            &tar_path,
            out_file.as_deref(),
            out_dir.as_deref(),
            sample_only,
            version.build,
        ),
        Command::Audio { out_dir, max_files } => {
            run_audio(&cache, &tar_path, out_dir.as_deref(), max_files)
        }
        Command::Unpack {
            out_dir,
            sample_models,
            skip_audio,
            max_audio_files,
        } => run_unpack(
            &cache,
            &tar_path,
            &cli.data_dir,
            &out_dir,
            UnpackRunOptions {
                sample_models,
                skip_audio,
                max_audio_files,
            },
            version,
        ),
        Command::DepTreeInterface {
            id,
            max_depth,
            out_file,
        } => run_dep_tree_interface(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::DepTreeScript {
            id,
            max_depth,
            out_file,
        } => run_dep_tree_script(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::DepTreeVarp {
            id,
            domain,
            max_depth,
            out_file,
        } => run_dep_tree_varp(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            domain,
            max_depth,
            &out_file,
            version,
        ),
        Command::DepTreeVarbit {
            id,
            max_depth,
            out_file,
        } => run_dep_tree_varbit(
            &cache,
            &tar_path,
            &cli.data_dir,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::DepTreeConfig {
            kind,
            id,
            max_depth,
            out_file,
        } => run_dep_tree_config(
            &cache,
            &tar_path,
            &cli.data_dir,
            kind,
            id,
            max_depth,
            &out_file,
            version,
        ),
        Command::TsExport { out_dir } => {
            run_ts_export(&cache, &tar_path, &cli.data_dir, &out_dir, version)
        }
        Command::TranspileScripts {
            out_dir,
            filter_script,
            max_scripts,
        } => run_transpile_scripts(
            &cache,
            &tar_path,
            &cli.data_dir,
            &out_dir,
            filter_script.as_deref(),
            max_scripts,
            version,
        ),
        Command::MigrateCheck {
            interface_group,
            out_file,
            source_cache_tar,
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
        } => run_migrate_check(
            &cache,
            &tar_path,
            &cli.data_dir,
            interface_group,
            &out_file,
            version,
            source_cache_tar.as_deref(),
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
        ),
        Command::MigrateScript {
            script_id,
            out_file,
            source_cache_tar,
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
        } => run_migrate_script(
            &cache,
            &tar_path,
            &cli.data_dir,
            script_id,
            &out_file,
            version,
            source_cache_tar.as_deref(),
            source_build,
            source_subbuild,
            remap,
            remap_buffer,
        ),
    }
}

fn run_interfaces(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: Option<&Path>,
    build: u32,
) -> Result<()> {
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

fn run_varps(
    cache: &FlatCache,
    tar_path: &Path,
    out_file: Option<&Path>,
    domain: VarDomainArg,
) -> Result<()> {
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

fn run_varbits(cache: &FlatCache, tar_path: &Path, out_file: Option<&Path>) -> Result<()> {
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

fn run_configs(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: Option<&Path>,
    build: u32,
) -> Result<()> {
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

fn run_cs2(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_file: Option<&Path>,
    out_dir: Option<&Path>,
    version: RuntimeVersion,
) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CLIENTSCRIPTS)?;
    let cache = FlatCache::open(cache.root())?;
    let opcode_book = OpcodeBook::load(data_dir, version.build, version.subbuild)?;
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let keep_decoded = out_file.is_some();
    let write_source = out_dir.is_some();
    let script_group_names = load_script_group_names(&index, data_dir)?;

    if let Some(path) = out_dir {
        fs::create_dir_all(path).with_context(|| format!("failed creating {}", path.display()))?;
    }

    let mut scripts = 0_usize;
    let mut instructions = 0_usize;
    let mut opcode_names = HashMap::<String, usize>::new();
    let mut decoded_all = Vec::new();

    if write_source {
        for group in &index.group_id {
            let files = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
            let single_file_group = files.len() == 1;

            for (file, bytes) in files {
                let script = decode_script(&bytes, &opcode_book, version.build)?;
                scripts += 1;
                instructions += script.code.len();
                for instruction in &script.code {
                    *opcode_names.entry(instruction.command.clone()).or_insert(0) += 1;
                }
                if keep_decoded {
                    decoded_all.push(script.clone());
                }

                if let Some(dir) = out_dir {
                    let hint = script_group_names
                        .get(group)
                        .map(String::as_str)
                        .or(script.name.as_deref())
                        .unwrap_or("script");
                    let source_name = sanitize_file_component(hint);
                    let file_name = if single_file_group {
                        format!("{group}_{source_name}.cs2")
                    } else {
                        format!("{group}_{file}_{source_name}.cs2")
                    };
                    let path = dir.join(file_name);
                    write_text(&path, &format_script_source(*group, file, &script))?;
                }
            }
        }
    } else {
        struct GroupCs2Result {
            scripts: usize,
            instructions: usize,
            opcode_counts: HashMap<String, usize>,
            decoded: Vec<CompiledScript>,
        }

        let group_results = index
            .group_id
            .par_iter()
            .map(|group| -> Result<GroupCs2Result> {
                let files = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
                let mut scripts = 0_usize;
                let mut instructions = 0_usize;
                let mut opcode_counts = HashMap::<String, usize>::new();
                let mut decoded = Vec::new();

                for (_, bytes) in files {
                    let script = decode_script(&bytes, &opcode_book, version.build)?;
                    for instruction in &script.code {
                        *opcode_counts
                            .entry(instruction.command.clone())
                            .or_insert(0) += 1;
                    }
                    instructions += script.code.len();
                    scripts += 1;
                    if keep_decoded {
                        decoded.push(script);
                    }
                }

                Ok(GroupCs2Result {
                    scripts,
                    instructions,
                    opcode_counts,
                    decoded,
                })
            })
            .collect::<Vec<_>>();

        for result in group_results {
            let result = result?;
            scripts += result.scripts;
            instructions += result.instructions;
            for (opcode, count) in result.opcode_counts {
                *opcode_names.entry(opcode).or_insert(0) += count;
            }
            if keep_decoded {
                decoded_all.extend(result.decoded);
            }
        }
    }

    if let Some(path) = out_file {
        write_json(path, &decoded_all)?;
    }
    print_json(&Cs2Summary {
        scripts,
        instructions,
        unique_opcodes: opcode_names.len(),
    })
}

fn run_models(
    cache: &FlatCache,
    tar_path: &Path,
    out_file: Option<&Path>,
    out_dir: Option<&Path>,
    sample_only: bool,
    build: u32,
) -> Result<()> {
    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MODELS_RT7)?;
    let cache = FlatCache::open(cache.root())?;
    let index = cache.archive_index(ARCHIVE_MODELS_RT7)?;
    let available_groups: HashSet<u32> = index.group_id.iter().copied().collect();

    let groups: Vec<u32> = if sample_only {
        let mut sample = (0_u32..=100).collect::<Vec<_>>();
        sample.extend([1_000, 5_000, 10_000, 50_000, 100_000]);
        if let Some(last) = index.group_id.last() {
            sample.push(*last);
        }
        sample.sort_unstable();
        sample.dedup();
        sample.retain(|group| available_groups.contains(group));
        sample
    } else {
        index.group_id.clone()
    };

    if let Some(path) = out_dir {
        fs::create_dir_all(path).with_context(|| format!("failed creating {}", path.display()))?;
        let mut parsed = Vec::new();
        let mut parsed_count = 0_usize;
        let mut parse_errors = 0_usize;

        for group in &groups {
            let files = cache.group_files_with_index(&index, ARCHIVE_MODELS_RT7, *group)?;
            let Some(bytes) = files.get(&0) else {
                continue;
            };
            match Model::decode(bytes, build) {
                Ok(model) => {
                    parsed_count += 1;
                    let model_path = path.join(format!("model_{group}.json"));
                    write_json(&model_path, &model)?;
                    if out_file.is_some() {
                        parsed.push((*group, model));
                    }
                }
                Err(_) => {
                    parse_errors += 1;
                }
            }
        }

        if let Some(path) = out_file {
            write_json(path, &parsed)?;
        }
        return print_json(&ModelsSummary {
            groups_parsed: parsed_count,
            parse_errors,
        });
    }

    struct ModelGroupResult {
        parsed_count: usize,
        parse_errors: usize,
        parsed_model: Option<(u32, Model)>,
    }

    let keep_models = out_file.is_some();
    let group_results = groups
        .par_iter()
        .map(|group| -> Result<ModelGroupResult> {
            let files = cache.group_files_with_index(&index, ARCHIVE_MODELS_RT7, *group)?;
            let Some(bytes) = files.get(&0) else {
                return Ok(ModelGroupResult {
                    parsed_count: 0,
                    parse_errors: 0,
                    parsed_model: None,
                });
            };
            match Model::decode(bytes, build) {
                Ok(model) => Ok(ModelGroupResult {
                    parsed_count: 1,
                    parse_errors: 0,
                    parsed_model: keep_models.then_some((*group, model)),
                }),
                Err(_) => Ok(ModelGroupResult {
                    parsed_count: 0,
                    parse_errors: 1,
                    parsed_model: None,
                }),
            }
        })
        .collect::<Vec<_>>();

    let mut parsed = Vec::new();
    let mut parsed_count = 0_usize;
    let mut parse_errors = 0_usize;
    for result in group_results {
        let result = result?;
        parsed_count += result.parsed_count;
        parse_errors += result.parse_errors;
        if let Some(model) = result.parsed_model {
            parsed.push(model);
        }
    }

    if let Some(path) = out_file {
        write_json(path, &parsed)?;
    }
    print_json(&ModelsSummary {
        groups_parsed: parsed_count,
        parse_errors,
    })
}

fn run_unpack(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    options: UnpackRunOptions,
    version: RuntimeVersion,
) -> Result<()> {
    let interface_dir = out_dir.join("interface");
    let config_dir = out_dir.join("config");
    let script_dir = out_dir.join("script");
    let model_dir = out_dir.join("model");
    let audio_dir = out_dir.join("audio");

    run_interfaces(cache, tar_path, Some(&interface_dir), version.build)?;
    run_varps(
        cache,
        tar_path,
        Some(&config_dir.join("varps.json")),
        VarDomainArg::All,
    )?;
    run_varbits(cache, tar_path, Some(&config_dir.join("varbits.json")))?;
    run_configs(cache, tar_path, Some(config_dir.as_path()), version.build)?;
    run_cs2(
        cache,
        tar_path,
        data_dir,
        Some(&script_dir.join("scripts.json")),
        Some(&script_dir.join("decompiled")),
        version,
    )?;

    if options.sample_models {
        run_models(
            cache,
            tar_path,
            Some(&model_dir.join("models_sample.json")),
            Some(&model_dir.join("decoded")),
            true,
            version.build,
        )?;
    } else {
        run_models(
            cache,
            tar_path,
            Some(&model_dir.join("models.json")),
            Some(&model_dir.join("decoded")),
            false,
            version.build,
        )?;
    }

    if !options.skip_audio {
        run_audio(cache, tar_path, Some(&audio_dir), options.max_audio_files)?;
    }

    run_top_level_exports(cache, tar_path, data_dir, out_dir, version.build)?;

    Ok(())
}

fn run_top_level_exports(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    build: u32,
) -> Result<()> {
    let hash_names = load_other_names_map(data_dir)?;

    export_archive_raw(
        cache,
        tar_path,
        ARCHIVE_BINARY,
        &out_dir.join("binary"),
        ".dat",
        &hash_names,
    )
    .context("export binary archive")?;
    export_archive_raw(
        cache,
        tar_path,
        ARCHIVE_TTF,
        &out_dir.join("ttf"),
        ".ttf",
        &hash_names,
    )
    .context("export ttf archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_FONTMETRICS,
        &out_dir.join("fontmetrics"),
        ".json",
        &hash_names,
        |_, _, data| parse_fontmetrics(data),
    )
    .context("export fontmetrics archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_VFX,
        &out_dir.join("vfx"),
        ".json",
        &hash_names,
        |_, _, data| decode_vfx(data),
    )
    .context("export vfx archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_ANIMATOR,
        &out_dir.join("animator"),
        ".json",
        &hash_names,
        |_, _, data| decode_animator_controller(data),
    )
    .context("export animator archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_CUTSCENE2D,
        &out_dir.join("cutscene2d"),
        ".json",
        &hash_names,
        |_, _, data| decode_cutscene2d(data),
    )
    .context("export cutscene2d archive")?;

    export_group_json(
        cache,
        tar_path,
        ARCHIVE_UI_ANIM,
        0,
        &out_dir.join("uianimcurve"),
        ".json",
        |_, _, data| parse_uianimcurve(data),
    )
    .context("export uianimcurve group")?;
    export_group_json(
        cache,
        tar_path,
        ARCHIVE_UI_ANIM,
        1,
        &out_dir.join("uianim"),
        ".json",
        |_, _, data| parse_uianim(data),
    )
    .context("export uianim group")?;

    export_mapsquares_json(cache, tar_path, &out_dir.join("maps"), build)
        .context("export mapsquares")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_GRAPHICS,
        &out_dir.join("config/graphics.defaults"),
        |id, data| parse_graphics_defaults(id, data, build),
    )
    .context("export graphics defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_AUDIO,
        &out_dir.join("config/audio.defaults"),
        |id, data| parse_audio_defaults(id, data, build),
    )
    .context("export audio defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_WEARPOS,
        &out_dir.join("config/wearpos.defaults"),
        parse_wearpos_defaults,
    )
    .context("export wearpos defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_WORLDMAP,
        &out_dir.join("config/worldmap.defaults"),
        parse_worldmap_defaults,
    )
    .context("export worldmap defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_TITLE,
        &out_dir.join("config/title.defaults"),
        parse_title_defaults,
    )
    .context("export title defaults")?;
    export_worldmap_dump(cache, tar_path, &out_dir.join("worldmap"))
        .context("export worldmap dump")?;
    export_worldarea_png(cache, tar_path, &out_dir.join("areas.png"))
        .context("export worldarea png")?;
    Ok(())
}

fn load_other_names_map(data_dir: &Path) -> Result<HashMap<i32, String>> {
    let other = data_dir.join("names/other.txt");
    if !other.is_file() {
        return Ok(HashMap::new());
    }
    load_hash_name_map(&other)
}

fn export_archive_raw(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    out_dir: &Path,
    extension: &str,
    hash_names: &HashMap<i32, String>,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let mut count = 0_usize;

    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, archive, *group)?;
        let group_name = resolve_group_name(&index, *group, hash_names);
        if files.len() == 1 && files.contains_key(&0) {
            let mut name = group_name
                .or_else(|| resolve_file_name(&index, *group, 0, hash_names))
                .unwrap_or_else(|| group.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            write_binary(
                &out_dir.join(sanitize_path_component(&name)),
                files[&0].as_slice(),
            )?;
            count += 1;
            continue;
        }

        let group_dir = out_dir.join(
            group_name
                .as_deref()
                .map(sanitize_path_component)
                .unwrap_or_else(|| group.to_string()),
        );
        fs::create_dir_all(&group_dir)
            .with_context(|| format!("failed creating {}", group_dir.display()))?;

        for (file, data) in files {
            let mut name = resolve_file_name(&index, *group, file, hash_names)
                .unwrap_or_else(|| file.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            write_binary(&group_dir.join(sanitize_path_component(&name)), &data)?;
            count += 1;
        }
    }

    Ok(count)
}

fn export_archive_json<T, F>(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    out_dir: &Path,
    extension: &str,
    hash_names: &HashMap<i32, String>,
    parse: F,
) -> Result<usize>
where
    T: Serialize,
    F: Fn(u32, u32, &[u8]) -> Result<T>,
{
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let mut count = 0_usize;

    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, archive, *group)?;
        let group_name = resolve_group_name(&index, *group, hash_names);
        if files.len() == 1 && files.contains_key(&0) {
            let mut name = group_name
                .or_else(|| resolve_file_name(&index, *group, 0, hash_names))
                .unwrap_or_else(|| group.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            let parsed = parse(*group, 0, files[&0].as_slice())?;
            write_json(&out_dir.join(sanitize_path_component(&name)), &parsed)?;
            count += 1;
            continue;
        }

        let group_dir = out_dir.join(
            group_name
                .as_deref()
                .map(sanitize_path_component)
                .unwrap_or_else(|| group.to_string()),
        );
        fs::create_dir_all(&group_dir)
            .with_context(|| format!("failed creating {}", group_dir.display()))?;

        for (file, data) in files {
            let mut name = resolve_file_name(&index, *group, file, hash_names)
                .unwrap_or_else(|| file.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            let parsed = parse(*group, file, &data)?;
            write_json(&group_dir.join(sanitize_path_component(&name)), &parsed)?;
            count += 1;
        }
    }

    Ok(count)
}

fn export_group_json<T, F>(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    group: u32,
    out_dir: &Path,
    extension: &str,
    parse: F,
) -> Result<usize>
where
    T: Serialize,
    F: Fn(u32, u32, &[u8]) -> Result<T>,
{
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }
    let Some(_payload) = cache.get(archive, group)? else {
        return Ok(0);
    };

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let files = cache.group_files_with_index(&index, archive, group)?;
    let mut count = 0_usize;
    for (file, data) in files {
        let parsed = parse(group, file, &data)?;
        let path = out_dir.join(format!("{file}{extension}"));
        write_json(&path, &parsed)?;
        count += 1;
    }
    Ok(count)
}

fn export_mapsquares_json(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    build: u32,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MAPSQUARES).is_err() {
        return Ok(0);
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;

    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let mut count = 0_usize;
    let mut parse_errors = 0_usize;
    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, ARCHIVE_MAPSQUARES, *group)?;
        let square_x = group & 0b111_1111;
        let square_z = group >> 7;
        match decode_map_square(&files, build) {
            Ok(decoded) => {
                let path = out_dir.join(format!("{square_x}_{square_z}.json"));
                write_json(&path, &decoded)?;
                count += 1;
            }
            Err(error) => {
                parse_errors += 1;
                let error_path = out_dir.join(format!("{square_x}_{square_z}.error.txt"));
                write_text(&error_path, &format!("{error:#}"))?;
            }
        }
    }

    if parse_errors > 0 {
        eprintln!("mapsquares: parsed={count} errors={parse_errors}");
    }

    Ok(count)
}

fn export_defaults_text<F>(
    cache: &FlatCache,
    tar_path: &Path,
    group: u32,
    out_file: &Path,
    parse: F,
) -> Result<usize>
where
    F: Fn(u32, &[u8]) -> Result<Vec<String>>,
{
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_DEFAULTS).is_err() {
        return Ok(0);
    }
    let index = cache.archive_index(ARCHIVE_DEFAULTS)?;
    if !index.group_id.contains(&group) {
        return Ok(0);
    }

    let files = cache.group_files_with_index(&index, ARCHIVE_DEFAULTS, group)?;
    if files.is_empty() {
        return Ok(0);
    }

    let mut file_ids = files.keys().copied().collect::<Vec<_>>();
    file_ids.sort_unstable();

    let mut lines = Vec::new();
    for file in &file_ids {
        let data = files
            .get(file)
            .with_context(|| format!("missing defaults file {file} in group {group}"))?;
        lines.extend(parse(*file, data)?);
        lines.push(String::new());
    }

    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    write_text(out_file, &lines.join("\n"))?;
    Ok(file_ids.len())
}

fn resolve_group_name(
    index: &crate::js5::ArchiveIndex,
    group: u32,
    hash_names: &HashMap<i32, String>,
) -> Option<String> {
    let names = index.group_name_hash.as_ref()?;
    let group_idx = usize::try_from(group).ok()?;
    let hash = *names.get(group_idx)?;
    if hash == -1 {
        return None;
    }
    hash_names.get(&hash).cloned()
}

fn resolve_file_name(
    index: &crate::js5::ArchiveIndex,
    group: u32,
    file: u32,
    hash_names: &HashMap<i32, String>,
) -> Option<String> {
    let group_names = index.group_file_names.as_ref()?;
    let group_idx = usize::try_from(group).ok()?;
    let file_idx = usize::try_from(file).ok()?;
    let file_hashes = group_names.get(group_idx)?.as_ref()?;
    let hash = *file_hashes.get(file_idx)?;
    if hash == -1 {
        return None;
    }
    hash_names.get(&hash).cloned()
}

fn sanitize_path_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']' | ',') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        String::from("unnamed")
    } else {
        out
    }
}

fn export_worldmap_dump(cache: &FlatCache, tar_path: &Path, out_dir: &Path) -> Result<()> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_WORLDMAP).is_err() {
        return Ok(());
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(ARCHIVE_WORLDMAP)?;
    if let Some(main_group) = find_group_by_name(&index, "main")
        && let Some(details_file) = find_file_by_name(&index, main_group, "details.dat")
        && let Some(labels_file) = find_file_by_name(&index, main_group, "labels.dat")
    {
        let lines = export_worldmap_legacy(cache, &index, main_group, details_file, labels_file)?;
        write_text(&out_dir.join("dump.wma"), &lines.join("\n"))?;
        return Ok(());
    }

    let details_group = find_group_by_name(&index, "details").unwrap_or(0);
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, details_group)? else {
        return Ok(());
    };
    let details_files = crate::js5::unpack_group(&index, details_group, &payload)?;
    let mut lines = Vec::new();
    for (id, data) in details_files {
        let debug_name = unpack_worldmap_details(id, &data, &mut lines)?;
        unpack_worldmap_static_elements(cache, &index, &debug_name, &mut lines)?;
        unpack_worldmap_labels(cache, &index, &debug_name, &mut lines)?;
        lines.push(String::new());
    }

    Ok(())
}

fn export_worldmap_legacy(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    group: u32,
    details_file: u32,
    labels_file: u32,
) -> Result<Vec<String>> {
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(Vec::new());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    let details = files.get(&details_file).with_context(|| {
        format!("legacy worldmap missing details file {details_file} in group {group}")
    })?;
    let labels = files.get(&labels_file).with_context(|| {
        format!("legacy worldmap missing labels file {labels_file} in group {group}")
    })?;

    let mut detail_packet = crate::packet::Packet::new(details);
    let mut lines = Vec::new();
    lines.push(String::from("[main]"));
    lines.push(format!(
        "origin={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(format!(
        "min={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(format!(
        "max={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(String::new());

    let mut label_packet = crate::packet::Packet::new(labels);
    let label_count = usize::from(label_packet.g2()?);
    for _ in 0..label_count {
        let text = label_packet.gjstr()?;
        let x = label_packet.g2()?;
        let y = label_packet.g2()?;
        let kind = label_packet.g1()?;
        lines.push(format!("label={x},{y},{text},{kind}"));
    }
    Ok(lines)
}

fn unpack_worldmap_details(id: u32, data: &[u8], lines: &mut Vec<String>) -> Result<String> {
    let mut packet = crate::packet::Packet::new(data);
    let debug_name = packet.gjstr()?;
    lines.push(format!("[{debug_name}]"));
    lines.push(format!("name={}", packet.gjstr()?));
    lines.push(format!("origin={}", format_coordgrid(packet.g4s()?)));
    lines.push(format!("background={}", format_colour(packet.g4s()?)));
    lines.push(format!("listed={}", yes_no(packet.g1()? == 1)));
    let default_zoom = packet.g1()?;
    lines.push(if default_zoom == u8::MAX {
        String::from("zoom=default")
    } else {
        format!("zoom={default_zoom}")
    });
    lines.push(format!("buildarea={}", packet.g1()?));
    let count = usize::from(packet.g1()?);
    for _ in 0..count {
        lines.push(format!(
            "subarea={},{},{},{},{},{},{},{},{}",
            packet.g1()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?
        ));
    }
    if !packet.is_done() {
        bail!("worldmap details {id} did not consume full payload");
    }
    Ok(debug_name)
}

fn unpack_worldmap_static_elements(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    debug_name: &str,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(group) = find_group_by_name(index, &format!("{debug_name}_staticelements")) else {
        return Ok(());
    };
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    for (_, data) in files {
        let mut packet = crate::packet::Packet::new(&data);
        lines.push(format!(
            "element={},{},{}",
            format_coordgrid(packet.g4s()?),
            format_map_element(packet.g2()?),
            packet.g1()?
        ));
    }
    Ok(())
}

fn unpack_worldmap_labels(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    debug_name: &str,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(group) = find_group_by_name(index, &format!("{debug_name}_labels")) else {
        return Ok(());
    };
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    for (_, data) in files {
        let mut packet = crate::packet::Packet::new(&data);
        lines.push(format!(
            "label={},{},{}",
            format_coordgrid(packet.g4s()?),
            format_map_element(packet.g2()?),
            packet.g1()?
        ));
    }
    Ok(())
}

fn format_coordgrid(value: i32) -> String {
    if value == -1 {
        return String::from("null");
    }
    let as_u32 = value as u32;
    let level = as_u32 >> 28;
    let x = (as_u32 >> 14) & 0x3fff;
    let z = as_u32 & 0x3fff;
    format!("{level}_{}_{}_{}_{}", x / 64, z / 64, x % 64, z % 64)
}

fn format_colour(value: i32) -> String {
    let as_u32 = value as u32;
    if as_u32 > 0x00ff_ffff {
        format!("0x{as_u32:08x}")
    } else {
        format!("0x{as_u32:06x}")
    }
}

fn format_map_element(value: u16) -> String {
    format!("mapelement_{value}")
}

fn find_group_by_name(index: &crate::js5::ArchiveIndex, name: &str) -> Option<u32> {
    let hash = java_string_hash(name);
    let hashes = index.group_name_hash.as_ref()?;
    index.group_id.iter().copied().find(|group| {
        usize::try_from(*group)
            .ok()
            .and_then(|idx| hashes.get(idx))
            .is_some_and(|value| *value == hash)
    })
}

fn find_file_by_name(index: &crate::js5::ArchiveIndex, group: u32, name: &str) -> Option<u32> {
    let hash = java_string_hash(name);
    let group_idx = usize::try_from(group).ok()?;
    let names = index.group_file_names.as_ref()?.get(group_idx)?.as_ref()?;
    names.iter().enumerate().find_map(|(file, entry_hash)| {
        if *entry_hash == hash {
            u32::try_from(file).ok()
        } else {
            None
        }
    })
}

fn export_worldarea_png(cache: &FlatCache, tar_path: &Path, out_file: &Path) -> Result<()> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_WORLDMAP).is_err() {
        return Ok(());
    }
    let index = cache.archive_index(ARCHIVE_WORLDMAP)?;
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, 3)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(&index, 3, &payload)?;

    let width = 128_usize * 8;
    let height = 256_usize * 8;
    let mut image = vec![0_u8; width * height * 3];

    for (file, data) in files {
        let square_x = usize::try_from(file & 0x7f).context("square_x overflow")?;
        let square_z = usize::try_from(file >> 7).context("square_z overflow")?;
        let colors = decode_worldmap_color(&data)?;
        for zone_x in 0..8_usize {
            for zone_z in 0..8_usize {
                let x = 8 * square_x + zone_x;
                let z = 8 * square_z + zone_z;
                if x >= width || z >= height {
                    continue;
                }
                let color = colors[8 * zone_x + zone_z];
                let offset = ((height - 1 - z) * width + x) * 3;
                image[offset] = u8::try_from((color >> 16) & 0xff).context("red overflow")?;
                image[offset + 1] = u8::try_from((color >> 8) & 0xff).context("green overflow")?;
                image[offset + 2] = u8::try_from(color & 0xff).context("blue overflow")?;
            }
        }
    }

    let width_u32 = u32::try_from(width).context("worldarea png width overflow")?;
    let height_u32 = u32::try_from(height).context("worldarea png height overflow")?;
    let Some(buffer) = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width_u32, height_u32, image)
    else {
        bail!("failed to build worldarea image buffer");
    };
    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    buffer
        .save(out_file)
        .with_context(|| format!("failed writing {}", out_file.display()))?;
    Ok(())
}

fn decode_worldmap_color(data: &[u8]) -> Result<[u32; 64]> {
    let mut result = [0_u32; 64];
    let mut packet = crate::packet::Packet::new(data);
    let mut index = 0_usize;
    let mut target = 0_usize;

    while target < 64 {
        let value = packet.g3()?;
        if packet.is_done() {
            target = 64;
        } else {
            target = target
                .checked_add(usize::from(packet.g1()?))
                .context("worldmap color run overflow")?;
        }
        while index < target && index < 64 {
            result[index] = value;
            index += 1;
        }
    }

    Ok(result)
}

fn parse_defaults_eof(kind: &str, id: u32, packet: &crate::packet::Packet<'_>) -> Result<()> {
    if packet.is_done() {
        return Ok(());
    }
    bail!("{kind}_{id} end of file not reached")
}

fn parse_audio_defaults(id: u32, data: &[u8], build: u32) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[audiodefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("audiodefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                let song = if build >= 912 {
                    packet.g4s()?
                } else {
                    i32::from(packet.g2()?)
                };
                lines.push(format!("titlescreensong={song}"));
            }
            opcode => bail!("audiodefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn format_wearpos(slot: u8) -> Result<&'static str> {
    let value = match slot {
        0 => "hat",
        1 => "back",
        2 => "front",
        3 => "righthand",
        4 => "torso",
        5 => "lefthand",
        6 => "arms",
        7 => "legs",
        8 => "head",
        9 => "hands",
        10 => "feet",
        11 => "jaw",
        12 => "ring",
        13 => "quiver",
        14 => "aura",
        15 => "wearpos_15",
        16 => "wearpos_16",
        17 => "pocket",
        18 => "wings",
        value => bail!("wearpos {value}"),
    };
    Ok(value)
}

fn parse_wearpos_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[wearposdefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("wearposdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(packet.g1()?.to_string());
                }
                lines.push(format!("unknown1={}", values.join(",")));
            }
            3 => lines.push(format!("lefthand={}", format_wearpos(packet.g1()?)?)),
            4 => lines.push(format!("righthand={}", format_wearpos(packet.g1()?)?)),
            5 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(format_wearpos(packet.g1()?)?.to_string());
                }
                lines.push(format!("lefthandextra={}", values.join(",")));
            }
            6 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(format_wearpos(packet.g1()?)?.to_string());
                }
                lines.push(format!("righthandextra={}", values.join(",")));
            }
            opcode => bail!("wearposdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_worldmap_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[worldmapdefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("worldmapdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => lines.push(format!("unknown1={}", packet.g4s()?)),
            2 => lines.push(format!("membersfillcolour=0x{:x}", packet.g4s()? as u32)),
            3 => lines.push(format!("membersbordercolour=0x{:x}", packet.g4s()? as u32)),
            4 => lines.push(format!("membersborderthickness={}", packet.g1()?)),
            5 => lines.push(format!("memberschamferwidth={}", packet.g1()?)),
            6 => lines.push(format!("mainarea={}", packet.g4s()?)),
            7 => lines.push(format!("textshadowcolour=0x{:x}", packet.g4s()? as u32)),
            100 => lines.push(format!("font0zoom0={}", packet.g2()?)),
            101 => lines.push(format!("font1zoom0={}", packet.g2()?)),
            102 => lines.push(format!("font2zoom0={}", packet.g2()?)),
            108 => lines.push(format!("font0zoom1={}", packet.g2()?)),
            109 => lines.push(format!("font1zoom1={}", packet.g2()?)),
            110 => lines.push(format!("font2zoom1={}", packet.g2()?)),
            116 => lines.push(format!("font0zoom2={}", packet.g2()?)),
            117 => lines.push(format!("font1zoom2={}", packet.g2()?)),
            118 => lines.push(format!("font2zoom2={}", packet.g2()?)),
            124 => lines.push(format!("font0zoom3={}", packet.g2()?)),
            125 => lines.push(format!("font1zoom3={}", packet.g2()?)),
            126 => lines.push(format!("font2zoom3={}", packet.g2()?)),
            132 => lines.push(format!("font0zoom4={}", packet.g2()?)),
            133 => lines.push(format!("font1zoom4={}", packet.g2()?)),
            134 => lines.push(format!("font2zoom4={}", packet.g2()?)),
            opcode => bail!("worldmapdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_title_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[titledefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("titledefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => lines.push(format!(
                "title={},{}",
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?
            )),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    lines.push(format!("unknown2={},{}", packet.g1()?, packet.g1()?));
                }
            }
            opcode => bail!("titledefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_graphics_defaults(id: u32, data: &[u8], build: u32) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[graphicsdefaults_{id}]")];
    let mut hitmark_count = 4_u8;

    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("graphicsdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                for i in 0..hitmark_count {
                    lines.push(format!("hitmark{i}pos={},{}", packet.g2s()?, packet.g2s()?));
                }
            }
            2 => {
                let model = if build < 681 {
                    packet.g2null()?
                } else {
                    packet.gsmart2or4null()?
                };
                lines.push(format!("performancemetricsmodel={model}"));
            }
            3 => {
                hitmark_count = packet.g1()?;
                lines.push(format!("hitmarkcount={hitmark_count}"));
            }
            4 => lines.push(String::from("unknown4=no")),
            5 => lines.push(format!("titleinterface={}", packet.g3()?)),
            6 => lines.push(format!("lobbyinterface={}", packet.g3()?)),
            7 => {
                for i in 0..10_u8 {
                    for j in 0..4_u8 {
                        lines.push(format!("playerrecol{i}s{j}={}", packet.g2null()?));
                        let count = usize::from(packet.g2()?);
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(packet.g2null()?.to_string());
                        }
                        lines.push(format!("playerrecol{i}d{j}={}", values.join(",")));
                    }
                }
            }
            8 => lines.push(String::from("npcchatline=no")),
            9 => lines.push(format!("npcchatlineduration={}", packet.g1()?)),
            10 => lines.push(String::from("playerchatline=no")),
            11 => lines.push(format!("playerchatlineduration={}", packet.g1()?)),
            12 => lines.push(format!("initialsize={},{}", packet.g2()?, packet.g2()?)),
            13 => lines.push(format!("headbarcount={}", packet.g1()?)),
            14 => lines.push(format!("headbarupdatecount={}", packet.g1()?)),
            15 => lines.push(format!("entityoverlayoffset={}", packet.g1()?)),
            16 => lines.push(String::from("somethingcamera=yes")),
            17 => lines.push(format!("objnumcolour=0x{:x}", packet.g4s()? as u32)),
            18 => lines.push(format!("objnumcolourk=0x{:x}", packet.g4s()? as u32)),
            19 => lines.push(format!("objnumcolourm=0x{:x}", packet.g4s()? as u32)),
            20 => lines.push(format!(
                "spotshadowtexture={},{}",
                packet.g2()?,
                packet.g1()?
            )),
            21 => lines.push(format!("minimapscale={}", packet.g1()?)),
            22 => {
                let p11full = packet.gsmart2or4null()?;
                let p12full = packet.gsmart2or4null()?;
                let b12full = packet.gsmart2or4null()?;
                let hintheadicon = packet.gsmart2or4null()?;
                let hintmapmarker = packet.gsmart2or4null()?;
                let mapflag = packet.gsmart2or4null()?;
                let mapflag_origin = (packet.g1s()?, packet.g1s()?);
                let cross = packet.gsmart2or4null()?;
                let mapdot = packet.gsmart2or4null()?;
                let nameicon = packet.gsmart2or4null()?;
                let floorshadow = packet.gsmart2or4null()?;
                let compass = packet.gsmart2or4null()?;
                let otherlevel = packet.gsmart2or4null()?;
                let mapedge = packet.gsmart2or4null()?;
                lines.push(format!(
                    "sprites={p11full},{p12full},{b12full},{hintheadicon},{hintmapmarker},{mapflag},{},{},{cross},{mapdot},{nameicon},{floorshadow},{compass},{otherlevel},{mapedge}",
                    mapflag_origin.0, mapflag_origin.1
                ));
            }
            23 => {
                for i in 0..10_u8 {
                    for j in 0..4_u8 {
                        lines.push(format!("playerretex{i}s{j}={}", packet.g2null()?));
                        let count = usize::from(packet.g2()?);
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(packet.g2null()?.to_string());
                        }
                        lines.push(format!("playerretex{i}d{j}={}", values.join(",")));
                    }
                }
            }
            24 => lines.push(format!("unknown24={}", packet.g4s()?)),
            25 => lines.push(format!(
                "unknown25={},{},{},{},{},{}",
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?
            )),
            26 => lines.push(format!("objnumcolourb=0x{:x}", packet.g4s()? as u32)),
            27 => lines.push(format!("objnumcolourt=0x{:x}", packet.g4s()? as u32)),
            28 => lines.push(format!("objnumcolourq=0x{:x}", packet.g4s()? as u32)),
            29 => lines.push(format!("unknown29={},{}", packet.g4s()?, packet.g4s()?)),
            opcode => bail!("graphicsdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct UiAnimCurveEntry {
    keyframes: Vec<[f32; 4]>,
}

fn parse_uianimcurve(data: &[u8]) -> Result<UiAnimCurveEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let count = usize::from(packet.g1()?);
    let mut keyframes = Vec::with_capacity(count);
    for _ in 0..count {
        keyframes.push([
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
        ]);
    }
    if !packet.is_done() {
        bail!("uianimcurve did not consume full payload");
    }
    Ok(UiAnimCurveEntry { keyframes })
}

#[derive(Clone, Debug, Serialize)]
struct UiAnimEntry {
    mode: u8,
    curve: Option<i32>,
    easing_type: Option<i32>,
    easing_unknown: bool,
    target: u8,
    target_mode: u8,
    values: Vec<Vec<i32>>,
}

fn parse_uianim(data: &[u8]) -> Result<UiAnimEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let mode = packet.g1()?;
    let (curve, easing_type, easing_unknown) = match mode {
        1 => (Some(packet.g4s()?), None, false),
        2 => (None, Some(packet.g4s()?), packet.g1()? == 1),
        value => bail!("unknown uianim mode {value}"),
    };

    let target = packet.g1()?;
    let target_mode = packet.g1()?;
    let count = usize::from(packet.g2()?);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        if target == 0 || target == 3 {
            values.push(vec![packet.g4s()?, packet.g4s()?]);
        } else if target == 6 {
            values.push(vec![packet.g4s()?, packet.g4s()?, packet.g4s()?]);
        } else {
            values.push(vec![packet.g4s()?]);
        }
    }
    if !packet.is_done() {
        bail!("uianim did not consume full payload");
    }

    Ok(UiAnimEntry {
        mode,
        curve,
        easing_type,
        easing_unknown,
        target,
        target_mode,
        values,
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum FontSourceType {
    SpriteBitmap,
    SpriteFontsheet,
    Vector,
}

#[derive(Clone, Debug, Serialize)]
struct FontGlyphInfo {
    width: u8,
    height: u8,
    bearing_y: u8,
}

#[derive(Clone, Debug, Serialize)]
struct FontSheetPosition {
    x: u16,
    y: u16,
}

#[derive(Clone, Debug, Serialize)]
struct FontKerningData {
    left_kern: Vec<Vec<i8>>,
    right_kern: Vec<Vec<i8>>,
}

#[derive(Clone, Debug, Serialize)]
struct FontMetricsEntry {
    source_type: FontSourceType,
    source_pack_id: Option<i32>,
    pixel_size: Option<u8>,
    glyph_info: Vec<FontGlyphInfo>,
    font_sheet_width: Option<u16>,
    font_sheet_height: Option<u16>,
    font_sheet_position: Vec<FontSheetPosition>,
    base_line: Option<u8>,
    upper_case_ascent: Option<u8>,
    byte3049: Option<u8>,
    max_ascent: Option<u8>,
    max_descent: Option<u8>,
    scale: Option<u8>,
    kerning_data: Option<FontKerningData>,
}

fn parse_fontmetrics(data: &[u8]) -> Result<FontMetricsEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let source_type = match packet.g1()? {
        0 => FontSourceType::SpriteBitmap,
        1 => FontSourceType::SpriteFontsheet,
        2 => FontSourceType::Vector,
        value => bail!("invalid font source type id {value}"),
    };

    match source_type {
        FontSourceType::Vector => {
            let entry = FontMetricsEntry {
                source_type,
                source_pack_id: Some(packet.g4s()?),
                pixel_size: Some(packet.g1()?),
                glyph_info: Vec::new(),
                font_sheet_width: None,
                font_sheet_height: None,
                font_sheet_position: Vec::new(),
                base_line: None,
                upper_case_ascent: None,
                byte3049: None,
                max_ascent: None,
                max_descent: None,
                scale: None,
                kerning_data: None,
            };
            if !packet.is_done() {
                bail!("fontmetrics vector did not consume full payload");
            }
            Ok(entry)
        }
        FontSourceType::SpriteBitmap | FontSourceType::SpriteFontsheet => {
            let complex_kerning = packet.g1()? == 1;
            let source_pack_id = match source_type {
                FontSourceType::SpriteFontsheet => Some(packet.g4s()?),
                FontSourceType::SpriteBitmap | FontSourceType::Vector => None,
            };

            let mut glyph_info = vec![
                FontGlyphInfo {
                    width: 0,
                    height: 0,
                    bearing_y: 0,
                };
                256
            ];
            for glyph in &mut glyph_info {
                glyph.width = packet.g1()?;
            }
            for glyph in &mut glyph_info {
                glyph.height = packet.g1()?;
            }
            for glyph in &mut glyph_info {
                glyph.bearing_y = packet.g1()?;
            }

            let font_sheet_width = packet.g2()?;
            let font_sheet_height = packet.g2()?;
            let mut positions = vec![FontSheetPosition { x: 0, y: 0 }; 256];
            for item in &mut positions {
                item.x = packet.g2()?;
            }
            for item in &mut positions {
                item.y = packet.g2()?;
            }

            let kerning_data = if complex_kerning {
                Some(parse_font_kerning(&mut packet)?)
            } else {
                None
            };
            let base_line = if complex_kerning {
                Some(0)
            } else {
                Some(packet.g1()?)
            };

            let entry = FontMetricsEntry {
                source_type,
                source_pack_id,
                pixel_size: None,
                glyph_info,
                font_sheet_width: Some(font_sheet_width),
                font_sheet_height: Some(font_sheet_height),
                font_sheet_position: positions,
                base_line,
                upper_case_ascent: Some(packet.g1()?),
                byte3049: Some(packet.g1()?),
                max_ascent: Some(packet.g1()?),
                max_descent: Some(packet.g1()?),
                scale: Some(packet.g1()?),
                kerning_data,
            };

            if !packet.is_done() {
                bail!("fontmetrics sprite did not consume full payload");
            }
            Ok(entry)
        }
    }
}

fn parse_font_kerning(packet: &mut crate::packet::Packet<'_>) -> Result<FontKerningData> {
    let mut right_kern = Vec::with_capacity(256);
    for _ in 0..256_usize {
        let mut kerns = Vec::with_capacity(256);
        let mut kern = 0_i32;
        for _ in 0..256_usize {
            kern += i32::from(packet.g1s()?);
            kerns.push(kern as i8);
        }
        right_kern.push(kerns);
    }

    let mut left_kern = Vec::with_capacity(256);
    for _ in 0..256_usize {
        let mut kerns = Vec::with_capacity(256);
        let mut kern = 0_i32;
        for _ in 0..256_usize {
            kern += i32::from(packet.g1s()?);
            kerns.push(kern as i8);
        }
        left_kern.push(kerns);
    }

    Ok(FontKerningData {
        left_kern,
        right_kern,
    })
}

fn read_f32_be(packet: &mut crate::packet::Packet<'_>) -> Result<f32> {
    Ok(f32::from_bits(packet.g4s()? as u32))
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn run_audio(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: Option<&Path>,
    max_files: Option<usize>,
) -> Result<()> {
    let mut available = Vec::new();
    for archive in AUDIO_ARCHIVES {
        if ensure_archive_complete(cache.root(), tar_path, archive).is_ok() {
            available.push(archive);
        }
    }
    let cache = FlatCache::open(cache.root())?;
    let mut archive_counts = BTreeMap::new();
    let mut kind_counts = BTreeMap::new();
    let mut extracted_embedded_ogg = 0_usize;
    let mut manifest = Vec::new();

    let mut processed = 0_usize;
    let process_limit = max_files.unwrap_or(usize::MAX);
    let mut limit_hit = false;

    for archive in available {
        let index = cache.archive_index(archive)?;
        let mut file_count = 0_usize;
        for group in &index.group_id {
            let files = cache.group_files_with_index(&index, archive, *group)?;
            for (file, data) in &files {
                if processed >= process_limit {
                    limit_hit = true;
                    break;
                }
                let inspection = inspect_audio_file(data);
                *kind_counts
                    .entry(inspection.kind.as_str().to_string())
                    .or_insert(0) += 1;
                let mut extracted_ogg = false;

                if let Some(out) = out_dir {
                    let raw_path =
                        out.join(format!("{archive}_{group}_{file}.{}", inspection.extension));
                    write_binary(&raw_path, data)?;

                    if inspection.kind == AudioKind::Jaga
                        && let Some(ogg) = inspection.embedded_ogg_slice(data)
                    {
                        let ogg_path = out.join(format!("{archive}_{group}_{file}.ogg"));
                        write_binary(&ogg_path, ogg)?;
                        extracted_ogg = true;
                        extracted_embedded_ogg += 1;
                    }
                }

                manifest.push(AudioManifestEntry {
                    archive,
                    group: *group,
                    file: *file,
                    size: data.len(),
                    kind: inspection.kind.as_str().to_string(),
                    raw_extension: inspection.extension.to_string(),
                    embedded_ogg_offset: inspection.embedded_ogg_offset,
                    extracted_ogg,
                });
                file_count += 1;
                processed += 1;
            }
            if limit_hit {
                break;
            }
        }
        archive_counts.insert(archive, file_count);
        if limit_hit {
            break;
        }
    }

    let manifest_path = if let Some(out) = out_dir {
        let manifest_path = out.join("audio_manifest.json");
        write_json(&manifest_path, &manifest)?;
        Some(manifest_path.display().to_string())
    } else {
        None
    };

    print_json(&AudioSummary {
        archives: archive_counts,
        kinds: kind_counts,
        extracted_embedded_ogg,
        manifest_path,
    })
}

fn format_script_source(group: u32, file: u32, script: &CompiledScript) -> String {
    let mut out = String::new();
    let script_name = script.name.as_deref().unwrap_or("null");
    let _ = writeln!(out, "// group={group} file={file}");
    let _ = writeln!(out, "// name={script_name}");
    let _ = writeln!(
        out,
        "// locals int={} object={} long={}",
        script.local_count_int, script.local_count_object, script.local_count_long
    );
    let _ = writeln!(
        out,
        "// args int={} object={} long={}",
        script.argument_count_int, script.argument_count_object, script.argument_count_long
    );
    for (index, instruction) in script.code.iter().enumerate() {
        let _ = writeln!(out, "{index:05}: {}", format_instruction(instruction));
    }
    out
}

fn format_instruction(instruction: &Instruction) -> String {
    format!(
        "{} {}",
        instruction.command,
        format_operand(&instruction.operand)
    )
    .trim_end()
    .to_string()
}

fn format_operand(operand: &Operand) -> String {
    match operand {
        Operand::Int(value) => value.to_string(),
        Operand::Long(value) => value.to_string(),
        Operand::Str(value) => format!("\"{}\"", escape_string(value)),
        Operand::Local(value) => format!("local_{value}"),
        Operand::VarRef(value) => {
            let mut tag = format!("{}:{}", value.domain.as_label(), value.id);
            if value.transmog {
                tag.push_str(":transmog");
            }
            tag
        }
        Operand::VarBitRef(value) => {
            let mut tag = format!("varbit:{}", value.id);
            if value.transmog {
                tag.push_str(":transmog");
            }
            tag
        }
        Operand::Branch(value) => format!("->{value}"),
        Operand::Switch(cases) => {
            let mut text = String::new();
            text.push('{');
            for (index, case) in cases.iter().enumerate() {
                if index != 0 {
                    text.push_str(", ");
                }
                let _ = write!(text, "{}->{}", case.value, case.target);
            }
            text.push('}');
            text
        }
        Operand::Script(value) => format!("script_{value}"),
        Operand::Array(value) => format!("array_{value}"),
        Operand::Count(value) => format!("count_{value}"),
        Operand::Byte(value) => value.to_string(),
    }
}

fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('"', "\\\"")
}

fn sanitize_file_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "script".to_string()
    } else {
        out
    }
}

fn load_script_group_names(
    index: &crate::js5::ArchiveIndex,
    data_dir: &Path,
) -> Result<HashMap<u32, String>> {
    let Some(group_hashes) = &index.group_name_hash else {
        return Ok(HashMap::new());
    };

    let names_path = data_dir.join("names/scripts.txt");
    if !names_path.is_file() {
        return Ok(HashMap::new());
    }

    let hash_names = load_hash_name_map(&names_path)?;
    let mut by_group = HashMap::new();
    for group in &index.group_id {
        let idx = usize::try_from(*group).context("script group index overflow")?;
        let hash = *group_hashes
            .get(idx)
            .with_context(|| format!("missing group hash slot for {group}"))?;
        if hash == -1 {
            continue;
        }
        if let Some(name) = hash_names.get(&hash) {
            by_group.insert(*group, extract_name_suffix(name));
        }
    }
    Ok(by_group)
}

fn load_hash_name_map(path: &Path) -> Result<HashMap<i32, String>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed reading {}", path.display()))?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        expand_name_pattern(name, &mut map);
    }
    Ok(map)
}

fn expand_name_pattern(name: &str, out: &mut HashMap<i32, String>) {
    if let Some(index) = name.find('#') {
        let prefix = &name[..index];
        let suffix = &name[index + 1..];
        for value in 0..500 {
            let expanded = format!("{prefix}{value}{suffix}");
            expand_name_pattern(&expanded, out);
        }
    } else {
        out.insert(java_string_hash(name), name.to_string());
    }
}

fn java_string_hash(value: &str) -> i32 {
    let mut hash = 0_i32;
    for c in value.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(c as i32);
    }
    hash
}

fn extract_name_suffix(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if let Some((_, suffix)) = inner.split_once(',') {
            return suffix.to_string();
        }
    }
    trimmed.to_string()
}

fn write_binary(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    fs::write(path, data).with_context(|| format!("failed writing {}", path.display()))
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("failed writing {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, data: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(data).context("failed to encode json")?;
    fs::write(path, json).with_context(|| format!("failed writing {}", path.display()))
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("failed to encode summary json")?
    );
    Ok(())
}

fn run_dep_tree_interface(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let root = EntityRef::new(EntityType::Interface, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

fn run_dep_tree_script(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let root = EntityRef::new(EntityType::Script, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

// Resolves dependency tree for a varp entry across 9 parameter sources.
#[allow(clippy::too_many_arguments)]
fn run_dep_tree_varp(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    domain: VarDomainArg,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let entity_type = match domain {
        VarDomainArg::Player => EntityType::VarPlayer,
        VarDomainArg::Npc => EntityType::VarNpc,
        VarDomainArg::Client => EntityType::VarClient,
        VarDomainArg::World => EntityType::VarWorld,
        VarDomainArg::Region => EntityType::VarRegion,
        VarDomainArg::Object => EntityType::VarObject,
        VarDomainArg::Clan => EntityType::VarClan,
        VarDomainArg::ClanSetting => EntityType::VarClanSetting,
        VarDomainArg::Controller => EntityType::VarController,
        VarDomainArg::Global => EntityType::VarGlobal,
        VarDomainArg::PlayerGroup => EntityType::VarPlayerGroup,
        VarDomainArg::All => bail!("dep-tree-varp requires a specific domain, not 'all'"),
    };
    let root = EntityRef::new(entity_type, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

fn run_dep_tree_varbit(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let root = EntityRef::new(EntityType::VarBit, id);
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

// Resolves dependency tree for a config entry across 8 parameter sources.
#[allow(clippy::too_many_arguments)]
fn run_dep_tree_config(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    kind: ConfigKindArg,
    id: u32,
    max_depth: u32,
    out_file: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    let entity_type = kind.entity_type();
    let root = EntityRef::new(entity_type, id).labeled(kind.label());
    let tree = build_tree(&ctx, &root, max_depth);
    write_json(out_file, &tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

// Loads both source and target caches for migration impact analysis.
#[allow(clippy::too_many_arguments)]
fn run_migrate_check(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    interface_group: u32,
    out_file: &Path,
    target_version: RuntimeVersion,
    source_cache_tar: Option<&Path>,
    source_build: u32,
    source_subbuild: u32,
    enable_remap: bool,
    remap_buffer: u32,
) -> Result<()> {
    let target_ctx = ResolverContext::load(
        cache,
        tar_path,
        data_dir,
        target_version.build,
        target_version.subbuild,
    )?;

    let source_tar = source_cache_tar.unwrap_or(tar_path);
    let source_ctx =
        ResolverContext::load(cache, source_tar, data_dir, source_build, source_subbuild)?;

    let analyzer = crate::migrate::MigrationAnalyzer::new(source_ctx, target_ctx);
    let report = if enable_remap {
        analyzer.remap_interface(interface_group, remap_buffer)
    } else {
        analyzer.analyze_interface(interface_group)
    };

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(out_file, &json)?;

    eprintln!(
        "migration report: {} entities ({} safe, {} missing, {} id_conflict, {} changed, {} script_changed) written to {}",
        report.total_entities,
        report.summary.safe,
        report.summary.missing,
        report.summary.id_conflict,
        report.summary.changed,
        report.summary.script_changed,
        out_file.display()
    );
    Ok(())
}

// Loads both source and target caches for single-script migration analysis.
#[allow(clippy::too_many_arguments)]
fn run_migrate_script(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    script_id: u32,
    out_file: &Path,
    target_version: RuntimeVersion,
    source_cache_tar: Option<&Path>,
    source_build: u32,
    source_subbuild: u32,
    enable_remap: bool,
    remap_buffer: u32,
) -> Result<()> {
    let target_ctx = ResolverContext::load(
        cache,
        tar_path,
        data_dir,
        target_version.build,
        target_version.subbuild,
    )?;

    let source_tar = source_cache_tar.unwrap_or(tar_path);
    let source_ctx =
        ResolverContext::load(cache, source_tar, data_dir, source_build, source_subbuild)?;

    let analyzer = crate::migrate::MigrationAnalyzer::new(source_ctx, target_ctx);
    let report = if enable_remap {
        analyzer.remap_script(script_id, remap_buffer)
    } else {
        analyzer.analyze_script(script_id)
    };

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(out_file, &json)?;

    eprintln!(
        "script migration report: {} entities ({} safe, {} missing, {} id_conflict, {} changed, {} script_changed) written to {}",
        report.total_entities,
        report.summary.safe,
        report.summary.missing,
        report.summary.id_conflict,
        report.summary.changed,
        report.summary.script_changed,
        out_file.display()
    );
    Ok(())
}

fn run_ts_export(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;
    fs::create_dir_all(out_dir)?;

    export_var_types(&ctx, out_dir)?;
    export_varbit_types(&ctx, out_dir)?;
    export_enum_types(&ctx, out_dir)?;
    export_struct_types(&ctx, out_dir)?;
    export_param_types(&ctx, out_dir)?;
    export_interface_ids(&ctx, out_dir)?;
    export_inv_types(&ctx, out_dir)?;
    export_obj_types(&ctx, out_dir)?;
    export_npc_types(&ctx, out_dir)?;
    export_loc_types(&ctx, out_dir)?;
    export_seq_types(&ctx, out_dir)?;
    export_spot_types(&ctx, out_dir)?;
    export_db_types(&ctx, out_dir)?;
    export_index(out_dir)?;

    eprintln!("typescript definitions exported to {}", out_dir.display());
    Ok(())
}

fn run_transpile_scripts(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    filter_script: Option<&str>,
    max_scripts: usize,
    version: RuntimeVersion,
) -> Result<()> {
    let ctx = ResolverContext::load(cache, tar_path, data_dir, version.build, version.subbuild)?;

    let opcode_book = OpcodeBook::load(data_dir, version.build, version.subbuild)?;

    let transpiler = Transpiler::new()
        .with_enums(&ctx.enums)
        .with_enums_map(&ctx.enums)
        .with_vars(&ctx.varps_by_domain)
        .with_varbits(&ctx.varbits)
        .with_params(&ctx.params)
        .with_script_names(&ctx.scripts, &opcode_book, version.build)
        .with_script_signatures(&ctx.scripts, &opcode_book, version.build)
        .with_components(&ctx.parsed_components);

    fs::create_dir_all(out_dir)?;

    // Generate type definitions so script imports resolve.
    // Skip if index.ts already exists (user may have run ts-export).
    if !out_dir.join("index.ts").exists() {
        export_var_types(&ctx, out_dir)?;
        export_varbit_types(&ctx, out_dir)?;
        export_enum_types(&ctx, out_dir)?;
        export_param_types(&ctx, out_dir)?;
        export_interface_ids(&ctx, out_dir)?;
        export_inv_types(&ctx, out_dir)?;
        export_obj_types(&ctx, out_dir)?;
        export_npc_types(&ctx, out_dir)?;
        export_loc_types(&ctx, out_dir)?;
        export_seq_types(&ctx, out_dir)?;
        export_spot_types(&ctx, out_dir)?;
        export_db_types(&ctx, out_dir)?;
        export_index(out_dir)?;
    }

    let mut script_count = 0;
    let mut errors = 0;
    let mut barrel_exports: Vec<String> = Vec::new();

    for (&script_id_raw, data) in &ctx.scripts {
        let script_id = crate::transpile::ScriptId(script_id_raw as i32);

        if let Some(filter) = filter_script {
            let name = transpiler.script_name_for(script_id);
            if name.map(|n| !n.contains(filter)).unwrap_or(true) {
                continue;
            }
        }

        match transpiler.transpile_from_bytes(data, &opcode_book, version.build, script_id) {
            Ok(ts) => {
                let script_name = transpiler
                    .script_name_for(script_id)
                    .unwrap_or_else(|| format!("script_{script_id}"));
                let filename = format!("{}.ts", sanitize_file_component(&script_name));
                let out_path = out_dir.join(&filename);
                fs::write(&out_path, &ts.source)?;
                barrel_exports.push(format!(
                    "export {{ script_{id} }} from './{filename_no_ext}';",
                    id = script_id,
                    filename_no_ext = filename.trim_end_matches(".ts")
                ));
                script_count += 1;
                if script_count >= max_scripts {
                    break;
                }
            }
            Err(e) => {
                eprintln!("failed to transpile script_{script_id}: {e}");
                errors += 1;
            }
        }
    }

    // Write scripts barrel file so you can import { script_N } from './scripts'
    if !barrel_exports.is_empty() {
        barrel_exports.sort();
        let mut lines = vec![
            "// Auto-generated scripts barrel".to_string(),
            "// Source: RS3 cache transpile-scripts".to_string(),
            String::new(),
        ];
        lines.extend(barrel_exports);
        write_text(&out_dir.join("scripts.ts"), &lines.join("\n"))?;
    }

    eprintln!(
        "transpiled {script_count} scripts ({errors} errors) to {}",
        out_dir.display()
    );
    Ok(())
}

fn export_var_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx
        .varps_by_domain
        .iter()
        .flat_map(|(domain, vars)| {
            vars.values().map(|entry| VarTypeEntry {
                id: entry.id,
                domain: *domain,
                var_name: entry.var_name.clone(),
                type_id: entry.type_id,
                lifetime: entry.lifetime,
                transmit_level: entry.transmit_level,
                client_code: entry.client_code,
                domain_default: entry.domain_default,
                wiki_sync: entry.wiki_sync,
            })
        })
        .collect();

    entries.sort_by_key(|e| (e.domain as u8, e.id));

    let mut lines = vec![
        "// Auto-generated Var definitions".to_string(),
        "// Source: RS3 cache var config".to_string(),
        String::new(),
        "export type VarDomain = 'player' | 'npc' | 'client' | 'world' | 'region' | 'object' | 'clan' | 'clan_setting' | 'controller' | 'player_group' | 'global';".to_string(),
        "export type VarType = 'int' | 'long' | 'string' | 'unknown';".to_string(),
        "export type VarLifetime = 'temp' | 'perm' | 'serverperm' | 'unknown';".to_string(),
        "export type VarTransmitLevel = 'never' | 'on_set_different' | 'on_set_always' | 'unknown';".to_string(),
        String::new(),
        "export interface VarEntry {".to_string(),
        "    id: number;".to_string(),
        "    domain: VarDomain;".to_string(),
        "    name: string;".to_string(),
        "    type: VarType;".to_string(),
        "    lifetime: VarLifetime;".to_string(),
        "    transmitLevel: VarTransmitLevel;".to_string(),
        "    clientCode: number | null;".to_string(),
        "    domainDefault: boolean;".to_string(),
        "    wikiSync: boolean;".to_string(),
        "}".to_string(),
        String::new(),
        // Use composite key: domain_id * 1000000 + var_id
        "export const VARS: ReadonlyMap<number, VarEntry> = new Map([".to_string(),
    ];
    for entry in &entries {
        let type_str = match entry.type_id {
            Some(0) => "'int'",
            Some(1) => "'long'",
            Some(2) => "'string'",
            _ => "'unknown'",
        };
        let lifetime = entry.lifetime.unwrap_or("unknown");
        let transmit = entry.transmit_level.unwrap_or("unknown");
        let client_code = match entry.client_code {
            Some(c) => c.to_string(),
            None => "null".to_string(),
        };
        let domain_label = entry.domain.as_label();
        let composite_key = (u64::from(entry.domain) * 1_000_000) + u64::from(entry.id);
        lines.push(format!(
            "    [{}, {{ id: {}, domain: '{}', name: '{}', type: {}, lifetime: '{}', transmitLevel: '{}', clientCode: {}, domainDefault: {}, wikiSync: {} }}],",
            composite_key,
            entry.id,
            domain_label,
            escape_ts_string(&entry.var_name),
            type_str,
            lifetime,
            transmit,
            client_code,
            entry.domain_default,
            entry.wiki_sync
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const VAR_COUNT = {};", entries.len()));

    write_text(&out_dir.join("vars.ts"), &lines.join("\n"))
}

struct VarTypeEntry {
    id: u32,
    domain: crate::vars::VarDomain,
    var_name: String,
    type_id: Option<u8>,
    lifetime: Option<&'static str>,
    transmit_level: Option<&'static str>,
    client_code: Option<u16>,
    domain_default: bool,
    wiki_sync: bool,
}

fn export_varbit_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated VarBit definitions".to_string(),
        "// Source: RS3 cache varbit config".to_string(),
        String::new(),
        "export interface VarBitEntry {".to_string(),
        "    id: number;".to_string(),
        "    name: string;".to_string(),
        "    domain: string | null;".to_string(),
        "    baseVar: number | null;".to_string(),
        "    startBit: number | null;".to_string(),
        "    endBit: number | null;".to_string(),
        "    wikiSync: boolean;".to_string(),
        "}".to_string(),
        String::new(),
    ];

    let mut entries: Vec<_> = ctx.varbits.values().cloned().collect();
    entries.sort_by_key(|e| e.id);

    lines.push("export const VARBITS: ReadonlyMap<number, VarBitEntry> = new Map([".to_string());
    for entry in &entries {
        let domain_str = match entry.domain {
            Some(d) => format!("'{}'", d.as_label()),
            None => "null".to_string(),
        };
        let base_var = entry
            .base_var
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let start_bit = entry
            .start_bit
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        let end_bit = entry
            .end_bit
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string());
        lines.push(format!(
            "    [{}, {{ id: {}, name: '{}', domain: {}, baseVar: {}, startBit: {}, endBit: {}, wikiSync: {} }}],",
            entry.id,
            entry.id,
            escape_ts_string(&entry.varbit_name),
            domain_str,
            base_var,
            start_bit,
            end_bit,
            entry.wiki_sync
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const VARBIT_COUNT = {};", entries.len()));

    write_text(&out_dir.join("varbits.ts"), &lines.join("\n"))
}

fn export_enum_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.enums.values().cloned().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Enum definitions".to_string(),
        "// Source: RS3 cache enum config".to_string(),
        String::new(),
    ];

    // ── Per-enum const objects with named constants ──
    let mut reverse_lookup: Vec<(i32, String)> = Vec::new();

    for entry in &entries {
        if entry.values.is_empty() {
            continue;
        }
        let obj_name = format!("Enum_{id}", id = entry.id);
        let mut props: Vec<String> = Vec::new();

        for pair in &entry.values {
            let prop = match &pair.value {
                crate::config::ScalarValue::Str(s) => {
                    let name = str_to_screaming_snake(s);
                    if name.is_empty() {
                        format!("KEY_{key}", key = pair.key)
                    } else {
                        name
                    }
                }
                _ => format!("KEY_{key}", key = pair.key),
            };
            // Deduplicate property names
            let unique_prop = if props.iter().any(|p| p.starts_with(&format!("{prop} ="))) {
                format!("{prop}_{key}", key = pair.key)
            } else {
                prop.clone()
            };
            props.push(format!("    {unique_prop}: {key},", key = pair.key));
            reverse_lookup.push((pair.key, format!("{obj_name}.{unique_prop}")));
        }

        lines.push(format!("export const {obj_name} = {{"));
        lines.extend(props);
        lines.push("} as const;".to_string());
        lines.push(String::new());
    }

    // ── Reverse lookup: enum value → qualified name ──
    reverse_lookup.sort_by_key(|(k, _)| *k);
    reverse_lookup.dedup_by_key(|(k, _)| *k);
    if !reverse_lookup.is_empty() {
        lines.push("// Reverse lookup: maps enum key values to qualified names.".to_string());
        lines.push(
            "export const ENUM_VALUE_TO_NAME: ReadonlyMap<number, string> = new Map([".to_string(),
        );
        for (key, name) in &reverse_lookup {
            lines.push(format!("    [{key}, '{name}'],"));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }

    // ── Existing types and runtime map ──
    lines.push("export interface EnumPair {".to_string());
    lines.push("    key: number;".to_string());
    lines.push("    value: number | string;".to_string());
    lines.push("    dense: boolean;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("export interface EnumEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    inputType: string;".to_string());
    lines.push("    outputType: string;".to_string());
    lines.push("    default: number | string | null;".to_string());
    lines.push("    values: EnumPair[];".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    lines.push("export const ENUMS: ReadonlyMap<number, EnumEntry> = new Map([".to_string());
    for entry in &entries {
        let input_type = match entry.input_type_char {
            Some(b'i') => "'int'",
            Some(b's') => "'string'",
            _ => "'unknown'",
        };
        let output_type = match entry.output_type_char {
            Some(b'i') => "'int'",
            Some(b's') => "'string'",
            _ => "'unknown'",
        };
        let default = match &entry.default {
            Some(crate::config::ScalarValue::Int(i)) => i.to_string(),
            Some(crate::config::ScalarValue::Long(l)) => l.to_string(),
            Some(crate::config::ScalarValue::Str(s)) => format!("'{}'", escape_ts_string(s)),
            None => "null".to_string(),
        };
        let values_json: String = entry
            .values
            .iter()
            .map(|pair| {
                let val_str = match &pair.value {
                    crate::config::ScalarValue::Int(i) => i.to_string(),
                    crate::config::ScalarValue::Long(l) => l.to_string(),
                    crate::config::ScalarValue::Str(s) => format!("'{}'", escape_ts_string(s)),
                };
                format!(
                    "{{ key: {}, value: {}, dense: {} }}",
                    pair.key, val_str, pair.dense
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "    [{}, {{ id: {}, inputType: {}, outputType: {}, default: {}, values: [{}] }}],",
            entry.id, entry.id, input_type, output_type, default, values_json
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const ENUM_COUNT = {};", entries.len()));

    write_text(&out_dir.join("enums.ts"), &lines.join("\n"))
}

/// Convert a lowercase or mixed-case string value (e.g. "`skill_type`",
/// "my value") to `SCREAMING_SNAKE_CASE` for use as a
/// TypeScript const property name.
fn str_to_screaming_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if c == ' ' || c == '-' || c == '/' || c == '.' {
            out.push('_');
        }
    }
    // Trim leading/trailing underscores
    let trimmed = out.trim_matches('_');
    // Can't start with a digit
    if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{trimmed}")
    } else if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

fn export_struct_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Struct definitions".to_string(),
        "// Source: RS3 cache struct config".to_string(),
        String::new(),
        "export interface StructParamEntry {".to_string(),
        "    id: number;".to_string(),
        "    value: number | string;".to_string(),
        "}".to_string(),
        String::new(),
        "export interface StructEntry {".to_string(),
        "    id: number;".to_string(),
        "    params: StructParamEntry[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    let mut entries: Vec<_> = ctx.structs.values().cloned().collect();
    entries.sort_by_key(|e| e.id);

    lines.push("export const STRUCTS: ReadonlyMap<number, StructEntry> = new Map([".to_string());
    for entry in &entries {
        let params_json = entry
            .params
            .iter()
            .map(|p| {
                format!(
                    "{{ id: {}, value: {} }}",
                    p.param_id,
                    format_scalar_value(&p.value)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "    [{}, {{ id: {}, params: [{}] }}],",
            entry.id, entry.id, params_json
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const STRUCT_COUNT = {};", entries.len()));

    write_text(&out_dir.join("structs.ts"), &lines.join("\n"))
}

fn format_scalar_value(value: &crate::config::ScalarValue) -> String {
    match value {
        crate::config::ScalarValue::Int(i) => i.to_string(),
        crate::config::ScalarValue::Long(l) => l.to_string(),
        crate::config::ScalarValue::Str(s) => format!("'{}'", escape_ts_string(s)),
    }
}

fn export_param_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.params.values().cloned().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Param definitions".to_string(),
        "// Source: RS3 cache param config".to_string(),
        String::new(),
        "export interface ParamEntry {".to_string(),
        "    id: number;".to_string(),
        "    typeChar: string | null;".to_string(),
        "    typeId: number | null;".to_string(),
        "    defaultInt: number | null;".to_string(),
        "    defaultString: string | null;".to_string(),
        "    autoDisable: boolean;".to_string(),
        "}".to_string(),
        String::new(),
        "export type ParamValue = number | string;".to_string(),
        String::new(),
    ];

    lines.push("export const PARAMS: ReadonlyMap<number, ParamEntry> = new Map([".to_string());
    for entry in &entries {
        let type_char = entry
            .type_char
            .map(|c| format!("'{}'", c as char))
            .unwrap_or_else(|| "null".to_string());
        let type_id = entry
            .type_id
            .map(|t| t.to_string())
            .unwrap_or_else(|| "null".to_string());
        let (default_int, default_string) = match &entry.default {
            Some(crate::config::ScalarValue::Int(i)) => (i.to_string(), "null".to_string()),
            Some(crate::config::ScalarValue::Long(l)) => (l.to_string(), "null".to_string()),
            Some(crate::config::ScalarValue::Str(s)) => {
                ("null".to_string(), format!("'{}'", escape_ts_string(s)))
            }
            None => ("null".to_string(), "null".to_string()),
        };
        lines.push(format!(
            "    [{}, {{ id: {}, typeChar: {}, typeId: {}, defaultInt: {}, defaultString: {}, autoDisable: {} }}],",
            entry.id, entry.id, type_char, type_id, default_int, default_string, entry.autodisable
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!("export const PARAM_COUNT = {};", entries.len()));

    write_text(&out_dir.join("params.ts"), &lines.join("\n"))
}

fn export_inv_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.invs.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Inventory definitions".to_string(),
        "// Source: RS3 cache inv config".to_string(),
        String::new(),
    ];

    lines.push("export interface InvStockEntry {".to_string());
    lines.push("    objId: number;".to_string());
    lines.push("    count: number;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("export interface InvEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    size: number | null;".to_string());
    lines.push("    stocks: InvStockEntry[];".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    if !entries.is_empty() {
        lines.push("export const INVS: ReadonlyMap<number, InvEntry> = new Map([".to_string());
        for entry in &entries {
            let size = entry
                .size
                .map(|s| s.to_string())
                .unwrap_or_else(|| "null".to_string());
            let stocks_json: String = entry
                .stocks
                .iter()
                .map(|s| format!("{{ objId: {}, count: {} }}", s.obj_id, s.count))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, size: {size}, stocks: [{stocks_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const INV_COUNT = {};", entries.len()));

    write_text(&out_dir.join("invs.ts"), &lines.join("\n"))
}

fn export_obj_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.objs.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Item (Obj) definitions".to_string(),
        "// Source: RS3 cache obj config".to_string(),
        String::new(),
        "export interface ObjEntry {".to_string(),
        "    id: number;".to_string(),
        "    name: string | null;".to_string(),
        "    ops: string[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    if !entries.is_empty() {
        lines.push("export const OBJS: ReadonlyMap<number, ObjEntry> = new Map([".to_string());
        for entry in &entries {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const OBJ_COUNT = {};", entries.len()));

    write_text(&out_dir.join("objs.ts"), &lines.join("\n"))
}

fn export_npc_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.npcs.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated NPC definitions".to_string(),
        "// Source: RS3 cache npc config".to_string(),
        String::new(),
        "export interface NpcEntry {".to_string(),
        "    id: number;".to_string(),
        "    name: string | null;".to_string(),
        "    ops: string[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    if !entries.is_empty() {
        lines.push("export const NPCS: ReadonlyMap<number, NpcEntry> = new Map([".to_string());
        for entry in &entries {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const NPC_COUNT = {};", entries.len()));

    write_text(&out_dir.join("npcs.ts"), &lines.join("\n"))
}

fn export_loc_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.locs.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Loc (Object) definitions".to_string(),
        "// Source: RS3 cache loc config".to_string(),
        String::new(),
        "export interface LocEntry {".to_string(),
        "    id: number;".to_string(),
        "    name: string | null;".to_string(),
        "    ops: string[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    if !entries.is_empty() {
        lines.push("export const LOCS: ReadonlyMap<number, LocEntry> = new Map([".to_string());
        for entry in &entries {
            let name = extract_oplist_name(&entry.ops);
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(o)))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, name: {name}, ops: [{ops_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const LOC_COUNT = {};", entries.len()));

    write_text(&out_dir.join("locs.ts"), &lines.join("\n"))
}

fn export_seq_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.seqs.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Sequence (Animation) definitions".to_string(),
        "// Source: RS3 cache seq config".to_string(),
        String::new(),
        "export interface SeqFrame {".to_string(),
        "    animId: number;".to_string(),
        "    frameId: number;".to_string(),
        "    delay: number;".to_string(),
        "}".to_string(),
        String::new(),
        "export interface SeqEntry {".to_string(),
        "    id: number;".to_string(),
        "    frames: SeqFrame[];".to_string(),
        "    stretches: boolean;".to_string(),
        "    priority: number | null;".to_string(),
        "    leftHand: number | null;".to_string(),
        "    rightHand: number | null;".to_string(),
        "    loopCount: number | null;".to_string(),
        "    params: StructParamEntry[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    if !entries.is_empty() {
        lines.push("export const SEQS: ReadonlyMap<number, SeqEntry> = new Map([".to_string());
        for entry in &entries {
            let frames_json: String = entry
                .frames
                .iter()
                .map(|f| {
                    format!(
                        "{{ animId: {}, frameId: {}, delay: {} }}",
                        f.anim_id, f.frame_id, f.delay
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let params_json: String = entry
                .params
                .iter()
                .map(|p| {
                    format!(
                        "{{ id: {}, value: {} }}",
                        p.param_id,
                        format_scalar_value(&p.value)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, frames: [{frames_json}], stretches: {stretches}, priority: {priority}, leftHand: {lefthand}, rightHand: {righthand}, loopCount: {loopcount}, params: [{params_json}] }}],",
                id = entry.id,
                stretches = entry.stretches,
                priority = entry.priority.map(|p| p.to_string()).unwrap_or_else(|| "null".to_string()),
                lefthand = entry.lefthand_raw.map(|l| l.to_string()).unwrap_or_else(|| "null".to_string()),
                righthand = entry.righthand_raw.map(|r| r.to_string()).unwrap_or_else(|| "null".to_string()),
                loopcount = entry.loopcount.map(|l| l.to_string()).unwrap_or_else(|| "null".to_string()),
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const SEQ_COUNT = {};", entries.len()));

    write_text(&out_dir.join("seqs.ts"), &lines.join("\n"))
}

fn export_spot_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = ctx.spots.values().collect();
    entries.sort_by_key(|e| e.id);

    let mut lines = vec![
        "// Auto-generated Spotanim (Graphic) definitions".to_string(),
        "// Source: RS3 cache spot config".to_string(),
        String::new(),
        "export interface SpotEntry {".to_string(),
        "    id: number;".to_string(),
        "    ops: string[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    if !entries.is_empty() {
        lines.push("export const SPOTS: ReadonlyMap<number, SpotEntry> = new Map([".to_string());
        for entry in &entries {
            let ops_json: String = entry
                .ops
                .iter()
                .map(|o| format!("'{}'", escape_ts_string(&format!("{o:?}"))))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, ops: [{ops_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const SPOT_COUNT = {};", entries.len()));

    write_text(&out_dir.join("spots.ts"), &lines.join("\n"))
}

/// Extract a name from op list entries like "name=Attack" or "name=Man".
fn extract_oplist_name(ops: &[String]) -> String {
    for op in ops {
        if let Some(name) = op.strip_prefix("name=") {
            return format!("'{}'", escape_ts_string(name));
        }
    }
    "null".to_string()
}

fn export_db_types(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    let mut lines = vec![
        "// Auto-generated Database definitions".to_string(),
        "// Source: RS3 cache DB tables and rows (archive 2, groups 40/41)".to_string(),
        String::new(),
        "export interface DbTableColumn {".to_string(),
        "    column: number;".to_string(),
        "    tupleTypes: number[];".to_string(),
        "    defaults: (number | string)[][];".to_string(),
        "}".to_string(),
        String::new(),
        "export interface DbTableEntry {".to_string(),
        "    id: number;".to_string(),
        "    columns: DbTableColumn[];".to_string(),
        "}".to_string(),
        String::new(),
        "export interface DbRowColumn {".to_string(),
        "    column: number;".to_string(),
        "    tupleTypes: number[];".to_string(),
        "    rows: (number | string)[][];".to_string(),
        "}".to_string(),
        String::new(),
        "export interface DbRowEntry {".to_string(),
        "    id: number;".to_string(),
        "    table: number | null;".to_string(),
        "    columns: DbRowColumn[];".to_string(),
        "}".to_string(),
        String::new(),
    ];

    // DB Tables (schemas)
    if !ctx.dbtables.is_empty() {
        let mut entries: Vec<_> = ctx.dbtables.values().collect();
        entries.sort_by_key(|e| e.id);
        lines.push(
            "export const DB_TABLES: ReadonlyMap<number, DbTableEntry> = new Map([".to_string(),
        );
        for entry in &entries {
            let cols_json: String = entry
                .columns
                .iter()
                .map(|c| {
                    let types = c
                        .tuple_types
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let defaults: String = c
                        .defaults
                        .iter()
                        .map(|row| {
                            let vals = row
                                .iter()
                                .map(|v| match v {
                                    crate::config::ScalarValue::Int(i) => i.to_string(),
                                    crate::config::ScalarValue::Long(l) => l.to_string(),
                                    crate::config::ScalarValue::Str(s) => {
                                        format!("'{}'", escape_ts_string(s))
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("[{vals}]")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "{{ column: {}, tupleTypes: [{}], defaults: [{}] }}",
                        c.column, types, defaults
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "    [{id}, {{ id: {id}, columns: [{cols_json}] }}],",
                id = entry.id
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!(
        "export const DB_TABLE_COUNT = {};",
        ctx.dbtables.len()
    ));

    // ── Reverse-engineered table schemas ──
    lines.push(String::new());
    lines.push("// Reverse-engineered table column meanings:".to_string());
    lines.push("// Table 163 (5,237 rows, 32 cols) — Items".to_string());
    lines.push("//   col  0: itemId (int)".to_string());
    lines.push("//   col  1: parentId (int) — parent item or category".to_string());
    lines.push("//   col  2: name (string)".to_string());
    lines.push("//   col  3: description (string)".to_string());
    lines.push("//   col  4: paramId (int) — linked param config entry".to_string());
    lines.push("//   col  5: typeId (int) — item type/category".to_string());
    lines.push("//   col  6: value (int, default 99) — shop price".to_string());
    lines.push("//   col  7: flags (int, default 268435454)".to_string());
    lines.push("//   col  8: stackable (int, default 1)".to_string());
    lines.push("//   col 11: membersOnly (boolean, default false)".to_string());
    lines.push("//   col 13: categoryId (int)".to_string());
    lines.push("//   col 23: modelId (int)".to_string());
    lines.push("//   col 24: modelId2 (int)".to_string());
    lines.push("//   col 26: color (int) — RGBA tint".to_string());
    lines.push("//   col 30: equipmentOverrides (int[6]) — only for special items".to_string());
    lines.push("//         index 0-5: stab/slash/crush/magic/range/strength bonus".to_string());
    lines.push("//   col 31: soundId (int)".to_string());
    lines.push("//".to_string());
    lines.push("// Table 29 (105 rows, 46 cols) — NPC stats".to_string());
    lines.push("//   cols 1-3: model IDs".to_string());
    lines.push("//   col  5: name (string)".to_string());
    lines.push("//   col  7: size (int)".to_string());
    lines.push("//   col  9: combatLevel (int)".to_string());
    lines.push("//   col 10: hitpoints (int)".to_string());
    lines.push("//   col 14: attack (int)".to_string());
    lines.push("//   col 17: defence (int)".to_string());
    lines.push("//   col 18: accuracy (int)".to_string());
    lines.push("//".to_string());
    lines.push("// Note: Most equipment/weapon stats are computed client-side".to_string());
    lines.push("// from item tier + category, not stored per-item in this table.".to_string());
    lines.push("// Only override stats (halos, special items) use col 30.".to_string());
    lines.push(String::new());

    // DB Rows (data) — grouped by table ID for navigability
    if !ctx.dbrows.is_empty() {
        let mut by_table: BTreeMap<u32, Vec<&crate::config::DbRowEntry>> = BTreeMap::new();
        for row in ctx.dbrows.values() {
            if let Some(table) = row.table {
                by_table.entry(table).or_default().push(row);
            }
        }
        lines.push(String::new());
        lines.push("// DB rows grouped by table ID. Key = tableId, value = rows.".to_string());
        lines.push(
            "export const DB_ROWS: ReadonlyMap<number, DbRowEntry[]> = new Map([".to_string(),
        );
        for (table_id, rows) in &by_table {
            let rows_json: String = rows
                .iter()
                .map(|r| {
                    let cols_json: String = r
                        .columns
                        .iter()
                        .map(|c| {
                            let types = c
                                .tuple_types
                                .iter()
                                .map(u16::to_string)
                                .collect::<Vec<_>>()
                                .join(", ");
                            let row_data: String = c
                                .rows
                                .iter()
                                .map(|row| {
                                    let vals = row
                                        .iter()
                                        .map(|v| match v {
                                            crate::config::ScalarValue::Int(i) => i.to_string(),
                                            crate::config::ScalarValue::Long(l) => l.to_string(),
                                            crate::config::ScalarValue::Str(s) => {
                                                format!("'{}'", escape_ts_string(s))
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    format!("[{vals}]")
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!(
                                "{{ column: {}, tupleTypes: [{}], rows: [{}] }}",
                                c.column, types, row_data
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "{{ id: {}, table: {}, columns: [{}] }}",
                        r.id, table_id, cols_json
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("    [{table_id}, [{rows_json}]],"));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const DB_ROW_COUNT = {};", ctx.dbrows.len()));

    // ── Typed wrappers for key tables ──
    lines.push(String::new());

    // ItemEntry (table 163 — 5,237 items)
    lines.push("export interface ItemEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    name: string | null;".to_string());
    lines.push("    description: string | null;".to_string());
    lines.push("    /** Shop / GE price. */".to_string());
    lines.push("    value: number;".to_string());
    lines.push("    /** Non-zero means stackable. */".to_string());
    lines.push("    stackable: boolean;".to_string());
    lines.push("    membersOnly: boolean;".to_string());
    lines.push("    categoryId: number | null;".to_string());
    lines.push("    parentId: number | null;".to_string());
    lines.push("    modelId: number | null;".to_string());
    lines.push("    /** RGBA tint (e.g. 16832257). */".to_string());
    lines.push("    color: number | null;".to_string());
    lines.push("    paramId: number | null;".to_string());
    lines.push("    soundId: number | null;".to_string());
    lines.push("    /** Key→value pairs for linked param configs. */".to_string());
    lines.push("    params: Array<{ key: number; value: number | string }>;".to_string());
    lines.push("    /** Equipment stat overrides (only 2 items). */".to_string());
    lines.push("    equipmentOverrides: number[] | null;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    let items: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|r| r.table == Some(163))
        .collect();
    if !items.is_empty() {
        lines.push("export const ITEMS: ReadonlyMap<number, ItemEntry> = new Map([".to_string());
        for row in &items {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 2);
            let desc = row_column_str(row, 3);
            let value = row_column_int(row, 6).unwrap_or(99);
            let stackable = row_column_int(row, 8).unwrap_or(1) != 0;
            let members = row_column_bool(row, 11);
            let category = row_column_int_or_null(row, 13);
            let parent = row_column_int_or_null(row, 1);
            let model = row_column_int_or_null(row, 23);
            let color = row_column_int_or_null(row, 26);
            let param = row_column_int_or_null(row, 4);
            let sound = row_column_int_or_null(row, 31);
            let eq_overrides = row_column_int_array(row, 30);
            let name_str = name
                .map(|n| format!("'{n}'"))
                .unwrap_or_else(|| "null".to_string());
            let desc_str = desc
                .map(|d| format!("'{d}'"))
                .unwrap_or_else(|| "null".to_string());
            let eq_str = eq_overrides
                .map(|a| {
                    format!(
                        "[{}]",
                        a.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
                    )
                })
                .unwrap_or_else(|| "null".to_string());
            lines.push(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, description: {desc_str}, value: {value}, stackable: {stackable}, membersOnly: {members}, categoryId: {category}, parentId: {parent}, modelId: {model}, color: {color}, paramId: {param}, soundId: {sound}, params: [], equipmentOverrides: {eq_str} }}],",
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const ITEM_COUNT = {};", items.len()));

    // NpcStatEntry (table 29 — 105 NPCs)
    lines.push(String::new());
    lines.push("export interface NpcStatEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    name: string | null;".to_string());
    lines.push("    combatLevel: number;".to_string());
    lines.push("    hitpoints: number;".to_string());
    lines.push("    attack: number;".to_string());
    lines.push("    defence: number;".to_string());
    lines.push("    accuracy: number;".to_string());
    lines.push("    size: number;".to_string());
    lines.push("    respawnMs: number | null;".to_string());
    lines.push("    modelIds: number[];".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    let npc_stats: Vec<_> = ctx
        .dbrows
        .values()
        .filter(|r| r.table == Some(29))
        .collect();
    if !npc_stats.is_empty() {
        lines.push(
            "export const NPC_STATS: ReadonlyMap<number, NpcStatEntry> = new Map([".to_string(),
        );
        for row in &npc_stats {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 5);
            let combat = row_column_int(row, 9).unwrap_or(0);
            let hp = row_column_int(row, 10).unwrap_or(0);
            let atk = row_column_int(row, 14).unwrap_or(0);
            let def = row_column_int(row, 17).unwrap_or(0);
            let acc = row_column_int(row, 18).unwrap_or(0);
            let size = row_column_int(row, 7).unwrap_or(1);
            let respawn = row_column_int_or_null(row, 13);
            let models: Vec<i32> = [1, 2, 3]
                .iter()
                .filter_map(|&c| row_column_int(row, c))
                .collect();
            let name_str = name
                .map(|n| format!("'{n}'"))
                .unwrap_or_else(|| "null".to_string());
            let model_str = format!(
                "[{}]",
                models
                    .iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            lines.push(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, combatLevel: {combat}, hitpoints: {hp}, attack: {atk}, defence: {def}, accuracy: {acc}, size: {size}, respawnMs: {respawn}, modelIds: {model_str} }}],",
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!(
        "export const NPC_STAT_COUNT = {};",
        npc_stats.len()
    ));

    // ClueLocationEntry (table 7 — 62 clue scroll locations)
    lines.push(String::new());
    lines.push("export interface ClueLocationEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    /** Difficulty tier (1-5). */".to_string());
    lines.push("    tier: number;".to_string());
    lines.push("    description: string | null;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    let clue_rows: Vec<_> = ctx.dbrows.values().filter(|r| r.table == Some(7)).collect();
    if !clue_rows.is_empty() {
        lines.push(
            "export const CLUE_LOCATIONS: ReadonlyMap<number, ClueLocationEntry> = new Map(["
                .to_string(),
        );
        for row in &clue_rows {
            let id = row_column_int(row, 0).unwrap_or(0);
            let tier = row_column_int(row, 1).unwrap_or(1);
            let desc = row_column_str(row, 2);
            let desc_str = desc
                .map(|d| format!("'{d}'"))
                .unwrap_or_else(|| "null".to_string());
            lines.push(format!(
                "    [{id}, {{ id: {id}, tier: {tier}, description: {desc_str} }}],",
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!(
        "export const CLUE_LOCATION_COUNT = {};",
        clue_rows.len()
    ));

    // ItemCategoryEntry (table 4 — 83 item categories)
    lines.push(String::new());
    lines.push("export interface ItemCategoryEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    name: string | null;".to_string());
    lines.push("    modelId: number | null;".to_string());
    lines.push("    iconId: number | null;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    let categories: Vec<_> = ctx.dbrows.values().filter(|r| r.table == Some(4)).collect();
    if !categories.is_empty() {
        lines.push(
            "export const ITEM_CATEGORIES: ReadonlyMap<number, ItemCategoryEntry> = new Map(["
                .to_string(),
        );
        for row in &categories {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 1);
            let model = row_column_int_or_null(row, 4);
            let icon = row_column_int_or_null(row, 5);
            let name_str = name
                .map(|n| format!("'{n}'"))
                .unwrap_or_else(|| "null".to_string());
            lines.push(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, modelId: {model}, iconId: {icon} }}],",
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!(
        "export const ITEM_CATEGORY_COUNT = {};",
        categories.len()
    ));

    // ItemSetEntry (table 5 — 160 outfits/sets)
    lines.push(String::new());
    lines.push("export interface ItemSetEntry {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    name: string | null;".to_string());
    lines.push("    description: string | null;".to_string());
    lines.push("    representativeItemId: number | null;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    let sets: Vec<_> = ctx.dbrows.values().filter(|r| r.table == Some(5)).collect();
    if !sets.is_empty() {
        lines.push(
            "export const ITEM_SETS: ReadonlyMap<number, ItemSetEntry> = new Map([".to_string(),
        );
        for row in &sets {
            let id = row_column_int(row, 0).unwrap_or(0);
            let name = row_column_str(row, 1);
            let desc = row_column_str(row, 2);
            let rep_item = row_column_int_or_null(row, 5);
            let name_str = name
                .map(|n| format!("'{n}'"))
                .unwrap_or_else(|| "null".to_string());
            let desc_str = desc
                .map(|d| format!("'{d}'"))
                .unwrap_or_else(|| "null".to_string());
            lines.push(format!(
                "    [{id}, {{ id: {id}, name: {name_str}, description: {desc_str}, representativeItemId: {rep_item} }}],",
            ));
        }
        lines.push("]);".to_string());
        lines.push(String::new());
    }
    lines.push(format!("export const ITEM_SET_COUNT = {};", sets.len()));

    // ── Column index constants for named access ──
    lines.push(String::new());
    lines.push("// Named column indices for table 163 (items).".to_string());
    lines.push("// Example: row.columns[ItemColumn.NAME]".to_string());
    lines.push("export const ItemColumn = {".to_string());
    lines.push("    ID: 0,".to_string());
    lines.push("    PARENT_ID: 1,".to_string());
    lines.push("    NAME: 2,".to_string());
    lines.push("    DESCRIPTION: 3,".to_string());
    lines.push("    PARAM_ID: 4,".to_string());
    lines.push("    TYPE_ID: 5,".to_string());
    lines.push("    VALUE: 6,".to_string());
    lines.push("    FLAGS: 7,".to_string());
    lines.push("    STACKABLE: 8,".to_string());
    lines.push("    MEMBERS_ONLY: 11,".to_string());
    lines.push("    CATEGORY_ID: 13,".to_string());
    lines.push("    MODEL_ID: 23,".to_string());
    lines.push("    MODEL_ID2: 24,".to_string());
    lines.push("    COLOR: 26,".to_string());
    lines.push("    EQUIPMENT_OVERRIDES: 30,".to_string());
    lines.push("    SOUND_ID: 31,".to_string());
    lines.push("} as const;".to_string());
    lines
        .push("export type ItemColumn = (typeof ItemColumn)[keyof typeof ItemColumn];".to_string());
    lines.push(String::new());

    lines.push("// Named column indices for table 29 (NPC stats).".to_string());
    lines.push("export const NpcColumn = {".to_string());
    lines.push("    ID: 0,".to_string());
    lines.push("    MODEL_ID1: 1,".to_string());
    lines.push("    MODEL_ID2: 2,".to_string());
    lines.push("    MODEL_ID3: 3,".to_string());
    lines.push("    NAME: 5,".to_string());
    lines.push("    SIZE: 7,".to_string());
    lines.push("    COMBAT_LEVEL: 9,".to_string());
    lines.push("    HITPOINTS: 10,".to_string());
    lines.push("    RESPAWN_MS: 13,".to_string());
    lines.push("    ATTACK: 14,".to_string());
    lines.push("    DEFENCE: 17,".to_string());
    lines.push("    ACCURACY: 18,".to_string());
    lines.push("} as const;".to_string());
    lines.push("export type NpcColumn = (typeof NpcColumn)[keyof typeof NpcColumn];".to_string());

    write_text(&out_dir.join("dbtables.ts"), &lines.join("\n"))
}

/// Extract the first int value from a specific column in a DB row.
fn row_column_int(row: &crate::config::DbRowEntry, col: u8) -> Option<i32> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| match v {
            crate::config::ScalarValue::Int(i) => Some(*i),
            _ => None,
        })
}

/// Extract the first string value from a specific column in a DB row.
fn row_column_str(row: &crate::config::DbRowEntry, col: u8) -> Option<String> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| match v {
            crate::config::ScalarValue::Str(s) => Some(escape_ts_string(s)),
            _ => None,
        })
}

/// Extract a boolean from a specific column (0=false, non-zero=true).
fn row_column_bool(row: &crate::config::DbRowEntry, col: u8) -> bool {
    row_column_int(row, col).unwrap_or(0) != 0
}

/// Extract an int as a TS null-or-number string.
fn row_column_int_or_null(row: &crate::config::DbRowEntry, col: u8) -> String {
    row_column_int(row, col)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".to_string())
}

/// Extract all ints from a tuple column as a Vec (equipment overrides etc.).
fn row_column_int_array(row: &crate::config::DbRowEntry, col: u8) -> Option<Vec<i32>> {
    row.columns
        .iter()
        .find(|c| c.column == col)
        .and_then(|c| c.rows.first())
        .map(|r| {
            r.iter()
                .filter_map(|v| match v {
                    crate::config::ScalarValue::Int(i) => Some(*i),
                    _ => None,
                })
                .collect()
        })
}

fn export_interface_ids(ctx: &ResolverContext, out_dir: &Path) -> Result<()> {
    // Flatten all component deps into a single map: component_id → ComponentDeps
    let all_components: std::collections::BTreeMap<u32, &crate::interface::ComponentDeps> = ctx
        .parsed_components
        .values()
        .flat_map(|group| group.iter())
        .map(|(&id, deps)| (id, deps))
        .collect();

    let mut lines = vec![
        "// Auto-generated Interface Component definitions".to_string(),
        "// Source: RS3 cache interfaces archive (parsed component deps)".to_string(),
        String::new(),
    ];

    // ── Named component ID constants ──
    let mut named_entries: Vec<(String, u32, &str)> = Vec::new();
    for (&id, deps) in &all_components {
        if let Some(ref name) = deps.name {
            let prop = sanitize_ts_prop(name);
            if !prop.is_empty() {
                named_entries.push((prop, id, &deps.component_type));
            }
        }
    }
    // Deduplicate by property name (keep first occurrence)
    named_entries.sort_by(|a, b| a.0.cmp(&b.0));
    named_entries.dedup_by(|a, b| a.0 == b.0);
    named_entries.sort_by_key(|e| e.1);

    if !named_entries.is_empty() {
        lines.push("// Named component IDs with their numeric values.".to_string());
        lines.push("export const ComponentId = {".to_string());
        for (prop, id, comp_type) in &named_entries {
            lines.push(format!("    /** {comp_type} (id={id}) */"));
            lines.push(format!("    {prop}: {id},"));
        }
        lines.push("} as const;".to_string());
        lines.push(String::new());
        lines.push(
            "export type ComponentId = (typeof ComponentId)[keyof typeof ComponentId];".to_string(),
        );
        lines.push(String::new());
    }

    // ── ComponentInfo interface and data ──
    lines.push("export interface ComponentInfo {".to_string());
    lines.push("    id: number;".to_string());
    lines.push("    type: string;".to_string());
    lines.push("    name: string | null;".to_string());
    lines.push("    children: number[];".to_string());
    lines.push("    scripts: number[];".to_string());
    lines.push("    varps: Array<{domain: string; id: number}>;".to_string());
    lines.push("    varbits: number[];".to_string());
    lines.push("    enums: number[];".to_string());
    lines.push("    params: number[];".to_string());
    lines.push("    invs: number[];".to_string());
    lines.push("    models: number[];".to_string());
    lines.push("    seqs: number[];".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    lines.push(
        "export const ALL_COMPONENTS: ReadonlyMap<number, ComponentInfo> = new Map([".to_string(),
    );
    for (&id, deps) in &all_components {
        let varp_items: Vec<String> = deps
            .varps
            .iter()
            .map(|v| {
                let (domain, id) = match v {
                    crate::interface::VarTransmitRef::Player(id) => ("player", *id),
                    crate::interface::VarTransmitRef::Npc(id) => ("npc", *id),
                    crate::interface::VarTransmitRef::Client(id) => ("client", *id),
                    crate::interface::VarTransmitRef::World(id) => ("world", *id),
                    crate::interface::VarTransmitRef::Region(id) => ("region", *id),
                    crate::interface::VarTransmitRef::Object(id) => ("object", *id),
                    crate::interface::VarTransmitRef::Clan(id) => ("clan", *id),
                    crate::interface::VarTransmitRef::ClanSetting(id) => ("clan_setting", *id),
                    crate::interface::VarTransmitRef::Controller(id) => ("controller", *id),
                    crate::interface::VarTransmitRef::Global(id) => ("global", *id),
                    crate::interface::VarTransmitRef::PlayerGroup(id) => ("player_group", *id),
                    crate::interface::VarTransmitRef::VarClientString(id) => ("client", *id),
                };
                format!("{{domain:'{domain}',id:{id}}}")
            })
            .collect();
        let scripts_json = set_to_json(&deps.scripts);
        let varbits_json = set_to_json(&deps.varbits);
        let enums_json = set_to_json(&deps.enums);
        let params_json = set_to_json(&deps.params);
        let invs_json = set_to_json(&deps.invs);
        let models_json = set_to_json(&deps.models);
        let seqs_json = set_to_json(&deps.seqs);
        let children_json: String = deps
            .children
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let name_str = match &deps.name {
            Some(n) => format!("'{}'", escape_ts_string(n)),
            None => "null".to_string(),
        };
        lines.push(format!(
            "    [{id}, {{ id:{id}, type:'{type}', name:{name}, children:[{children}], scripts:[{scripts}], varps:[{varps}], varbits:[{varbits}], enums:[{enums}], params:[{params}], invs:[{invs}], models:[{models}], seqs:[{seqs}] }}],",
            id = id,
            type = deps.component_type,
            name = name_str,
            children = children_json,
            scripts = scripts_json,
            varps = varp_items.join(", "),
            varbits = varbits_json,
            enums = enums_json,
            params = params_json,
            invs = invs_json,
            models = models_json,
            seqs = seqs_json,
        ));
    }
    lines.push("]);".to_string());
    lines.push(String::new());
    lines.push(format!(
        "export const COMPONENT_COUNT = {};",
        all_components.len()
    ));

    write_text(&out_dir.join("interfaces.ts"), &lines.join("\n"))
}

fn set_to_json(set: &std::collections::HashSet<u32>) -> String {
    let mut items: Vec<u32> = set.iter().copied().collect();
    items.sort_unstable();
    items
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn export_index(out_dir: &Path) -> Result<()> {
    let lines = vec![
        "// Auto-generated index file".to_string(),
        "// Source: RS3 cache ts-export".to_string(),
        String::new(),
        "export {".to_string(),
        "    VARS,".to_string(),
        "    VAR_COUNT,".to_string(),
        "    type VarEntry,".to_string(),
        "    type VarDomain,".to_string(),
        "    type VarType,".to_string(),
        "    type VarLifetime,".to_string(),
        "    type VarTransmitLevel,".to_string(),
        "} from './vars';".to_string(),
        String::new(),
        "export {".to_string(),
        "    VARBITS,".to_string(),
        "    VARBIT_COUNT,".to_string(),
        "    type VarBitEntry,".to_string(),
        "} from './varbits';".to_string(),
        String::new(),
        "export {".to_string(),
        "    ENUMS,".to_string(),
        "    ENUM_COUNT,".to_string(),
        "    ENUM_VALUE_TO_NAME,".to_string(),
        "    type EnumEntry,".to_string(),
        "    type EnumPair,".to_string(),
        "} from './enums';".to_string(),
        String::new(),
        "export {".to_string(),
        "    STRUCTS,".to_string(),
        "    STRUCT_COUNT,".to_string(),
        "    type StructEntry,".to_string(),
        "    type StructParamEntry,".to_string(),
        "} from './structs';".to_string(),
        String::new(),
        "export {".to_string(),
        "    PARAMS,".to_string(),
        "    PARAM_COUNT,".to_string(),
        "    type ParamEntry,".to_string(),
        "    type ParamValue,".to_string(),
        "} from './params';".to_string(),
        String::new(),
        "export {".to_string(),
        "    ComponentId,".to_string(),
        "    ALL_COMPONENTS,".to_string(),
        "    COMPONENT_COUNT,".to_string(),
        "    type ComponentInfo,".to_string(),
        "} from './interfaces';".to_string(),
        "export {{ type InvEntry, INVS, INV_COUNT }} from './invs';".to_string(),
        "export {{ type ObjEntry, OBJS, OBJ_COUNT }} from './objs';".to_string(),
        "export {{ type NpcEntry, NPCS, NPC_COUNT }} from './npcs';".to_string(),
        "export {{ type LocEntry, LOCS, LOC_COUNT }} from './locs';".to_string(),
        "export {{ type SeqEntry, SEQS, SEQ_COUNT }} from './seqs';".to_string(),
        "export {{ type SpotEntry, SPOTS, SPOT_COUNT }} from './spots';".to_string(),
        "export {{ type ItemEntry, ITEMS, ITEM_COUNT,".to_string(),
        "    type ItemCategoryEntry, ITEM_CATEGORIES, ITEM_CATEGORY_COUNT,".to_string(),
        "    type ItemSetEntry, ITEM_SETS, ITEM_SET_COUNT,".to_string(),
        "    type NpcStatEntry, NPC_STATS, NPC_STAT_COUNT,".to_string(),
        "    type ClueLocationEntry, CLUE_LOCATIONS, CLUE_LOCATION_COUNT,".to_string(),
        "    ItemColumn, type ItemColumn, NpcColumn, type NpcColumn,".to_string(),
        "}} from './dbtables';".to_string(),
        "export {{ DB_TABLES, DB_TABLE_COUNT, DB_ROWS, DB_ROW_COUNT,".to_string(),
        "    type DbTableEntry, type DbRowEntry, type DbTableColumn, type DbRowColumn }} from './dbtables';".to_string(),
    ];

    write_text(&out_dir.join("index.ts"), &lines.join("\n"))
}

fn escape_ts_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Convert a RS3 interface component name (`snake_case` or kebab-case)
/// to a valid TypeScript object property name (also `snake_case`, but
/// with hyphens and spaces replaced by underscores).
fn sanitize_ts_prop(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if c == '-' || c == ' ' || c == '/' {
            out.push('_');
        }
        // drop other chars
    }
    // Property can't start with a digit
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    // Can't be empty
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_manifest_entry_serializes() {
        let entry = AudioManifestEntry {
            archive: 14,
            group: 1,
            file: 0,
            size: 123,
            kind: "jaga".to_string(),
            raw_extension: "jaga".to_string(),
            embedded_ogg_offset: Some(32),
            extracted_ogg: true,
        };
        let json = serde_json::to_string(&entry).expect("serialize manifest entry");
        assert!(json.contains("\"archive\":14"));
        assert!(json.contains("\"kind\":\"jaga\""));
    }

    #[test]
    fn sanitize_file_component_rewrites_unsupported_chars() {
        assert_eq!("hello_world", sanitize_file_component("hello/world"));
        assert_eq!("script", sanitize_file_component(""));
    }

    #[test]
    fn extract_name_suffix_parses_tag_syntax() {
        assert_eq!(
            "100guide_flour_drawitems",
            extract_name_suffix("[clientscript,100guide_flour_drawitems]")
        );
        assert_eq!("plain_name", extract_name_suffix("plain_name"));
    }

    #[test]
    fn java_string_hash_matches_known_value() {
        assert_eq!(2_111_159_123, java_string_hash("[clientscript,script0]"));
    }

    #[test]
    fn worldmap_format_helpers_match_expected_shape() {
        assert_eq!("null", format_coordgrid(-1));
        assert_eq!("0_0_0_0_0", format_coordgrid(0));
        assert_eq!("0_50_248_42_54", format_coordgrid(53_132_854));
        assert_eq!("0x00ab12", format_colour(43_794));
        assert_eq!("0xff00ab12", format_colour(-16_733_422));
        assert_eq!("mapelement_42", format_map_element(42));
    }

    #[test]
    fn format_script_source_renders_headers_and_code() {
        let script = CompiledScript {
            name: Some("my/script".to_string()),
            local_count_int: 1,
            local_count_object: 2,
            local_count_long: 3,
            argument_count_int: 4,
            argument_count_object: 5,
            argument_count_long: 6,
            code: vec![Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(42),
            }],
        };

        let source = format_script_source(10, 0, &script);
        assert!(source.contains("// group=10 file=0"));
        assert!(source.contains("// name=my/script"));
        assert!(source.contains("00000: push_constant_int 42"));
    }
}
