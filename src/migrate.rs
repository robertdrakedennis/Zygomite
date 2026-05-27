#![allow(clippy::ref_option)]

use crate::config::ScalarValue;
use crate::dep_tree::{EntityKey, EntityType, ResolverContext};
use crate::interface::{ComponentDeps, VarTransmitRef};
use crate::overlay_deps::{
    DependencyConfidence, DependencySite, collect_component_dependency_sites,
    collect_script_dependency_sites,
};
use crate::script::{CompiledScript, Operand, decode_script, encode_script};
use crate::transpile::ScriptCatalog;
use crate::validate::{Cs2Validator, ValidationError, extend_validation_catalog};
use crate::vars::VarDomain;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

// ── Migration conflict report structures ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConflictStatus {
    Safe,
    Missing,
    IdConflict,
    Changed,
    ScriptChanged,
    Unknown,
    /// Asset (model, graphic, cursor, texture, etc.) — tracked but
    /// cannot be deeply compared without loading the archive content.
    Asset,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictEntry {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: ConflictStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diffs: Option<Vec<FieldDiff>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldDiff {
    pub field: String,
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictReport {
    pub source_build: u32,
    pub target_build: u32,
    pub interface_group: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,
    pub total_components: usize,
    pub total_entities: usize,
    pub summary: ConflictSummary,
    pub entities: Vec<ConflictEntry>,
    /// Present when --remap is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remap: Option<RemapTable>,
    /// Present when --remap is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_updates: Option<Vec<ReferenceUpdate>>,
    /// Present when --remap is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allocation: Option<AllocationInfo>,
    /// Present when target compile validation runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_validation: Option<TargetValidationReport>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConflictSummary {
    pub safe: usize,
    pub missing: usize,
    pub id_conflict: usize,
    pub changed: usize,
    pub script_changed: usize,
    pub unknown: usize,
    /// Assets (models, graphics, cursors, textures, fonts, stylesheets)
    /// that were tracked but cannot be deeply compared.
    pub asset: usize,
}

/// A conflict report for a single script and its transitive dependencies.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptReport {
    pub source_build: u32,
    pub target_build: u32,
    pub script_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_name: Option<String>,
    pub total_entities: usize,
    pub summary: ConflictSummary,
    pub entities: Vec<ConflictEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remap: Option<RemapTable>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_updates: Option<Vec<ReferenceUpdate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allocation: Option<AllocationInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_validation: Option<TargetValidationReport>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TargetValidationSummary {
    pub components_checked: usize,
    pub components_blocked: usize,
    pub scripts_checked: usize,
    pub scripts_encoded: usize,
    pub scripts_valid: usize,
    pub scripts_with_errors: usize,
    pub scripts_with_warnings: usize,
    pub scripts_blocked: usize,
    pub dependency_sites: usize,
    pub exact_sites: usize,
    pub heuristic_sites: usize,
    pub unsupported_sites: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetValidationReport {
    pub target_build: u32,
    pub remap_applied: bool,
    pub summary: TargetValidationSummary,
    pub components: Vec<ComponentTargetValidation>,
    pub scripts: Vec<ScriptTargetValidation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentTargetValidation {
    pub component_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub dependency_sites: usize,
    pub heuristic_sites: Vec<DependencySite>,
    pub unsupported_sites: Vec<DependencySite>,
    pub blocking_issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptTargetValidation {
    pub source_script_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_packed_id: Option<u32>,
    pub target_script_id: u32,
    pub target_packed_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoded_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    pub dependency_sites: usize,
    pub heuristic_sites: Vec<DependencySite>,
    pub unsupported_sites: Vec<DependencySite>,
    pub blockers: Vec<String>,
    pub reference_updates: Vec<RefUpdateEntry>,
    pub validation_errors: Vec<ValidationError>,
    pub validation_warnings: Vec<String>,
}

// ── Remap planning types ──

/// Maps old (source) IDs to new (target) IDs for entities that need shifting.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RemapTable {
    /// `script_id` → `new_script_id`
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub scripts: BTreeMap<u32, u32>,
    /// "domain:id" → { domain, id }
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub varps: BTreeMap<String, VarpRemapTarget>,
    /// `varbit_id` → `new_varbit_id`
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub varbits: BTreeMap<u32, u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VarpRemapTarget {
    pub domain: String,
    pub id: u32,
}

/// Describes one reference that needs updating after ID shifts.
#[derive(Debug, Clone, Serialize)]
pub struct ReferenceUpdate {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub id: u32,
    pub updates: Vec<RefUpdateEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefUpdateEntry {
    /// Human-readable location of the reference (e.g. "instruction[3]").
    pub location: String,
    /// Old reference value.
    pub from: String,
    /// New reference value.
    pub to: String,
}

/// Describes where free IDs were sourced from in the target build.
#[derive(Debug, Clone, Serialize)]
pub struct AllocationInfo {
    pub scripts: RangeAlloc,
    pub varps_player: RangeAlloc,
    pub varps_npc: RangeAlloc,
    pub varps_client: RangeAlloc,
    pub varps_world: RangeAlloc,
    pub varps_region: RangeAlloc,
    pub varps_object: RangeAlloc,
    pub varps_clan: RangeAlloc,
    pub varps_clan_setting: RangeAlloc,
    pub varps_controller: RangeAlloc,
    pub varps_global: RangeAlloc,
    pub varps_player_group: RangeAlloc,
    pub varbits: RangeAlloc,
}

#[derive(Debug, Clone, Serialize)]
pub struct RangeAlloc {
    pub target_max: u32,
    pub allocated_from: u32,
    pub count: usize,
}

impl AllocationInfo {
    fn new() -> Self {
        Self {
            scripts: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_player: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_npc: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_client: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_world: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_region: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_object: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_clan: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_clan_setting: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_controller: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_global: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varps_player_group: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
            varbits: RangeAlloc {
                target_max: 0,
                allocated_from: 0,
                count: 0,
            },
        }
    }
}

// ── Helpers ──

fn var_transmit_to_entity(var_ref: &VarTransmitRef) -> (EntityType, u32) {
    match var_ref {
        VarTransmitRef::Player(id) => (EntityType::VarPlayer, *id),
        VarTransmitRef::Npc(id) => (EntityType::VarNpc, *id),
        VarTransmitRef::Client(id) => (EntityType::VarClient, *id),
        VarTransmitRef::World(id) => (EntityType::VarWorld, *id),
        VarTransmitRef::Region(id) => (EntityType::VarRegion, *id),
        VarTransmitRef::Object(id) => (EntityType::VarObject, *id),
        VarTransmitRef::Clan(id) => (EntityType::VarClan, *id),
        VarTransmitRef::ClanSetting(id) => (EntityType::VarClanSetting, *id),
        VarTransmitRef::Controller(id) => (EntityType::VarController, *id),
        VarTransmitRef::Global(id) => (EntityType::VarGlobal, *id),
        VarTransmitRef::PlayerGroup(id) => (EntityType::VarPlayerGroup, *id),
        VarTransmitRef::VarClientString(id) => (EntityType::VarClient, *id),
    }
}

fn push_diff<T: std::fmt::Display + PartialEq>(
    diffs: &mut Vec<FieldDiff>,
    field: &str,
    source: &T,
    target: &T,
) {
    if source != target {
        diffs.push(FieldDiff {
            field: field.to_string(),
            source: source.to_string(),
            target: target.to_string(),
        });
    }
}

fn push_diff_opt<T: std::fmt::Debug + PartialEq>(
    diffs: &mut Vec<FieldDiff>,
    field: &str,
    source: &Option<T>,
    target: &Option<T>,
) {
    if source != target {
        diffs.push(FieldDiff {
            field: field.to_string(),
            source: format!("{source:?}"),
            target: format!("{target:?}"),
        });
    }
}

/// Returns `(pop_count, asset_entity_type)` for asset-related commands.
/// Returns `(0, None)` for non-asset or unknown commands.
fn asset_command_info(cmd: &str) -> (usize, Option<EntityType>) {
    // Commands that consume asset IDs from the stack.
    // Returns (total_pops, Some(asset_type)) for asset commands.
    // The last-popped (first-pushed) value is the asset reference.
    if cmd.contains("model") {
        if cmd.contains("angle")
            || cmd.contains("zoom")
            || cmd.contains("xof")
            || cmd.contains("yof")
        {
            return (0, None);
        }
        (2, Some(EntityType::Model)) // pops (comp_id, model_id)
    } else if cmd.contains("graphic") || cmd.contains("sprite") {
        (2, Some(EntityType::Graphic))
    } else if cmd.contains("cursor") {
        (2, Some(EntityType::Cursor))
    } else if cmd.contains("font") {
        (2, Some(EntityType::FontMetrics))
    } else if cmd.contains("texture") {
        (2, Some(EntityType::Texture))
    } else if cmd.contains("stylesheet") || cmd.contains("style") {
        (2, Some(EntityType::Stylesheet))
    } else if cmd.contains("seq") || cmd.contains("anim") {
        (2, Some(EntityType::Seq))
    } else {
        (0, None)
    }
}

fn alloc_for(alloc: &mut AllocationInfo, domain: VarDomain) -> &mut RangeAlloc {
    match domain {
        VarDomain::Player => &mut alloc.varps_player,
        VarDomain::Npc => &mut alloc.varps_npc,
        VarDomain::Client => &mut alloc.varps_client,
        VarDomain::World => &mut alloc.varps_world,
        VarDomain::Region => &mut alloc.varps_region,
        VarDomain::Object => &mut alloc.varps_object,
        VarDomain::Clan => &mut alloc.varps_clan,
        VarDomain::ClanSetting => &mut alloc.varps_clan_setting,
        VarDomain::Controller => &mut alloc.varps_controller,
        VarDomain::Global => &mut alloc.varps_global,
        VarDomain::PlayerGroup => &mut alloc.varps_player_group,
    }
}

fn non_empty(diffs: Vec<FieldDiff>) -> Option<Vec<FieldDiff>> {
    if diffs.is_empty() { None } else { Some(diffs) }
}

/// Shared pattern for all compare methods: returns (SAFE, None) or
/// (`changed_status`, `diffs`) when both source and target exist.
fn compare_pair<S, T>(
    source: Option<&S>,
    target: Option<&T>,
    build_diffs: impl FnOnce(&S, &T) -> Vec<FieldDiff>,
    changed_status: ConflictStatus,
) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
    match (source, target) {
        (Some(_), None) => (ConflictStatus::Missing, None),
        (None, Some(_)) => (ConflictStatus::IdConflict, None),
        (None, None) => (ConflictStatus::Missing, None),
        (Some(s), Some(t)) => {
            let diffs = build_diffs(s, t);
            (
                if diffs.is_empty() {
                    ConflictStatus::Safe
                } else {
                    changed_status
                },
                non_empty(diffs),
            )
        }
    }
}

/// Accumulate conflict counts from a slice of entities.
fn accumulate_summary(entities: &[ConflictEntry]) -> ConflictSummary {
    let mut s = ConflictSummary::default();
    for e in entities {
        match e.status {
            ConflictStatus::Safe => s.safe += 1,
            ConflictStatus::Missing => s.missing += 1,
            ConflictStatus::IdConflict => s.id_conflict += 1,
            ConflictStatus::Changed => s.changed += 1,
            ConflictStatus::ScriptChanged => s.script_changed += 1,
            ConflictStatus::Unknown => s.unknown += 1,
            ConflictStatus::Asset => s.asset += 1,
        }
    }
    s
}

fn format_scalar_opt(v: &Option<ScalarValue>) -> String {
    match v {
        Some(ScalarValue::Int(i)) => i.to_string(),
        Some(ScalarValue::Long(l)) => l.to_string(),
        Some(ScalarValue::Str(s)) => s.clone(),
        None => "null".to_string(),
    }
}

// ── Analyzer ──

pub struct MigrationAnalyzer {
    source: ResolverContext,
    target: ResolverContext,
    source_script_catalog: ScriptCatalog,
    target_script_catalog: ScriptCatalog,
    /// Cached set of all interface component IDs in the source build.
    source_component_ids: HashSet<u32>,
    /// Cached set of all interface component IDs in the target build.
    target_component_ids: HashSet<u32>,
}

impl MigrationAnalyzer {
    pub fn new(source: ResolverContext, target: ResolverContext) -> Self {
        let empty_group_names = HashMap::<u32, String>::new();
        let source_script_catalog = crate::transpile::build_script_catalog(
            &source.scripts,
            &empty_group_names,
            &source.opcode_book,
            source.build,
        );
        let target_script_catalog = crate::transpile::build_script_catalog(
            &target.scripts,
            &empty_group_names,
            &target.opcode_book,
            target.build,
        );
        let source_component_ids: HashSet<u32> = source
            .parsed_components
            .values()
            .flat_map(|g| g.keys())
            .copied()
            .collect();
        let target_component_ids: HashSet<u32> = target
            .parsed_components
            .values()
            .flat_map(|g| g.keys())
            .copied()
            .collect();
        Self {
            source,
            target,
            source_script_catalog,
            target_script_catalog,
            source_component_ids,
            target_component_ids,
        }
    }

    pub fn analyze_interface(&self, group_id: u32) -> ConflictReport {
        let mut entities = Vec::new();
        let mut visited: HashSet<EntityKey> = HashSet::new();

        if let Some(comps) = self.source.parsed_components.get(&group_id) {
            for (&comp_id, comp_deps) in comps {
                self.collect_entity(
                    EntityType::Component,
                    comp_id,
                    comp_deps.name.clone(),
                    &mut entities,
                    &mut visited,
                );
                self.walk_component_deps(comp_deps, &mut entities, &mut visited);
            }
        }

        self.build_report(group_id, entities)
    }

    fn walk_component_deps(
        &self,
        comp_deps: &ComponentDeps,
        entities: &mut Vec<ConflictEntry>,
        visited: &mut HashSet<EntityKey>,
    ) {
        let mut script_ids = comp_deps.scripts.iter().copied().collect::<Vec<_>>();
        script_ids.sort_unstable();
        for script_id in script_ids {
            let key = EntityKey::new(EntityType::Script, script_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Script, script_id, None, entities, visited);
                self.walk_script(script_id, entities, visited);
            }
        }
        let mut var_refs = comp_deps.varps.iter().cloned().collect::<Vec<_>>();
        var_refs.sort_by_key(|var_ref| {
            let (entity_type, id) = var_transmit_to_entity(var_ref);
            (entity_type.as_label().to_string(), id)
        });
        for var_ref in var_refs {
            let (et, id) = var_transmit_to_entity(&var_ref);
            let key = EntityKey::new(et, id);
            if visited.insert(key) {
                let name = self
                    .source
                    .varps_by_domain
                    .get(&Self::var_ref_domain(&var_ref))
                    .and_then(|vars| vars.get(&id))
                    .map(|v| v.var_name.clone());
                self.collect_entity(et, id, name, entities, visited);
            }
        }
        for varbit_id in sorted_hashset_ids(&comp_deps.varbits) {
            let key = EntityKey::new(EntityType::VarBit, varbit_id);
            if visited.insert(key) {
                let name = self
                    .source
                    .varbits
                    .get(&varbit_id)
                    .map(|v| v.varbit_name.clone());
                self.collect_entity(EntityType::VarBit, varbit_id, name, entities, visited);
            }
        }
        for enum_id in sorted_hashset_ids(&comp_deps.enums) {
            if visited.insert(EntityKey::new(EntityType::Enum, enum_id)) {
                self.collect_entity(EntityType::Enum, enum_id, None, entities, visited);
            }
        }
        for param_id in sorted_hashset_ids(&comp_deps.params) {
            if visited.insert(EntityKey::new(EntityType::Param, param_id)) {
                self.collect_entity(EntityType::Param, param_id, None, entities, visited);
            }
        }
        for model_id in sorted_hashset_ids(&comp_deps.models) {
            if visited.insert(EntityKey::new(EntityType::Model, model_id)) {
                self.collect_entity(EntityType::Model, model_id, None, entities, visited);
            }
        }
        for seq_id in sorted_hashset_ids(&comp_deps.seqs) {
            if visited.insert(EntityKey::new(EntityType::Seq, seq_id)) {
                self.collect_entity(EntityType::Seq, seq_id, None, entities, visited);
            }
        }
        for graphic_id in sorted_hashset_ids(&comp_deps.graphics) {
            if visited.insert(EntityKey::new(EntityType::Graphic, graphic_id)) {
                self.collect_entity(EntityType::Graphic, graphic_id, None, entities, visited);
            }
        }
        for inv_id in sorted_hashset_ids(&comp_deps.invs) {
            if visited.insert(EntityKey::new(EntityType::Inv, inv_id)) {
                self.collect_entity(EntityType::Inv, inv_id, None, entities, visited);
            }
        }
        // Asset types tracked for completeness; cannot be deeply
        // compared without full archive loading.
        for cursor_id in sorted_hashset_ids(&comp_deps.cursors) {
            if visited.insert(EntityKey::new(EntityType::Cursor, cursor_id)) {
                self.collect_entity(EntityType::Cursor, cursor_id, None, entities, visited);
            }
        }
        for font_id in sorted_hashset_ids(&comp_deps.fontmetrics) {
            if visited.insert(EntityKey::new(EntityType::FontMetrics, font_id)) {
                self.collect_entity(EntityType::FontMetrics, font_id, None, entities, visited);
            }
        }
        for tex_id in sorted_hashset_ids(&comp_deps.textures) {
            if visited.insert(EntityKey::new(EntityType::Texture, tex_id)) {
                self.collect_entity(EntityType::Texture, tex_id, None, entities, visited);
            }
        }
        for ss_id in sorted_hashset_ids(&comp_deps.stylesheets) {
            if visited.insert(EntityKey::new(EntityType::Stylesheet, ss_id)) {
                self.collect_entity(EntityType::Stylesheet, ss_id, None, entities, visited);
            }
        }
        for stat_id in sorted_hashset_ids(&comp_deps.stats) {
            if visited.insert(EntityKey::new(EntityType::Config, stat_id)) {
                self.collect_entity(EntityType::Config, stat_id, None, entities, visited);
            }
        }
    }

    /// Decodes a script from its raw bytes on demand (lazy path).
    /// Checks the decoded cache first for already-decoded scripts.
    fn get_script(
        &self,
        ctx: &ResolverContext,
        script_catalog: &ScriptCatalog,
        script_id: u32,
    ) -> Option<CompiledScript> {
        let packed_id = self.resolve_script_packed_id(script_catalog, script_id)?;
        if let Some(script) = ctx.decoded_scripts.get(&packed_id) {
            return Some(script.clone());
        }
        // Lazy path: decode from raw bytes
        ctx.scripts
            .get(&packed_id)
            .and_then(|bytes| crate::script::decode_script(bytes, &ctx.opcode_book, ctx.build).ok())
    }

    fn resolve_script_packed_id(
        &self,
        script_catalog: &ScriptCatalog,
        script_id: u32,
    ) -> Option<u32> {
        script_catalog
            .resolve_call_target(script_id as i32)
            .map(|metadata| metadata.packed_id.0 as u32)
    }

    fn walk_script(
        &self,
        script_id: u32,
        entities: &mut Vec<ConflictEntry>,
        visited: &mut HashSet<EntityKey>,
    ) {
        if let Some(script) = self.get_script(&self.source, &self.source_script_catalog, script_id)
        {
            // ── Pass 1: operand-level deps (varps, varbits, script calls) ──
            for instruction in &script.code {
                match &instruction.operand {
                    crate::script::Operand::VarRef(var_ref) => {
                        let ref_entity = crate::dep_tree::var_ref_to_entity_ref(var_ref);
                        let key = EntityKey::new(ref_entity.entity_type, ref_entity.id);
                        if visited.insert(key) {
                            self.collect_entity(
                                ref_entity.entity_type,
                                ref_entity.id,
                                None,
                                entities,
                                visited,
                            );
                        }
                    }
                    crate::script::Operand::VarBitRef(vbr) => {
                        let id = u32::from(vbr.id);
                        if visited.insert(EntityKey::new(EntityType::VarBit, id)) {
                            self.collect_entity(EntityType::VarBit, id, None, entities, visited);
                        }
                    }
                    crate::script::Operand::Script(called_id) => {
                        let id = *called_id as u32;
                        if visited.insert(EntityKey::new(EntityType::Script, id)) {
                            self.collect_entity(EntityType::Script, id, None, entities, visited);
                            self.walk_script(id, entities, visited);
                        }
                    }
                    _ => {}
                }
            }

            // ── Pass 2: stack simulation for asset references ──
            // Track pushed int constants and record them as asset deps
            // when consumed by asset-related cc_/if_ commands.
            let mut stack: Vec<u32> = Vec::new();
            for instruction in &script.code {
                match &instruction.operand {
                    crate::script::Operand::Int(val)
                        if instruction.command == "push_constant_int" =>
                    {
                        stack.push(*val as u32);
                    }
                    _ => {
                        let name = &instruction.command;
                        let (pops, asset_type) = asset_command_info(name);
                        if let Some(at) = asset_type {
                            // Pop args from stack. The last popped value is the
                            // first pushed, which is the asset ID.
                            let mut popped = Vec::new();
                            for _ in 0..pops {
                                if let Some(v) = stack.pop() {
                                    popped.push(v);
                                }
                            }
                            // First pushed (last popped) = asset reference
                            if let Some(&asset_id) = popped.last() {
                                let key = EntityKey::new(at, asset_id);
                                if visited.insert(key) {
                                    self.collect_entity(at, asset_id, None, entities, visited);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn collect_entity(
        &self,
        entity_type: EntityType,
        id: u32,
        name: Option<String>,
        entities: &mut Vec<ConflictEntry>,
        _visited: &mut HashSet<EntityKey>,
    ) {
        let (status, diffs) = self.compare_entity(entity_type, id);
        let (source_summary, target_summary) = self.entity_summaries(entity_type, id);
        entities.push(ConflictEntry {
            entity_type: entity_type.as_label().to_string(),
            id,
            sub_id: None,
            name: name.or_else(|| self.lookup_name(entity_type, id)),
            status,
            source_summary,
            target_summary,
            diffs,
        });
    }

    fn compare_entity(
        &self,
        entity_type: EntityType,
        id: u32,
    ) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match entity_type {
            EntityType::VarPlayer
            | EntityType::VarNpc
            | EntityType::VarClient
            | EntityType::VarWorld
            | EntityType::VarRegion
            | EntityType::VarObject
            | EntityType::VarClan
            | EntityType::VarClanSetting
            | EntityType::VarController
            | EntityType::VarGlobal
            | EntityType::VarPlayerGroup => self.compare_varp(entity_type, id),
            EntityType::VarBit => self.compare_varbit(id),
            EntityType::Script => self.compare_script(id),
            EntityType::Enum => self.compare_enum(id),
            EntityType::Param => self.compare_param(id),
            EntityType::Seq => self.compare_seq(id),
            EntityType::Component => self.compare_component(id),
            EntityType::Inv => self.compare_inv(id),
            // Asset types — tracked for completeness, no deep comparison.
            EntityType::Model
            | EntityType::Graphic
            | EntityType::Cursor
            | EntityType::FontMetrics
            | EntityType::Texture
            | EntityType::Stylesheet
            | EntityType::Config => self.compare_asset(entity_type, id),
            _ => (ConflictStatus::Unknown, None),
        }
    }

    fn compare_asset(
        &self,
        _entity_type: EntityType,
        id: u32,
    ) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match (
            self.source_component_ids.contains(&id),
            self.target_component_ids.contains(&id),
        ) {
            (true, false) => (ConflictStatus::Missing, None),
            (false, true) => (ConflictStatus::IdConflict, None),
            _ => (ConflictStatus::Asset, None),
        }
    }

    fn compare_inv(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match (self.source.invs.get(&id), self.target.invs.get(&id)) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
                let mut diffs = Vec::new();
                push_diff_opt(&mut diffs, "size", &s.size, &t.size);
                push_diff(&mut diffs, "stock_count", &s.stocks.len(), &t.stocks.len());
                (
                    if diffs.is_empty() {
                        ConflictStatus::Safe
                    } else {
                        ConflictStatus::Changed
                    },
                    non_empty(diffs),
                )
            }
        }
    }

    fn compare_varp(
        &self,
        entity_type: EntityType,
        id: u32,
    ) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        let domain = Self::entity_type_to_domain(entity_type);
        let s = domain.and_then(|d| self.source.varps_by_domain.get(&d).and_then(|v| v.get(&id)));
        let t = domain.and_then(|d| self.target.varps_by_domain.get(&d).and_then(|v| v.get(&id)));
        compare_pair(
            s,
            t,
            |s, t| {
                let mut diffs = Vec::new();
                push_diff(&mut diffs, "name", &s.var_name, &t.var_name);
                push_diff_opt(&mut diffs, "type_id", &s.type_id, &t.type_id);
                push_diff_opt(&mut diffs, "lifetime", &s.lifetime, &t.lifetime);
                push_diff_opt(
                    &mut diffs,
                    "transmit_level",
                    &s.transmit_level,
                    &t.transmit_level,
                );
                push_diff_opt(&mut diffs, "client_code", &s.client_code, &t.client_code);
                push_diff(
                    &mut diffs,
                    "domain_default",
                    &s.domain_default,
                    &t.domain_default,
                );
                push_diff(&mut diffs, "wiki_sync", &s.wiki_sync, &t.wiki_sync);
                diffs
            },
            ConflictStatus::Changed,
        )
    }

    fn compare_varbit(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match (self.source.varbits.get(&id), self.target.varbits.get(&id)) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
                let mut diffs = Vec::new();
                push_diff(&mut diffs, "name", &s.varbit_name, &t.varbit_name);
                push_diff_opt(&mut diffs, "domain", &s.domain, &t.domain);
                push_diff_opt(&mut diffs, "base_var", &s.base_var, &t.base_var);
                push_diff_opt(&mut diffs, "start_bit", &s.start_bit, &t.start_bit);
                push_diff_opt(&mut diffs, "end_bit", &s.end_bit, &t.end_bit);
                push_diff(&mut diffs, "wiki_sync", &s.wiki_sync, &t.wiki_sync);
                (
                    if diffs.is_empty() {
                        ConflictStatus::Safe
                    } else {
                        ConflictStatus::Changed
                    },
                    non_empty(diffs),
                )
            }
        }
    }

    fn compare_script(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match (
            self.get_script(&self.source, &self.source_script_catalog, id),
            self.get_script(&self.target, &self.target_script_catalog, id),
        ) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
                let mut diffs = Vec::new();
                push_diff_opt(&mut diffs, "name", &s.name, &t.name);
                push_diff(
                    &mut diffs,
                    "arg_count_int",
                    &s.argument_count_int,
                    &t.argument_count_int,
                );
                push_diff(
                    &mut diffs,
                    "arg_count_obj",
                    &s.argument_count_object,
                    &t.argument_count_object,
                );
                push_diff(
                    &mut diffs,
                    "arg_count_long",
                    &s.argument_count_long,
                    &t.argument_count_long,
                );
                push_diff(
                    &mut diffs,
                    "local_count_int",
                    &s.local_count_int,
                    &t.local_count_int,
                );
                push_diff(
                    &mut diffs,
                    "local_count_obj",
                    &s.local_count_object,
                    &t.local_count_object,
                );
                push_diff(
                    &mut diffs,
                    "local_count_long",
                    &s.local_count_long,
                    &t.local_count_long,
                );
                push_diff(
                    &mut diffs,
                    "instruction_count",
                    &s.code.len(),
                    &t.code.len(),
                );
                (
                    if diffs.is_empty() {
                        ConflictStatus::Safe
                    } else {
                        ConflictStatus::ScriptChanged
                    },
                    non_empty(diffs),
                )
            }
        }
    }

    fn compare_enum(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match (self.source.enums.get(&id), self.target.enums.get(&id)) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
                let mut diffs = Vec::new();
                push_diff_opt(
                    &mut diffs,
                    "input_type",
                    &s.input_type_char,
                    &t.input_type_char,
                );
                push_diff_opt(
                    &mut diffs,
                    "output_type",
                    &s.output_type_char,
                    &t.output_type_char,
                );
                push_diff(&mut diffs, "value_count", &s.values.len(), &t.values.len());
                push_diff(
                    &mut diffs,
                    "default",
                    &format_scalar_opt(&s.default),
                    &format_scalar_opt(&t.default),
                );
                (
                    if diffs.is_empty() {
                        ConflictStatus::Safe
                    } else {
                        ConflictStatus::Changed
                    },
                    non_empty(diffs),
                )
            }
        }
    }

    fn compare_param(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match (self.source.params.get(&id), self.target.params.get(&id)) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
                let mut diffs = Vec::new();
                push_diff_opt(&mut diffs, "type_char", &s.type_char, &t.type_char);
                push_diff_opt(&mut diffs, "type_id", &s.type_id, &t.type_id);
                push_diff(
                    &mut diffs,
                    "default",
                    &format_scalar_opt(&s.default),
                    &format_scalar_opt(&t.default),
                );
                push_diff(&mut diffs, "autodisable", &s.autodisable, &t.autodisable);
                (
                    if diffs.is_empty() {
                        ConflictStatus::Safe
                    } else {
                        ConflictStatus::Changed
                    },
                    non_empty(diffs),
                )
            }
        }
    }

    fn compare_seq(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        match (self.source.seqs.get(&id), self.target.seqs.get(&id)) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
                let mut diffs = Vec::new();
                push_diff(&mut diffs, "frame_count", &s.frames.len(), &t.frames.len());
                push_diff(&mut diffs, "stretches", &s.stretches, &t.stretches);
                push_diff_opt(&mut diffs, "priority", &s.priority, &t.priority);
                push_diff_opt(&mut diffs, "lefthand_raw", &s.lefthand_raw, &t.lefthand_raw);
                push_diff_opt(
                    &mut diffs,
                    "righthand_raw",
                    &s.righthand_raw,
                    &t.righthand_raw,
                );
                push_diff_opt(&mut diffs, "loopcount", &s.loopcount, &t.loopcount);
                (
                    if diffs.is_empty() {
                        ConflictStatus::Safe
                    } else {
                        ConflictStatus::Changed
                    },
                    non_empty(diffs),
                )
            }
        }
    }

    fn compare_component(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
        let s_comp = self
            .source
            .parsed_components
            .values()
            .find_map(|g| g.get(&id));
        let t_comp = self
            .target
            .parsed_components
            .values()
            .find_map(|g| g.get(&id));
        match (s_comp, t_comp) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
                let mut diffs = Vec::new();
                push_diff(&mut diffs, "type", &s.component_type, &t.component_type);
                push_diff_opt(&mut diffs, "name", &s.name, &t.name);
                push_diff(
                    &mut diffs,
                    "child_count",
                    &s.children.len(),
                    &t.children.len(),
                );
                push_diff(
                    &mut diffs,
                    "script_count",
                    &s.scripts.len(),
                    &t.scripts.len(),
                );
                push_diff(&mut diffs, "varp_count", &s.varps.len(), &t.varps.len());
                push_diff(
                    &mut diffs,
                    "varbit_count",
                    &s.varbits.len(),
                    &t.varbits.len(),
                );
                push_diff(&mut diffs, "enum_count", &s.enums.len(), &t.enums.len());
                (
                    if diffs.is_empty() {
                        ConflictStatus::Safe
                    } else {
                        ConflictStatus::Changed
                    },
                    non_empty(diffs),
                )
            }
        }
    }

    fn entity_summaries(
        &self,
        entity_type: EntityType,
        id: u32,
    ) -> (Option<String>, Option<String>) {
        match entity_type {
            EntityType::VarPlayer
            | EntityType::VarNpc
            | EntityType::VarClient
            | EntityType::VarWorld
            | EntityType::VarRegion
            | EntityType::VarObject
            | EntityType::VarClan
            | EntityType::VarClanSetting
            | EntityType::VarController
            | EntityType::VarGlobal
            | EntityType::VarPlayerGroup => {
                let domain = Self::entity_type_to_domain(entity_type);
                let s = domain
                    .and_then(|d| self.source.varps_by_domain.get(&d).and_then(|v| v.get(&id)));
                let t = domain
                    .and_then(|d| self.target.varps_by_domain.get(&d).and_then(|v| v.get(&id)));
                (
                    s.map(|v| {
                        format!(
                            "name={} type={:?} lifetime={}",
                            v.var_name,
                            v.type_id,
                            v.lifetime.unwrap_or("unknown")
                        )
                    }),
                    t.map(|v| {
                        format!(
                            "name={} type={:?} lifetime={}",
                            v.var_name,
                            v.type_id,
                            v.lifetime.unwrap_or("unknown")
                        )
                    }),
                )
            }
            EntityType::VarBit => {
                let s = self.source.varbits.get(&id);
                let t = self.target.varbits.get(&id);
                (
                    s.map(|v| {
                        format!(
                            "name={} base={:?} bits={:?}-{:?}",
                            v.varbit_name, v.base_var, v.start_bit, v.end_bit
                        )
                    }),
                    t.map(|v| {
                        format!(
                            "name={} base={:?} bits={:?}-{:?}",
                            v.varbit_name, v.base_var, v.start_bit, v.end_bit
                        )
                    }),
                )
            }
            EntityType::Script => {
                let s = self.get_script(&self.source, &self.source_script_catalog, id);
                let t = self.get_script(&self.target, &self.target_script_catalog, id);
                (
                    s.map(|s| {
                        format!(
                            "instr={} locals={} args={}",
                            s.code.len(),
                            s.local_count_int,
                            s.argument_count_int
                        )
                    }),
                    t.map(|s| {
                        format!(
                            "instr={} locals={} args={}",
                            s.code.len(),
                            s.local_count_int,
                            s.argument_count_int
                        )
                    }),
                )
            }
            _ => (None, None),
        }
    }

    fn lookup_name(&self, entity_type: EntityType, id: u32) -> Option<String> {
        match entity_type {
            EntityType::Script => self
                .get_script(&self.source, &self.source_script_catalog, id)
                .and_then(|s| s.name),
            _ => None,
        }
    }

    fn build_report(&self, group_id: u32, entities: Vec<ConflictEntry>) -> ConflictReport {
        let summary = accumulate_summary(&entities);
        let components = self.source.parsed_components.get(&group_id);
        ConflictReport {
            source_build: self.source.build,
            target_build: self.target.build,
            interface_group: group_id,
            interface_name: components.and_then(|c| c.values().find_map(|c| c.name.clone())),
            total_components: components.map(BTreeMap::len).unwrap_or(0),
            total_entities: entities.len(),
            summary,
            entities,
            remap: None,
            reference_updates: None,
            allocation: None,
            target_validation: None,
        }
    }

    // ── Remap planning ──

    pub fn remap_interface(&self, group_id: u32, buffer: u32) -> ConflictReport {
        let mut report = self.analyze_interface(group_id);

        let (remap_table, alloc) = self.allocate_free_ids(&report.entities, buffer);
        let ref_updates = self.build_reference_updates(group_id, &remap_table);

        report.remap = Some(remap_table);
        report.reference_updates = Some(ref_updates);
        report.allocation = Some(alloc);

        report
    }

    pub fn validate_interface_target(
        &self,
        group_id: u32,
        entities: &[ConflictEntry],
        remap: Option<&RemapTable>,
        allow_heuristic_sites: bool,
    ) -> TargetValidationReport {
        let empty_remap = RemapTable::default();
        let remap = remap.unwrap_or(&empty_remap);
        let overlays = self.prepare_script_overlays(entities, remap);
        let merged_ctx =
            self.build_target_validation_context_from_overlays(entities, remap, &overlays);
        let extra_scripts = overlays
            .iter()
            .filter_map(|overlay| {
                overlay
                    .bytes
                    .as_deref()
                    .map(|bytes| (overlay.target_packed_id, bytes))
            })
            .collect::<Vec<_>>();
        let merged_catalog = extend_validation_catalog(
            &self.target_script_catalog,
            &self.target.opcode_book,
            self.target.build,
            &extra_scripts,
        );
        let mut scripts = self.validate_target_scripts_from_overlays(
            overlays,
            &merged_ctx,
            &merged_catalog,
            allow_heuristic_sites,
        );
        scripts.sort_by_key(|script| script.target_script_id);
        let component_checks = self.validate_interface_components(
            group_id,
            remap,
            &merged_ctx,
            &merged_catalog,
            allow_heuristic_sites,
        );
        let summary = summarize_target_validation(&component_checks, &scripts);

        TargetValidationReport {
            target_build: self.target.build,
            remap_applied: !remap.scripts.is_empty()
                || !remap.varps.is_empty()
                || !remap.varbits.is_empty(),
            summary,
            components: component_checks,
            scripts,
        }
    }

    // ── Script-level analysis ──

    /// Analyze a single script and its full transitive dependency tree.
    pub fn analyze_script(&self, script_id: u32) -> ScriptReport {
        let mut entities = Vec::new();
        let mut visited: HashSet<EntityKey> = HashSet::new();

        // Start with the script itself
        let key = EntityKey::new(EntityType::Script, script_id);
        visited.insert(key);
        self.collect_entity(
            EntityType::Script,
            script_id,
            None,
            &mut entities,
            &mut visited,
        );
        self.walk_script(script_id, &mut entities, &mut visited);

        let summary = accumulate_summary(&entities);

        let script_name = self
            .get_script(&self.source, &self.source_script_catalog, script_id)
            .and_then(|s| s.name);

        ScriptReport {
            source_build: self.source.build,
            target_build: self.target.build,
            script_id,
            script_name,
            total_entities: entities.len(),
            summary,
            entities,
            remap: None,
            reference_updates: None,
            allocation: None,
            target_validation: None,
        }
    }

    /// Analyze a single script with remap planning.
    pub fn remap_script(&self, script_id: u32, buffer: u32) -> ScriptReport {
        let mut report = self.analyze_script(script_id);

        let (remap_table, alloc) = self.allocate_free_ids(&report.entities, buffer);
        let ref_updates = self.build_script_ref_updates(script_id, &remap_table);

        report.remap = Some(remap_table);
        report.reference_updates = Some(ref_updates);
        report.allocation = Some(alloc);

        report
    }

    pub fn validate_script_target(
        &self,
        entities: &[ConflictEntry],
        remap: Option<&RemapTable>,
        allow_heuristic_sites: bool,
    ) -> TargetValidationReport {
        let empty_remap = RemapTable::default();
        let remap = remap.unwrap_or(&empty_remap);
        let mut scripts = self.validate_target_scripts(entities, remap, allow_heuristic_sites);
        scripts.sort_by_key(|script| script.target_script_id);
        let summary = summarize_target_validation(&[], &scripts);

        TargetValidationReport {
            target_build: self.target.build,
            remap_applied: !remap.scripts.is_empty()
                || !remap.varps.is_empty()
                || !remap.varbits.is_empty(),
            summary,
            components: Vec::new(),
            scripts,
        }
    }

    fn validate_target_scripts(
        &self,
        entities: &[ConflictEntry],
        remap: &RemapTable,
        allow_heuristic_sites: bool,
    ) -> Vec<ScriptTargetValidation> {
        let overlays = self.prepare_script_overlays(entities, remap);
        let merged_ctx =
            self.build_target_validation_context_from_overlays(entities, remap, &overlays);
        let extra_scripts = overlays
            .iter()
            .filter_map(|overlay| {
                overlay
                    .bytes
                    .as_deref()
                    .map(|bytes| (overlay.target_packed_id, bytes))
            })
            .collect::<Vec<_>>();
        let merged_catalog = extend_validation_catalog(
            &self.target_script_catalog,
            &self.target.opcode_book,
            self.target.build,
            &extra_scripts,
        );
        self.validate_target_scripts_from_overlays(
            overlays,
            &merged_ctx,
            &merged_catalog,
            allow_heuristic_sites,
        )
    }

    fn validate_target_scripts_from_overlays(
        &self,
        overlays: Vec<PreparedScriptOverlay>,
        merged_ctx: &ResolverContext,
        merged_catalog: &ScriptCatalog,
        allow_heuristic_sites: bool,
    ) -> Vec<ScriptTargetValidation> {
        let merged_signatures = merged_catalog.signature_map();

        overlays
            .into_par_iter()
            .map(|overlay| {
                let validator = Cs2Validator::new(merged_ctx);
                let dependency_sites = overlay.dependency_sites;
                let heuristic_sites = dependency_sites
                    .iter()
                    .filter(|site| site.confidence == DependencyConfidence::Heuristic)
                    .cloned()
                    .collect::<Vec<_>>();
                let unsupported_sites = Vec::new();
                let mut blockers = Vec::new();
                if !allow_heuristic_sites && !heuristic_sites.is_empty() {
                    blockers.push(format!(
                        "heuristic dependency sites present ({})",
                        heuristic_sites.len()
                    ));
                }
                if let (Some(script), None) = (overlay.script.as_ref(), overlay.failure.as_ref()) {
                    let report = validator.validate_compiled(
                        overlay.target_script_id,
                        script,
                        merged_catalog,
                        &merged_signatures,
                        overlay.script_name.clone(),
                    );
                    ScriptTargetValidation {
                        source_script_id: overlay.source_script_id,
                        source_packed_id: overlay.source_packed_id,
                        target_script_id: overlay.target_script_id,
                        target_packed_id: overlay.target_packed_id,
                        script_name: overlay.script_name,
                        encoded_bytes: overlay.encoded_bytes,
                        failure: None,
                        dependency_sites: dependency_sites.len(),
                        heuristic_sites,
                        unsupported_sites,
                        blockers,
                        reference_updates: overlay.reference_updates,
                        validation_errors: report.errors,
                        validation_warnings: report.warnings,
                    }
                } else {
                    ScriptTargetValidation {
                        source_script_id: overlay.source_script_id,
                        source_packed_id: overlay.source_packed_id,
                        target_script_id: overlay.target_script_id,
                        target_packed_id: overlay.target_packed_id,
                        script_name: overlay.script_name,
                        encoded_bytes: overlay.encoded_bytes,
                        failure: overlay.failure,
                        dependency_sites: dependency_sites.len(),
                        heuristic_sites,
                        unsupported_sites,
                        blockers,
                        reference_updates: overlay.reference_updates,
                        validation_errors: Vec::new(),
                        validation_warnings: Vec::new(),
                    }
                }
            })
            .collect()
    }

    fn prepare_script_overlays(
        &self,
        entities: &[ConflictEntry],
        remap: &RemapTable,
    ) -> Vec<PreparedScriptOverlay> {
        let script_ids = collect_script_entity_ids(entities);
        script_ids
            .into_par_iter()
            .map(|script_id| self.prepare_script_overlay(script_id, remap))
            .collect()
    }

    fn prepare_script_overlay(
        &self,
        source_script_id: u32,
        remap: &RemapTable,
    ) -> PreparedScriptOverlay {
        let target_script_id = remap
            .scripts
            .get(&source_script_id)
            .copied()
            .unwrap_or(source_script_id);
        let target_packed_id = target_script_id << 16;
        let source_packed_id =
            self.resolve_script_packed_id(&self.source_script_catalog, source_script_id);

        let Some(source_script) =
            self.get_script(&self.source, &self.source_script_catalog, source_script_id)
        else {
            return PreparedScriptOverlay {
                source_script_id,
                source_packed_id,
                target_script_id,
                target_packed_id,
                script_name: None,
                bytes: None,
                script: None,
                encoded_bytes: None,
                failure: Some(format!("source script {source_script_id} not found")),
                dependency_sites: Vec::new(),
                reference_updates: Vec::new(),
            };
        };

        let script_name = source_script.name.clone();
        let (rewritten_script, reference_updates) =
            self.rewrite_script_for_target(&source_script, remap);
        let dependency_sites = collect_script_dependency_sites(&rewritten_script);
        match encode_script(
            &rewritten_script,
            &self.target.opcode_book,
            self.target.build,
        ) {
            Ok(bytes) => {
                let encoded_len = bytes.len();
                match decode_script(&bytes, &self.target.opcode_book, self.target.build) {
                    Ok(decoded) => PreparedScriptOverlay {
                        source_script_id,
                        source_packed_id,
                        target_script_id,
                        target_packed_id,
                        script_name,
                        bytes: Some(bytes),
                        script: Some(decoded),
                        encoded_bytes: Some(encoded_len),
                        failure: None,
                        dependency_sites,
                        reference_updates,
                    },
                    Err(err) => PreparedScriptOverlay {
                        source_script_id,
                        source_packed_id,
                        target_script_id,
                        target_packed_id,
                        script_name,
                        bytes: None,
                        script: None,
                        encoded_bytes: Some(encoded_len),
                        failure: Some(format!("target decode failed: {err}")),
                        dependency_sites,
                        reference_updates,
                    },
                }
            }
            Err(err) => PreparedScriptOverlay {
                source_script_id,
                source_packed_id,
                target_script_id,
                target_packed_id,
                script_name,
                bytes: None,
                script: None,
                encoded_bytes: None,
                failure: Some(err.to_string()),
                dependency_sites,
                reference_updates,
            },
        }
    }

    fn rewrite_script_for_target(
        &self,
        script: &CompiledScript,
        remap: &RemapTable,
    ) -> (CompiledScript, Vec<RefUpdateEntry>) {
        let mut rewritten = script.clone();
        let mut updates = Vec::new();

        for (index, instruction) in rewritten.code.iter_mut().enumerate() {
            match &mut instruction.operand {
                Operand::VarRef(var_ref) => {
                    let key = format!("{}:{}", var_ref.domain.as_label(), var_ref.id);
                    if let Some(target) = remap.varps.get(&key) {
                        updates.push(RefUpdateEntry {
                            location: format!("instruction[{index}]"),
                            from: format!("varp {key}"),
                            to: format!("varp {}:{}", target.domain, target.id),
                        });
                        var_ref.id = target.id as u16;
                    }
                }
                Operand::VarBitRef(varbit_ref) => {
                    if let Some(&target_id) = remap.varbits.get(&u32::from(varbit_ref.id)) {
                        updates.push(RefUpdateEntry {
                            location: format!("instruction[{index}]"),
                            from: format!("varbit {}", varbit_ref.id),
                            to: format!("varbit {target_id}"),
                        });
                        varbit_ref.id = target_id as u16;
                    }
                }
                Operand::Script(script_id) => {
                    if let Some(&target_id) = remap.scripts.get(&(*script_id as u32)) {
                        updates.push(RefUpdateEntry {
                            location: format!("instruction[{index}]"),
                            from: format!("script {script_id}"),
                            to: format!("script {target_id}"),
                        });
                        *script_id = target_id as i32;
                    }
                }
                _ => {}
            }
        }

        (rewritten, updates)
    }

    fn build_target_validation_context_from_overlays(
        &self,
        entities: &[ConflictEntry],
        remap: &RemapTable,
        _overlays: &[PreparedScriptOverlay],
    ) -> ResolverContext {
        let mut varps_by_domain = self.target.varps_by_domain.clone();
        for (domain, source_id) in collect_varp_entity_ids(entities) {
            let source_entry = self
                .source
                .varps_by_domain
                .get(&domain)
                .and_then(|vars| vars.get(&source_id))
                .cloned();
            let Some(mut source_entry) = source_entry else {
                continue;
            };
            let key = format!("{}:{}", domain.as_label(), source_id);
            let target_id = remap.varps.get(&key).map_or(source_id, |target| target.id);
            source_entry.id = target_id;
            varps_by_domain
                .entry(domain)
                .or_default()
                .insert(target_id, source_entry);
        }

        let mut varbits = self.target.varbits.clone();
        for source_id in collect_varbit_entity_ids(entities) {
            let Some(mut source_entry) = self.source.varbits.get(&source_id).cloned() else {
                continue;
            };
            let target_id = remap.varbits.get(&source_id).copied().unwrap_or(source_id);
            source_entry.id = target_id;
            if let Some(base_var) = source_entry.base_var {
                let domain = source_entry.domain.unwrap_or(VarDomain::Player);
                let key = format!("{}:{}", domain.as_label(), base_var);
                if let Some(target) = remap.varps.get(&key) {
                    source_entry.base_var = Some(target.id);
                }
            }
            varbits.insert(target_id, source_entry);
        }

        ResolverContext {
            build: self.target.build,
            opcode_book: self.target.opcode_book.clone(),
            interfaces: BTreeMap::new(),
            scripts: BTreeMap::new(),
            varps_by_domain,
            varbits,
            params: self.target.params.clone(),
            enums: self.target.enums.clone(),
            structs: self.target.structs.clone(),
            decoded_scripts: BTreeMap::new(),
            parsed_components: BTreeMap::new(),
            npcs: self.target.npcs.clone(),
            objs: self.target.objs.clone(),
            locs: self.target.locs.clone(),
            seqs: self.target.seqs.clone(),
            spots: self.target.spots.clone(),
            invs: self.target.invs.clone(),
            dbtables: self.target.dbtables.clone(),
            dbrows: self.target.dbrows.clone(),
        }
    }

    fn validate_interface_components(
        &self,
        group_id: u32,
        remap: &RemapTable,
        merged_ctx: &ResolverContext,
        merged_catalog: &ScriptCatalog,
        allow_heuristic_sites: bool,
    ) -> Vec<ComponentTargetValidation> {
        let Some(components) = self.source.parsed_components.get(&group_id) else {
            return Vec::new();
        };

        let mut checks = components
            .iter()
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|(&component_id, deps)| {
                let dependency_sites = collect_component_dependency_sites(deps);
                let mut heuristic_sites = Vec::new();
                let mut unsupported_sites = Vec::new();
                let mut blocking_issues = Vec::new();

                for site in dependency_sites.iter().cloned() {
                    let remapped_site = self.remap_dependency_site(site, remap);
                    if remapped_site.confidence == DependencyConfidence::Heuristic {
                        heuristic_sites.push(remapped_site.clone());
                    }
                    match self.validate_dependency_site(&remapped_site, merged_ctx, merged_catalog)
                    {
                        DependencySiteValidation::Resolved => {}
                        DependencySiteValidation::Missing(reason) => blocking_issues.push(reason),
                        DependencySiteValidation::Unsupported => {
                            unsupported_sites.push(remapped_site);
                        }
                    }
                }

                if !allow_heuristic_sites && !heuristic_sites.is_empty() {
                    blocking_issues.push(format!(
                        "heuristic dependency sites present ({})",
                        heuristic_sites.len()
                    ));
                }
                if !unsupported_sites.is_empty() {
                    blocking_issues.push(format!(
                        "unsupported dependency site types present ({})",
                        unsupported_sites.len()
                    ));
                }

                ComponentTargetValidation {
                    component_id,
                    name: deps.name.clone(),
                    dependency_sites: dependency_sites.len(),
                    heuristic_sites,
                    unsupported_sites,
                    blocking_issues,
                }
            })
            .collect::<Vec<_>>();

        checks.sort_by_key(|component| component.component_id);
        checks
    }

    fn remap_dependency_site(
        &self,
        mut site: DependencySite,
        remap: &RemapTable,
    ) -> DependencySite {
        match site.entity_type {
            EntityType::Script => {
                site.id = remap.scripts.get(&site.id).copied().unwrap_or(site.id);
            }
            EntityType::VarBit => {
                site.id = remap.varbits.get(&site.id).copied().unwrap_or(site.id);
            }
            EntityType::VarPlayer
            | EntityType::VarNpc
            | EntityType::VarClient
            | EntityType::VarWorld
            | EntityType::VarRegion
            | EntityType::VarObject
            | EntityType::VarClan
            | EntityType::VarClanSetting
            | EntityType::VarController
            | EntityType::VarGlobal
            | EntityType::VarPlayerGroup => {
                if let Some(domain) = Self::entity_type_to_domain(site.entity_type) {
                    let key = format!("{}:{}", domain.as_label(), site.id);
                    if let Some(target) = remap.varps.get(&key) {
                        site.id = target.id;
                    }
                }
            }
            _ => {}
        }
        site
    }

    fn validate_dependency_site(
        &self,
        site: &DependencySite,
        merged_ctx: &ResolverContext,
        merged_catalog: &ScriptCatalog,
    ) -> DependencySiteValidation {
        match site.entity_type {
            EntityType::Script => {
                if merged_catalog.resolve_call_target(site.id as i32).is_some() {
                    DependencySiteValidation::Resolved
                } else {
                    DependencySiteValidation::Missing(format!(
                        "{} {} missing at {}",
                        site.entity_type.as_label(),
                        site.id,
                        site.location
                    ))
                }
            }
            EntityType::VarBit => {
                if merged_ctx.varbits.contains_key(&site.id) {
                    DependencySiteValidation::Resolved
                } else {
                    DependencySiteValidation::Missing(format!(
                        "varbit {} missing at {}",
                        site.id, site.location
                    ))
                }
            }
            EntityType::Enum => simple_site_exists(site, merged_ctx.enums.contains_key(&site.id)),
            EntityType::Param => simple_site_exists(site, merged_ctx.params.contains_key(&site.id)),
            EntityType::Struct => {
                simple_site_exists(site, merged_ctx.structs.contains_key(&site.id))
            }
            EntityType::Inv => simple_site_exists(site, merged_ctx.invs.contains_key(&site.id)),
            EntityType::Seq => simple_site_exists(site, merged_ctx.seqs.contains_key(&site.id)),
            EntityType::DbTable => {
                simple_site_exists(site, merged_ctx.dbtables.contains_key(&site.id))
            }
            EntityType::DbRow => simple_site_exists(site, merged_ctx.dbrows.contains_key(&site.id)),
            EntityType::Obj => simple_site_exists(site, merged_ctx.objs.contains_key(&site.id)),
            EntityType::Npc => simple_site_exists(site, merged_ctx.npcs.contains_key(&site.id)),
            EntityType::Loc => simple_site_exists(site, merged_ctx.locs.contains_key(&site.id)),
            EntityType::Component => {
                if self.target_component_ids.contains(&site.id) {
                    DependencySiteValidation::Resolved
                } else {
                    DependencySiteValidation::Missing(format!(
                        "component {} missing at {}",
                        site.id, site.location
                    ))
                }
            }
            EntityType::VarPlayer
            | EntityType::VarNpc
            | EntityType::VarClient
            | EntityType::VarWorld
            | EntityType::VarRegion
            | EntityType::VarObject
            | EntityType::VarClan
            | EntityType::VarClanSetting
            | EntityType::VarController
            | EntityType::VarGlobal
            | EntityType::VarPlayerGroup => {
                let Some(domain) = Self::entity_type_to_domain(site.entity_type) else {
                    return DependencySiteValidation::Unsupported;
                };
                let exists = merged_ctx
                    .varps_by_domain
                    .get(&domain)
                    .and_then(|vars| vars.get(&site.id))
                    .is_some();
                if exists {
                    DependencySiteValidation::Resolved
                } else {
                    DependencySiteValidation::Missing(format!(
                        "{} {} missing at {}",
                        site.entity_type.as_label(),
                        site.id,
                        site.location
                    ))
                }
            }
            EntityType::Graphic
            | EntityType::Model
            | EntityType::Cursor
            | EntityType::FontMetrics
            | EntityType::Texture
            | EntityType::Stylesheet
            | EntityType::Config => DependencySiteValidation::Unsupported,
            _ => DependencySiteValidation::Unsupported,
        }
    }

    /// Build reference updates starting from a single script.
    fn build_script_ref_updates(&self, script_id: u32, remap: &RemapTable) -> Vec<ReferenceUpdate> {
        let mut updates = Vec::new();
        let mut visited: HashSet<EntityKey> = HashSet::new();
        self.collect_script_ref_updates(script_id, remap, &mut updates, &mut visited);
        updates
    }

    fn allocate_free_ids(
        &self,
        entities: &[ConflictEntry],
        buffer: u32,
    ) -> (RemapTable, AllocationInfo) {
        let mut remap = RemapTable::default();
        let mut alloc = AllocationInfo::new();

        // Scripts
        {
            let max_script = self
                .target_script_catalog
                .iter()
                .map(|metadata| metadata.group_id.0 as u32)
                .max()
                .unwrap_or(0);
            let start = max_script.saturating_add(buffer);
            let mut next_id = start;
            for e in entities {
                if e.entity_type == "script"
                    && matches!(
                        e.status,
                        ConflictStatus::Missing | ConflictStatus::IdConflict
                    )
                {
                    remap.scripts.insert(e.id, next_id);
                    next_id += 1;
                }
            }
            alloc.scripts = RangeAlloc {
                target_max: max_script,
                allocated_from: start,
                count: remap.scripts.len(),
            };
        }

        // Varps — per domain
        for domain in &[
            VarDomain::Player,
            VarDomain::Npc,
            VarDomain::Client,
            VarDomain::World,
            VarDomain::Region,
            VarDomain::Object,
            VarDomain::Clan,
            VarDomain::ClanSetting,
            VarDomain::Controller,
            VarDomain::Global,
            VarDomain::PlayerGroup,
        ] {
            let max_varp = self
                .target
                .varps_by_domain
                .get(domain)
                .map(|vars| vars.keys().copied().max().unwrap_or(0))
                .unwrap_or(0);
            let start = max_varp.saturating_add(buffer);
            let mut next_id = start;

            for e in entities {
                if e.entity_type == Self::varp_type_label(*domain)
                    && matches!(
                        e.status,
                        ConflictStatus::Missing | ConflictStatus::IdConflict
                    )
                {
                    let key = format!("{}:{}", domain.as_label(), e.id);
                    remap.varps.insert(
                        key,
                        VarpRemapTarget {
                            domain: domain.as_label().to_string(),
                            id: next_id,
                        },
                    );
                    next_id += 1;
                }
            }
            let ra = alloc_for(&mut alloc, *domain);
            *ra = RangeAlloc {
                target_max: max_varp,
                allocated_from: start,
                count: (next_id - start) as usize,
            };
        }

        // Varbits
        {
            let max_bit = self.target.varbits.keys().copied().max().unwrap_or(0);
            let start = max_bit.saturating_add(buffer);
            let mut next_id = start;
            for e in entities {
                if e.entity_type == "varbit"
                    && matches!(
                        e.status,
                        ConflictStatus::Missing | ConflictStatus::IdConflict
                    )
                {
                    remap.varbits.insert(e.id, next_id);
                    next_id += 1;
                }
            }
            alloc.varbits = RangeAlloc {
                target_max: max_bit,
                allocated_from: start,
                count: remap.varbits.len(),
            };
        }

        (remap, alloc)
    }

    fn build_reference_updates(&self, group_id: u32, remap: &RemapTable) -> Vec<ReferenceUpdate> {
        let mut updates = Vec::new();

        // Walk scripts in the dependency tree
        let mut visited: HashSet<EntityKey> = HashSet::new();
        if let Some(comps) = self.source.parsed_components.get(&group_id) {
            for comp_deps in comps.values() {
                for script_id in sorted_hashset_ids(&comp_deps.scripts) {
                    self.collect_script_ref_updates(script_id, remap, &mut updates, &mut visited);
                }
            }
        }

        // Walk components in the dependency tree
        if let Some(comps) = self.source.parsed_components.get(&group_id) {
            for (&comp_id, comp_deps) in comps {
                let mut comp_updates = Vec::new();

                for script_id in sorted_hashset_ids(&comp_deps.scripts) {
                    if let Some(&new_id) = remap.scripts.get(&script_id) {
                        comp_updates.push(RefUpdateEntry {
                            location: format!("scripts[{script_id}]"),
                            from: format!("script {script_id}"),
                            to: format!("script {new_id}"),
                        });
                    }
                }

                let mut var_refs = comp_deps.varps.iter().cloned().collect::<Vec<_>>();
                var_refs.sort_by_key(|var_ref| {
                    let (entity_type, id) = var_transmit_to_entity(var_ref);
                    (entity_type.as_label().to_string(), id)
                });
                for var_ref in var_refs {
                    let (_, id) = var_transmit_to_entity(&var_ref);
                    let domain = Self::var_ref_domain(&var_ref);
                    let key = format!("{}:{}", domain.as_label(), id);
                    if let Some(target) = remap.varps.get(&key) {
                        comp_updates.push(RefUpdateEntry {
                            location: format!("varps[{key}]"),
                            from: format!("varp {key}"),
                            to: format!("varp {}:{}", target.domain, target.id),
                        });
                    }
                }

                for varbit_id in sorted_hashset_ids(&comp_deps.varbits) {
                    if let Some(&new_id) = remap.varbits.get(&varbit_id) {
                        comp_updates.push(RefUpdateEntry {
                            location: format!("varbits[{varbit_id}]"),
                            from: format!("varbit {varbit_id}"),
                            to: format!("varbit {new_id}"),
                        });
                    }
                }

                if !comp_updates.is_empty() {
                    sort_ref_update_entries(&mut comp_updates);
                    updates.push(ReferenceUpdate {
                        entity_type: "component".to_string(),
                        id: comp_id,
                        updates: comp_updates,
                    });
                }
            }
        }

        sort_reference_updates(&mut updates);
        updates
    }

    fn collect_script_ref_updates(
        &self,
        script_id: u32,
        remap: &RemapTable,
        updates: &mut Vec<ReferenceUpdate>,
        visited: &mut HashSet<EntityKey>,
    ) {
        if !visited.insert(EntityKey::new(EntityType::Script, script_id)) {
            return;
        }

        let mut script_updates = Vec::new();

        if let Some(script) = self.get_script(&self.source, &self.source_script_catalog, script_id)
        {
            for (i, instruction) in script.code.iter().enumerate() {
                match &instruction.operand {
                    crate::script::Operand::VarRef(var_ref) => {
                        let ref_entity = crate::dep_tree::var_ref_to_entity_ref(var_ref);
                        let domain = Self::entity_type_to_domain(ref_entity.entity_type);
                        if let Some(domain) = domain {
                            let key = format!("{}:{}", domain.as_label(), ref_entity.id);
                            if let Some(target) = remap.varps.get(&key) {
                                script_updates.push(RefUpdateEntry {
                                    location: format!("instruction[{i}]"),
                                    from: format!("varp {key}"),
                                    to: format!("varp {}:{}", target.domain, target.id),
                                });
                            }
                        }
                    }
                    crate::script::Operand::VarBitRef(vbr) => {
                        let id = u32::from(vbr.id);
                        if let Some(&new_id) = remap.varbits.get(&id) {
                            script_updates.push(RefUpdateEntry {
                                location: format!("instruction[{i}]"),
                                from: format!("varbit {id}"),
                                to: format!("varbit {new_id}"),
                            });
                        }
                    }
                    crate::script::Operand::Script(called_id) => {
                        let id = *called_id as u32;
                        if let Some(&new_id) = remap.scripts.get(&id) {
                            script_updates.push(RefUpdateEntry {
                                location: format!("instruction[{i}]"),
                                from: format!("script {id}"),
                                to: format!("script {new_id}"),
                            });
                        }
                        // Recurse into called script
                        self.collect_script_ref_updates(id, remap, updates, visited);
                    }
                    _ => {}
                }
            }
        }

        if !script_updates.is_empty() {
            sort_ref_update_entries(&mut script_updates);
            updates.push(ReferenceUpdate {
                entity_type: "script".to_string(),
                id: script_id,
                updates: script_updates,
            });
        }
    }

    fn varp_type_label(domain: VarDomain) -> &'static str {
        match domain {
            VarDomain::Player => "varplayer",
            VarDomain::Npc => "varnpc",
            VarDomain::Client => "varclient",
            VarDomain::World => "varworld",
            VarDomain::Region => "varregion",
            VarDomain::Object => "varobject",
            VarDomain::Clan => "varclan",
            VarDomain::ClanSetting => "varclansetting",
            VarDomain::Controller => "varcontroller",
            VarDomain::Global => "varglobal",
            VarDomain::PlayerGroup => "varplayergroup",
        }
    }

    fn var_ref_domain(var_ref: &VarTransmitRef) -> VarDomain {
        match var_ref {
            VarTransmitRef::Player(_) => VarDomain::Player,
            VarTransmitRef::Npc(_) => VarDomain::Npc,
            VarTransmitRef::Client(_) => VarDomain::Client,
            VarTransmitRef::World(_) => VarDomain::World,
            VarTransmitRef::Region(_) => VarDomain::Region,
            VarTransmitRef::Object(_) => VarDomain::Object,
            VarTransmitRef::Clan(_) => VarDomain::Clan,
            VarTransmitRef::ClanSetting(_) => VarDomain::ClanSetting,
            VarTransmitRef::Controller(_) => VarDomain::Controller,
            VarTransmitRef::Global(_) => VarDomain::Global,
            VarTransmitRef::PlayerGroup(_) => VarDomain::PlayerGroup,
            VarTransmitRef::VarClientString(_) => VarDomain::Client,
        }
    }

    fn entity_type_to_domain(et: EntityType) -> Option<VarDomain> {
        match et {
            EntityType::VarPlayer => Some(VarDomain::Player),
            EntityType::VarNpc => Some(VarDomain::Npc),
            EntityType::VarClient => Some(VarDomain::Client),
            EntityType::VarWorld => Some(VarDomain::World),
            EntityType::VarRegion => Some(VarDomain::Region),
            EntityType::VarObject => Some(VarDomain::Object),
            EntityType::VarClan => Some(VarDomain::Clan),
            EntityType::VarClanSetting => Some(VarDomain::ClanSetting),
            EntityType::VarController => Some(VarDomain::Controller),
            EntityType::VarGlobal => Some(VarDomain::Global),
            EntityType::VarPlayerGroup => Some(VarDomain::PlayerGroup),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedScriptOverlay {
    source_script_id: u32,
    source_packed_id: Option<u32>,
    target_script_id: u32,
    target_packed_id: u32,
    script_name: Option<String>,
    bytes: Option<Vec<u8>>,
    script: Option<CompiledScript>,
    encoded_bytes: Option<usize>,
    failure: Option<String>,
    dependency_sites: Vec<DependencySite>,
    reference_updates: Vec<RefUpdateEntry>,
}

fn collect_script_entity_ids(entities: &[ConflictEntry]) -> BTreeSet<u32> {
    entities
        .iter()
        .filter(|entity| entity.entity_type == "script")
        .map(|entity| entity.id)
        .collect()
}

fn sorted_hashset_ids(values: &HashSet<u32>) -> Vec<u32> {
    let mut ids = values.iter().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn sort_ref_update_entries(values: &mut [RefUpdateEntry]) {
    values.sort_by(|left, right| {
        left.location
            .cmp(&right.location)
            .then_with(|| left.from.cmp(&right.from))
            .then_with(|| left.to.cmp(&right.to))
    });
}

fn sort_reference_updates(values: &mut [ReferenceUpdate]) {
    values.sort_by(|left, right| {
        left.entity_type
            .cmp(&right.entity_type)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn collect_varp_entity_ids(entities: &[ConflictEntry]) -> Vec<(VarDomain, u32)> {
    let mut values = entities
        .iter()
        .filter_map(|entity| {
            entity_type_label_to_domain(&entity.entity_type).map(|domain| (domain, entity.id))
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|(domain, id)| (domain.as_label().to_string(), *id));
    values.dedup();
    values
}

fn collect_varbit_entity_ids(entities: &[ConflictEntry]) -> BTreeSet<u32> {
    entities
        .iter()
        .filter(|entity| entity.entity_type == "varbit")
        .map(|entity| entity.id)
        .collect()
}

fn entity_type_label_to_domain(label: &str) -> Option<VarDomain> {
    match label {
        "varplayer" => Some(VarDomain::Player),
        "varnpc" => Some(VarDomain::Npc),
        "varclient" => Some(VarDomain::Client),
        "varworld" => Some(VarDomain::World),
        "varregion" => Some(VarDomain::Region),
        "varobject" => Some(VarDomain::Object),
        "varclan" => Some(VarDomain::Clan),
        "varclansetting" => Some(VarDomain::ClanSetting),
        "varcontroller" => Some(VarDomain::Controller),
        "varglobal" => Some(VarDomain::Global),
        "varplayergroup" => Some(VarDomain::PlayerGroup),
        _ => None,
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum DependencySiteValidation {
    Resolved,
    Missing(String),
    Unsupported,
}

fn simple_site_exists(site: &DependencySite, exists: bool) -> DependencySiteValidation {
    if exists {
        DependencySiteValidation::Resolved
    } else {
        DependencySiteValidation::Missing(format!(
            "{} {} missing at {}",
            site.entity_type.as_label(),
            site.id,
            site.location
        ))
    }
}

fn summarize_target_validation(
    components: &[ComponentTargetValidation],
    scripts: &[ScriptTargetValidation],
) -> TargetValidationSummary {
    let mut summary = TargetValidationSummary {
        components_checked: components.len(),
        components_blocked: components
            .iter()
            .filter(|component| !component.blocking_issues.is_empty())
            .count(),
        scripts_checked: scripts.len(),
        ..TargetValidationSummary::default()
    };

    for component in components {
        summary.dependency_sites += component.dependency_sites;
        summary.exact_sites += component
            .dependency_sites
            .saturating_sub(component.heuristic_sites.len());
        summary.heuristic_sites += component.heuristic_sites.len();
        summary.unsupported_sites += component.unsupported_sites.len();
    }

    for script in scripts {
        if script.encoded_bytes.is_some() {
            summary.scripts_encoded += 1;
        }
        summary.dependency_sites += script.dependency_sites;
        summary.exact_sites += script
            .dependency_sites
            .saturating_sub(script.heuristic_sites.len());
        summary.heuristic_sites += script.heuristic_sites.len();
        summary.unsupported_sites += script.unsupported_sites.len();
        if script.failure.is_some() {
            summary.scripts_blocked += 1;
            continue;
        }
        if script.validation_errors.is_empty() && script.blockers.is_empty() {
            summary.scripts_valid += 1;
        } else {
            if !script.validation_errors.is_empty() {
                summary.scripts_with_errors += 1;
            }
            if !script.blockers.is_empty() {
                summary.scripts_blocked += 1;
            }
        }
        if !script.validation_warnings.is_empty() {
            summary.scripts_with_warnings += 1;
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::{MigrationAnalyzer, RemapTable, VarpRemapTarget};
    use crate::config::ParamEntry;
    use crate::dep_tree::ResolverContext;
    use crate::interface::{ComponentDeps, VarTransmitRef};
    use crate::script::{CompiledScript, Instruction, OpcodeBook, Operand, encode_script};
    use crate::vars::{VarBitEntry, VarDomain, VarEntry};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::PathBuf;

    fn test_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
    }

    fn simple_script(name: &str, instructions: Vec<Instruction>, locals: u16) -> CompiledScript {
        CompiledScript {
            name: Some(name.to_string()),
            local_count_int: locals,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: instructions,
        }
    }

    fn build_ctx(
        build: u32,
        scripts: &[(u32, CompiledScript)],
        varps: &[(VarDomain, u32, VarEntry)],
        varbits: &[(u32, VarBitEntry)],
        components: &[(u32, u32, ComponentDeps)],
    ) -> crate::error::Result<ResolverContext> {
        let opcode_book = OpcodeBook::load(&test_data_dir(), build, 0)?;
        let mut raw_scripts = BTreeMap::new();
        let mut decoded_scripts = BTreeMap::new();
        for (script_group_id, script) in scripts {
            let packed_id = script_group_id << 16;
            raw_scripts.insert(packed_id, encode_script(script, &opcode_book, build)?);
            decoded_scripts.insert(packed_id, script.clone());
        }

        let mut varps_by_domain: HashMap<VarDomain, BTreeMap<u32, VarEntry>> = HashMap::new();
        for (domain, id, entry) in varps {
            varps_by_domain
                .entry(*domain)
                .or_default()
                .insert(*id, entry.clone());
        }

        let mut varbit_map = BTreeMap::new();
        for (id, entry) in varbits {
            varbit_map.insert(*id, entry.clone());
        }

        let mut parsed_components = BTreeMap::new();
        for (interface_id, component_id, deps) in components {
            parsed_components
                .entry(*interface_id)
                .or_insert_with(BTreeMap::new)
                .insert(*component_id, deps.clone());
        }

        Ok(ResolverContext {
            build,
            opcode_book,
            interfaces: BTreeMap::new(),
            scripts: raw_scripts,
            varps_by_domain,
            varbits: varbit_map,
            params: BTreeMap::<u32, ParamEntry>::new(),
            enums: BTreeMap::new(),
            structs: BTreeMap::new(),
            decoded_scripts,
            parsed_components,
            npcs: BTreeMap::new(),
            objs: BTreeMap::new(),
            locs: BTreeMap::new(),
            seqs: BTreeMap::new(),
            spots: BTreeMap::new(),
            invs: BTreeMap::new(),
            dbtables: BTreeMap::new(),
            dbrows: BTreeMap::new(),
        })
    }

    fn player_var(id: u32) -> VarEntry {
        VarEntry {
            id,
            domain: VarDomain::Player,
            var_name: format!("varplayer_{id}"),
            type_id: Some(0),
            lifetime: None,
            transmit_level: None,
            client_code: None,
            domain_default: true,
            wiki_sync: false,
        }
    }

    fn player_varbit(id: u32, base_var: u32) -> VarBitEntry {
        VarBitEntry {
            id,
            varbit_name: format!("varbit_{id}"),
            domain: Some(VarDomain::Player),
            base_var: Some(base_var),
            start_bit: Some(0),
            end_bit: Some(1),
            wiki_sync: false,
        }
    }

    #[test]
    fn validate_script_target_rewrites_and_encodes_dependency_bundle() -> crate::error::Result<()> {
        let source_root = simple_script(
            "[proc,source_root]",
            vec![
                Instruction {
                    opcode: 0,
                    command: "push_var".to_string(),
                    operand: Operand::VarRef(crate::script::VarRef {
                        domain: VarDomain::Player,
                        id: 1,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varbit".to_string(),
                    operand: Operand::VarBitRef(crate::script::VarBitRef {
                        id: 5,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "gosub_with_params".to_string(),
                    operand: Operand::Script(200),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
            1,
        );
        let source_leaf = simple_script(
            "[proc,source_leaf]",
            vec![Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            }],
            0,
        );
        let source_ctx = build_ctx(
            947,
            &[(100, source_root), (200, source_leaf)],
            &[(VarDomain::Player, 1, player_var(1))],
            &[(5, player_varbit(5, 1))],
            &[],
        )?;
        let target_ctx = build_ctx(910, &[], &[], &[], &[])?;

        let analyzer = MigrationAnalyzer::new(source_ctx, target_ctx);
        let script_report = analyzer.analyze_script(100);
        let mut remap = RemapTable::default();
        remap.scripts.insert(200, 700);
        remap.varps.insert(
            "player:1".to_string(),
            VarpRemapTarget {
                domain: "player".to_string(),
                id: 500,
            },
        );
        remap.varbits.insert(5, 600);

        let target_validation =
            analyzer.validate_script_target(&script_report.entities, Some(&remap), false);
        assert_eq!(target_validation.summary.scripts_checked, 2);
        assert_eq!(target_validation.summary.scripts_encoded, 2);
        assert_eq!(target_validation.summary.scripts_valid, 2);
        assert_eq!(target_validation.summary.scripts_with_errors, 0);
        assert_eq!(target_validation.summary.scripts_blocked, 0);

        let root = target_validation
            .scripts
            .iter()
            .find(|script| script.source_script_id == 100)
            .expect("root script validation");
        assert!(
            root.reference_updates
                .iter()
                .any(|update| update.to == "varp player:500")
        );
        assert!(
            root.reference_updates
                .iter()
                .any(|update| update.to == "varbit 600")
        );
        assert!(
            root.reference_updates
                .iter()
                .any(|update| update.to == "script 700")
        );
        Ok(())
    }

    #[test]
    fn validate_interface_target_checks_component_refs_against_overlay_bundle()
    -> crate::error::Result<()> {
        let source_root = simple_script(
            "[proc,source_root]",
            vec![Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            }],
            0,
        );
        let mut component = ComponentDeps {
            component_type: "layer".to_string(),
            name: Some("stockmarket".to_string()),
            children: Vec::new(),
            scripts: HashSet::new(),
            onload_scripts: HashSet::new(),
            varps: HashSet::new(),
            varbits: HashSet::new(),
            invs: HashSet::new(),
            stats: HashSet::new(),
            graphics: HashSet::new(),
            models: HashSet::new(),
            cursors: HashSet::new(),
            stylesheets: HashSet::new(),
            params: HashSet::new(),
            seqs: HashSet::new(),
            fontmetrics: HashSet::new(),
            textures: HashSet::new(),
            enums: HashSet::new(),
        };
        component.scripts.insert(100);
        component.varps.insert(VarTransmitRef::Player(1));
        component.varbits.insert(5);

        let source_ctx = build_ctx(
            947,
            &[(100, source_root)],
            &[(VarDomain::Player, 1, player_var(1))],
            &[(5, player_varbit(5, 1))],
            &[(77, 1, component)],
        )?;
        let target_ctx = build_ctx(910, &[], &[], &[], &[])?;

        let analyzer = MigrationAnalyzer::new(source_ctx, target_ctx);
        let interface_report = analyzer.analyze_interface(77);
        let target_validation =
            analyzer.validate_interface_target(77, &interface_report.entities, None, false);

        assert_eq!(target_validation.summary.components_checked, 1);
        assert_eq!(target_validation.summary.components_blocked, 0);
        assert_eq!(target_validation.summary.scripts_checked, 1);
        assert_eq!(target_validation.summary.scripts_valid, 1);
        assert!(
            target_validation
                .components
                .iter()
                .all(|component| component.blocking_issues.is_empty())
        );
        Ok(())
    }
}
