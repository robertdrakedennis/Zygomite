//! The `overlay-plan` planning algorithm (concern 4): `PlanBuilder` and its
//! `&mut self` driver methods, the ref-walk/selection/proof free functions, and
//! the archive descriptors + binary scanners they use.
//!
//! Moved verbatim from the former flat `overlay_plan.rs`.

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
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use super::{MAX_PLAN_WARNINGS, OVERLAY_PLAN_VERSION, ProofState};
use super::manifest::{ArchiveMode, ArchiveRef, CacheOverlayManifest, OverlayRoots, RegionSpec};
use super::plan_output::{
    DependencyEdgeSample, OverlayBlockedIssue, OverlayPlanArchiveFiles, OverlayPlanArchiveGroups,
    OverlayPlanAudit, OverlayPlanDb, OverlayPlanImports, OverlayPlanOutput, OverlayPlanProof,
    OverlayProofIssue, OverlayProofSummary, OverlayPlanSelected, OverlaySemanticManifest,
    OverlayWarning, Rs3CacheManifest,
};
use super::refs::{
    ArchiveDef, ConfigSemanticIndex, ConfigTarget, PendingRef, RawGroupTarget, RefKind, RootKind,
    SelectionMode, SemanticRefKey, normalize_graph_ref_kind, semantic_refs_file_name,
};

pub type ConfigGroupFileCache = HashMap<(RootKind, u32, u32), Option<BTreeMap<u32, Vec<u8>>>>;
/// The `(loc_ids, npc_ids)` extracted from one donor map group, or `None` when the
/// group is absent.
pub type MapGroupLocNpcIds = Option<(Vec<u32>, Vec<u32>)>;
/// One scanned map group: its id paired with its extracted ids.
pub type MapGroupRef = (u32, MapGroupLocNpcIds);
pub type MapGroupRefs = Vec<MapGroupRef>;

pub struct PlanBuilder<'a> {
    pub manifest: CacheOverlayManifest,
    pub roots: OverlayRoots,
    pub base_cache: FlatCache,
    pub donor_cache: FlatCache,
    pub data_dir: &'a Path,
    pub base_build: u32,
    pub donor_build: u32,
    pub base_subbuild: u32,
    pub donor_subbuild: u32,
    pub group_selections: BTreeMap<u32, BTreeSet<u32>>,
    pub file_selections: BTreeMap<(u32, u32), BTreeSet<u32>>,
    pub primary_maps: BTreeSet<u32>,
    pub primary_objs: BTreeSet<u32>,
    pub primary_npcs: BTreeSet<u32>,
    pub primary_locs: BTreeSet<u32>,
    pub primary_structs: BTreeSet<u32>,
    pub primary_enums: BTreeSet<u32>,
    pub primary_varbits: BTreeSet<u32>,
    pub primary_varps: BTreeSet<u32>,
    pub primary_db_tables: BTreeSet<u32>,
    pub primary_db_rows: BTreeSet<u32>,
    pub primary_interfaces: BTreeSet<u32>,
    pub primary_scripts: BTreeSet<u32>,
    pub dependencies: BTreeMap<RefKind, BTreeSet<u32>>,
    pub warnings: Vec<OverlayWarning>,
    pub blocked: Vec<OverlayBlockedIssue>,
    pub pending: VecDeque<PendingRef>,
    pub seen_refs: HashSet<(RefKind, u32)>,
    pub indexes: HashMap<(RootKind, u32), Option<ArchiveIndex>>,
    pub config_group_files: ConfigGroupFileCache,
    pub full_archive_selections: BTreeSet<u32>,
    pub auto_allowed_missing_varbits: BTreeSet<u32>,
    pub auto_allowed_missing_varps: BTreeSet<u32>,
    pub semantic_index: ConfigSemanticIndex,
    pub db_schema_changes: BTreeSet<u32>,
    pub warning_overflow: usize,
    pub proof: ProofState,
    pub analyzer: Option<MigrationAnalyzer>,
    pub base_manifest: Rs3CacheManifest,
    pub donor_manifest: Rs3CacheManifest,
}

