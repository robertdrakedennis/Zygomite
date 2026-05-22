// Tree resolver functions take many typed parameters (archive, group, flags, etc.).
// Self type is verbose in deeply nested types; implicit_hasher avoids boilerplate
// when constructing HashSet/HashMap; collapsible_if is clearer in guard chains.
#![allow(
    clippy::too_many_arguments,
    clippy::use_self,
    clippy::implicit_hasher,
    clippy::collapsible_if
)]

use crate::cache::FlatCache;
use crate::config::{
    DbRowEntry, DbTableEntry, EnumEntry, InvEntry, OpListEntry, ParamEntry, ScalarValue, SeqEntry,
    SpotEntry, StructEntry,
};
use crate::constants::{
    ARCHIVE_CLIENTSCRIPTS, ARCHIVE_CONFIG, ARCHIVE_ENUM_CONFIG, ARCHIVE_INTERFACES,
    ARCHIVE_LOC_CONFIG, ARCHIVE_NPC_CONFIG, ARCHIVE_OBJ_CONFIG, ARCHIVE_SEQ_CONFIG,
    ARCHIVE_SPOT_CONFIG, ARCHIVE_STRUCT_CONFIG, CONFIG_GROUP_DBROW, CONFIG_GROUP_DBTABLE,
    CONFIG_GROUP_INV, CONFIG_GROUP_LOC_LEGACY, CONFIG_GROUP_NPC_LEGACY, CONFIG_GROUP_OBJ_LEGACY,
    CONFIG_GROUP_SEQ, CONFIG_GROUP_SPOT, CONFIG_GROUP_VAR_BIT, CONFIG_GROUP_VAR_CLAN,
    CONFIG_GROUP_VAR_CLAN_SETTING, CONFIG_GROUP_VAR_CLIENT, CONFIG_GROUP_VAR_CONTROLLER,
    CONFIG_GROUP_VAR_GLOBAL, CONFIG_GROUP_VAR_NPC, CONFIG_GROUP_VAR_OBJECT,
    CONFIG_GROUP_VAR_PLAYER, CONFIG_GROUP_VAR_PLAYER_GROUP, CONFIG_GROUP_VAR_REGION,
    CONFIG_GROUP_VAR_SHARED, CONFIG_GROUP_VAR_WORLD,
};
use crate::interface::{ComponentDeps, parse_component_deps};
use crate::js5;
use crate::script::{CompiledScript, OpcodeBook, Operand, decode_script};
use crate::vars::{VarBitEntry, VarDomain, VarEntry, parse_var, parse_varbit};
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Interface,
    Component,
    Script,
    VarPlayer,
    VarNpc,
    VarClient,
    VarWorld,
    VarRegion,
    VarObject,
    VarClan,
    VarClanSetting,
    VarController,
    VarGlobal,
    VarPlayerGroup,
    VarBit,
    Param,
    Enum,
    Struct,
    DbTable,
    DbRow,
    Loc,
    Npc,
    Obj,
    Seq,
    Spot,
    Inv,
    Cursor,
    Graphic,
    Model,
    FontMetrics,
    Texture,
    Material,
    Stylesheet,
    Billboard,
    Vfx,
    ParticleEmitter,
    ParticleEffector,
    Headbar,
    Hitmark,
    Light,
    SkyBox,
    WorldArea,
    Achievement,
    Quest,
    Idk,
    Bas,
    Mel,
    Water,
    Category,
    ControllerConfig,
    Area,
    Hunt,
    MesAnim,
    ItemCode,
    GameLogEvent,
    BugTemplate,
    QuickChatCat,
    QuickChatPhrase,
    UiAnimCurve,
    UiAnim,
    AnimatorController,
    Cutscene2D,
    SeqGroup,
    Underlay,
    Overlay,
    Msi,
    Config,
}

