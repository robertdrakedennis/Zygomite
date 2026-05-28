use crate::cache::FlatCache;
use crate::config::{
    parse_bas, parse_dbrow, parse_dbtable, parse_enum, parse_loc, parse_material, parse_npc,
    parse_obj, parse_quest, parse_seq, parse_seqgroup, parse_spot, parse_struct,
};
use crate::constants::{
    ARCHIVE_BINARY, ARCHIVE_CLIENTSCRIPTS, ARCHIVE_CONFIG, ARCHIVE_ENUM_CONFIG, ARCHIVE_INTERFACES,
    ARCHIVE_LOC_CONFIG, ARCHIVE_MATERIALS, ARCHIVE_MODELS_RT7, ARCHIVE_NPC_CONFIG,
    ARCHIVE_OBJ_CONFIG, ARCHIVE_PARTICLES, ARCHIVE_SEQ_CONFIG, ARCHIVE_SPOT_CONFIG,
    ARCHIVE_STRUCT_CONFIG, CONFIG_GROUP_BAS, CONFIG_GROUP_DBROW, CONFIG_GROUP_DBTABLE,
    CONFIG_GROUP_MATERIAL_ARCHIVE26, CONFIG_GROUP_QUEST, CONFIG_GROUP_SEQGROUP,
    CONFIG_GROUP_VAR_BIT, CONFIG_GROUP_VAR_PLAYER,
};
use crate::dep_tree::ResolverContext;
use crate::fixture::default_tar_path;
use crate::js5::ArchiveIndex;
use crate::migrate::{ConflictEntry, MigrationAnalyzer, TargetValidationReport};
use crate::overlay_deps::DependencySite;
use crate::packet::Packet;
use anyhow::{Context, Result, ensure};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

pub const OVERLAY_PLAN_VERSION: u32 = 1;