impl PlanBuilder<'_> {
    pub fn queue(&mut self, kind: RefKind, id: u32, source: impl Into<String>, mode: SelectionMode) {
        if self.seen_refs.insert((kind, id)) {
            self.pending.push_back(PendingRef {
                kind,
                id,
                source: source.into(),
                mode,
            });
        }
    }

    pub fn add_dependency(&mut self, kind: RefKind, id: u32) {
        self.dependencies.entry(kind).or_default().insert(id);
    }

    pub fn add_group(&mut self, archive: &ArchiveDef, group_id: u32) {
        self.group_selections
            .entry(archive.id)
            .or_default()
            .insert(group_id);
    }

    pub fn add_file(&mut self, archive: &ArchiveDef, group_id: u32, file_id: u32) {
        self.file_selections
            .entry((archive.id, group_id))
            .or_default()
            .insert(file_id);
    }

    pub fn add_warning(&mut self, warning: OverlayWarning) {
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

    pub fn add_blocked(&mut self, issue: OverlayBlockedIssue) {
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

    pub fn semantic_donor_root(&self) -> &Path {
        Path::new(&self.roots.donor_semantic_root)
    }

    pub fn get_index(
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

    pub fn read_raw_group(
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

    pub fn read_group_files(
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

    pub fn cached_file_bytes(
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

    pub fn read_file_bytes(
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

    pub fn analyzer(&mut self) -> Result<&MigrationAnalyzer> {
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

pub fn seed_imports(builder: &mut PlanBuilder<'_>) -> Result<()> {
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
        builder.primary_maps.extend(donor_index.group_id);
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
    for id in builder.manifest.imports.spots.clone() {
        builder.queue(RefKind::Spot, id, "manifest", SelectionMode::Primary);
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

pub fn seed_full_archive_selection(
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
    builder
        .group_selections
        .entry(archive.id)
        .or_default()
        .extend(donor_index.group_id);
    Ok(())
}

pub fn build_selections(builder: &mut PlanBuilder<'_>, allow_heuristic_sites: bool) -> Result<()> {
    let map_groups = builder.primary_maps.iter().copied().collect::<Vec<_>>();
    if !map_groups.is_empty() {
        process_map_groups(builder, &map_groups)?;
    }
    while let Some(reference) = builder.pending.pop_front() {
        process_ref(builder, &reference, allow_heuristic_sites)?;
    }
    Ok(())
}

pub fn process_map_groups(builder: &mut PlanBuilder<'_>, map_groups: &[u32]) -> Result<()> {
    let archive = archive_maps();
    let donor_index = builder
        .get_index(RootKind::Donor, &archive)?
        .with_context(|| {
            format!(
                "Donor map archive index missing under {}",
                builder.roots.donor_raw_root
            )
        })?;
    let scanned = if let Some(cache_path) = map_refs_cache_path(&builder.donor_manifest, map_groups)
        && let Ok(bytes) = fs::read(&cache_path)
        && let Ok(scanned) = serde_json::from_slice::<MapGroupRefs>(&bytes)
    {
        scanned
    } else {
        let donor_cache = builder.donor_cache.clone();
        let scanned = map_groups
            .par_iter()
            .map(|&group_id| scan_map_group_refs(&donor_cache, &donor_index, &archive, group_id))
            .collect::<Result<MapGroupRefs>>()?;
        if let Some(cache_path) = map_refs_cache_path(&builder.donor_manifest, map_groups) {
            if let Some(parent) = cache_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&cache_path, serde_json::to_vec(&scanned)?)
                .with_context(|| format!("writing map refs cache {}", cache_path.display()))?;
        }
        scanned
    };
    let mut selected_groups = Vec::with_capacity(scanned.len());
    for (group_id, refs) in scanned {
        let Some((loc_ids, npc_ids)) = refs else {
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
            continue;
        };
        selected_groups.push(group_id);
        let source = format!("map_{group_id}");
        for loc_id in loc_ids {
            builder.queue(
                RefKind::Loc,
                loc_id,
                source.clone(),
                SelectionMode::Dependency,
            );
        }
        for npc_id in npc_ids {
            builder.queue(
                RefKind::Npc,
                npc_id,
                source.clone(),
                SelectionMode::Dependency,
            );
        }
    }
    builder
        .group_selections
        .entry(archive.id)
        .or_default()
        .extend(selected_groups);
    Ok(())
}

pub fn map_refs_cache_path(donor_manifest: &Rs3CacheManifest, map_groups: &[u32]) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    OVERLAY_PLAN_VERSION.hash(&mut hasher);
    manifest_fingerprint(donor_manifest).hash(&mut hasher);
    map_groups.hash(&mut hasher);
    Some(
        PathBuf::from(home)
            .join(".cache/alerion/rs3-cache-rs-map-refs")
            .join(format!("{:016x}.json", hasher.finish())),
    )
}

pub fn scan_map_group_refs(
    donor_cache: &FlatCache,
    donor_index: &ArchiveIndex,
    archive: &ArchiveDef,
    group_id: u32,
) -> Result<MapGroupRef> {
    if !is_group_present(donor_index, group_id) {
        return Ok((group_id, None));
    }
    let files = donor_cache
        .group_files_with_index(donor_index, archive.id, group_id)
        .with_context(|| format!("reading donor map group {group_id}"))?;
    let loc_ids = match files.get(&0) {
        Some(file) => parse_loc_ids(file)?,
        None => Vec::new(),
    };
    let npc_ids = match files.get(&2) {
        Some(file) => parse_npc_ids(file)?,
        None => Vec::new(),
    };
    Ok((group_id, Some((loc_ids, npc_ids))))
}

pub fn process_ref(
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
            let target = config_target(RefKind::Enum, reference.id);
            process_gated_config_ref(builder, reference, &target, "enum")?;
        }
        RefKind::VarBit => {
            let target = config_target(RefKind::VarBit, reference.id);
            process_gated_config_ref(builder, reference, &target, "varbit")?;
        }
        RefKind::Varp => {
            let target = config_target(RefKind::Varp, reference.id);
            process_gated_config_ref(builder, reference, &target, "varp")?;
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

pub fn process_covered_ref_dependencies(
    builder: &mut PlanBuilder<'_>,
    reference: &PendingRef,
) -> Result<()> {
    if reference.mode != SelectionMode::Dependency {
        return Ok(());
    }
    match reference.kind {
        RefKind::Loc => {
            scan_config_dependencies(builder, "loc", reference.id, &reference.source)?;
            let _ =
                scan_binary_multivar_dependencies(builder, "loc", reference.id, &reference.source)?;
            Ok(())
        }
        RefKind::Npc => {
            scan_config_dependencies(builder, "npc", reference.id, &reference.source)?;
            let _ =
                scan_binary_multivar_dependencies(builder, "npc", reference.id, &reference.source)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

pub fn ref_covered_by_full_archive(builder: &PlanBuilder<'_>, reference: &PendingRef) -> Result<bool> {
    Ok(full_archive_for_ref(builder, reference)?
        .is_some_and(|archive| builder.full_archive_selections.contains(&archive.id)))
}

pub fn full_archive_for_ref(
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

pub fn process_config_ref(
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

pub fn process_shallow_config_ref(builder: &mut PlanBuilder<'_>, target: &ConfigTarget) -> Result<()> {
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

pub fn process_gated_config_ref(
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
    if state == CompareState::Same {
        queue_varbit_base_varp_dependency(builder, target, &reference.source)?;
        return scan_config_dependencies(builder, semantic_kind, target.id, &reference.source);
    }

    if state == CompareState::MissingTarget {
        if !is_allowed_missing_target_id(builder, target.kind, target.id) {
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

    if !is_allowed_conflict_id(builder, target.kind, target.id) {
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

pub fn process_struct_ref(
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

pub fn process_raw_group_ref(builder: &mut PlanBuilder<'_>, target: &RawGroupTarget) -> Result<()> {
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

pub fn scan_raw_group_dependencies(
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

pub fn process_dbtable_ref(builder: &mut PlanBuilder<'_>, table_id: u32) -> Result<()> {
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

pub fn process_dbrow_ref(builder: &mut PlanBuilder<'_>, row_id: u32) -> Result<()> {
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

pub fn scan_dbrow_dependencies(builder: &mut PlanBuilder<'_>, row_id: u32) {
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

pub fn scan_config_dependencies(
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

pub fn scan_refs_for_kind(
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

pub fn semantic_kind_allows_ref_kind(semantic_kind: &str, kind: RefKind) -> bool {
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

pub fn scan_binary_multivar_dependencies(
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

pub fn compare_file(
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

pub fn validate_donor_config_decodes(
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

pub fn queue_varbit_base_varp_dependency(
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

pub fn is_allowed_missing_target_id(builder: &PlanBuilder<'_>, kind: RefKind, id: u32) -> bool {
    match kind {
        RefKind::Enum => builder.manifest.allow.enum_ids.contains(&id),
        RefKind::VarBit => {
            builder.manifest.allow.varbit_ids.contains(&id)
                || builder.auto_allowed_missing_varbits.contains(&id)
        }
        RefKind::Varp => {
            builder.manifest.allow.varp_ids.contains(&id)
                || builder.auto_allowed_missing_varps.contains(&id)
        }
        _ => false,
    }
}

pub fn is_allowed_conflict_id(builder: &PlanBuilder<'_>, kind: RefKind, id: u32) -> bool {
    match kind {
        RefKind::Enum => builder.manifest.allow.enum_ids.contains(&id),
        RefKind::VarBit => builder.manifest.allow.varbit_conflict_ids.contains(&id),
        RefKind::Varp => builder.manifest.allow.varp_conflict_ids.contains(&id),
        _ => false,
    }
}

pub fn prove_script_ref(
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

pub fn prove_interface_ref(
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

pub fn script_validation_issue(
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

pub fn interface_validation_issue(
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

pub fn select_script_bytes(
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

pub fn select_interface_group(builder: &mut PlanBuilder<'_>, interface_id: u32) -> Result<()> {
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

pub fn queue_supported_report_entities(
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

pub fn is_supported_proof_entity(entity_type: &str) -> bool {
    matches!(
        entity_type,
        "varplayer" | "component" | "param" | "config" | "inv"
    )
}

pub fn finalize_plan(
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

pub fn build_overlay_proof(
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

pub fn classify_warning_issue(index: usize, warning: &OverlayWarning) -> Option<OverlayProofIssue> {
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

pub fn write_overlay_plan_audit(
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

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let encoded = serde_json::to_vec_pretty(value).context("encoding overlay plan json")?;
    fs::write(path, encoded).with_context(|| format!("writing {}", path.display()))
}

pub fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> Result<()> {
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

pub fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn resolve_roots(manifest: &CacheOverlayManifest) -> Result<OverlayRoots> {
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

pub fn absolutize(path: &Path) -> Result<String> {
    if path.is_absolute() {
        return Ok(path.display().to_string());
    }
    Ok(std::env::current_dir()?.join(path).display().to_string())
}

pub fn read_semantic_manifest(root: &Path) -> Result<Rs3CacheManifest> {
    let path = root.join(".rs3-cache-manifest.json");
    serde_json::from_slice(&fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("decoding {}", path.display()))
}

pub fn manifest_fingerprint(manifest: &Rs3CacheManifest) -> String {
    format!(
        "{}.{}:{}:{}",
        manifest.build, manifest.subbuild, manifest.cache_fingerprint, manifest.tool_version
    )
}

pub fn compare_mode(mode: ArchiveMode) -> &'static str {
    match mode {
        ArchiveMode::Auto => "patch",
        ArchiveMode::Patch => "patch",
        ArchiveMode::HardSwap => "hard-swap",
    }
}

pub fn resolve_archive_mode(manifest: &CacheOverlayManifest, archive: &ArchiveDef) -> &'static str {
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

pub fn lookup_archive_mode(
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

pub fn normalize_archive_key(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
}

pub fn archive_ref_key(archive_ref: &ArchiveRef) -> String {
    match archive_ref {
        ArchiveRef::Name(name) => normalize_archive_key(name),
        ArchiveRef::Id(id) => id.to_string(),
    }
}

pub fn archive_for_manifest_ref(value: &ArchiveRef) -> Result<ArchiveDef> {
    match value {
        ArchiveRef::Id(id) => archive_for_id(*id),
        ArchiveRef::Name(name) => archive_for_name(name),
    }
}

pub fn archive_for_name(name: &str) -> Result<ArchiveDef> {
    let normalized = normalize_archive_key(name);
    all_archives()
        .into_iter()
        .find(|archive| {
            normalize_archive_key(archive.name) == normalized
                || archive.id.to_string() == name.trim()
        })
        .with_context(|| format!("Unsupported archive reference {name}."))
}

pub fn archive_for_id(id: u32) -> Result<ArchiveDef> {
    all_archives()
        .into_iter()
        .find(|archive| archive.id == id)
        .with_context(|| format!("Unsupported archive id {id}."))
}

pub fn all_archives() -> Vec<ArchiveDef> {
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

pub fn archive_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_CONFIG,
        donor_id: ARCHIVE_CONFIG,
        name: "config",
    }
}
pub fn archive_interfaces() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_INTERFACES,
        donor_id: ARCHIVE_INTERFACES,
        name: "interfaces",
    }
}
pub fn archive_maps() -> ArchiveDef {
    ArchiveDef {
        id: 5,
        donor_id: 5,
        name: "mapsv2",
    }
}
pub fn archive_sprites() -> ArchiveDef {
    ArchiveDef {
        id: 8,
        donor_id: 8,
        name: "sprites",
    }
}
pub fn archive_clientscripts() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_CLIENTSCRIPTS,
        donor_id: ARCHIVE_CLIENTSCRIPTS,
        name: "scripts",
    }
}
pub fn archive_loc_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_LOC_CONFIG,
        donor_id: ARCHIVE_LOC_CONFIG,
        name: "loc.config",
    }
}
pub fn archive_npc_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_NPC_CONFIG,
        donor_id: ARCHIVE_NPC_CONFIG,
        name: "npc.config",
    }
}
pub fn archive_obj_config() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_OBJ_CONFIG,
        donor_id: ARCHIVE_OBJ_CONFIG,
        name: "obj.config",
    }
}
pub fn archive_materials() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_MATERIALS,
        donor_id: ARCHIVE_MATERIALS,
        name: "materials",
    }
}
pub fn archive_models_rt7() -> ArchiveDef {
    ArchiveDef {
        id: ARCHIVE_MODELS_RT7,
        donor_id: ARCHIVE_MODELS_RT7,
        name: "modelsrt7",
    }
}
pub fn archive_anims_rt7() -> ArchiveDef {
    ArchiveDef {
        id: 48,
        donor_id: 48,
        name: "animsrt7",
    }
}
pub fn archive_dbtable_index() -> ArchiveDef {
    ArchiveDef {
        id: 49,
        donor_id: 49,
        name: "dbtableindex",
    }
}
pub fn archive_anim_keyframes() -> ArchiveDef {
    ArchiveDef {
        id: 56,
        donor_id: 56,
        name: "anims.keyframes",
    }
}

pub fn config_target(kind: RefKind, id: u32) -> ConfigTarget {
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

pub fn resolve_model_target(builder: &PlanBuilder<'_>, id: u32) -> RawGroupTarget {
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
pub enum CompareState {
    MissingDonor,
    MissingTarget,
    Same,
    Conflict,
}

pub fn missing_config(builder: &mut PlanBuilder<'_>, target: &ConfigTarget) {
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

pub fn is_group_present(index: &ArchiveIndex, group_id: u32) -> bool {
    index.group_id.binary_search(&group_id).is_ok()
}

pub fn parse_loc_ids(data: &[u8]) -> Result<Vec<u32>> {
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

pub fn parse_npc_ids(data: &[u8]) -> Result<Vec<u32>> {
    let mut ids = Vec::new();
    let mut buf = Packet::new(data);
    while buf.len().saturating_sub(buf.pos()) >= 4 {
        let _ = buf.g2()?;
        ids.push(u32::from(buf.g2()?));
    }
    Ok(ids)
}

pub fn skip_packet(buf: &mut Packet<'_>, count: usize) -> Result<()> {
    let next = buf
        .pos()
        .checked_add(count)
        .context("packet skip overflow")?;
    Ok(buf.set_pos(next)?)
}

#[derive(Default)]
pub struct MultivarRefs {
    pub varbit: Option<u32>,
    pub varp: Option<u32>,
}

pub fn scan_multivar_refs(ops: &[String]) -> MultivarRefs {
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

pub fn parse_first_u32(input: &str) -> Option<u32> {
    input
        .split([',', ' '])
        .next()
        .and_then(|value| value.parse::<u32>().ok())
}

pub fn semantic_kind_to_ref_kind(kind: &str) -> RefKind {
    if kind == "spotanim" {
        RefKind::Spot
    } else {
        normalize_graph_ref_kind(&SemanticRefKey::from_label(kind)).unwrap_or(RefKind::Struct)
    }
}

pub fn db_row_dependency_kinds(table_id: u32) -> HashSet<RefKind> {
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

pub fn normalize_region(region: RegionSpec) -> Result<u32> {
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

pub fn pack_map_square_group_id(region_x: u32, region_z: u32) -> u32 {
    region_x | (region_z << 7)
}

pub fn selected_archive_ids(builder: &PlanBuilder<'_>) -> Vec<u32> {
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

pub fn group_selections_for_report(builder: &PlanBuilder<'_>) -> Result<Vec<OverlayPlanArchiveGroups>> {
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

pub fn file_selections_for_report(builder: &PlanBuilder<'_>) -> Result<Vec<OverlayPlanArchiveFiles>> {
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

pub fn dependencies_for_report(builder: &PlanBuilder<'_>) -> BTreeMap<String, Vec<u32>> {
    let mut out = BTreeMap::new();
    for (kind, ids) in &builder.dependencies {
        out.insert(kind.as_str().to_string(), ids.iter().copied().collect());
    }
    out
}

pub fn sorted_set(values: Option<&BTreeSet<u32>>) -> Vec<u32> {
    values
        .map(|set| set.iter().copied().collect())
        .unwrap_or_default()
}

pub fn sorted_iter(values: impl Iterator<Item = u32>) -> Vec<u32> {
    let mut out = values.collect::<Vec<_>>();
    out.sort_unstable();
    out
}

pub fn dependency_site_label(site: &DependencySite) -> String {
    format!(
        "{}_{} at {}",
        site.entity_type.as_label(),
        site.id,
        site.location
    )
}

pub fn scan_rt7_model_material_ids(data: &[u8]) -> Result<Vec<u32>> {
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

pub fn scan_rt7_legacy_meshes(
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

pub fn scan_rt7_shared_mesh(
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

pub fn add_rt7_material_id(material_ids: &mut BTreeSet<u32>, material_argument: u16) {
    if material_argument > 0 {
        material_ids.insert(u32::from(material_argument - 1));
    }
}

pub fn rt7_g1(data: &[u8], pos: &mut usize, field: &str) -> Result<u8> {
    ensure!(
        *pos < data.len(),
        "{field} exceeds RT7 model length at {pos} >= {}",
        data.len()
    );
    let value = data[*pos];
    *pos += 1;
    Ok(value)
}

pub fn rt7_g2le(data: &[u8], pos: &mut usize, field: &str) -> Result<u16> {
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

pub fn rt7_g4le(data: &[u8], pos: &mut usize, field: &str) -> Result<u32> {
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

pub fn rt7_skip(data: &[u8], pos: &mut usize, count: usize, field: &str) -> Result<()> {
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