impl EntityType {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Interface => "interface",
            Self::Component => "component",
            Self::Script => "script",
            Self::VarPlayer => "varplayer",
            Self::VarNpc => "varnpc",
            Self::VarClient => "varclient",
            Self::VarWorld => "varworld",
            Self::VarRegion => "varregion",
            Self::VarObject => "varobject",
            Self::VarClan => "varclan",
            Self::VarClanSetting => "varclansetting",
            Self::VarController => "varcontroller",
            Self::VarGlobal => "varglobal",
            Self::VarPlayerGroup => "varplayergroup",
            Self::VarBit => "varbit",
            Self::Param => "param",
            Self::Enum => "enum",
            Self::Struct => "struct",
            Self::DbTable => "dbtable",
            Self::DbRow => "dbrow",
            Self::Loc => "loc",
            Self::Npc => "npc",
            Self::Obj => "obj",
            Self::Seq => "seq",
            Self::Spot => "spot",
            Self::Inv => "inv",
            Self::Cursor => "cursor",
            Self::Graphic => "graphic",
            Self::Model => "model",
            Self::FontMetrics => "fontmetrics",
            Self::Texture => "texture",
            Self::Material => "material",
            Self::Stylesheet => "stylesheet",
            Self::Billboard => "billboard",
            Self::Vfx => "vfx",
            Self::ParticleEmitter => "particle_emitter",
            Self::ParticleEffector => "particle_effector",
            Self::Headbar => "headbar",
            Self::Hitmark => "hitmark",
            Self::Light => "light",
            Self::SkyBox => "skybox",
            Self::WorldArea => "worldarea",
            Self::Achievement => "achievement",
            Self::Quest => "quest",
            Self::Idk => "idk",
            Self::Bas => "bas",
            Self::Mel => "mel",
            Self::Water => "water",
            Self::Category => "category",
            Self::ControllerConfig => "controller_config",
            Self::Area => "area",
            Self::Hunt => "hunt",
            Self::MesAnim => "mesanim",
            Self::ItemCode => "itemcode",
            Self::GameLogEvent => "gamelogevent",
            Self::BugTemplate => "bugtemplate",
            Self::QuickChatCat => "quickchatcat",
            Self::QuickChatPhrase => "quickchatphrase",
            Self::UiAnimCurve => "uianimcurve",
            Self::UiAnim => "uianim",
            Self::AnimatorController => "animator_controller",
            Self::Cutscene2D => "cutscene2d",
            Self::SeqGroup => "seqgroup",
            Self::Underlay => "underlay",
            Self::Overlay => "overlay",
            Self::Msi => "msi",
            Self::Config => "config",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EntityKey {
    pub entity_type: EntityType,
    pub id: u32,
    pub sub_id: u32,
}

impl EntityKey {
    pub fn new(entity_type: EntityType, id: u32) -> Self {
        Self {
            entity_type,
            id,
            sub_id: 0,
        }
    }