const MAX_PLAN_WARNINGS: usize = 2048;
const MAX_EDGE_SAMPLES: usize = 256;
type SemanticRefBuckets = HashMap<u32, HashMap<SemanticRefKey, Vec<u32>>>;
type ConfigGroupFileCache = HashMap<(RootKind, u32, u32), Option<BTreeMap<u32, Vec<u8>>>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheOverlayManifest {
    #[serde(default)]
    roots: OverlayRootOverrides,
    #[serde(default)]
    base_raw_root: Option<PathBuf>,
    #[serde(default)]
    donor_raw_root: Option<PathBuf>,
    #[serde(default)]
    base_semantic_root: Option<PathBuf>,
    #[serde(default)]
    donor_semantic_root: Option<PathBuf>,
    #[serde(default)]
    base_pack_root: Option<PathBuf>,
    #[serde(default)]
    output_pack_root: Option<PathBuf>,
    #[serde(default)]
    client_output_pack_root: Option<PathBuf>,
    #[serde(default)]
    imports: OverlayImports,
    #[serde(default)]
    archive_modes: BTreeMap<String, ArchiveMode>,
    #[serde(default)]
    conflict_policy: Option<String>,
    #[serde(default)]
    allow: OverlayAllow,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_field_names,
    reason = "overlay manifest JSON contract uses *_root field names"
)]
struct OverlayRootOverrides {
    #[serde(default)]
    base_raw_root: Option<PathBuf>,
    #[serde(default)]
    donor_raw_root: Option<PathBuf>,
    #[serde(default)]
    base_semantic_root: Option<PathBuf>,
    #[serde(default)]
    donor_semantic_root: Option<PathBuf>,
    #[serde(default)]
    base_pack_root: Option<PathBuf>,
    #[serde(default)]
    output_pack_root: Option<PathBuf>,
    #[serde(default)]
    client_output_pack_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OverlayImports {
    #[serde(default)]
    map_archive: Option<String>,
    #[serde(default)]
    full_archives: Vec<ArchiveRef>,
    #[serde(default)]
    config_groups: Vec<u32>,
    #[serde(default)]
    maps: Vec<u32>,
    #[serde(default)]
    regions: Vec<RegionSpec>,
    #[serde(default)]
    objs: Vec<u32>,
    #[serde(default)]
    npcs: Vec<u32>,
    #[serde(default)]
    locs: Vec<u32>,
    #[serde(default)]
    seqs: Vec<u32>,
    #[serde(default)]
    bas: Vec<u32>,
    #[serde(default)]
    structs: Vec<u32>,
    #[serde(default)]
    quests: Vec<u32>,
    #[serde(default)]
    enums: Vec<u32>,
    #[serde(default)]
    varbits: Vec<u32>,
    #[serde(default)]
    varps: Vec<u32>,
    #[serde(default)]
    db_tables: Vec<u32>,
    #[serde(default)]
    db_rows: Vec<u32>,
    #[serde(default)]
    interfaces: Vec<u32>,
    #[serde(default)]
    scripts: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ArchiveRef {
    Name(String),
    Id(u32),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RegionSpec {
    Id(u32),
    Text(String),
    Coord { x: u32, z: u32 },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ArchiveMode {
    Auto,
    Patch,
    HardSwap,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OverlayAllow {
    #[serde(default)]
    db_table_schema_changes: Vec<u32>,
    #[serde(default)]
    enum_ids: Vec<u32>,
    #[serde(default)]
    varbit_ids: Vec<u32>,
    #[serde(default)]
    varp_ids: Vec<u32>,
    #[serde(default)]
    varbit_conflict_ids: Vec<u32>,
    #[serde(default)]
    varp_conflict_ids: Vec<u32>,
    #[serde(default)]
    hard_swap_archives: Vec<ArchiveRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_field_names,
    reason = "overlay plan JSON contract uses *_root field names"
)]
struct OverlayRoots {
    base_raw_root: String,
    donor_raw_root: String,
    base_semantic_root: String,
    donor_semantic_root: String,
    base_pack_root: String,
    output_pack_root: String,
    client_output_pack_root: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanOutput {
    roots: OverlayRoots,
    conflict_policy: String,
    hard_swap_archives: Vec<String>,
    patch_archives: Vec<String>,
    selected: OverlayPlanSelected,
    imports: OverlayPlanImports,
    dependencies: BTreeMap<String, Vec<u32>>,
    db: OverlayPlanDb,
    blocked_conflicts: Vec<OverlayBlockedIssue>,
    warnings: Vec<OverlayWarning>,
    semantic_source: &'static str,
    semantic_manifest: OverlaySemanticManifest,
    dependency_edges_sample: Vec<DependencyEdgeSample>,
    plan_version: u32,
    planner_fingerprint: String,
    proof: OverlayPlanProof,
    audit: OverlayPlanAudit,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanSelected {
    groups: Vec<OverlayPlanArchiveGroups>,
    files: Vec<OverlayPlanArchiveFiles>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanArchiveGroups {
    archive: String,
    archive_id: u32,
    mode: &'static str,
    groups: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanArchiveFiles {
    archive: String,
    archive_id: u32,
    mode: &'static str,
    group_id: u32,
    file_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanImports {
    config_groups: Vec<u32>,
    maps: Vec<u32>,
    objs: Vec<u32>,
    npcs: Vec<u32>,
    locs: Vec<u32>,
    structs: Vec<u32>,
    enums: Vec<u32>,
    varbits: Vec<u32>,
    varps: Vec<u32>,
    db_tables: Vec<u32>,
    db_rows: Vec<u32>,
    interfaces: Vec<u32>,
    scripts: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanDb {
    tables: Vec<u32>,
    rows: Vec<u32>,
    index_groups: Vec<u32>,
    schema_changes: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlaySemanticManifest {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    donor_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Rs3CacheManifest {
    tool_version: String,
    build: u32,
    subbuild: u32,
    cache_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayWarning {
    kind: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ref_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayBlockedIssue {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ref_kind: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanProof {
    status: &'static str,
    strict: bool,
    unsupported_site_count: usize,
    heuristic_site_count: usize,
    script_summary: OverlayProofSummary,
    component_summary: OverlayProofSummary,
    next_actions: Vec<String>,
    blockers: Vec<OverlayProofIssue>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayProofSummary {
    checked: usize,
    blocked: usize,
    valid: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayProofIssue {
    kind: &'static str,
    location: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ref_kind: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayPlanAudit {
    relative_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DependencyEdgeSample {
    from: String,
    to: String,
    kind: String,
    reason: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum RefKind {
    Model,
    Anim,
    Seq,
    Bas,
    Spot,
    Material,
    Struct,
    Enum,
    VarBit,
    Varp,
    Obj,
    Npc,
    Loc,
    Quest,
    DbRow,
    DbTable,
    Sprite,
    SeqGroup,
    Interface,
    Script,
}

impl RefKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Anim => "anim",
            Self::Seq => "seq",
            Self::Bas => "bas",
            Self::Spot => "spot",
            Self::Material => "material",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::VarBit => "varbit",
            Self::Varp => "varp",
            Self::Obj => "obj",
            Self::Npc => "npc",
            Self::Loc => "loc",
            Self::Quest => "quest",
            Self::DbRow => "dbrow",
            Self::DbTable => "dbtable",
            Self::Sprite => "sprite",
            Self::SeqGroup => "seqgroup",
            Self::Interface => "interface",
            Self::Script => "script",
        }
    }

    fn from_entity_type(entity_type: &str) -> Option<Self> {
        Some(match entity_type {
            "script" => Self::Script,
            "interface" => Self::Interface,
            "obj" => Self::Obj,
            "npc" => Self::Npc,
            "loc" => Self::Loc,
            "seq" => Self::Seq,
            "bas" => Self::Bas,
            "spot" => Self::Spot,
            "struct" => Self::Struct,
            "enum" => Self::Enum,
            "varbit" => Self::VarBit,
            "varplayer" => Self::Varp,
            "quest" => Self::Quest,
            "dbtable" => Self::DbTable,
            "dbrow" => Self::DbRow,
            "model" => Self::Model,
            "anim" => Self::Anim,
            "material" => Self::Material,
            "seqgroup" => Self::SeqGroup,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
struct ArchiveDef {
    id: u32,
    donor_id: u32,
    name: &'static str,
}

#[derive(Debug, Clone)]
struct ConfigTarget {
    archive: ArchiveDef,
    group_id: u32,
    file_id: u32,
    id: u32,
    kind: RefKind,
}

#[derive(Debug, Clone)]
struct RawGroupTarget {
    archive: ArchiveDef,
    group_id: u32,
    id: u32,
    kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionMode {
    Primary,
    Dependency,
}

#[derive(Debug, Clone)]
struct PendingRef {
    kind: RefKind,
    id: u32,
    source: String,
    mode: SelectionMode,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
enum RootKind {
    Base,
    Donor,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum SemanticRefKey {
    Anim,
    Bas,
    Cursor,
    DbRow,
    DbTable,
    Enum,
    Graphic,
    Interface,
    Loc,
    Material,
    Model,
    Msi,
    MultivarVarbit,
    MultivarVarp,
    Npc,
    Obj,
    Param,
    Quest,
    Script,
    Seq,
    SeqGroup,
    Spot,
    Spotanim,
    Sprite,
    Struct,
    VarBit,
    Varp,
    VarpClan,
    VarpClanSetting,
    VarpClient,
    VarpController,
    VarpNpc,
    VarpObject,
    VarpPlayer,
    VarpRegion,
    VarpWorld,
    Vfx,
    Other(Box<str>),
}

impl SemanticRefKey {
    fn from_label(label: &str) -> Self {
        match label {
            "anim" => Self::Anim,
            "bas" => Self::Bas,
            "cursor" => Self::Cursor,
            "dbrow" => Self::DbRow,
            "dbtable" => Self::DbTable,
            "enum" => Self::Enum,
            "graphic" => Self::Graphic,
            "interface" => Self::Interface,
            "loc" => Self::Loc,
            "material" => Self::Material,
            "model" => Self::Model,
            "msi" => Self::Msi,
            "multivar_varbit" => Self::MultivarVarbit,
            "multivar_varp" => Self::MultivarVarp,
            "npc" => Self::Npc,
            "obj" => Self::Obj,
            "param" => Self::Param,
            "quest" => Self::Quest,
            "script" => Self::Script,
            "seq" => Self::Seq,
            "seqgroup" => Self::SeqGroup,
            "spot" => Self::Spot,
            "spotanim" => Self::Spotanim,
            "sprite" => Self::Sprite,
            "struct" => Self::Struct,
            "varbit" => Self::VarBit,
            "varp" => Self::Varp,
            "varp_clan" => Self::VarpClan,
            "varp_clan_setting" => Self::VarpClanSetting,
            "varp_client" => Self::VarpClient,
            "varp_controller" => Self::VarpController,
            "varp_npc" => Self::VarpNpc,
            "varp_object" => Self::VarpObject,
            "varp_player" => Self::VarpPlayer,
            "varp_region" => Self::VarpRegion,
            "varp_world" => Self::VarpWorld,
            "vfx" => Self::Vfx,
            other => Self::Other(other.into()),
        }
    }

    fn as_label(&self) -> &str {
        match self {
            Self::Anim => "anim",
            Self::Bas => "bas",
            Self::Cursor => "cursor",
            Self::DbRow => "dbrow",
            Self::DbTable => "dbtable",
            Self::Enum => "enum",
            Self::Graphic => "graphic",
            Self::Interface => "interface",
            Self::Loc => "loc",
            Self::Material => "material",
            Self::Model => "model",
            Self::Msi => "msi",
            Self::MultivarVarbit => "multivar_varbit",
            Self::MultivarVarp => "multivar_varp",
            Self::Npc => "npc",
            Self::Obj => "obj",
            Self::Param => "param",
            Self::Quest => "quest",
            Self::Script => "script",
            Self::Seq => "seq",
            Self::SeqGroup => "seqgroup",
            Self::Spot => "spot",
            Self::Spotanim => "spotanim",
            Self::Sprite => "sprite",
            Self::Struct => "struct",
            Self::VarBit => "varbit",
            Self::Varp => "varp",
            Self::VarpClan => "varp_clan",
            Self::VarpClanSetting => "varp_clan_setting",
            Self::VarpClient => "varp_client",
            Self::VarpController => "varp_controller",
            Self::VarpNpc => "varp_npc",
            Self::VarpObject => "varp_object",
            Self::VarpPlayer => "varp_player",
            Self::VarpRegion => "varp_region",
            Self::VarpWorld => "varp_world",
            Self::Vfx => "vfx",
            Self::Other(other) => other.as_ref(),
        }
    }
}

#[derive(Debug, Clone)]
struct RefGraphRepository {
    graphs: HashMap<String, HashMap<u32, HashMap<SemanticRefKey, Vec<u32>>>>,
}

#[derive(Debug, Clone)]
struct ConfigSemanticIndex {
    ref_graph: RefGraphRepository,
    dependency_edges_sample: Vec<DependencyEdgeSample>,
    edge_sample_overflow: usize,
    missing_refs_kinds_logged: HashSet<String>,
    partial_refs_kinds_logged: HashSet<String>,
}

#[derive(Debug, Clone)]
struct ProofState {
    script_checked: usize,
    script_blocked: usize,
    script_valid: usize,
    component_checked: usize,
    component_blocked: usize,
    component_valid: usize,
    blockers: Vec<OverlayProofIssue>,
}

struct PlanBuilder<'a> {
    manifest: CacheOverlayManifest,
    roots: OverlayRoots,
    base_cache: FlatCache,
    donor_cache: FlatCache,
    data_dir: &'a Path,
    base_build: u32,
    donor_build: u32,
    base_subbuild: u32,
    donor_subbuild: u32,
    group_selections: BTreeMap<u32, BTreeSet<u32>>,
    file_selections: BTreeMap<(u32, u32), BTreeSet<u32>>,
    primary_maps: BTreeSet<u32>,
    primary_objs: BTreeSet<u32>,
    primary_npcs: BTreeSet<u32>,
    primary_locs: BTreeSet<u32>,
    primary_structs: BTreeSet<u32>,
    primary_enums: BTreeSet<u32>,
    primary_varbits: BTreeSet<u32>,
    primary_varps: BTreeSet<u32>,
    primary_db_tables: BTreeSet<u32>,
    primary_db_rows: BTreeSet<u32>,
    primary_interfaces: BTreeSet<u32>,
    primary_scripts: BTreeSet<u32>,
    dependencies: BTreeMap<RefKind, BTreeSet<u32>>,
    warnings: Vec<OverlayWarning>,
    blocked: Vec<OverlayBlockedIssue>,
    pending: VecDeque<PendingRef>,
    seen_refs: HashSet<(RefKind, u32)>,
    indexes: HashMap<(RootKind, u32), Option<ArchiveIndex>>,
    config_group_files: ConfigGroupFileCache,
    full_archive_selections: BTreeSet<u32>,
    auto_allowed_missing_varbits: BTreeSet<u32>,
    auto_allowed_missing_varps: BTreeSet<u32>,
    semantic_index: ConfigSemanticIndex,
    db_schema_changes: BTreeSet<u32>,
    warning_overflow: usize,
    proof: ProofState,
    analyzer: Option<MigrationAnalyzer>,
    base_manifest: Rs3CacheManifest,
    donor_manifest: Rs3CacheManifest,
}

#[derive(Debug, Clone, Copy)]
pub struct OverlayPlanCommandOptions<'a> {
    pub manifest: &'a Path,
    pub out_file: Option<&'a Path>,
    pub audit_dir: Option<&'a Path>,
    pub allow_heuristic_sites: bool,
    pub data_dir: &'a Path,
    pub base_build: u32,
    pub donor_build: u32,
    pub base_subbuild: u32,
    pub donor_subbuild: u32,
}

pub fn run_overlay_plan_command(options: OverlayPlanCommandOptions<'_>) -> Result<()> {
    let OverlayPlanCommandOptions {
        manifest,
        out_file,
        audit_dir,
        allow_heuristic_sites,
        data_dir,
        base_build,
        donor_build,
        base_subbuild,
        donor_subbuild,
    } = options;
    ensure!(
        donor_build == 947,
        "native overlay-plan currently supports donor build 947 only"
    );
    let manifest_value: CacheOverlayManifest = serde_json::from_slice(
        &fs::read(manifest).with_context(|| format!("reading {}", manifest.display()))?,
    )
    .with_context(|| format!("decoding overlay manifest {}", manifest.display()))?;
    let roots = resolve_roots(&manifest_value)?;
    let donor_manifest = read_semantic_manifest(&PathBuf::from(&roots.donor_semantic_root))?;
    let base_manifest = read_semantic_manifest(&PathBuf::from(&roots.base_semantic_root))?;
    ensure!(
        donor_manifest.build == donor_build && donor_manifest.subbuild == donor_subbuild,
        "donor semantic tree build mismatch: expected {}.{}, found {}.{}",
        donor_build,
        donor_subbuild,
        donor_manifest.build,
        donor_manifest.subbuild
    );
    ensure!(
        base_manifest.build == base_build && base_manifest.subbuild == base_subbuild,
        "base semantic tree build mismatch: expected {}.{}, found {}.{}",
        base_build,
        base_subbuild,
        base_manifest.build,
        base_manifest.subbuild
    );

    let semantic_index = ConfigSemanticIndex::new(Path::new(&roots.donor_semantic_root))?;
    let mut builder = PlanBuilder {
        manifest: manifest_value,
        roots: roots.clone(),
        base_cache: FlatCache::open(&roots.base_raw_root)?,
        donor_cache: FlatCache::open(&roots.donor_raw_root)?,
        data_dir,
        base_build,
        donor_build,
        base_subbuild,
        donor_subbuild,
        group_selections: BTreeMap::new(),
        file_selections: BTreeMap::new(),
        primary_maps: BTreeSet::new(),
        primary_objs: BTreeSet::new(),
        primary_npcs: BTreeSet::new(),
        primary_locs: BTreeSet::new(),
        primary_structs: BTreeSet::new(),
        primary_enums: BTreeSet::new(),
        primary_varbits: BTreeSet::new(),
        primary_varps: BTreeSet::new(),
        primary_db_tables: BTreeSet::new(),
        primary_db_rows: BTreeSet::new(),
        primary_interfaces: BTreeSet::new(),
        primary_scripts: BTreeSet::new(),
        dependencies: BTreeMap::new(),
        warnings: Vec::new(),
        blocked: Vec::new(),
        pending: VecDeque::new(),
        seen_refs: HashSet::new(),
        indexes: HashMap::new(),
        config_group_files: HashMap::new(),
        full_archive_selections: BTreeSet::new(),
        auto_allowed_missing_varbits: BTreeSet::new(),
        auto_allowed_missing_varps: BTreeSet::new(),
        semantic_index,
        db_schema_changes: BTreeSet::new(),
        warning_overflow: 0,
        proof: ProofState {
            script_checked: 0,
            script_blocked: 0,
            script_valid: 0,
            component_checked: 0,
            component_blocked: 0,
            component_valid: 0,
            blockers: Vec::new(),
        },
        analyzer: None,
        donor_manifest,
        base_manifest,
    };

    seed_imports(&mut builder)?;
    build_selections(&mut builder, allow_heuristic_sites)?;
    let mut plan = finalize_plan(builder, allow_heuristic_sites)?;
    if let Some(dir) = audit_dir {
        plan.audit = write_overlay_plan_audit(dir, &plan.proof, &plan.blocked_conflicts)?;
    }

    if let Some(path) = out_file {
        write_json(path, &serde_json::to_value(&plan)?)?;
    } else {
        print_json(&serde_json::to_value(&plan)?)?;
    }

    eprintln!(
        "overlay plan: {} blocked conflicts, {} unsupported proof gap(s), {} heuristic proof gap(s)",
        plan.blocked_conflicts.len(),
        plan.proof.unsupported_site_count,
        plan.proof.heuristic_site_count
    );
    Ok(())
}

impl ConfigSemanticIndex {
    fn new(semantic_root: &Path) -> Result<Self> {
        Ok(Self {
            ref_graph: RefGraphRepository::new(semantic_root)?,
            dependency_edges_sample: Vec::new(),
            edge_sample_overflow: 0,
            missing_refs_kinds_logged: HashSet::new(),
            partial_refs_kinds_logged: HashSet::new(),
        })
    }

    fn record_dependency_edge(&mut self, edge: DependencyEdgeSample) {
        if self.dependency_edges_sample.len() < MAX_EDGE_SAMPLES {
            self.dependency_edges_sample.push(edge);
        } else {
            self.edge_sample_overflow += 1;
        }
    }

    fn edge_sample_warning(&self) -> Option<OverlayWarning> {
        if self.edge_sample_overflow == 0 {
            return None;
        }
        Some(OverlayWarning {
            kind: "risk".to_string(),
            archive: None,
            id: None,
            ref_kind: None,
            message: format!(
                "{} additional dependency edge(s) omitted from plan sample after first {}.",
                self.edge_sample_overflow, MAX_EDGE_SAMPLES
            ),
        })
    }
}

impl RefGraphRepository {
    fn new(semantic_root: &Path) -> Result<Self> {
        let refs_dir = semantic_root.join("refs");
        let parsed = [
            "obj", "npc", "loc", "spot", "seq", "bas", "enum", "struct", "dbtable", "dbrow",
            "varbit", "varp", "param", "seqgroup",
        ]
        .par_iter()
        .map(|kind| -> Result<Option<(String, SemanticRefBuckets)>> {
            let file_path = refs_dir.join(format!("{kind}.json"));
            if !file_path.is_file() {
                return Ok(None);
            }
            let bytes =
                fs::read(&file_path).with_context(|| format!("reading {}", file_path.display()))?;
            let mut by_id = HashMap::new();
            if *kind == "varp" {
                let parsed: HashMap<String, HashMap<u32, HashMap<String, Vec<u32>>>> =
                    serde_json::from_slice(&bytes)
                        .with_context(|| format!("decoding {}", file_path.display()))?;
                for (_, domain_entries) in parsed {
                    for (id, edges) in domain_entries {
                        let mapped = by_id.entry(id).or_insert_with(HashMap::new);
                        for (ref_kind, ids) in edges {
                            if ids.is_empty() {
                                continue;
                            }
                            mapped
                                .entry(SemanticRefKey::from_label(&ref_kind))
                                .or_insert_with(Vec::new)
                                .extend(ids);
                        }
                    }
                }
            } else {
                let parsed: HashMap<u32, HashMap<String, Vec<u32>>> =
                    serde_json::from_slice(&bytes)
                        .with_context(|| format!("decoding {}", file_path.display()))?;
                for (id, edges) in parsed {
                    let mut mapped = HashMap::new();
                    for (ref_kind, ids) in edges {
                        if !ids.is_empty() {
                            mapped.insert(SemanticRefKey::from_label(&ref_kind), ids);
                        }
                    }
                    by_id.insert(id, mapped);
                }
            }
            if by_id.is_empty() {
                return Ok(None);
            }
            Ok(Some(((*kind).to_string(), by_id)))
        })
        .collect::<Vec<_>>();
        let mut graphs = HashMap::new();
        for result in parsed {
            if let Some((kind, graph)) = result? {
                graphs.insert(kind, graph);
            }
        }
        Ok(Self { graphs })
    }

    fn has_kind(&self, kind: &str) -> bool {
        self.graphs.contains_key(kind)
    }

    fn get_refs(&self, kind: &str, id: u32) -> Option<&HashMap<SemanticRefKey, Vec<u32>>> {
        self.graphs.get(kind)?.get(&id)
    }

    fn get_dbrow_table_id(&self, row_id: u32) -> Option<u32> {
        self.get_refs("dbrow", row_id)?
            .get(&SemanticRefKey::DbTable)?
            .first()
            .copied()
    }
}

impl PlanBuilder<'_> {
    fn queue(&mut self, kind: RefKind, id: u32, source: impl Into<String>, mode: SelectionMode) {
        if self.seen_refs.insert((kind, id)) {
            self.pending.push_back(PendingRef {
                kind,
                id,
                source: source.into(),
                mode,
            });
        }
    }

    fn add_dependency(&mut self, kind: RefKind, id: u32) {
        self.dependencies.entry(kind).or_default().insert(id);
    }

    fn add_group(&mut self, archive: &ArchiveDef, group_id: u32) {
        self.group_selections
            .entry(archive.id)
            .or_default()
            .insert(group_id);
    }

    fn add_file(&mut self, archive: &ArchiveDef, group_id: u32, file_id: u32) {
        self.file_selections
            .entry((archive.id, group_id))
            .or_default()
            .insert(file_id);
    }

    fn add_warning(&mut self, warning: OverlayWarning) {
        if self.warnings.iter().any(|existing| {
            existing.kind == warning.kind
                && existing.archive == warning.archive
                && existing.id == warning.id
                && existing.ref_kind == warning.ref_kind
                && existing.message == warning.message
        }) {
            return;
        }
        if self.warnings.len() >= MAX_PLAN_WARNINGS {
            self.warning_overflow += 1;
            return;
        }
        self.warnings.push(warning);
    }

    fn add_blocked(&mut self, issue: OverlayBlockedIssue) {
        if self.blocked.iter().any(|existing| {
            existing.kind == issue.kind
                && existing.archive_id == issue.archive_id
                && existing.group_id == issue.group_id
                && existing.file_id == issue.file_id
                && existing.id == issue.id
                && existing.message == issue.message
        }) {
            return;
        }
        self.blocked.push(issue);
    }

    fn semantic_donor_root(&self) -> &Path {
        Path::new(&self.roots.donor_semantic_root)
    }

    fn get_index(
        &mut self,
        root_kind: RootKind,
        archive: &ArchiveDef,
    ) -> Result<Option<ArchiveIndex>> {
        if let Some(index) = self.indexes.get(&(root_kind, archive.id)) {
            return Ok(index.clone());
        }
        let cache = match root_kind {
            RootKind::Base => &self.base_cache,
            RootKind::Donor => &self.donor_cache,
        };
        let archive_id = match root_kind {
            RootKind::Donor => archive.donor_id,
            RootKind::Base => archive.id,
        };
        let index = if cache.get(255, archive_id)?.is_some() {
            Some(cache.archive_index(archive_id)?)
        } else {
            None
        };
        self.indexes.insert((root_kind, archive.id), index.clone());
        Ok(index)
    }

    fn read_raw_group(
        &self,
        root_kind: RootKind,
        archive: &ArchiveDef,
        group_id: u32,
    ) -> Result<Option<Vec<u8>>> {
        let archive_id = match root_kind {
            RootKind::Donor => archive.donor_id,
            RootKind::Base => archive.id,
        };
        let cache = match root_kind {
            RootKind::Base => &self.base_cache,
            RootKind::Donor => &self.donor_cache,
        };
        Ok(cache.get(archive_id, group_id)?)
    }

    fn read_group_files(
        &mut self,
        root_kind: RootKind,
        archive: &ArchiveDef,
        group_id: u32,
    ) -> Result<Option<&BTreeMap<u32, Vec<u8>>>> {
        let key = (root_kind, archive.id, group_id);
        if self.config_group_files.contains_key(&key) {
            return Ok(self.config_group_files.get(&key).and_then(Option::as_ref));
        }
        let files = match self.get_index(root_kind, archive)? {
            Some(index) if is_group_present(&index, group_id) => {
                let cache = match root_kind {
                    RootKind::Base => &self.base_cache,
                    RootKind::Donor => &self.donor_cache,
                };
                let archive_id = match root_kind {
                    RootKind::Donor => archive.donor_id,
                    RootKind::Base => archive.id,
                };
                Some(cache.group_files_with_index(&index, archive_id, group_id)?)
            }
            _ => None,
        };
        self.config_group_files.insert(key, files);
        Ok(self.config_group_files.get(&key).and_then(Option::as_ref))
    }

    fn cached_file_bytes(
        &self,
        root_kind: RootKind,
        archive: &ArchiveDef,
        group_id: u32,
        file_id: u32,
    ) -> Option<&[u8]> {
        self.config_group_files
            .get(&(root_kind, archive.id, group_id))
            .and_then(Option::as_ref)
            .and_then(|files| files.get(&file_id))
            .map(Vec::as_slice)
    }

    fn read_file_bytes(
        &mut self,
        root_kind: RootKind,
        archive: &ArchiveDef,
        group_id: u32,
        file_id: u32,
    ) -> Result<Option<&[u8]>> {
        let Some(files) = self.read_group_files(root_kind, archive, group_id)? else {
            return Ok(None);
        };
        Ok(files.get(&file_id).map(Vec::as_slice))
    }

    fn analyzer(&mut self) -> Result<&MigrationAnalyzer> {
        if self.analyzer.is_none() {
            let donor_tar = default_tar_path();
            let base_tar = default_tar_path();
            let donor_ctx = ResolverContext::load_lazy(
                &self.donor_cache,
                &donor_tar,
                self.data_dir,
                self.donor_build,
                self.donor_subbuild,
            )?;
            let base_ctx = ResolverContext::load_lazy(
                &self.base_cache,
                &base_tar,
                self.data_dir,
                self.base_build,
                self.base_subbuild,
            )?;
            self.analyzer = Some(MigrationAnalyzer::new(donor_ctx, base_ctx));
        }
        Ok(self.analyzer.as_ref().expect("analyzer"))
    }
}

fn seed_imports(builder: &mut PlanBuilder<'_>) -> Result<()> {
    if builder.manifest.imports.map_archive.as_deref() == Some("full") {
        builder.full_archive_selections.insert(archive_maps().id);
        let donor_index = builder
            .get_index(RootKind::Donor, &archive_maps())?
            .with_context(|| {
                format!(
                    "Donor maps archive index missing: {}",
                    builder.roots.donor_raw_root
                )
            })?;
        for group_id in donor_index.group_id {
            builder.primary_maps.insert(group_id);
        }
    }
    for archive_ref in builder.manifest.imports.full_archives.clone() {
        seed_full_archive_selection(builder, &archive_ref)?;
    }
    let config_groups = builder.manifest.imports.config_groups.clone();
    if !config_groups.is_empty() {
        let donor_index = builder.get_index(RootKind::Donor, &archive_config())?;
        for group_id in config_groups {
            if let Some(index) = donor_index.as_ref()
                && !is_group_present(index, group_id)
            {
                builder.add_warning(OverlayWarning {
                    kind: "noop".to_string(),
                    archive: Some(archive_config().name.to_string()),
                    id: Some(group_id),
                    ref_kind: None,
                    message: format!(
                        "Donor config group {group_id} is empty; skipping map visual config import."
                    ),
                });
                continue;
            }
            builder.add_group(&archive_config(), group_id);
        }
    }
    for group_id in builder.manifest.imports.maps.clone() {
        builder.primary_maps.insert(group_id);
    }
    for region in builder.manifest.imports.regions.clone() {
        builder.primary_maps.insert(normalize_region(region)?);
    }
    for id in builder.manifest.imports.objs.clone() {
        builder.primary_objs.insert(id);
        builder.queue(RefKind::Obj, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.npcs.clone() {
        builder.primary_npcs.insert(id);
        builder.queue(RefKind::Npc, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.locs.clone() {
        builder.primary_locs.insert(id);
        builder.queue(RefKind::Loc, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.seqs.clone() {
        builder.queue(RefKind::Seq, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.bas.clone() {
        builder.queue(RefKind::Bas, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.structs.clone() {
        builder.primary_structs.insert(id);
        builder.queue(RefKind::Struct, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.quests.clone() {
        builder.queue(RefKind::Quest, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.enums.clone() {
        builder.primary_enums.insert(id);
        builder.queue(RefKind::Enum, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.varbits.clone() {
        builder.primary_varbits.insert(id);
        builder.queue(RefKind::VarBit, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.varps.clone() {
        builder.primary_varps.insert(id);
        builder.queue(RefKind::Varp, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.db_tables.clone() {
        builder.primary_db_tables.insert(id);
        builder.queue(RefKind::DbTable, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.db_rows.clone() {
        builder.primary_db_rows.insert(id);
        builder.queue(RefKind::DbRow, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.interfaces.clone() {
        builder.primary_interfaces.insert(id);
        builder.queue(RefKind::Interface, id, "manifest", SelectionMode::Primary);
    }
    for id in builder.manifest.imports.scripts.clone() {
        builder.primary_scripts.insert(id);
        builder.queue(RefKind::Script, id, "manifest", SelectionMode::Primary);
    }
    Ok(())
}

fn seed_full_archive_selection(
    builder: &mut PlanBuilder<'_>,
    archive_ref: &ArchiveRef,
) -> Result<()> {
    let archive = archive_for_manifest_ref(archive_ref)?;
    let donor_index = builder
        .get_index(RootKind::Donor, &archive)?
        .with_context(|| {
            format!(
                "Donor {} archive index missing: {}",
                archive.name, builder.roots.donor_raw_root
            )
        })?;
    builder.full_archive_selections.insert(archive.id);
    for group_id in donor_index.group_id {
        builder.add_group(&archive, group_id);
    }
    Ok(())
}

fn build_selections(builder: &mut PlanBuilder<'_>, allow_heuristic_sites: bool) -> Result<()> {
    let map_groups = builder.primary_maps.iter().copied().collect::<Vec<_>>();
    for group_id in map_groups {
        process_map_group(builder, group_id)?;
    }
    while let Some(reference) = builder.pending.pop_front() {
        process_ref(builder, &reference, allow_heuristic_sites)?;
    }
    Ok(())
}

fn process_ref(
    builder: &mut PlanBuilder<'_>,
    reference: &PendingRef,
    allow_heuristic_sites: bool,
) -> Result<()> {
    builder.add_dependency(reference.kind, reference.id);
    if ref_covered_by_full_archive(builder, reference)? {
        process_covered_ref_dependencies(builder, reference)?;
        return Ok(());
    }

    match reference.kind {
        RefKind::Loc => {
            let target = config_target(RefKind::Loc, reference.id);
            process_config_ref(builder, reference, &target, "loc")?;
        }
        RefKind::Npc => {
            let target = config_target(RefKind::Npc, reference.id);
            process_config_ref(builder, reference, &target, "npc")?;
        }
        RefKind::Obj => {
            let target = config_target(RefKind::Obj, reference.id);
            process_config_ref(builder, reference, &target, "obj")?;
        }
        RefKind::Seq => {
            let target = config_target(RefKind::Seq, reference.id);
            process_config_ref(builder, reference, &target, "seq")?;
        }
        RefKind::Bas => {
            let target = config_target(RefKind::Bas, reference.id);
            process_config_ref(builder, reference, &target, "bas")?;
        }
        RefKind::Spot => {
            let target = config_target(RefKind::Spot, reference.id);
            process_config_ref(builder, reference, &target, "spotanim")?;
        }
        RefKind::Struct => {
            let target = config_target(RefKind::Struct, reference.id);
            process_struct_ref(builder, reference, &target)?;
        }
        RefKind::Enum => {
            let allow_ids = builder.manifest.allow.enum_ids.clone();
            let target = config_target(RefKind::Enum, reference.id);
            process_gated_config_ref(builder, reference, &target, "enum", &allow_ids, &allow_ids)?;
        }
        RefKind::VarBit => {
            let allow_ids = builder.manifest.allow.varbit_ids.clone();
            let conflict_allow_ids = builder.manifest.allow.varbit_conflict_ids.clone();
            let target = config_target(RefKind::VarBit, reference.id);
            process_gated_config_ref(
                builder,
                reference,
                &target,
                "varbit",
                &allow_ids,
                &conflict_allow_ids,
            )?;
        }
        RefKind::Varp => {
            let allow_ids = builder.manifest.allow.varp_ids.clone();
            let conflict_allow_ids = builder.manifest.allow.varp_conflict_ids.clone();
            let target = config_target(RefKind::Varp, reference.id);
            process_gated_config_ref(
                builder,
                reference,
                &target,
                "varp",
                &allow_ids,
                &conflict_allow_ids,
            )?;
        }
        RefKind::Quest => {
            let target = config_target(RefKind::Quest, reference.id);
            process_shallow_config_ref(builder, &target)?;
        }
        RefKind::DbTable => process_dbtable_ref(builder, reference.id)?,
        RefKind::DbRow => process_dbrow_ref(builder, reference.id)?,
        RefKind::Model => {
            let target = resolve_model_target(builder, reference.id);
            process_raw_group_ref(builder, &target)?;
        }
        RefKind::Anim => process_raw_group_ref(
            builder,
            &RawGroupTarget {
                archive: archive_anims_rt7(),
                group_id: reference.id,
                id: reference.id,
                kind: reference.kind,
            },
        )?,
        RefKind::Material => process_config_ref(
            builder,
            reference,
            &ConfigTarget {
                archive: archive_materials(),
                group_id: CONFIG_GROUP_MATERIAL_ARCHIVE26,
                file_id: reference.id,
                id: reference.id,
                kind: RefKind::Material,
            },
            "material",
        )?,
        RefKind::Sprite => process_raw_group_ref(
            builder,
            &RawGroupTarget {
                archive: archive_sprites(),
                group_id: reference.id,
                id: reference.id,
                kind: RefKind::Sprite,
            },
        )?,
        RefKind::SeqGroup => process_config_ref(
            builder,
            reference,
            &ConfigTarget {
                archive: archive_config(),
                group_id: CONFIG_GROUP_SEQGROUP,
                file_id: reference.id,
                id: reference.id,
                kind: RefKind::SeqGroup,
            },
            "seqgroup",
        )?,
        RefKind::Interface => prove_interface_ref(builder, reference.id, allow_heuristic_sites)?,
        RefKind::Script => prove_script_ref(builder, reference.id, allow_heuristic_sites)?,
    }
    Ok(())
}

fn process_covered_ref_dependencies(
    builder: &mut PlanBuilder<'_>,
    reference: &PendingRef,
) -> Result<()> {
    if reference.mode != SelectionMode::Dependency {
        return Ok(());
    }
    match reference.kind {
        RefKind::Loc => scan_config_dependencies(builder, "loc", reference.id, &reference.source),
        RefKind::Npc => scan_config_dependencies(builder, "npc", reference.id, &reference.source),
        _ => Ok(()),
    }
}

fn ref_covered_by_full_archive(builder: &PlanBuilder<'_>, reference: &PendingRef) -> Result<bool> {
    Ok(full_archive_for_ref(builder, reference)?
        .is_some_and(|archive| builder.full_archive_selections.contains(&archive.id)))
}

fn full_archive_for_ref(
    builder: &PlanBuilder<'_>,
    reference: &PendingRef,
) -> Result<Option<ArchiveDef>> {
    Ok(Some(match reference.kind {
        RefKind::Loc => archive_loc_config(),
        RefKind::Npc => archive_npc_config(),
        RefKind::Obj => archive_obj_config(),
        RefKind::Model => resolve_model_target(builder, reference.id).archive,
        RefKind::Material => archive_materials(),
        RefKind::Anim => archive_anims_rt7(),
        _ => return Ok(None),
    }))
}

fn process_map_group(builder: &mut PlanBuilder<'_>, group_id: u32) -> Result<()> {
    let archive = archive_maps();
    let donor_index = builder
        .get_index(RootKind::Donor, &archive)?
        .with_context(|| {
            format!(
                "Donor map archive index missing under {}",
                builder.roots.donor_raw_root
            )
        })?;
    if !is_group_present(&donor_index, group_id) {
        builder.add_blocked(OverlayBlockedIssue {
            kind: "missing".to_string(),
            archive: Some(archive.name.to_string()),
            archive_id: Some(archive.id),
            group_id: Some(group_id),
            file_id: None,
            id: None,
            ref_kind: None,
            message: format!("Donor map group {group_id} missing."),
        });
        return Ok(());
    }
    builder.add_group(&archive, group_id);
    let files = builder
        .donor_cache
        .group_files_with_index(&donor_index, archive.id, group_id)
        .with_context(|| format!("reading donor map group {group_id}"))?;
    if let Some(loc_file) = files.get(&1) {
        for loc_id in parse_loc_ids(loc_file)? {
            builder.queue(
                RefKind::Loc,
                loc_id,
                format!("map_{group_id}"),
                SelectionMode::Dependency,
            );
        }
    }
    if let Some(npc_file) = files.get(&2) {
        for npc_id in parse_npc_ids(npc_file)? {
            builder.queue(
                RefKind::Npc,
                npc_id,
                format!("map_{group_id}"),
                SelectionMode::Dependency,
            );
        }
    }
    Ok(())
}

fn process_config_ref(
    builder: &mut PlanBuilder<'_>,
    reference: &PendingRef,
    target: &ConfigTarget,
    semantic_kind: &str,
) -> Result<()> {
    let state = compare_file(builder, &target.archive, target.group_id, target.file_id)?;
    if state == CompareState::MissingDonor {
        missing_config(builder, target);
        return Ok(());
    }
    if !validate_donor_config_decodes(builder, target)? {
        return Ok(());
    }
    match state {
        CompareState::Conflict => {
            builder.add_warning(OverlayWarning {
                kind: "risk".to_string(),
                archive: Some(target.archive.name.to_string()),
                id: Some(target.id),
                ref_kind: Some(target.kind.as_str().to_string()),
                message: format!(
                    "{}_{} overrides differing 910 bytes with selected 939 bytes.",
                    target.kind.as_str(),
                    target.id
                ),
            });
            builder.add_file(&target.archive, target.group_id, target.file_id);
        }
        CompareState::MissingTarget => {
            builder.add_file(&target.archive, target.group_id, target.file_id);
        }
        CompareState::Same | CompareState::MissingDonor => {}
    }
    scan_config_dependencies(builder, semantic_kind, target.id, &reference.source)
}

fn process_shallow_config_ref(builder: &mut PlanBuilder<'_>, target: &ConfigTarget) -> Result<()> {
    let state = compare_file(builder, &target.archive, target.group_id, target.file_id)?;
    if state == CompareState::MissingDonor {
        missing_config(builder, target);
        return Ok(());
    }
    if !validate_donor_config_decodes(builder, target)? {
        return Ok(());
    }
    match state {
        CompareState::Conflict => {
            builder.add_warning(OverlayWarning {
                kind: "risk".to_string(),
                archive: Some(target.archive.name.to_string()),
                id: Some(target.id),
                ref_kind: Some(target.kind.as_str().to_string()),
                message: format!(
                    "{}_{} overrides differing 910 bytes with selected 939 bytes.",
                    target.kind.as_str(),
                    target.id
                ),
            });
            builder.add_file(&target.archive, target.group_id, target.file_id);
        }
        CompareState::MissingTarget => {
            builder.add_file(&target.archive, target.group_id, target.file_id);
        }
        CompareState::Same | CompareState::MissingDonor => {}
    }
    Ok(())
}

fn process_gated_config_ref(
    builder: &mut PlanBuilder<'_>,
    reference: &PendingRef,
    target: &ConfigTarget,
    semantic_kind: &str,
    allow_ids: &[u32],
    conflict_allow_ids: &[u32],
) -> Result<()> {
    let state = compare_file(builder, &target.archive, target.group_id, target.file_id)?;
    if state == CompareState::MissingDonor {
        missing_config(builder, target);
        return Ok(());
    }
    if !validate_donor_config_decodes(builder, target)? {
        return Ok(());
    }
    if state == CompareState::Same {
        queue_varbit_base_varp_dependency(builder, target, &reference.source)?;
        return scan_config_dependencies(builder, semantic_kind, target.id, &reference.source);
    }

    let allowed_missing_target = allowed_missing_target_ids(builder, target.kind, allow_ids);
    if state == CompareState::MissingTarget {
        if !allowed_missing_target.contains(&target.id) {
            builder.add_blocked(OverlayBlockedIssue {
                kind: "blocked-ref".to_string(),
                archive: Some(target.archive.name.to_string()),
                archive_id: Some(target.archive.id),
                group_id: Some(target.group_id),
                file_id: Some(target.file_id),
                id: Some(target.id),
                ref_kind: Some(target.kind.as_str().to_string()),
                message: format!(
                    "{}_{} is missing in target but {} imports require explicit allowlist.",
                    target.kind.as_str(),
                    target.id,
                    target.kind.as_str()
                ),
            });
            return Ok(());
        }
        builder.add_file(&target.archive, target.group_id, target.file_id);
        queue_varbit_base_varp_dependency(builder, target, &reference.source)?;
        return scan_config_dependencies(builder, semantic_kind, target.id, &reference.source);
    }

    if !conflict_allow_ids.contains(&target.id) {
        builder.add_blocked(OverlayBlockedIssue {
            kind: "conflict".to_string(),
            archive: Some(target.archive.name.to_string()),
            archive_id: Some(target.archive.id),
            group_id: Some(target.group_id),
            file_id: Some(target.file_id),
            id: Some(target.id),
            ref_kind: Some(target.kind.as_str().to_string()),
            message: format!(
                "{}_{} differs in target and requires explicit allowlist.",
                target.kind.as_str(),
                target.id
            ),
        });
        return Ok(());
    }
    builder.add_file(&target.archive, target.group_id, target.file_id);
    queue_varbit_base_varp_dependency(builder, target, &reference.source)?;
    scan_config_dependencies(builder, semantic_kind, target.id, &reference.source)
}

fn process_struct_ref(
    builder: &mut PlanBuilder<'_>,
    reference: &PendingRef,
    target: &ConfigTarget,
) -> Result<()> {
    let state = compare_file(builder, &target.archive, target.group_id, target.file_id)?;
    if state == CompareState::MissingDonor {
        missing_config(builder, target);
        return Ok(());
    }
    if !validate_donor_config_decodes(builder, target)? {
        return Ok(());
    }
    if state == CompareState::Same {
        scan_config_dependencies(builder, "struct", target.id, &reference.source)?;
    }
    Ok(())
}

fn process_raw_group_ref(builder: &mut PlanBuilder<'_>, target: &RawGroupTarget) -> Result<()> {
    let Some(donor_raw) =
        builder.read_raw_group(RootKind::Donor, &target.archive, target.group_id)?
    else {
        builder.add_blocked(OverlayBlockedIssue {
            kind: "missing".to_string(),
            archive: Some(target.archive.name.to_string()),
            archive_id: Some(target.archive.id),
            group_id: Some(target.group_id),
            file_id: None,
            id: Some(target.id),
            ref_kind: Some(target.kind.as_str().to_string()),
            message: format!(
                "Donor {}_{} group missing.",
                target.kind.as_str(),
                target.id
            ),
        });
        return Ok(());
    };
    scan_raw_group_dependencies(builder, target, &donor_raw)?;
    let base_raw = builder.read_raw_group(RootKind::Base, &target.archive, target.group_id)?;
    match base_raw {
        Some(base_raw) if base_raw != donor_raw => {
            builder.add_warning(OverlayWarning {
                kind: "risk".to_string(),
                archive: Some(target.archive.name.to_string()),
                id: Some(target.id),
                ref_kind: Some(target.kind.as_str().to_string()),
                message: format!(
                    "{}_{} overrides differing 910 {} group with selected 939 bytes.",
                    target.kind.as_str(),
                    target.id,
                    target.archive.name
                ),
            });
            builder.add_group(&target.archive, target.group_id);
        }
        None => builder.add_group(&target.archive, target.group_id),
        _ => {}
    }
    Ok(())
}

fn scan_raw_group_dependencies(
    builder: &mut PlanBuilder<'_>,
    target: &RawGroupTarget,
    donor_raw: &[u8],
) -> Result<()> {
    if target.archive.id != archive_models_rt7().id {
        return Ok(());
    }
    let Some(donor_index) = builder.get_index(RootKind::Donor, &target.archive)? else {
        builder.add_blocked(OverlayBlockedIssue {
            kind: "missing".to_string(),
            archive: Some(target.archive.name.to_string()),
            archive_id: Some(target.archive.id),
            group_id: Some(target.group_id),
            file_id: None,
            id: Some(target.id),
            ref_kind: Some(target.kind.as_str().to_string()),
            message: format!(
                "Donor {}_{} index missing.",
                target.kind.as_str(),
                target.id
            ),
        });
        return Ok(());
    };
    let files = builder.donor_cache.group_files_with_index(
        &donor_index,
        target.archive.donor_id,
        target.group_id,
    )?;
    for file in files.values() {
        match scan_rt7_model_material_ids(file) {
            Ok(material_ids) => {
                for material_id in material_ids {
                    builder.queue(
                        RefKind::Material,
                        material_id,
                        format!("{}_{}", target.kind.as_str(), target.id),
                        SelectionMode::Dependency,
                    );
                }
            }
            Err(error) => {
                builder.add_blocked(OverlayBlockedIssue {
                    kind: "blocked-ref".to_string(),
                    archive: Some(target.archive.name.to_string()),
                    archive_id: Some(target.archive.id),
                    group_id: Some(target.group_id),
                    file_id: None,
                    id: Some(target.id),
                    ref_kind: Some(target.kind.as_str().to_string()),
                    message: format!(
                        "Donor {}_{} failed RT7 material scan: {error}",
                        target.kind.as_str(),
                        target.id
                    ),
                });
                return Ok(());
            }
        }
    }
    let _ = donor_raw;
    Ok(())
}

fn process_dbtable_ref(builder: &mut PlanBuilder<'_>, table_id: u32) -> Result<()> {
    let target = config_target(RefKind::DbTable, table_id);
    let state = compare_file(builder, &target.archive, target.group_id, target.file_id)?;
    if state == CompareState::MissingDonor {
        missing_config(builder, &target);
        return Ok(());
    }
    if !validate_donor_config_decodes(builder, &target)? {
        return Ok(());
    }
    let allow_schema = builder
        .manifest
        .allow
        .db_table_schema_changes
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    if state == CompareState::Conflict && !allow_schema.contains(&table_id) {
        builder.db_schema_changes.insert(table_id);
        builder.add_blocked(OverlayBlockedIssue {
            kind: "db-schema".to_string(),
            archive: Some(target.archive.name.to_string()),
            archive_id: Some(target.archive.id),
            group_id: Some(target.group_id),
            file_id: Some(target.file_id),
            id: Some(table_id),
            ref_kind: Some("dbtable".to_string()),
            message: format!(
                "dbtable_{table_id} schema differs from 910; selective DB import requires explicit schema allowlist."
            ),
        });
        return Ok(());
    }
    if matches!(state, CompareState::MissingTarget | CompareState::Conflict) {
        builder.add_file(&target.archive, target.group_id, target.file_id);
    }
    process_raw_group_ref(
        builder,
        &RawGroupTarget {
            archive: archive_dbtable_index(),
            group_id: table_id,
            id: table_id,
            kind: RefKind::DbTable,
        },
    )
}

fn process_dbrow_ref(builder: &mut PlanBuilder<'_>, row_id: u32) -> Result<()> {
    let target = config_target(RefKind::DbRow, row_id);
    let state = compare_file(builder, &target.archive, target.group_id, target.file_id)?;
    if state == CompareState::MissingDonor {
        missing_config(builder, &target);
        return Ok(());
    }
    if !validate_donor_config_decodes(builder, &target)? {
        return Ok(());
    }
    match state {
        CompareState::Conflict => {
            builder.add_warning(OverlayWarning {
                kind: "risk".to_string(),
                archive: Some(target.archive.name.to_string()),
                id: Some(row_id),
                ref_kind: Some("dbrow".to_string()),
                message: format!(
                    "dbrow_{row_id} overrides differing 910 bytes with selected 939 bytes."
                ),
            });
            builder.add_file(&target.archive, target.group_id, target.file_id);
        }
        CompareState::MissingTarget => {
            builder.add_file(&target.archive, target.group_id, target.file_id);
        }
        CompareState::Same | CompareState::MissingDonor => {}
    }
    scan_dbrow_dependencies(builder, row_id);
    Ok(())
}

fn scan_dbrow_dependencies(builder: &mut PlanBuilder<'_>, row_id: u32) {
    let table_id = builder
        .semantic_index
        .ref_graph
        .get_dbrow_table_id(row_id)
        .unwrap_or(0);
    let allowed = db_row_dependency_kinds(table_id);
    scan_refs_for_kind(
        builder,
        "dbrow",
        row_id,
        &format!("dbrow_{row_id}"),
        Some(&allowed),
    );
}

fn scan_config_dependencies(
    builder: &mut PlanBuilder<'_>,
    semantic_kind: &str,
    id: u32,
    source: &str,
) -> Result<()> {
    match semantic_kind {
        "obj" => {
            scan_refs_for_kind(builder, semantic_kind, id, source, None);
            Ok(())
        }
        "loc" => {
            let has_semantic_refs = builder
                .semantic_index
                .ref_graph
                .get_refs(semantic_kind, id)
                .is_some();
            scan_refs_for_kind(builder, semantic_kind, id, source, None);
            if !has_semantic_refs {
                let _ = scan_binary_multivar_dependencies(builder, "loc", id, source)?;
            }
            Ok(())
        }
        "npc" => {
            let has_semantic_refs = builder
                .semantic_index
                .ref_graph
                .get_refs(semantic_kind, id)
                .is_some();
            scan_refs_for_kind(builder, semantic_kind, id, source, None);
            if !has_semantic_refs {
                let _ = scan_binary_multivar_dependencies(builder, "npc", id, source)?;
            }
            Ok(())
        }
        _ => {
            scan_refs_for_kind(builder, semantic_kind, id, source, None);
            Ok(())
        }
    }
}

fn scan_refs_for_kind(
    builder: &mut PlanBuilder<'_>,
    semantic_kind: &str,
    id: u32,
    source: &str,
    allowed_kinds: Option<&HashSet<RefKind>>,
) {
    let mut queued = Vec::new();
    if let Some(refs) = builder.semantic_index.ref_graph.get_refs(semantic_kind, id) {
        let self_kind = semantic_kind_to_ref_kind(semantic_kind);
        let mut ref_entries = refs.iter().collect::<Vec<_>>();
        ref_entries.sort_by(|(left, _), (right, _)| left.as_label().cmp(right.as_label()));
        for (ref_kind, ref_ids) in ref_entries {
            if matches!(ref_kind, SemanticRefKey::Param) {
                continue;
            }
            let Some(kind) = normalize_graph_ref_kind(ref_kind) else {
                continue;
            };
            if !semantic_kind_allows_ref_kind(semantic_kind, kind) {
                continue;
            }
            if let Some(allowed) = allowed_kinds
                && !allowed.contains(&kind)
            {
                continue;
            }
            if kind == RefKind::Material && ref_ids.contains(&65535) {
                continue;
            }
            let is_multivar = matches!(
                ref_kind,
                SemanticRefKey::MultivarVarbit | SemanticRefKey::MultivarVarp
            );
            for &ref_id in ref_ids {
                if kind == self_kind && ref_id == id {
                    continue;
                }
                queued.push((kind, ref_id, is_multivar));
            }
        }
        let queued_source = format!("{source}->{semantic_kind}_{id}");
        let from = format!("{semantic_kind}_{id}");
        for (kind, ref_id, is_multivar) in queued {
            if is_multivar {
                match kind {
                    RefKind::VarBit => {
                        builder.auto_allowed_missing_varbits.insert(ref_id);
                    }
                    RefKind::Varp => {
                        builder.auto_allowed_missing_varps.insert(ref_id);
                    }
                    _ => {}
                }
            }
            builder.queue(
                kind,
                ref_id,
                queued_source.clone(),
                SelectionMode::Dependency,
            );
            builder
                .semantic_index
                .record_dependency_edge(DependencyEdgeSample {
                    from: from.clone(),
                    to: format!("{}_{}", kind.as_str(), ref_id),
                    kind: kind.as_str().to_string(),
                    reason: "refs",
                });
        }
        return;
    }

    if builder.semantic_index.ref_graph.has_kind(semantic_kind) {
        if builder
            .semantic_index
            .partial_refs_kinds_logged
            .insert(semantic_kind.to_string())
        {
            builder.add_warning(OverlayWarning {
                kind: "risk".to_string(),
                archive: None,
                id: None,
                ref_kind: Some(semantic_kind_to_ref_kind(semantic_kind).as_str().to_string()),
                message: format!(
                    "refs/{}.json has no entry for {}_{}; falling back to donor binary dependency scan where supported.",
                    semantic_refs_file_name(semantic_kind),
                    semantic_kind,
                    id
                ),
            });
        }
        return;
    }

    if semantic_kind == "seqgroup" {
        return;
    }

    if builder
        .semantic_index
        .missing_refs_kinds_logged
        .insert(semantic_kind.to_string())
    {
        builder.add_warning(OverlayWarning {
            kind: "risk".to_string(),
            archive: None,
            id: None,
            ref_kind: Some(
                semantic_kind_to_ref_kind(semantic_kind)
                    .as_str()
                    .to_string(),
            ),
            message: format!(
                "refs/{}.json missing under semantic root; run cache:semantic:sync-947.",
                semantic_refs_file_name(semantic_kind)
            ),
        });
    }
}

fn semantic_kind_allows_ref_kind(semantic_kind: &str, kind: RefKind) -> bool {
    match semantic_kind {
        "obj" => matches!(
            kind,
            RefKind::Model | RefKind::Obj | RefKind::Material | RefKind::Quest
        ),
        "loc" => matches!(
            kind,
            RefKind::Model
                | RefKind::Loc
                | RefKind::VarBit
                | RefKind::Varp
                | RefKind::Seq
                | RefKind::Bas
                | RefKind::Spot
                | RefKind::Material
                | RefKind::Quest
        ),
        "npc" => matches!(
            kind,
            RefKind::Model
                | RefKind::Npc
                | RefKind::VarBit
                | RefKind::Varp
                | RefKind::Seq
                | RefKind::Bas
                | RefKind::Spot
                | RefKind::Quest
        ),
        _ => true,
    }
}

fn scan_binary_multivar_dependencies(
    builder: &mut PlanBuilder<'_>,
    kind: &str,
    id: u32,
    source: &str,
) -> Result<bool> {
    let target = config_target(
        if kind == "loc" {
            RefKind::Loc
        } else {
            RefKind::Npc
        },
        id,
    );
    let Some(donor) = builder.read_file_bytes(
        RootKind::Donor,
        &target.archive,
        target.group_id,
        target.file_id,
    )?
    else {
        return Ok(false);
    };
    let ops = if kind == "loc" {
        parse_loc(id, donor)?.ops
    } else {
        parse_npc(id, donor)?.ops
    };
    let refs = scan_multivar_refs(&ops);
    let from = format!("{kind}_{id}");
    let scanned_varbit = if let Some(varbit) = refs.varbit {
        builder.auto_allowed_missing_varbits.insert(varbit);
        builder.queue(
            RefKind::VarBit,
            varbit,
            format!("{source}->{from}"),
            SelectionMode::Dependency,
        );
        builder
            .semantic_index
            .record_dependency_edge(DependencyEdgeSample {
                from: from.clone(),
                to: format!("varbit_{varbit}"),
                kind: "varbit".to_string(),
                reason: "binary",
            });
        true
    } else {
        false
    };
    let scanned_varp = if let Some(varp) = refs.varp {
        builder.auto_allowed_missing_varps.insert(varp);
        builder.queue(
            RefKind::Varp,
            varp,
            format!("{source}->{from}"),
            SelectionMode::Dependency,
        );
        builder
            .semantic_index
            .record_dependency_edge(DependencyEdgeSample {
                from,
                to: format!("varp_{varp}"),
                kind: "varp".to_string(),
                reason: "binary",
            });
        true
    } else {
        false
    };
    Ok(scanned_varbit || scanned_varp)
}

fn compare_file(
    builder: &mut PlanBuilder<'_>,
    archive: &ArchiveDef,
    group_id: u32,
    file_id: u32,
) -> Result<CompareState> {
    builder.read_group_files(RootKind::Donor, archive, group_id)?;
    builder.read_group_files(RootKind::Base, archive, group_id)?;
    let Some(donor) = builder.cached_file_bytes(RootKind::Donor, archive, group_id, file_id) else {
        return Ok(CompareState::MissingDonor);
    };
    let Some(base) = builder.cached_file_bytes(RootKind::Base, archive, group_id, file_id) else {
        return Ok(CompareState::MissingTarget);
    };
    Ok(if base == donor {
        CompareState::Same
    } else {
        CompareState::Conflict
    })
}

fn validate_donor_config_decodes(
    builder: &mut PlanBuilder<'_>,
    target: &ConfigTarget,
) -> Result<bool> {
    let donor_build = builder.donor_build;
    let Some(donor) = builder.read_file_bytes(
        RootKind::Donor,
        &target.archive,
        target.group_id,
        target.file_id,
    )?
    else {
        return Ok(true);
    };
    let result = match target.kind {
        RefKind::Obj => parse_obj(target.id, donor).map(|_| ()),
        RefKind::Npc => parse_npc(target.id, donor).map(|_| ()),
        RefKind::Loc => parse_loc(target.id, donor).map(|_| ()),
        RefKind::Struct => parse_struct(target.id, donor).map(|_| ()),
        RefKind::Enum => parse_enum(target.id, donor).map(|_| ()),
        RefKind::VarBit => crate::vars::parse_varbit(target.id, donor).map(|_| ()),
        RefKind::Varp => {
            crate::vars::parse_var(crate::vars::VarDomain::Player, target.id, donor).map(|_| ())
        }
        RefKind::DbTable => parse_dbtable(target.id, donor).map(|_| ()),
        RefKind::DbRow => parse_dbrow(target.id, donor).map(|_| ()),
        RefKind::Quest => parse_quest(target.id, donor).map(|_| ()),
        RefKind::Seq => parse_seq(target.id, donor).map(|_| ()),
        RefKind::Spot => parse_spot(target.id, donor).map(|_| ()),
        RefKind::Bas => parse_bas(target.id, donor, donor_build).map(|_| ()),
        RefKind::Material => parse_material(target.id, donor).map(|_| ()),
        RefKind::SeqGroup => parse_seqgroup(target.id, donor).map(|_| ()),
        _ => Ok(()),
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) => {
            builder.add_blocked(OverlayBlockedIssue {
                kind: "blocked-ref".to_string(),
                archive: Some(target.archive.name.to_string()),
                archive_id: Some(target.archive.id),
                group_id: Some(target.group_id),
                file_id: Some(target.file_id),
                id: Some(target.id),
                ref_kind: Some(target.kind.as_str().to_string()),
                message: format!(
                    "Donor {}_{} failed runtime decoder: {error}",
                    target.kind.as_str(),
                    target.id
                ),
            });
            Ok(false)
        }
    }
}

fn queue_varbit_base_varp_dependency(
    builder: &mut PlanBuilder<'_>,
    target: &ConfigTarget,
    source: &str,
) -> Result<()> {
    if target.kind != RefKind::VarBit {
        return Ok(());
    }
    let Some(donor) = builder.read_file_bytes(
        RootKind::Donor,
        &target.archive,
        target.group_id,
        target.file_id,
    )?
    else {
        return Ok(());
    };
    let entry = crate::vars::parse_varbit(target.id, donor)?;
    if let Some(base_varp) = entry.base_var {
        if builder.auto_allowed_missing_varbits.contains(&target.id) {
            builder.auto_allowed_missing_varps.insert(base_varp);
        }
        builder.queue(
            RefKind::Varp,
            base_varp,
            format!("{source}->varbit_{}", target.id),
            SelectionMode::Dependency,
        );
    }
    Ok(())
}

fn allowed_missing_target_ids(
    builder: &PlanBuilder<'_>,
    kind: RefKind,
    allow_ids: &[u32],
) -> HashSet<u32> {
    let mut merged = allow_ids.iter().copied().collect::<HashSet<_>>();
    match kind {
        RefKind::VarBit => merged.extend(builder.auto_allowed_missing_varbits.iter().copied()),
        RefKind::Varp => merged.extend(builder.auto_allowed_missing_varps.iter().copied()),
        _ => {}
    }
    merged
}

fn prove_script_ref(
    builder: &mut PlanBuilder<'_>,
    script_id: u32,
    allow_heuristic_sites: bool,
) -> Result<()> {
    builder.proof.script_checked += 1;
    let analyzer = builder.analyzer()?;
    let report = analyzer.analyze_script(script_id);
    let validation = analyzer.validate_script_target(&report.entities, None, allow_heuristic_sites);
    if let Some(issue) = script_validation_issue(script_id, &validation) {
        builder.proof.script_blocked += 1;
        builder.proof.blockers.push(issue);
        return Ok(());
    }
    builder.proof.script_valid += 1;
    select_script_bytes(builder, script_id, &validation)?;
    let source = format!("script_{script_id}");
    queue_supported_report_entities(builder, &report.entities, &source);
    Ok(())
}

fn prove_interface_ref(
    builder: &mut PlanBuilder<'_>,
    interface_id: u32,
    allow_heuristic_sites: bool,
) -> Result<()> {
    builder.proof.component_checked += 1;
    let analyzer = builder.analyzer()?;
    let report = analyzer.analyze_interface(interface_id);
    let validation = analyzer.validate_interface_target(
        interface_id,
        &report.entities,
        None,
        allow_heuristic_sites,
    );
    if let Some(issue) = interface_validation_issue(interface_id, &validation) {
        builder.proof.component_blocked += 1;
        builder.proof.blockers.push(issue);
        return Ok(());
    }
    builder.proof.component_valid += 1;
    select_interface_group(builder, interface_id)?;
    let source = format!("interface_{interface_id}");
    queue_supported_report_entities(builder, &report.entities, &source);
    Ok(())
}

fn script_validation_issue(
    script_id: u32,
    validation: &TargetValidationReport,
) -> Option<OverlayProofIssue> {
    let script = validation.scripts.first()?;
    if !script.heuristic_sites.is_empty() {
        return Some(OverlayProofIssue {
            kind: "heuristic",
            location: format!("script_{script_id}"),
            ref_kind: Some("script".to_string()),
            message: format!(
                "script_{script_id} target proof still depends on {} heuristic site(s).",
                script.heuristic_sites.len()
            ),
        });
    }
    if !script.unsupported_sites.is_empty() {
        return Some(OverlayProofIssue {
            kind: "unsupported",
            location: format!("script_{script_id}"),
            ref_kind: Some("script".to_string()),
            message: format!(
                "script_{script_id} target proof has {} unsupported site(s).",
                script.unsupported_sites.len()
            ),
        });
    }
    if !script.blockers.is_empty()
        || !script.validation_errors.is_empty()
        || script.failure.is_some()
    {
        let detail = script
            .failure
            .clone()
            .or_else(|| script.blockers.first().cloned())
            .or_else(|| {
                script
                    .validation_errors
                    .first()
                    .map(|error| format!("{error:?}"))
            })
            .unwrap_or_else(|| "unknown target validation failure".to_string());
        return Some(OverlayProofIssue {
            kind: "unsupported",
            location: format!("script_{script_id}"),
            ref_kind: Some("script".to_string()),
            message: format!("script_{script_id} target validation failed: {detail}"),
        });
    }
    None
}

fn interface_validation_issue(
    interface_id: u32,
    validation: &TargetValidationReport,
) -> Option<OverlayProofIssue> {
    let unsupported_components = validation
        .components
        .iter()
        .filter(|component| {
            !component.heuristic_sites.is_empty()
                || !component.unsupported_sites.is_empty()
                || !component.blocking_issues.is_empty()
        })
        .collect::<Vec<_>>();
    if let Some(component) = unsupported_components.first() {
        let detail = component
            .blocking_issues
            .first()
            .cloned()
            .or_else(|| {
                component
                    .unsupported_sites
                    .first()
                    .map(|site| format!("unsupported {}", dependency_site_label(site)))
            })
            .or_else(|| {
                component
                    .heuristic_sites
                    .first()
                    .map(|site| format!("heuristic {}", dependency_site_label(site)))
            })
            .unwrap_or_else(|| "unknown component validation failure".to_string());
        let kind =
            if !component.unsupported_sites.is_empty() || !component.blocking_issues.is_empty() {
                "unsupported"
            } else {
                "heuristic"
            };
        return Some(OverlayProofIssue {
            kind,
            location: format!(
                "interface_{interface_id}:component_{}",
                component.component_id
            ),
            ref_kind: Some("interface".to_string()),
            message: format!("interface_{interface_id} target validation failed: {detail}"),
        });
    }
    None
}

fn select_script_bytes(
    builder: &mut PlanBuilder<'_>,
    script_id: u32,
    validation: &TargetValidationReport,
) -> Result<()> {
    let Some(script) = validation.scripts.first() else {
        return Ok(());
    };
    let packed_id = script.source_packed_id.unwrap_or(script_id << 16);
    let group_id = packed_id >> 16;
    let file_id = packed_id & 0xFFFF;
    let archive = archive_clientscripts();
    let state = compare_file(builder, &archive, group_id, file_id)?;
    if state == CompareState::MissingDonor {
        builder.proof.script_blocked += 1;
        builder.proof.blockers.push(OverlayProofIssue {
            kind: "unsupported",
            location: format!("script_{script_id}"),
            ref_kind: Some("script".to_string()),
            message: format!("script_{script_id} donor bytes missing from scripts archive."),
        });
        return Ok(());
    }
    match state {
        CompareState::Conflict => {
            builder.add_warning(OverlayWarning {
                kind: "risk".to_string(),
                archive: Some(archive.name.to_string()),
                id: Some(script_id),
                ref_kind: Some("script".to_string()),
                message: format!(
                    "script_{script_id} overrides differing 910 bytes with selected 939 bytes."
                ),
            });
            builder.add_file(&archive, group_id, file_id);
        }
        CompareState::MissingTarget => builder.add_file(&archive, group_id, file_id),
        CompareState::Same | CompareState::MissingDonor => {}
    }
    Ok(())
}

fn select_interface_group(builder: &mut PlanBuilder<'_>, interface_id: u32) -> Result<()> {
    let archive = archive_interfaces();
    let donor = builder.read_raw_group(RootKind::Donor, &archive, interface_id)?;
    let Some(donor) = donor else {
        builder.proof.component_blocked += 1;
        builder.proof.blockers.push(OverlayProofIssue {
            kind: "unsupported",
            location: format!("interface_{interface_id}"),
            ref_kind: Some("interface".to_string()),
            message: format!("interface_{interface_id} donor group missing."),
        });
        return Ok(());
    };
    let base = builder.read_raw_group(RootKind::Base, &archive, interface_id)?;
    match base {
        Some(base) if base != donor => {
            builder.add_warning(OverlayWarning {
                kind: "risk".to_string(),
                archive: Some(archive.name.to_string()),
                id: Some(interface_id),
                ref_kind: Some("interface".to_string()),
                message: format!(
                    "interface_{interface_id} overrides differing 910 bytes with selected 939 bytes."
                ),
            });
            builder.add_group(&archive, interface_id);
        }
        None => builder.add_group(&archive, interface_id),
        _ => {}
    }
    Ok(())
}

fn queue_supported_report_entities(
    builder: &mut PlanBuilder<'_>,
    entities: &[ConflictEntry],
    source: &str,
) {
    for entity in entities {
        if let Some(kind) = RefKind::from_entity_type(&entity.entity_type) {
            if matches!(kind, RefKind::Script | RefKind::Interface)
                && source.starts_with(kind.as_str())
            {
                continue;
            }
            builder.queue(
                kind,
                entity.id,
                source.to_string(),
                SelectionMode::Dependency,
            );
            continue;
        }
        if is_supported_proof_entity(&entity.entity_type) {
            continue;
        }
        builder.proof.blockers.push(OverlayProofIssue {
            kind: "unsupported",
            location: source.to_string(),
            ref_kind: Some(if source.starts_with("script_") {
                "script".to_string()
            } else {
                "interface".to_string()
            }),
            message: format!(
                "{} proof requires unsupported dependency type {}_{}.",
                source, entity.entity_type, entity.id
            ),
        });
        if source.starts_with("script_") {
            builder.proof.script_blocked += 1;
        } else {
            builder.proof.component_blocked += 1;
        }
        return;
    }
}

fn is_supported_proof_entity(entity_type: &str) -> bool {
    matches!(
        entity_type,
        "varplayer" | "component" | "param" | "config" | "inv"
    )
}

fn finalize_plan(
    mut builder: PlanBuilder<'_>,
    allow_heuristic_sites: bool,
) -> Result<OverlayPlanOutput> {
    if let Some(edge_warning) = builder.semantic_index.edge_sample_warning() {
        builder.add_warning(edge_warning);
    }
    if builder.warning_overflow > 0 {
        builder.add_warning(OverlayWarning {
            kind: "risk".to_string(),
            archive: None,
            id: None,
            ref_kind: None,
            message: format!(
                "{} additional warning(s) omitted after first {}.",
                builder.warning_overflow, MAX_PLAN_WARNINGS
            ),
        });
    }

    let proof = build_overlay_proof(
        &builder.warnings,
        &builder.blocked,
        builder.proof.clone(),
        allow_heuristic_sites,
        builder.base_build,
        builder.donor_build,
    );
    let planner_fingerprint = format!(
        "rs3-cache-rs@{}:overlay-plan-v{}:native:{}.{}/{}.{}",
        env!("CARGO_PKG_VERSION"),
        OVERLAY_PLAN_VERSION,
        builder.base_build,
        builder.base_subbuild,
        builder.donor_build,
        builder.donor_subbuild
    );

    let selected_ids = selected_archive_ids(&builder);
    let mut hard_swap_archives = Vec::new();
    let mut patch_archives = Vec::new();
    for archive_id in selected_ids {
        let archive = archive_for_id(archive_id)?;
        match resolve_archive_mode(&builder.manifest, &archive) {
            "hard-swap" => hard_swap_archives.push(archive.name.to_string()),
            "patch" => patch_archives.push(archive.name.to_string()),
            _ => {}
        }
    }
    let blocked_conflicts = builder.blocked.clone();
    let warnings = builder.warnings.clone();
    let semantic_manifest_path = builder
        .semantic_donor_root()
        .join(".rs3-cache-manifest.json")
        .display()
        .to_string();
    let dependency_edges_sample = builder.semantic_index.dependency_edges_sample.clone();
    let mut plan = OverlayPlanOutput {
        roots: builder.roots.clone(),
        conflict_policy: builder
            .manifest
            .conflict_policy
            .clone()
            .unwrap_or_else(|| "fail".to_string()),
        hard_swap_archives,
        patch_archives,
        selected: OverlayPlanSelected {
            groups: group_selections_for_report(&builder)?,
            files: file_selections_for_report(&builder)?,
        },
        imports: OverlayPlanImports {
            config_groups: sorted_set(builder.group_selections.get(&archive_config().id)),
            maps: sorted_iter(builder.primary_maps.iter().copied()),
            objs: sorted_iter(builder.primary_objs.iter().copied()),
            npcs: sorted_iter(builder.primary_npcs.iter().copied()),
            locs: sorted_iter(builder.primary_locs.iter().copied()),
            structs: sorted_iter(builder.primary_structs.iter().copied()),
            enums: sorted_iter(builder.primary_enums.iter().copied()),
            varbits: sorted_iter(builder.primary_varbits.iter().copied()),
            varps: sorted_iter(builder.primary_varps.iter().copied()),
            db_tables: sorted_iter(builder.primary_db_tables.iter().copied()),
            db_rows: sorted_iter(builder.primary_db_rows.iter().copied()),
            interfaces: sorted_iter(builder.primary_interfaces.iter().copied()),
            scripts: sorted_iter(builder.primary_scripts.iter().copied()),
        },
        dependencies: dependencies_for_report(&builder),
        db: OverlayPlanDb {
            tables: sorted_set(builder.dependencies.get(&RefKind::DbTable)),
            rows: sorted_set(builder.dependencies.get(&RefKind::DbRow)),
            index_groups: sorted_set(builder.group_selections.get(&archive_dbtable_index().id)),
            schema_changes: sorted_iter(builder.db_schema_changes.iter().copied()),
        },
        blocked_conflicts,
        warnings,
        semantic_source: "refs",
        semantic_manifest: OverlaySemanticManifest {
            path: semantic_manifest_path,
            donor_fingerprint: Some(manifest_fingerprint(&builder.donor_manifest)),
            base_fingerprint: Some(manifest_fingerprint(&builder.base_manifest)),
        },
        dependency_edges_sample,
        plan_version: OVERLAY_PLAN_VERSION,
        planner_fingerprint,
        proof,
        audit: OverlayPlanAudit::default(),
    };
    plan.hard_swap_archives.sort();
    plan.patch_archives.sort();
    Ok(plan)
}

fn build_overlay_proof(
    warnings: &[OverlayWarning],
    blocked_conflicts: &[OverlayBlockedIssue],
    proof_state: ProofState,
    allow_heuristic_sites: bool,
    base_build: u32,
    donor_build: u32,
) -> OverlayPlanProof {
    let mut blockers = proof_state.blockers;
    for (index, warning) in warnings.iter().enumerate() {
        if let Some(issue) = classify_warning_issue(index, warning) {
            blockers.push(issue);
        }
    }
    let unsupported_site_count = blockers
        .iter()
        .filter(|issue| issue.kind == "unsupported")
        .count();
    let heuristic_site_count = blockers
        .iter()
        .filter(|issue| issue.kind == "heuristic")
        .count();
    let blocked = !blocked_conflicts.is_empty()
        || unsupported_site_count > 0
        || (!allow_heuristic_sites && heuristic_site_count > 0);

    let mut next_actions = BTreeSet::new();
    if !blocked_conflicts.is_empty() {
        next_actions.insert(
            "resolve blockedConflicts or extend explicit allowlists before apply".to_string(),
        );
    }
    if unsupported_site_count > 0 {
        next_actions.insert(format!(
            "run rs3-cache-rs migrate-check --validate-target or migrate-script --validate-target for donor {donor_build} -> target {base_build} clientscript/interface slices"
        ));
    }
    if heuristic_site_count > 0 {
        next_actions.insert(
            "refresh semantic trees with bun run cache:semantic:sync-947 or rs3-cache-rs prepare-overlay before apply".to_string(),
        );
    }

    OverlayPlanProof {
        status: if blocked { "blocked" } else { "ok" },
        strict: !allow_heuristic_sites,
        unsupported_site_count,
        heuristic_site_count,
        script_summary: OverlayProofSummary {
            checked: proof_state.script_checked,
            blocked: proof_state.script_blocked,
            valid: proof_state.script_valid,
        },
        component_summary: OverlayProofSummary {
            checked: proof_state.component_checked,
            blocked: proof_state.component_blocked,
            valid: proof_state.component_valid,
        },
        next_actions: next_actions.into_iter().collect(),
        blockers,
    }
}

fn classify_warning_issue(index: usize, warning: &OverlayWarning) -> Option<OverlayProofIssue> {
    if warning.kind != "risk" {
        return None;
    }
    let message = warning.message.to_ascii_lowercase();
    if message.contains("falling back to donor binary dependency scan")
        || message.contains("missing under semantic root")
        || message.contains("additional warning(s) omitted")
        || message.contains("has no entry for")
    {
        return Some(OverlayProofIssue {
            kind: "heuristic",
            location: format!("warning[{index}]"),
            ref_kind: warning.ref_kind.clone(),
            message: warning.message.clone(),
        });
    }
    None
}

fn write_overlay_plan_audit(
    audit_dir: &Path,
    proof: &OverlayPlanProof,
    blocked_conflicts: &[OverlayBlockedIssue],
) -> Result<OverlayPlanAudit> {
    fs::create_dir_all(audit_dir).with_context(|| format!("creating {}", audit_dir.display()))?;

    let summary_path = audit_dir.join("summary.json");
    write_json(
        &summary_path,
        &json!({
            "proof": proof,
            "blockedConflicts": blocked_conflicts,
            "blockedConflictCount": blocked_conflicts.len(),
        }),
    )?;

    let unsupported = proof
        .blockers
        .iter()
        .filter(|issue| issue.kind == "unsupported")
        .cloned()
        .collect::<Vec<_>>();
    write_jsonl(&audit_dir.join("unsupported_sites.jsonl"), &unsupported)?;

    let heuristic = proof
        .blockers
        .iter()
        .filter(|issue| issue.kind == "heuristic")
        .cloned()
        .collect::<Vec<_>>();
    write_jsonl(&audit_dir.join("heuristic_sites.jsonl"), &heuristic)?;

    let script_failures = proof
        .blockers
        .iter()
        .filter(|issue| issue.ref_kind.as_deref() == Some("script"))
        .cloned()
        .collect::<Vec<_>>();
    write_jsonl(&audit_dir.join("scripts_failed.jsonl"), &script_failures)?;

    let component_failures = proof
        .blockers
        .iter()
        .filter(|issue| issue.ref_kind.as_deref() == Some("interface"))
        .cloned()
        .collect::<Vec<_>>();
    write_jsonl(
        &audit_dir.join("components_failed.jsonl"),
        &component_failures,
    )?;

    Ok(OverlayPlanAudit {
        relative_paths: vec![
            "summary.json".to_string(),
            "unsupported_sites.jsonl".to_string(),
            "heuristic_sites.jsonl".to_string(),
            "scripts_failed.jsonl".to_string(),
            "components_failed.jsonl".to_string(),
        ],
    })
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let encoded = serde_json::to_vec_pretty(value).context("encoding overlay plan json")?;
    fs::write(path, encoded).with_context(|| format!("writing {}", path.display()))
}

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut out = String::new();
    for row in rows {
        out.push_str(&serde_json::to_string(row)?);
        out.push('\n');
    }
    fs::write(path, out).with_context(|| format!("writing {}", path.display()))
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn resolve_roots(manifest: &CacheOverlayManifest) -> Result<OverlayRoots> {
    Ok(OverlayRoots {
        base_raw_root: absolutize(
            manifest
                .roots
                .base_raw_root
                .as_ref()
                .or(manifest.base_raw_root.as_ref())
                .with_context(|| "overlay manifest missing roots.baseRawRoot/baseRawRoot")?,
        )?,
        donor_raw_root: absolutize(
            manifest
                .roots
                .donor_raw_root
                .as_ref()
                .or(manifest.donor_raw_root.as_ref())
                .with_context(|| "overlay manifest missing roots.donorRawRoot/donorRawRoot")?,
        )?,
        base_semantic_root: absolutize(
            manifest
                .roots
                .base_semantic_root
                .as_ref()
                .or(manifest.base_semantic_root.as_ref())
                .with_context(
                    || "overlay manifest missing roots.baseSemanticRoot/baseSemanticRoot",
                )?,
        )?,
        donor_semantic_root: absolutize(
            manifest
                .roots
                .donor_semantic_root
                .as_ref()
                .or(manifest.donor_semantic_root.as_ref())
                .with_context(
                    || "overlay manifest missing roots.donorSemanticRoot/donorSemanticRoot",
                )?,
        )?,
        base_pack_root: absolutize(
            manifest
                .roots
                .base_pack_root
                .as_ref()
                .or(manifest.base_pack_root.as_ref())
                .with_context(|| "overlay manifest missing roots.basePackRoot/basePackRoot")?,
        )?,
        output_pack_root: absolutize(
            manifest
                .roots
                .output_pack_root
                .as_ref()
                .or(manifest.output_pack_root.as_ref())
                .with_context(|| "overlay manifest missing roots.outputPackRoot/outputPackRoot")?,
        )?,
        client_output_pack_root: absolutize(
            manifest
                .roots
                .client_output_pack_root
                .as_ref()
                .or(manifest.client_output_pack_root.as_ref())
                .with_context(
                    || "overlay manifest missing roots.clientOutputPackRoot/clientOutputPackRoot",
                )?,
        )?,
    })
}

fn absolutize(path: &Path) -> Result<String> {
    if path.is_absolute() {
        return Ok(path.display().to_string());
    }
    Ok(std::env::current_dir()?.join(path).display().to_string())
}

fn read_semantic_manifest(root: &Path) -> Result<Rs3CacheManifest> {
    let path = root.join(".rs3-cache-manifest.json");
    serde_json::from_slice(&fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("decoding {}", path.display()))
}

fn manifest_fingerprint(manifest: &Rs3CacheManifest) -> String {
    format!(
        "{}.{}:{}:{}",
        manifest.build, manifest.subbuild, manifest.cache_fingerprint, manifest.tool_version
    )
}

fn compare_mode(mode: ArchiveMode) -> &'static str {
    match mode {
        ArchiveMode::Auto => "patch",
        ArchiveMode::Patch => "patch",
        ArchiveMode::HardSwap => "hard-swap",
    }
}

fn resolve_archive_mode(manifest: &CacheOverlayManifest, archive: &ArchiveDef) -> &'static str {
    if let Some(mode) = lookup_archive_mode(&manifest.archive_modes, archive)
        && mode != ArchiveMode::Auto
    {
        return compare_mode(mode);
    }
    let hard_swap_allow = manifest
        .allow
        .hard_swap_archives
        .iter()
        .map(archive_ref_key)
        .collect::<HashSet<_>>();
    if hard_swap_allow.contains(&normalize_archive_key(archive.name))
        || hard_swap_allow.contains(&archive.id.to_string())
    {
        return "hard-swap";
    }
    if archive.id == archive_maps().id {
        "hard-swap"
    } else {
        "patch"
    }
}

fn lookup_archive_mode(
    modes: &BTreeMap<String, ArchiveMode>,
    archive: &ArchiveDef,
) -> Option<ArchiveMode> {
    let mut names = HashSet::from([normalize_archive_key(archive.name), archive.id.to_string()]);
    if archive.id == archive_maps().id {
        names.insert("maps".to_string());
    }
    for (key, value) in modes {
        if names.contains(&normalize_archive_key(key)) {
            return Some(*value);
        }
    }
    None
}

fn normalize_archive_key(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
}

fn archive_ref_key(archive_ref: &ArchiveRef) -> String {
    match archive_ref {
        ArchiveRef::Name(name) => normalize_archive_key(name),
        ArchiveRef::Id(id) => id.to_string(),
    }
}

fn archive_for_manifest_ref(value: &ArchiveRef) -> Result<ArchiveDef> {
    match value {
        ArchiveRef::Id(id) => archive_for_id(*id),
        ArchiveRef::Name(name) => archive_for_name(name),
    }
}

fn archive_for_name(name: &str) -> Result<ArchiveDef> {
    let normalized = normalize_archive_key(name);
    all_archives()
        .into_iter()
        .find(|archive| {
            normalize_archive_key(archive.name) == normalized
                || archive.id.to_string() == name.trim()
        })
        .with_context(|| format!("Unsupported archive reference {name}."))
}

fn archive_for_id(id: u32) -> Result<ArchiveDef> {
    all_archives()
        .into_iter()
        .find(|archive| archive.id == id)
        .with_context(|| format!("Unsupported archive id {id}."))
}

fn all_archives() -> Vec<ArchiveDef> {
    vec![
        ArchiveDef {
            id: 0,
            donor_id: 48,
            name: "anims",
        },
        ArchiveDef {
            id: 1,
            donor_id: 1,
            name: "bases",
        },
        archive_config(),
        archive_interfaces(),
        archive_maps(),
        ArchiveDef {
            id: 7,
            donor_id: 7,
            name: "models",
        },
        archive_sprites(),
        ArchiveDef {
            id: ARCHIVE_BINARY,
            donor_id: ARCHIVE_BINARY,
            name: "binary",
        },
        archive_clientscripts(),
        ArchiveDef {
            id: 13,
            donor_id: 13,
            name: "fontmetrics",
        },
        archive_loc_config(),
        ArchiveDef {
            id: ARCHIVE_ENUM_CONFIG,
            donor_id: ARCHIVE_ENUM_CONFIG,
            name: "enum.config",
        },
        archive_npc_config(),
        archive_obj_config(),
        ArchiveDef {
            id: ARCHIVE_SEQ_CONFIG,
            donor_id: ARCHIVE_SEQ_CONFIG,
            name: "seq.config",
        },
        ArchiveDef {
            id: ARCHIVE_SPOT_CONFIG,
            donor_id: ARCHIVE_SPOT_CONFIG,
            name: "spot.config",
        },
        ArchiveDef {
            id: ARCHIVE_STRUCT_CONFIG,
            donor_id: ARCHIVE_STRUCT_CONFIG,
            name: "struct.config",
        },
        ArchiveDef {
            id: 23,
            donor_id: 23,
            name: "worldmap",
        },
        archive_materials(),
        ArchiveDef {
            id: ARCHIVE_PARTICLES,
            donor_id: ARCHIVE_PARTICLES,
            name: "particles",
        },
        ArchiveDef {
            id: 29,
            donor_id: 29,
            name: "billboards",
        },
        ArchiveDef {
            id: 36,
            donor_id: 36,
            name: "vfx",
        },
        ArchiveDef {
            id: 40,
            donor_id: 40,
            name: "audiostreams",
        },
        archive_models_rt7(),
        archive_anims_rt7(),
        archive_dbtable_index(),
        ArchiveDef {
            id: 52,
            donor_id: 52,
            name: "textures.dxt",
        },
        ArchiveDef {
            id: 53,
            donor_id: 53,
            name: "textures.png",
        },
        ArchiveDef {
            id: 54,
            donor_id: 54,
            name: "textures.png.mipped",
        },
        ArchiveDef {
            id: 55,
            donor_id: 55,
            name: "textures.etc",
        },
        archive_anim_keyframes(),
        ArchiveDef {
            id: 57,
            donor_id: 57,
            name: "achievements",
        },
        ArchiveDef {
            id: 58,
            donor_id: 58,
            name: "fontmetrics2",
        },
        ArchiveDef {
            id: 59,
            donor_id: 59,
            name: "ttf",
        },
        ArchiveDef {
            id: 60,
            donor_id: 60,
            name: "stylesheets",
        },
        ArchiveDef {
            id: 61,
            donor_id: 61,
            name: "vfx2",
        },
        ArchiveDef {
            id: 62,
            donor_id: 62,
            name: "animator",
        },
        ArchiveDef {
            id: 65,
            donor_id: 65,
            name: "uianim",
        },
        ArchiveDef {
            id: 66,
            donor_id: 66,
            name: "cutscene2d",
        },
    ]
}

fn archive_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_CONFIG,
        donor_id: ARCHIVE_CONFIG,
        name: "config",
    }
}
fn archive_interfaces() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_INTERFACES,
        donor_id: ARCHIVE_INTERFACES,
        name: "interfaces",
    }
}
fn archive_maps() -> ArchiveDef {
    ArchiveDef {
        id: 5,
        donor_id: 5,
        name: "mapsv2",
    }
}
fn archive_sprites() -> ArchiveDef {
    ArchiveDef {
        id: 8,
        donor_id: 8,
        name: "sprites",
    }
}
fn archive_clientscripts() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_CLIENTSCRIPTS,
        donor_id: ARCHIVE_CLIENTSCRIPTS,
        name: "scripts",
    }
}
fn archive_loc_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_LOC_CONFIG,
        donor_id: ARCHIVE_LOC_CONFIG,
        name: "loc.config",
    }
}
fn archive_npc_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_NPC_CONFIG,
        donor_id: ARCHIVE_NPC_CONFIG,
        name: "npc.config",
    }
}
fn archive_obj_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_OBJ_CONFIG,
        donor_id: ARCHIVE_OBJ_CONFIG,
        name: "obj.config",
    }
}
fn archive_materials() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_MATERIALS,
        donor_id: ARCHIVE_MATERIALS,
        name: "materials",
    }
}
fn archive_models_rt7() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_MODELS_RT7,
        donor_id: ARCHIVE_MODELS_RT7,
        name: "modelsrt7",
    }
}
fn archive_anims_rt7() -> ArchiveDef {
    ArchiveDef {
        id: 48,
        donor_id: 48,
        name: "animsrt7",
    }
}
fn archive_dbtable_index() -> ArchiveDef {
    ArchiveDef {
        id: 49,
        donor_id: 49,
        name: "dbtableindex",
    }
}
fn archive_anim_keyframes() -> ArchiveDef {
    ArchiveDef {
        id: 56,
        donor_id: 56,
        name: "anims.keyframes",
    }
}

fn config_target(kind: RefKind, id: u32) -> ConfigTarget {
    match kind {
        RefKind::Loc => ConfigTarget {
            archive: archive_loc_config(),
            group_id: id >> 8,
            file_id: id & 0xFF,
            id,
            kind,
        },
        RefKind::Npc => ConfigTarget {
            archive: archive_npc_config(),
            group_id: id >> 7,
            file_id: id & 0x7F,
            id,
            kind,
        },
        RefKind::Obj => ConfigTarget {
            archive: archive_obj_config(),
            group_id: id >> 8,
            file_id: id & 0xFF,
            id,
            kind,
        },
        RefKind::Seq => ConfigTarget {
            archive: ArchiveDef {
                id: ARCHIVE_SEQ_CONFIG,
                donor_id: ARCHIVE_SEQ_CONFIG,
                name: "seq.config",
            },
            group_id: id >> 7,
            file_id: id & 0x7F,
            id,
            kind,
        },
        RefKind::Spot => ConfigTarget {
            archive: ArchiveDef {
                id: ARCHIVE_SPOT_CONFIG,
                donor_id: ARCHIVE_SPOT_CONFIG,
                name: "spot.config",
            },
            group_id: id >> 8,
            file_id: id & 0xFF,
            id,
            kind,
        },
        RefKind::Struct => ConfigTarget {
            archive: ArchiveDef {
                id: ARCHIVE_STRUCT_CONFIG,
                donor_id: ARCHIVE_STRUCT_CONFIG,
                name: "struct.config",
            },
            group_id: id >> 5,
            file_id: id & 0x1F,
            id,
            kind,
        },
        RefKind::Enum => ConfigTarget {
            archive: ArchiveDef {
                id: ARCHIVE_ENUM_CONFIG,
                donor_id: ARCHIVE_ENUM_CONFIG,
                name: "enum.config",
            },
            group_id: id >> 8,
            file_id: id & 0xFF,
            id,
            kind,
        },
        RefKind::Bas => ConfigTarget {
            archive: archive_config(),
            group_id: CONFIG_GROUP_BAS,
            file_id: id,
            id,
            kind,
        },
        RefKind::VarBit => ConfigTarget {
            archive: archive_config(),
            group_id: CONFIG_GROUP_VAR_BIT,
            file_id: id,
            id,
            kind,
        },
        RefKind::Varp => ConfigTarget {
            archive: archive_config(),
            group_id: CONFIG_GROUP_VAR_PLAYER,
            file_id: id,
            id,
            kind,
        },
        RefKind::Quest => ConfigTarget {
            archive: archive_config(),
            group_id: CONFIG_GROUP_QUEST,
            file_id: id,
            id,
            kind,
        },
        RefKind::DbTable => ConfigTarget {
            archive: archive_config(),
            group_id: CONFIG_GROUP_DBTABLE,
            file_id: id,
            id,
            kind,
        },
        RefKind::DbRow => ConfigTarget {
            archive: archive_config(),
            group_id: CONFIG_GROUP_DBROW,
            file_id: id,
            id,
            kind,
        },
        _ => panic!("No config target for {}", kind.as_str()),
    }
}

fn resolve_model_target(builder: &PlanBuilder<'_>, id: u32) -> RawGroupTarget {
    let rt7_path = Path::new(&builder.roots.donor_raw_root).join(format!(
        "{}/{}.dat",
        archive_models_rt7().donor_id,
        id
    ));
    let base_rt7_path = Path::new(&builder.roots.base_raw_root).join(format!(
        "{}/{}.dat",
        archive_models_rt7().id,
        id
    ));
    if rt7_path.is_file() || base_rt7_path.is_file() {
        return RawGroupTarget {
            archive: archive_models_rt7(),
            group_id: id,
            id,
            kind: RefKind::Model,
        };
    }
    RawGroupTarget {
        archive: ArchiveDef {
            id: 7,
            donor_id: 7,
            name: "models",
        },
        group_id: id,
        id,
        kind: RefKind::Model,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareState {
    MissingDonor,
    MissingTarget,
    Same,
    Conflict,
}

fn missing_config(builder: &mut PlanBuilder<'_>, target: &ConfigTarget) {
    builder.add_blocked(OverlayBlockedIssue {
        kind: "missing".to_string(),
        archive: Some(target.archive.name.to_string()),
        archive_id: Some(target.archive.id),
        group_id: Some(target.group_id),
        file_id: Some(target.file_id),
        id: Some(target.id),
        ref_kind: Some(target.kind.as_str().to_string()),
        message: format!("Donor {}_{} missing.", target.kind.as_str(), target.id),
    });
}

fn is_group_present(index: &ArchiveIndex, group_id: u32) -> bool {
    index.group_id.binary_search(&group_id).is_ok()
}

fn parse_loc_ids(data: &[u8]) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    let mut buf = Packet::new(data);
    let mut loc_id = -1_i32;
    let mut loc_id_offset = buf.g_extended_1or2()?;
    while loc_id_offset != 0 && !buf.is_done() {
        loc_id += loc_id_offset;
        let mut coord_offset = i32::from(buf.gsmart1or2()?);
        while coord_offset != 0 {
            let info = buf.g1()?;
            if (info & 0x80) != 0 {
                let scalerottrans = buf.g1()?;
                if (scalerottrans & 0x1) != 0 {
                    skip_packet(&mut buf, 8)?;
                }
                if (scalerottrans & 0x2) != 0 {
                    skip_packet(&mut buf, 2)?;
                }
                if (scalerottrans & 0x4) != 0 {
                    skip_packet(&mut buf, 2)?;
                }
                if (scalerottrans & 0x8) != 0 {
                    skip_packet(&mut buf, 2)?;
                }
                if (scalerottrans & 0x10) == 0 {
                    if (scalerottrans & 0x20) != 0 {
                        skip_packet(&mut buf, 2)?;
                    }
                    if (scalerottrans & 0x40) != 0 {
                        skip_packet(&mut buf, 2)?;
                    }
                    if (scalerottrans & 0x80) != 0 {
                        skip_packet(&mut buf, 2)?;
                    }
                } else {
                    skip_packet(&mut buf, 2)?;
                }
            }
            ids.push(u32::try_from(loc_id).context("negative loc id while parsing map loc file")?);
            coord_offset = i32::from(buf.gsmart1or2()?);
        }
        loc_id_offset = buf.g_extended_1or2()?;
    }
    Ok(ids)
}

fn parse_npc_ids(data: &[u8]) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    let mut buf = Packet::new(data);
    while buf.len().saturating_sub(buf.pos()) >= 4 {
        let _ = buf.g2()?;
        ids.push(u32::from(buf.g2()?));
    }
    Ok(ids)
}

fn skip_packet(buf: &mut Packet<'_>, count: usize) -> Result<()> {
    let next = buf
        .pos()
        .checked_add(count)
        .context("packet skip overflow")?;
    Ok(buf.set_pos(next)?)
}

#[derive(Default)]
struct MultivarRefs {
    varbit: Option<u32>,
    varp: Option<u32>,
}

fn scan_multivar_refs(ops: &[String]) -> MultivarRefs {
    let mut refs = MultivarRefs::default();
    for op in ops {
        if let Some(rest) = op.strip_prefix("multivar=varbit:")
            && let Some(id) = parse_first_u32(rest)
        {
            refs.varbit = Some(id);
        }
        if let Some(rest) = op.strip_prefix("multivar=varp:")
            && let Some(id) = parse_first_u32(rest)
        {
            refs.varp = Some(id);
        }
    }
    refs
}

fn parse_first_u32(input: &str) -> Option<u32> {
    input
        .split([',', ' '])
        .next()
        .and_then(|value| value.parse::<u32>().ok())
}

fn normalize_graph_ref_kind(kind: &SemanticRefKey) -> Option<RefKind> {
    Some(match kind {
        SemanticRefKey::Graphic | SemanticRefKey::Spotanim | SemanticRefKey::Spot => RefKind::Spot,
        SemanticRefKey::MultivarVarbit => RefKind::VarBit,
        SemanticRefKey::MultivarVarp => RefKind::Varp,
        SemanticRefKey::Anim => RefKind::Anim,
        SemanticRefKey::Material => RefKind::Material,
        SemanticRefKey::Model => RefKind::Model,
        SemanticRefKey::Sprite => RefKind::Sprite,
        SemanticRefKey::SeqGroup => RefKind::SeqGroup,
        SemanticRefKey::Interface => RefKind::Interface,
        SemanticRefKey::Script => RefKind::Script,
        SemanticRefKey::Obj => RefKind::Obj,
        SemanticRefKey::Npc => RefKind::Npc,
        SemanticRefKey::Loc => RefKind::Loc,
        SemanticRefKey::Seq => RefKind::Seq,
        SemanticRefKey::Bas => RefKind::Bas,
        SemanticRefKey::Enum => RefKind::Enum,
        SemanticRefKey::Struct => RefKind::Struct,
        SemanticRefKey::DbTable => RefKind::DbTable,
        SemanticRefKey::DbRow => RefKind::DbRow,
        SemanticRefKey::Varp => RefKind::Varp,
        SemanticRefKey::VarBit => RefKind::VarBit,
        SemanticRefKey::Quest => RefKind::Quest,
        _ => return None,
    })
}

fn semantic_refs_file_name(kind: &str) -> &str {
    if kind == "spotanim" { "spot" } else { kind }
}

fn semantic_kind_to_ref_kind(kind: &str) -> RefKind {
    if kind == "spotanim" {
        RefKind::Spot
    } else {
        normalize_graph_ref_kind(&SemanticRefKey::from_label(kind)).unwrap_or(RefKind::Struct)
    }
}

fn db_row_dependency_kinds(table_id: u32) -> HashSet<RefKind> {
    match table_id {
        74 => HashSet::from([RefKind::Obj, RefKind::Npc, RefKind::Loc]),
        88 => HashSet::from([RefKind::Obj, RefKind::DbRow]),
        89 | 94 => HashSet::from([RefKind::Obj]),
        _ => HashSet::from([
            RefKind::Obj,
            RefKind::Npc,
            RefKind::Loc,
            RefKind::Enum,
            RefKind::Struct,
            RefKind::VarBit,
            RefKind::Varp,
            RefKind::Quest,
            RefKind::DbTable,
            RefKind::DbRow,
        ]),
    }
}

fn normalize_region(region: RegionSpec) -> Result<u32> {
    match region {
        RegionSpec::Id(id) => Ok(id),
        RegionSpec::Text(text) => {
            let trimmed = text.trim();
            if let Some((x, z)) = trimmed
                .split_once(['_', ':', ','])
                .and_then(|(x, z)| Some((x.parse::<u32>().ok()?, z.parse::<u32>().ok()?)))
            {
                Ok(pack_map_square_group_id(x, z))
            } else {
                trimmed
                    .parse::<u32>()
                    .with_context(|| format!("Invalid region spec {trimmed}."))
            }
        }
        RegionSpec::Coord { x, z } => Ok(pack_map_square_group_id(x, z)),
    }
}

fn pack_map_square_group_id(region_x: u32, region_z: u32) -> u32 {
    region_x | (region_z << 7)
}

fn selected_archive_ids(builder: &PlanBuilder<'_>) -> Vec<u32> {
    let mut ids = BTreeSet::new();
    ids.extend(builder.group_selections.keys().copied());
    ids.extend(
        builder
            .file_selections
            .keys()
            .map(|(archive_id, _)| *archive_id),
    );
    ids.into_iter().collect()
}

fn group_selections_for_report(builder: &PlanBuilder<'_>) -> Result<Vec<OverlayPlanArchiveGroups>> {
    let mut rows = Vec::new();
    for (archive_id, groups) in &builder.group_selections {
        let archive = archive_for_id(*archive_id)?;
        rows.push(OverlayPlanArchiveGroups {
            archive: archive.name.to_string(),
            archive_id: *archive_id,
            mode: resolve_archive_mode(&builder.manifest, &archive),
            groups: groups.iter().copied().collect(),
        });
    }
    rows.sort_by_key(|row| row.archive_id);
    Ok(rows)
}

fn file_selections_for_report(builder: &PlanBuilder<'_>) -> Result<Vec<OverlayPlanArchiveFiles>> {
    let mut rows = Vec::new();
    for ((archive_id, group_id), file_ids) in &builder.file_selections {
        let archive = archive_for_id(*archive_id)?;
        rows.push(OverlayPlanArchiveFiles {
            archive: archive.name.to_string(),
            archive_id: *archive_id,
            mode: resolve_archive_mode(&builder.manifest, &archive),
            group_id: *group_id,
            file_ids: file_ids.iter().copied().collect(),
        });
    }
    rows.sort_by_key(|row| (row.archive_id, row.group_id));
    Ok(rows)
}

fn dependencies_for_report(builder: &PlanBuilder<'_>) -> BTreeMap<String, Vec<u32>> {
    let mut out = BTreeMap::new();
    for (kind, ids) in &builder.dependencies {
        out.insert(kind.as_str().to_string(), ids.iter().copied().collect());
    }
    out
}

fn sorted_set(values: Option<&BTreeSet<u32>>) -> Vec<u32> {
    values
        .map(|set| set.iter().copied().collect())
        .unwrap_or_default()
}

fn sorted_iter(values: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut out = values.collect::<Vec<_>>();
    out.sort_unstable();
    out
}

fn dependency_site_label(site: &DependencySite) -> String {
    format!(
        "{}_{} at {}",
        site.entity_type.as_label(),
        site.id,
        site.location
    )
}

fn scan_rt7_model_material_ids(data: &[u8]) -> Result<Vec<u32>> {
    if data.is_empty() || data[0] != 2 {
        return Ok(Vec::new());
    }
    let mut pos = 0usize;
    let format = rt7_g1(data, &mut pos, "format")?;
    if format != 2 {
        return Ok(Vec::new());
    }
    let version = rt7_g1(data, &mut pos, "version")?;
    let _ = rt7_g1(data, &mut pos, "always_0f")?;
    let mesh_count = usize::from(rt7_g1(data, &mut pos, "meshCount")?);
    let _ = rt7_g1(data, &mut pos, "unkCount0")?;
    let unk_count1 = usize::from(rt7_g1(data, &mut pos, "unkCount1")?);
    let unk_count2 = usize::from(rt7_g1(data, &mut pos, "unkCount2")?);
    let unk_count3 = usize::from(rt7_g1(data, &mut pos, "unkCount3")?);
    let unk_count4 = if version >= 5 {
        usize::from(rt7_g1(data, &mut pos, "unkCount4")?)
    } else {
        0
    };
    let mut material_ids = BTreeSet::new();
    if version <= 3 {
        scan_rt7_legacy_meshes(data, &mut pos, mesh_count, &mut material_ids)?;
    } else {
        scan_rt7_shared_mesh(data, &mut pos, mesh_count, &mut material_ids)?;
    }
    rt7_skip(data, &mut pos, unk_count1 * 39, "unk1Buffer")?;
    rt7_skip(data, &mut pos, unk_count2 * 50, "unk2Buffer")?;
    rt7_skip(data, &mut pos, unk_count3 * 18, "unk3Buffer")?;
    ensure!(unk_count4 == 0, "unsupported unk4Buffer count {unk_count4}");
    Ok(material_ids.into_iter().collect())
}

fn scan_rt7_legacy_meshes(
    data: &[u8],
    pos: &mut usize,
    mesh_count: usize,
    material_ids: &mut BTreeSet<u32>,
) -> Result<()> {
    for _ in 0..mesh_count {
        let group_flags = rt7_g1(data, pos, "legacy.groupFlags")?;
        rt7_skip(data, pos, 4, "legacy.unkint")?;
        add_rt7_material_id(
            material_ids,
            rt7_g2le(data, pos, "legacy.materialArgument")?,
        );
        let face_count = usize::from(rt7_g2le(data, pos, "legacy.faceCount")?);
        let has_vertices = (group_flags & 0x1) != 0;
        let has_vertex_alpha = (group_flags & 0x2) != 0;
        let has_face_bones = (group_flags & 0x4) != 0;
        let has_bone_ids = (group_flags & 0x8) != 0;
        let has_skin = (group_flags & 0x20) != 0;
        if has_vertices {
            rt7_skip(data, pos, face_count * 2, "legacy.colourBuffer")?;
        }
        if has_vertex_alpha {
            rt7_skip(data, pos, face_count, "legacy.alphaBuffer")?;
        }
        if has_face_bones {
            rt7_skip(data, pos, face_count * 2, "legacy.faceboneidBuffer")?;
        }
        let index_buffer_count = usize::from(rt7_g1(data, pos, "legacy.indexBuffers.length")?);
        for _ in 0..index_buffer_count {
            let index_count = usize::from(rt7_g2le(data, pos, "legacy.indexBuffer.length")?);
            rt7_skip(data, pos, index_count * 2, "legacy.indexBuffer")?;
        }
        let vertex_count = if has_vertices {
            usize::from(rt7_g2le(data, pos, "legacy.vertexCount")?)
        } else {
            0
        };
        if has_vertices {
            rt7_skip(data, pos, vertex_count * 6, "legacy.positionBuffer")?;
            rt7_skip(data, pos, vertex_count * 3, "legacy.normalBuffer")?;
            rt7_skip(data, pos, vertex_count * 4, "legacy.tangentBuffer")?;
            rt7_skip(data, pos, vertex_count * 4, "legacy.uvBuffer")?;
        }
        if has_bone_ids {
            rt7_skip(data, pos, vertex_count * 2, "legacy.boneidBuffer")?;
        }
        if has_skin {
            let skin_weight_count = usize::try_from(rt7_g4le(data, pos, "legacy.skinWeightCount")?)
                .context("legacy.skinWeightCount overflow")?;
            rt7_skip(data, pos, skin_weight_count * 2, "legacy.skinBoneBuffer")?;
            rt7_skip(data, pos, skin_weight_count, "legacy.skinWeightBuffer")?;
        }
    }
    Ok(())
}

fn scan_rt7_shared_mesh(
    data: &[u8],
    pos: &mut usize,
    mesh_count: usize,
    material_ids: &mut BTreeSet<u32>,
) -> Result<()> {
    let group_flags = rt7_g1(data, pos, "meshdata.groupFlags")?;
    let _ = rt7_g1(data, pos, "meshdata.unkint")?;
    let _ = rt7_g2le(data, pos, "meshdata.faceCount")?;
    let has_vertices = (group_flags & 0x1) != 0;
    let has_bone_ids = (group_flags & 0x8) != 0;
    let has_skin = (group_flags & 0x20) != 0;
    let has_face_bones = (group_flags & 0x4) != 0;
    let vertex_count = usize::try_from(rt7_g4le(data, pos, "meshdata.vertexCount")?)
        .context("meshdata.vertexCount overflow")?;
    if has_vertices {
        rt7_skip(data, pos, vertex_count * 6, "meshdata.positionBuffer")?;
        rt7_skip(data, pos, vertex_count * 3, "meshdata.normalBuffer")?;
        rt7_skip(data, pos, vertex_count * 4, "meshdata.tangentBuffer")?;
        rt7_skip(data, pos, vertex_count * 4, "meshdata.uvBuffer")?;
    }
    if has_bone_ids {
        rt7_skip(data, pos, vertex_count * 2, "meshdata.boneidBuffer")?;
    }
    if has_skin {
        for _ in 0..vertex_count {
            let id_count = usize::from(rt7_g2le(data, pos, "meshdata.skin.ids.length")?);
            rt7_skip(data, pos, id_count * 2, "meshdata.skin.ids")?;
            let weight_count = usize::from(rt7_g2le(data, pos, "meshdata.skin.weights.length")?);
            rt7_skip(data, pos, weight_count, "meshdata.skin.weights")?;
        }
    }
    if has_vertices {
        rt7_skip(data, pos, vertex_count * 2, "meshdata.vertexColours")?;
        rt7_skip(data, pos, vertex_count, "meshdata.vertexAlpha")?;
    }
    if has_face_bones {
        rt7_skip(data, pos, vertex_count * 2, "meshdata.vertexFacebones")?;
    }
    for _ in 0..mesh_count {
        let _ = rt7_g1(data, pos, "render.groupFlags")?;
        rt7_skip(data, pos, 4, "render.unkint")?;
        add_rt7_material_id(
            material_ids,
            rt7_g2le(data, pos, "render.materialArgument")?,
        );
        let _ = rt7_g1(data, pos, "render.unkbyte2")?;
        let index_count = usize::from(rt7_g2le(data, pos, "render.buf.length")?);
        rt7_skip(
            data,
            pos,
            index_count * if vertex_count <= 0xFFFF { 2 } else { 4 },
            "render.buf",
        )?;
    }
    Ok(())
}

fn add_rt7_material_id(material_ids: &mut BTreeSet<u32>, material_argument: u16) {
    if material_argument > 0 {
        material_ids.insert(u32::from(material_argument - 1));
    }
}

fn rt7_g1(data: &[u8], pos: &mut usize, field: &str) -> Result<u8> {
    ensure!(
        *pos < data.len(),
        "{field} exceeds RT7 model length at {pos} >= {}",
        data.len()
    );
    let value = data[*pos];
    *pos += 1;
    Ok(value)
}

fn rt7_g2le(data: &[u8], pos: &mut usize, field: &str) -> Result<u16> {
    ensure!(
        pos.saturating_add(2) <= data.len(),
        "{field} exceeds RT7 model length at {} + 2 > {}",
        *pos,
        data.len()
    );
    let value = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    Ok(value)
}

fn rt7_g4le(data: &[u8], pos: &mut usize, field: &str) -> Result<u32> {
    ensure!(
        pos.saturating_add(4) <= data.len(),
        "{field} exceeds RT7 model length at {} + 4 > {}",
        *pos,
        data.len()
    );
    let value = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    Ok(value)
}

fn rt7_skip(data: &[u8], pos: &mut usize, count: usize, field: &str) -> Result<()> {
    ensure!(
        pos.saturating_add(count) <= data.len(),
        "{field} exceeds RT7 model length at {} + {} > {}",
        *pos,
        count,
        data.len()
    );
    *pos += count;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        OverlayBlockedIssue, OverlayWarning, RefGraphRepository, build_overlay_proof,
        classify_warning_issue, normalize_archive_key, write_overlay_plan_audit,
    };
    use std::fs;

    #[test]
    fn normalizes_archive_key() {
        assert_eq!(
            normalize_archive_key("textures.png.mipped"),
            "texturespngmipped"
        );
        assert_eq!(normalize_archive_key("vfx2"), "vfx2");
    }

    #[test]
    fn fallback_warning_is_heuristic_gap() {
        let issue = classify_warning_issue(
            1,
            &OverlayWarning {
                kind: "risk".to_string(),
                message:
                    "refs/loc.json has no entry for loc_1; falling back to donor binary dependency scan where supported."
                        .to_string(),
                ref_kind: Some("loc".to_string()),
                archive: None,
                id: None,
            },
        )
        .expect("proof issue");
        assert_eq!(issue.kind, "heuristic");
        assert_eq!(issue.location, "warning[1]");
    }

    #[test]
    fn heuristic_gap_blocks_when_not_allowed() {
        let proof = build_overlay_proof(
            &[OverlayWarning {
                kind: "risk".to_string(),
                message: "refs/loc.json missing under semantic root; run cache:semantic:sync-947."
                    .to_string(),
                ref_kind: Some("loc".to_string()),
                archive: None,
                id: None,
            }],
            &[],
            super::ProofState {
                script_checked: 0,
                script_blocked: 0,
                script_valid: 0,
                component_checked: 0,
                component_blocked: 0,
                component_valid: 0,
                blockers: Vec::new(),
            },
            false,
            910,
            947,
        );
        assert_eq!(proof.status, "blocked");
        assert_eq!(proof.heuristic_site_count, 1);
    }

    #[test]
    fn heuristic_gap_is_allowed_when_enabled() {
        let proof = build_overlay_proof(
            &[OverlayWarning {
                kind: "risk".to_string(),
                message: "refs/loc.json missing under semantic root; run cache:semantic:sync-947."
                    .to_string(),
                ref_kind: Some("loc".to_string()),
                archive: None,
                id: None,
            }],
            &[],
            super::ProofState {
                script_checked: 0,
                script_blocked: 0,
                script_valid: 0,
                component_checked: 0,
                component_blocked: 0,
                component_valid: 0,
                blockers: Vec::new(),
            },
            true,
            910,
            947,
        );
        assert_eq!(proof.status, "ok");
        assert_eq!(proof.heuristic_site_count, 1);
        assert!(!proof.strict);
    }

    #[test]
    fn audit_writes_expected_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let proof = build_overlay_proof(
            &[],
            &[OverlayBlockedIssue {
                kind: "conflict".to_string(),
                archive: None,
                archive_id: None,
                group_id: None,
                file_id: None,
                id: Some(7),
                ref_kind: Some("varbit".to_string()),
                message: "varbit_7 differs".to_string(),
            }],
            super::ProofState {
                script_checked: 1,
                script_blocked: 1,
                script_valid: 0,
                component_checked: 0,
                component_blocked: 0,
                component_valid: 0,
                blockers: vec![super::OverlayProofIssue {
                    kind: "unsupported",
                    location: "script_42".to_string(),
                    ref_kind: Some("script".to_string()),
                    message: "script_42 target validation failed".to_string(),
                }],
            },
            false,
            910,
            947,
        );

        let audit = write_overlay_plan_audit(temp.path(), &proof, &[]).expect("write audit");
        assert!(audit.relative_paths.contains(&"summary.json".to_string()));
        assert!(temp.path().join("summary.json").is_file());
        assert!(temp.path().join("unsupported_sites.jsonl").is_file());
        assert!(temp.path().join("scripts_failed.jsonl").is_file());
    }

    #[test]
    fn manifest_deserializes_script_and_interface_imports() {
        let manifest: super::CacheOverlayManifest = serde_json::from_value(serde_json::json!({
            "roots": {
                "baseRawRoot": "/tmp/base-raw",
                "donorRawRoot": "/tmp/donor-raw",
                "baseSemanticRoot": "/tmp/base-semantic",
                "donorSemanticRoot": "/tmp/donor-semantic",
                "basePackRoot": "/tmp/base-pack",
                "outputPackRoot": "/tmp/output-pack",
                "clientOutputPackRoot": "/tmp/client-pack"
            },
            "imports": {
                "interfaces": [1213, 1218],
                "scripts": [548, 5690]
            }
        }))
        .expect("manifest");

        assert_eq!(manifest.imports.interfaces, vec![1213, 1218]);
        assert_eq!(manifest.imports.scripts, vec![548, 5690]);
    }

    #[test]
    fn ref_repository_keeps_empty_entries_and_loads_varp_kind() {
        let temp = tempfile::tempdir().expect("tempdir");
        let refs = temp.path().join("refs");
        fs::create_dir_all(&refs).expect("refs dir");
        fs::write(refs.join("bas.json"), "{\n  \"1159\": {}\n}\n").expect("bas refs");
        fs::write(
            refs.join("varp.json"),
            "{\n  \"player\": {\n    \"42\": {\"varbit\": [7]}\n  }\n}\n",
        )
        .expect("varp refs");

        let repo = RefGraphRepository::new(temp.path()).expect("repo");
        assert!(repo.has_kind("bas"));
        assert!(repo.get_refs("bas", 1159).is_some());
        assert!(repo.has_kind("varp"));
        assert_eq!(
            repo.get_refs("varp", 42)
                .and_then(|refs| refs.get(&super::SemanticRefKey::VarBit))
                .expect("varp refs"),
            &vec![7]
        );
    }
}