    pub fn with_sub(entity_type: EntityType, id: u32, sub_id: u32) -> Self {
        Self {
            entity_type,
            id,
            sub_id,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EntityRef {
    pub entity_type: EntityType,
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl EntityRef {
    pub fn key(&self) -> EntityKey {
        EntityKey {
            entity_type: self.entity_type,
            id: self.id,
            sub_id: self.sub_id.unwrap_or(0),
        }
    }

    pub fn new(entity_type: EntityType, id: u32) -> Self {
        Self {
            entity_type,
            id,
            sub_id: None,
            label: None,
        }
    }

    pub fn with_sub(entity_type: EntityType, id: u32, sub_id: u32) -> Self {
        Self {
            entity_type,
            id,
            sub_id: Some(sub_id),
            label: None,
        }
    }

    #[must_use]
    pub fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DependencyTree {
    pub root: DependencyNode,
    pub max_depth: u32,
    pub total_nodes: u32,
    pub cycles_detected: u32,
    pub max_depth_hits: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind")]
pub enum DependencyNode {
    Interface {
        id: u32,
        name: Option<String>,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    Component {
        interface: u32,
        id: u32,
        component_type: String,
        name: Option<String>,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    Script {
        id: u32,
        name: Option<String>,
        local_count: u16,
        argument_count: u16,
        instruction_count: usize,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarPlayer {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarNpc {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarClient {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarWorld {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarRegion {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarObject {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarClan {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarClanSetting {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarController {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarGlobal {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarPlayerGroup {
        id: u32,
        name: String,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    VarBit {
        id: u32,
        name: String,
        base_var: Option<u32>,
        start_bit: Option<u8>,
        end_bit: Option<u8>,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    Param {
        id: u32,
        type_char: Option<u8>,
        type_id: Option<u16>,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    Enum {
        id: u32,
        input_type: Option<String>,
        output_type: Option<String>,
        default: Option<String>,
        value_count: usize,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    Struct {
        id: u32,
        param_count: usize,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    Config {
        config_kind: String,
        id: u32,
        name: Option<String>,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
    Cycle {
        target: String,
    },
    MaxDepth {
        target: String,
    },
    Generic {
        entity_type: String,
        id: u32,
        name: Option<String>,
        #[serde(rename = "dependencies")]
        deps: Vec<DependencyNode>,
    },
}

pub struct ResolverContext {
    pub build: u32,
    pub opcode_book: OpcodeBook,
    pub interfaces: BTreeMap<u32, BTreeMap<u32, Vec<u8>>>,
    pub scripts: BTreeMap<u32, Vec<u8>>,
    pub varps_by_domain: HashMap<VarDomain, BTreeMap<u32, VarEntry>>,
    pub varbits: BTreeMap<u32, VarBitEntry>,
    pub params: BTreeMap<u32, ParamEntry>,
    pub enums: BTreeMap<u32, EnumEntry>,
    pub structs: BTreeMap<u32, StructEntry>,
    pub decoded_scripts: BTreeMap<u32, CompiledScript>,
    pub parsed_components: BTreeMap<u32, BTreeMap<u32, ComponentDeps>>,
    pub npcs: BTreeMap<u32, OpListEntry>,
    pub objs: BTreeMap<u32, OpListEntry>,
    pub locs: BTreeMap<u32, OpListEntry>,
    pub seqs: BTreeMap<u32, SeqEntry>,
    pub spots: BTreeMap<u32, SpotEntry>,
    pub invs: BTreeMap<u32, InvEntry>,
    pub dbtables: BTreeMap<u32, DbTableEntry>,
    pub dbrows: BTreeMap<u32, DbRowEntry>,
}

impl ResolverContext {
    pub fn load(
        cache: &FlatCache,
        tar_path: &Path,
        data_dir: &Path,
        build: u32,
        subbuild: u32,
    ) -> Result<Self> {
        let opcode_book = OpcodeBook::load(data_dir, build, subbuild)?;

        let mut interfaces = BTreeMap::new();
        if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_INTERFACES)
            .is_ok()
        {
            let cache2 = FlatCache::open(cache.root())?;
            let index = cache2.archive_index(ARCHIVE_INTERFACES)?;
            for group in &index.group_id {
                let files = cache2.group_files_with_index(&index, ARCHIVE_INTERFACES, *group)?;
                interfaces.insert(*group, files);
            }
        }

        let mut scripts = BTreeMap::new();
        if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CLIENTSCRIPTS)
            .is_ok()
        {
            let cache2 = FlatCache::open(cache.root())?;
            let index = cache2.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
            for group in &index.group_id {
                let files = cache2.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
                for (file, data) in files {
                    let script_id = (group << 16) | file;
                    scripts.insert(script_id, data);
                }
            }
        }

        let mut varps_by_domain = HashMap::new();
        let var_domains = [
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
        for (group_id, domain) in var_domains {
            if let Some(payload) = cache.get(ARCHIVE_CONFIG, group_id)? {
                let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
                let vars = js5::unpack_group(&config_index, group_id, &payload)?;
                let mut map = BTreeMap::new();
                for (id, bytes) in vars {
                    if let Ok(entry) = parse_var(domain, id, &bytes) {
                        map.insert(id, entry);
                    }
                }
                varps_by_domain.insert(domain, map);
            }
        }

        let mut varbits = BTreeMap::new();
        if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_BIT)? {
            let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
            let raw = js5::unpack_group(&config_index, CONFIG_GROUP_VAR_BIT, &payload)?;
            for (id, bytes) in raw {
                if let Ok(entry) = parse_varbit(id, &bytes) {
                    varbits.insert(id, entry);
                }
            }
        }

        let mut params = BTreeMap::new();
        if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_SHARED)? {
            let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
            let raw = js5::unpack_group(&config_index, CONFIG_GROUP_VAR_SHARED, &payload)?;
            for (id, bytes) in raw {
                if let Ok(entry) = crate::config::parse_param(id, &bytes) {
                    params.insert(id, entry);
                }
            }
        }

        let mut enums = BTreeMap::new();
        if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_ENUM_CONFIG)
            .is_ok()
        {
            let cache2 = FlatCache::open(cache.root())?;
            let index = cache2.archive_index(ARCHIVE_ENUM_CONFIG)?;
            for group in &index.group_id {
                let files = cache2.group_files_with_index(&index, ARCHIVE_ENUM_CONFIG, *group)?;
                for (file, data) in files {
                    let enum_id = (group << 8) | file;
                    if let Ok(entry) = crate::config::parse_enum(enum_id, &data) {
                        enums.insert(enum_id, entry);
                    }
                }
            }
        }

        let mut structs = BTreeMap::new();
        if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_STRUCT_CONFIG)
            .is_ok()
        {
            let cache2 = FlatCache::open(cache.root())?;
            let index = cache2.archive_index(ARCHIVE_STRUCT_CONFIG)?;
            for group in &index.group_id {
                let files = cache2.group_files_with_index(&index, ARCHIVE_STRUCT_CONFIG, *group)?;
                for (file, data) in files {
                    let struct_id = (group << 5) | file;
                    if let Ok(entry) = crate::config::parse_struct(struct_id, &data) {
                        structs.insert(struct_id, entry);
                    }
                }
            }
        }

        // ── Load npcs, objs, locs, seqs, spots, invs from their config archives ──
        let load_config_archive = |archive: u32,
                                   bit_shift: u32,
                                   legacy_group: u32,
                                   parser: fn(u32, &[u8]) -> Result<OpListEntry>|
         -> Result<BTreeMap<u32, OpListEntry>> {
            if crate::fixture::ensure_archive_complete(cache.root(), tar_path, archive).is_ok() {
                let c2 = FlatCache::open(cache.root())?;
                let idx = c2.archive_index(archive)?;
                let mut map = BTreeMap::new();
                for group in &idx.group_id {
                    let files = c2.group_files_with_index(&idx, archive, *group)?;
                    for (file, data) in files {
                        let id = (group << bit_shift) | file;
                        if let Ok(entry) = parser(id, &data) {
                            map.insert(id, entry);
                        }
                    }
                }
                return Ok(map);
            }
            if let Some(payload) = cache.get(ARCHIVE_CONFIG, legacy_group)? {
                let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
                let entries = js5::unpack_group(&config_index, legacy_group, &payload)?;
                return Ok(entries
                    .into_iter()
                    .filter_map(|(id, data)| parser(id, &data).ok().map(|e| (id, e)))
                    .collect());
            }
            Ok(BTreeMap::new())
        };

        let npcs = load_config_archive(
            ARCHIVE_NPC_CONFIG,
            7,
            CONFIG_GROUP_NPC_LEGACY,
            crate::config::parse_npc,
        )?;
        let objs = load_config_archive(
            ARCHIVE_OBJ_CONFIG,
            8,
            CONFIG_GROUP_OBJ_LEGACY,
            crate::config::parse_obj,
        )?;
        let locs = load_config_archive(
            ARCHIVE_LOC_CONFIG,
            8,
            CONFIG_GROUP_LOC_LEGACY,
            crate::config::parse_loc,
        )?;

        // Sequences and spots use dedicated archives
        let seqs: BTreeMap<u32, SeqEntry> = {
            let mut map = BTreeMap::new();
            if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_SEQ_CONFIG)
                .is_ok()
            {
                let c2 = FlatCache::open(cache.root())?;
                let idx = c2.archive_index(ARCHIVE_SEQ_CONFIG)?;
                for group in &idx.group_id {
                    let files = c2.group_files_with_index(&idx, ARCHIVE_SEQ_CONFIG, *group)?;
                    for (file, data) in files {
                        let id = (group << 7) | file;
                        if let Ok(entry) = crate::config::parse_seq(id, &data) {
                            map.insert(id, entry);
                        }
                    }
                }
            } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SEQ)? {
                let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
                let entries = js5::unpack_group(&config_index, CONFIG_GROUP_SEQ, &payload)?;
                for (id, data) in entries {
                    if let Ok(entry) = crate::config::parse_seq(id, &data) {
                        map.insert(id, entry);
                    }
                }
            }
            map
        };

        let spots: BTreeMap<u32, SpotEntry> = {
            let mut map = BTreeMap::new();
            if crate::fixture::ensure_archive_complete(cache.root(), tar_path, ARCHIVE_SPOT_CONFIG)
                .is_ok()
            {
                let c2 = FlatCache::open(cache.root())?;
                let idx = c2.archive_index(ARCHIVE_SPOT_CONFIG)?;
                for group in &idx.group_id {
                    let files = c2.group_files_with_index(&idx, ARCHIVE_SPOT_CONFIG, *group)?;
                    for (file, data) in files {
                        let id = (group << 8) | file;
                        if let Ok(entry) = crate::config::parse_spot(id, &data) {
                            map.insert(id, entry);
                        }
                    }
                }
            } else if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_SPOT)? {
                let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
                let entries = js5::unpack_group(&config_index, CONFIG_GROUP_SPOT, &payload)?;
                for (id, data) in entries {
                    if let Ok(entry) = crate::config::parse_spot(id, &data) {
                        map.insert(id, entry);
                    }
                }
            }
            map
        };

        // Inventories: CONFIG_GROUP_INV within ARCHIVE_CONFIG
        let invs: BTreeMap<u32, InvEntry> = {
            let mut map = BTreeMap::new();
            if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_INV)? {
                let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
                let entries = js5::unpack_group(&config_index, CONFIG_GROUP_INV, &payload)?;
                for (id, data) in entries {
                    if let Ok(entry) = crate::config::parse_inv(id, &data) {
                        map.insert(id, entry);
                    }
                }
            }
            map
        };

        // DB tables and rows — the embedded SQLite-like database system
        let dbtables: BTreeMap<u32, DbTableEntry> = {
            let mut map = BTreeMap::new();
            if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_DBTABLE)? {
                let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
                let entries = js5::unpack_group(&config_index, CONFIG_GROUP_DBTABLE, &payload)?;
                for (id, data) in entries {
                    if let Ok(entry) = crate::config::parse_dbtable(id, &data) {
                        map.insert(id, entry);
                    }
                }
            }
            map
        };

        let dbrows: BTreeMap<u32, DbRowEntry> = {
            let mut map = BTreeMap::new();
            if let Some(payload) = cache.get(ARCHIVE_CONFIG, CONFIG_GROUP_DBROW)? {
                let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
                let entries = js5::unpack_group(&config_index, CONFIG_GROUP_DBROW, &payload)?;
                for (id, data) in entries {
                    if let Ok(entry) = crate::config::parse_dbrow(id, &data) {
                        map.insert(id, entry);
                    }
                }
            }
            map
        };

        // ── Decode scripts ──
        let mut decoded_scripts = BTreeMap::new();
        for (&script_id, bytes) in &scripts {
            if let Ok(script) = decode_script(bytes, &opcode_book, build) {
                decoded_scripts.insert(script_id, script);
            }
        }

        let mut parsed_components = BTreeMap::new();
        for (&group, files) in &interfaces {
            let mut comps = BTreeMap::new();
            for (&comp_id, data) in files {
                if let Ok(deps) = parse_component_deps(comp_id, data, build) {
                    comps.insert(comp_id, deps);
                }
            }
            if !comps.is_empty() {
                parsed_components.insert(group, comps);
            }
        }

        Ok(Self {
            build,
            opcode_book,
            interfaces,
            scripts,
            varps_by_domain,
            varbits,
            params,
            enums,
            structs,
            decoded_scripts,
            parsed_components,
            npcs,
            objs,
            locs,
            seqs,
            spots,
            invs,
            dbtables,
            dbrows,
        })
    }
}

pub fn resolve(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    stats.total_nodes += 1;

    if depth >= max_depth {
        stats.max_depth_hits += 1;
        return DependencyNode::MaxDepth {
            target: format_ref(entity),
        };
    }

    let key = entity.key();
    if !visited.insert(key) {
        stats.cycles_detected += 1;
        return DependencyNode::Cycle {
            target: format_ref(entity),
        };
    }

    match entity.entity_type {
        EntityType::Interface => resolve_interface(ctx, entity, visited, depth, max_depth, stats),
        EntityType::Component => resolve_component(ctx, entity, visited, depth, max_depth, stats),
        EntityType::Script => resolve_script(ctx, entity, visited, depth, max_depth, stats),
        EntityType::VarPlayer => resolve_var(
            ctx,
            entity,
            VarDomain::Player,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarNpc => resolve_var(
            ctx,
            entity,
            VarDomain::Npc,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarClient => resolve_var(
            ctx,
            entity,
            VarDomain::Client,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarWorld => resolve_var(
            ctx,
            entity,
            VarDomain::World,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarRegion => resolve_var(
            ctx,
            entity,
            VarDomain::Region,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarObject => resolve_var(
            ctx,
            entity,
            VarDomain::Object,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarClan => resolve_var(
            ctx,
            entity,
            VarDomain::Clan,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarClanSetting => resolve_var(
            ctx,
            entity,
            VarDomain::ClanSetting,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarController => resolve_var(
            ctx,
            entity,
            VarDomain::Controller,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarGlobal => resolve_var(
            ctx,
            entity,
            VarDomain::Global,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarPlayerGroup => resolve_var(
            ctx,
            entity,
            VarDomain::PlayerGroup,
            visited,
            depth,
            max_depth,
            stats,
        ),
        EntityType::VarBit => resolve_varbit(ctx, entity, visited, depth, max_depth, stats),
        EntityType::Param => resolve_param(ctx, entity, visited, depth, max_depth, stats),
        EntityType::Enum => resolve_enum(ctx, entity, visited, depth, max_depth, stats),
        EntityType::Struct => resolve_struct(ctx, entity, visited, depth, max_depth, stats),
        _ => resolve_generic(ctx, entity, visited, depth, max_depth, stats),
    }
}

pub struct TreeStats {
    pub total_nodes: u32,
    pub cycles_detected: u32,
    pub max_depth_hits: u32,
}

fn resolve_interface(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let group = entity.id;
    let files = ctx.interfaces.get(&group);
    let components = ctx.parsed_components.get(&group);

    let mut deps = Vec::new();

    if let Some(comps) = components {
        for (&comp_id, comp_deps) in comps {
            let comp_node = resolve_component_deps(
                ctx,
                group,
                comp_id,
                comp_deps,
                visited,
                depth + 1,
                max_depth,
                stats,
            );
            deps.push(comp_node);
        }
    }

    let name = files.and_then(|f| f.keys().next().map(|_| format!("interface_{group}")));

    DependencyNode::Interface {
        id: group,
        name,
        deps,
    }
}

fn resolve_component(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let interface_id = entity.id;
    let comp_id = entity.sub_id.unwrap_or(0);

    if let Some(comps) = ctx.parsed_components.get(&interface_id) {
        if let Some(comp_deps) = comps.get(&comp_id) {
            return resolve_component_deps(
                ctx,
                interface_id,
                comp_id,
                comp_deps,
                visited,
                depth,
                max_depth,
                stats,
            );
        }
    }

    DependencyNode::Component {
        interface: interface_id,
        id: comp_id,
        component_type: "unknown".to_string(),
        name: None,
        deps: Vec::new(),
    }
}

fn resolve_component_deps(
    ctx: &ResolverContext,
    interface_id: u32,
    comp_id: u32,
    comp_deps: &ComponentDeps,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let mut deps = Vec::new();

    // Note: comp_deps.children contains the parent layer ID, not child IDs.
    // This is a structural relationship, not a true dependency, so we don't
    // traverse it to avoid false cycle detection from shared parents.

    for &script_id in &comp_deps.scripts {
        let script_ref = EntityRef::new(EntityType::Script, script_id);
        deps.push(resolve(
            ctx,
            &script_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for var_ref in &comp_deps.varps {
        let entity_ref = var_to_entity_ref(var_ref);
        deps.push(resolve(
            ctx,
            &entity_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &varbit_id in &comp_deps.varbits {
        let varbit_ref = EntityRef::new(EntityType::VarBit, varbit_id);
        deps.push(resolve(
            ctx,
            &varbit_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &inv_id in &comp_deps.invs {
        let inv_ref = EntityRef::new(EntityType::Inv, inv_id);
        deps.push(resolve(ctx, &inv_ref, visited, depth + 1, max_depth, stats));
    }

    for &stat_id in &comp_deps.stats {
        let stat_ref = EntityRef::new(EntityType::Config, stat_id).labeled("stat");
        deps.push(resolve(
            ctx,
            &stat_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &graphic_id in &comp_deps.graphics {
        let graphic_ref = EntityRef::new(EntityType::Graphic, graphic_id);
        deps.push(resolve(
            ctx,
            &graphic_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &model_id in &comp_deps.models {
        let model_ref = EntityRef::new(EntityType::Model, model_id);
        deps.push(resolve(
            ctx,
            &model_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &cursor_id in &comp_deps.cursors {
        let cursor_ref = EntityRef::new(EntityType::Cursor, cursor_id);
        deps.push(resolve(
            ctx,
            &cursor_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &stylesheet_id in &comp_deps.stylesheets {
        let stylesheet_ref = EntityRef::new(EntityType::Stylesheet, stylesheet_id);
        deps.push(resolve(
            ctx,
            &stylesheet_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &param_id in &comp_deps.params {
        let param_ref = EntityRef::new(EntityType::Param, param_id);
        deps.push(resolve(
            ctx,
            &param_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &seq_id in &comp_deps.seqs {
        let seq_ref = EntityRef::new(EntityType::Seq, seq_id);
        deps.push(resolve(ctx, &seq_ref, visited, depth + 1, max_depth, stats));
    }

    for &fontmetrics_id in &comp_deps.fontmetrics {
        let font_ref = EntityRef::new(EntityType::FontMetrics, fontmetrics_id);
        deps.push(resolve(
            ctx,
            &font_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &texture_id in &comp_deps.textures {
        let texture_ref = EntityRef::new(EntityType::Texture, texture_id);
        deps.push(resolve(
            ctx,
            &texture_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    for &enum_id in &comp_deps.enums {
        let enum_ref = EntityRef::new(EntityType::Enum, enum_id);
        deps.push(resolve(
            ctx,
            &enum_ref,
            visited,
            depth + 1,
            max_depth,
            stats,
        ));
    }

    DependencyNode::Component {
        interface: interface_id,
        id: comp_id,
        component_type: comp_deps.component_type.clone(),
        name: comp_deps.name.clone(),
        deps,
    }
}

fn resolve_script(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let script_id = entity.id;

    if let Some(script) = ctx.decoded_scripts.get(&script_id) {
        let mut deps = Vec::new();

        for instruction in &script.code {
            match &instruction.operand {
                Operand::VarRef(var_ref) => {
                    let entity_ref = var_ref_to_entity_ref(var_ref);
                    deps.push(resolve(
                        ctx,
                        &entity_ref,
                        visited,
                        depth + 1,
                        max_depth,
                        stats,
                    ));
                }
                Operand::VarBitRef(varbit_ref) => {
                    let varbit_entity =
                        EntityRef::new(EntityType::VarBit, u32::from(varbit_ref.id));
                    deps.push(resolve(
                        ctx,
                        &varbit_entity,
                        visited,
                        depth + 1,
                        max_depth,
                        stats,
                    ));
                }
                Operand::Script(called_id) => {
                    let called_ref = EntityRef::new(EntityType::Script, *called_id as u32);
                    deps.push(resolve(
                        ctx,
                        &called_ref,
                        visited,
                        depth + 1,
                        max_depth,
                        stats,
                    ));
                }
                _ => {}
            }
        }

        let local_count =
            script.local_count_int + script.local_count_object + script.local_count_long;
        let arg_count =
            script.argument_count_int + script.argument_count_object + script.argument_count_long;

        return DependencyNode::Script {
            id: script_id,
            name: script.name.clone(),
            local_count,
            argument_count: arg_count,
            instruction_count: script.code.len(),
            deps,
        };
    }

    DependencyNode::Script {
        id: script_id,
        name: None,
        local_count: 0,
        argument_count: 0,
        instruction_count: 0,
        deps: Vec::new(),
    }
}

fn resolve_var(
    ctx: &ResolverContext,
    entity: &EntityRef,
    domain: VarDomain,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let id = entity.id;

    if let Some(varps) = ctx.varps_by_domain.get(&domain) {
        if let Some(entry) = varps.get(&id) {
            let mut deps = Vec::new();

            if let Some(type_id) = entry.type_id {
                let param_ref = EntityRef::new(EntityType::Param, u32::from(type_id));
                deps.push(resolve(
                    ctx,
                    &param_ref,
                    visited,
                    depth + 1,
                    max_depth,
                    stats,
                ));
            }

            return match domain {
                VarDomain::Player => DependencyNode::VarPlayer {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::Npc => DependencyNode::VarNpc {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::Client => DependencyNode::VarClient {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::World => DependencyNode::VarWorld {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::Region => DependencyNode::VarRegion {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::Object => DependencyNode::VarObject {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::Clan => DependencyNode::VarClan {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::ClanSetting => DependencyNode::VarClanSetting {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::Controller => DependencyNode::VarController {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::Global => DependencyNode::VarGlobal {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
                VarDomain::PlayerGroup => DependencyNode::VarPlayerGroup {
                    id,
                    name: entry.var_name.clone(),
                    deps,
                },
            };
        }
    }

    let name = format!("var{}_{}", domain.as_label(), id);
    match domain {
        VarDomain::Player => DependencyNode::VarPlayer {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::Npc => DependencyNode::VarNpc {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::Client => DependencyNode::VarClient {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::World => DependencyNode::VarWorld {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::Region => DependencyNode::VarRegion {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::Object => DependencyNode::VarObject {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::Clan => DependencyNode::VarClan {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::ClanSetting => DependencyNode::VarClanSetting {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::Controller => DependencyNode::VarController {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::Global => DependencyNode::VarGlobal {
            id,
            name,
            deps: Vec::new(),
        },
        VarDomain::PlayerGroup => DependencyNode::VarPlayerGroup {
            id,
            name,
            deps: Vec::new(),
        },
    }
}

fn resolve_varbit(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let id = entity.id;

    if let Some(entry) = ctx.varbits.get(&id) {
        let mut deps = Vec::new();

        if let Some(base_var) = entry.base_var {
            if let Some(domain) = entry.domain {
                let var_ref = EntityRef::new(var_domain_to_entity_type(domain), base_var);
                deps.push(resolve(ctx, &var_ref, visited, depth + 1, max_depth, stats));
            }
        }

        return DependencyNode::VarBit {
            id,
            name: entry.varbit_name.clone(),
            base_var: entry.base_var,
            start_bit: entry.start_bit,
            end_bit: entry.end_bit,
            deps,
        };
    }

    DependencyNode::VarBit {
        id,
        name: format!("varbit_{id}"),
        base_var: None,
        start_bit: None,
        end_bit: None,
        deps: Vec::new(),
    }
}

fn resolve_param(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let id = entity.id;

    if let Some(entry) = ctx.params.get(&id) {
        let mut deps = Vec::new();

        if let Some(ScalarValue::Int(type_id)) = &entry.default {
            if let Some(type_char) = entry.type_char {
                if type_char == b's' || type_char == b'S' {
                    let struct_ref = EntityRef::new(EntityType::Struct, *type_id as u32);
                    deps.push(resolve(
                        ctx,
                        &struct_ref,
                        visited,
                        depth + 1,
                        max_depth,
                        stats,
                    ));
                }
            }
        }

        return DependencyNode::Param {
            id,
            type_char: entry.type_char,
            type_id: entry.type_id,
            deps,
        };
    }

    DependencyNode::Param {
        id,
        type_char: None,
        type_id: None,
        deps: Vec::new(),
    }
}

fn resolve_enum(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let id = entity.id;

    if let Some(entry) = ctx.enums.get(&id) {
        let mut deps = Vec::new();

        for pair in &entry.values {
            if let ScalarValue::Int(struct_id) = &pair.value {
                if let Some(input_char) = entry.input_type_char {
                    if input_char == b's' || input_char == b'S' {
                        let struct_ref = EntityRef::new(EntityType::Struct, *struct_id as u32);
                        deps.push(resolve(
                            ctx,
                            &struct_ref,
                            visited,
                            depth + 1,
                            max_depth,
                            stats,
                        ));
                    }
                }
            }
        }

        let input_type = entry.input_type_char.map(type_char_to_string);
        let output_type = entry.output_type_char.map(type_char_to_string);
        let default = entry.default.as_ref().map(scalar_to_string);

        return DependencyNode::Enum {
            id,
            input_type,
            output_type,
            default,
            value_count: entry.values.len(),
            deps,
        };
    }

    DependencyNode::Enum {
        id,
        input_type: None,
        output_type: None,
        default: None,
        value_count: 0,
        deps: Vec::new(),
    }
}

fn resolve_struct(
    ctx: &ResolverContext,
    entity: &EntityRef,
    visited: &mut HashSet<EntityKey>,
    depth: u32,
    max_depth: u32,
    stats: &mut TreeStats,
) -> DependencyNode {
    let id = entity.id;

    if let Some(entry) = ctx.structs.get(&id) {
        let mut deps = Vec::new();

        for param_entry in &entry.params {
            let param_ref = EntityRef::new(EntityType::Param, param_entry.param_id);
            deps.push(resolve(
                ctx,
                &param_ref,
                visited,
                depth + 1,
                max_depth,
                stats,
            ));

            if let ScalarValue::Int(struct_id) = &param_entry.value {
                let nested_struct_ref = EntityRef::new(EntityType::Struct, *struct_id as u32);
                deps.push(resolve(
                    ctx,
                    &nested_struct_ref,
                    visited,
                    depth + 1,
                    max_depth,
                    stats,
                ));
            }
        }

        return DependencyNode::Struct {
            id,
            param_count: entry.params.len(),
            deps,
        };
    }

    DependencyNode::Struct {
        id,
        param_count: 0,
        deps: Vec::new(),
    }
}

fn resolve_generic(
    _ctx: &ResolverContext,
    entity: &EntityRef,
    _visited: &mut HashSet<EntityKey>,
    _depth: u32,
    _max_depth: u32,
    _stats: &mut TreeStats,
) -> DependencyNode {
    DependencyNode::Generic {
        entity_type: entity.entity_type.as_label().to_string(),
        id: entity.id,
        name: entity.label.clone(),
        deps: Vec::new(),
    }
}

fn var_to_entity_ref(domain: &crate::interface::VarTransmitRef) -> EntityRef {
    match domain {
        crate::interface::VarTransmitRef::Player(id) => EntityRef::new(EntityType::VarPlayer, *id),
        crate::interface::VarTransmitRef::Npc(id) => EntityRef::new(EntityType::VarNpc, *id),
        crate::interface::VarTransmitRef::Client(id) => EntityRef::new(EntityType::VarClient, *id),
        crate::interface::VarTransmitRef::World(id) => EntityRef::new(EntityType::VarWorld, *id),
        crate::interface::VarTransmitRef::Region(id) => EntityRef::new(EntityType::VarRegion, *id),
        crate::interface::VarTransmitRef::Object(id) => EntityRef::new(EntityType::VarObject, *id),
        crate::interface::VarTransmitRef::Clan(id) => EntityRef::new(EntityType::VarClan, *id),
        crate::interface::VarTransmitRef::ClanSetting(id) => {
            EntityRef::new(EntityType::VarClanSetting, *id)
        }
        crate::interface::VarTransmitRef::Controller(id) => {
            EntityRef::new(EntityType::VarController, *id)
        }
        crate::interface::VarTransmitRef::Global(id) => EntityRef::new(EntityType::VarGlobal, *id),
        crate::interface::VarTransmitRef::PlayerGroup(id) => {
            EntityRef::new(EntityType::VarPlayerGroup, *id)
        }
        crate::interface::VarTransmitRef::VarClientString(id) => {
            EntityRef::new(EntityType::VarClient, *id)
        }
    }
}

pub fn var_ref_to_entity_ref(var_ref: &crate::script::VarRef) -> EntityRef {
    let entity_type = var_domain_to_entity_type(var_ref.domain);
    EntityRef::new(entity_type, u32::from(var_ref.id))
}

fn var_domain_to_entity_type(domain: VarDomain) -> EntityType {
    match domain {
        VarDomain::Player => EntityType::VarPlayer,
        VarDomain::Npc => EntityType::VarNpc,
        VarDomain::Client => EntityType::VarClient,
        VarDomain::World => EntityType::VarWorld,
        VarDomain::Region => EntityType::VarRegion,
        VarDomain::Object => EntityType::VarObject,
        VarDomain::Clan => EntityType::VarClan,
        VarDomain::ClanSetting => EntityType::VarClanSetting,
        VarDomain::Controller => EntityType::VarController,
        VarDomain::PlayerGroup => EntityType::VarPlayerGroup,
        VarDomain::Global => EntityType::VarGlobal,
    }
}

fn format_ref(entity: &EntityRef) -> String {
    if let Some(sub_id) = entity.sub_id {
        format!("{}_{}#{}", entity.entity_type.as_label(), entity.id, sub_id)
    } else {
        format!("{}_{}", entity.entity_type.as_label(), entity.id)
    }
}

fn type_char_to_string(c: u8) -> String {
    match c {
        b'i' => "int".to_string(),
        b'l' => "long".to_string(),
        b's' => "string".to_string(),
        b'S' => "struct".to_string(),
        _ => format!("char_{}", c as char),
    }
}

fn scalar_to_string(s: &ScalarValue) -> String {
    match s {
        ScalarValue::Int(v) => v.to_string(),
        ScalarValue::Long(v) => v.to_string(),
        ScalarValue::Str(v) => v.clone(),
    }
}

pub fn build_tree(ctx: &ResolverContext, root: &EntityRef, max_depth: u32) -> DependencyTree {
    let mut visited = HashSet::new();
    let mut stats = TreeStats {
        total_nodes: 0,
        cycles_detected: 0,
        max_depth_hits: 0,
    };
    let root_node = resolve(ctx, root, &mut visited, 0, max_depth, &mut stats);

    DependencyTree {
        root: root_node,
        max_depth,
        total_nodes: stats.total_nodes,
        cycles_detected: stats.cycles_detected,
        max_depth_hits: stats.max_depth_hits,
    }
}
