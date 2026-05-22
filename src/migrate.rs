#![allow(clippy::ref_option)]

use crate::config::ScalarValue;
use crate::dep_tree::{EntityKey, EntityType, ResolverContext};
use crate::interface::{ComponentDeps, VarTransmitRef};
use crate::vars::VarDomain;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

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
}

impl MigrationAnalyzer {
    pub fn new(source: ResolverContext, target: ResolverContext) -> Self {
        Self { source, target }
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
        for &script_id in &comp_deps.scripts {
            let key = EntityKey::new(EntityType::Script, script_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Script, script_id, None, entities, visited);
                self.walk_script(script_id, entities, visited);
            }
        }
        for var_ref in &comp_deps.varps {
            let (et, id) = var_transmit_to_entity(var_ref);
            let key = EntityKey::new(et, id);
            if visited.insert(key) {
                let name = self
                    .source
                    .varps_by_domain
                    .get(&Self::var_ref_domain(var_ref))
                    .and_then(|vars| vars.get(&id))
                    .map(|v| v.var_name.clone());
                self.collect_entity(et, id, name, entities, visited);
            }
        }
        for &varbit_id in &comp_deps.varbits {
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
        for &enum_id in &comp_deps.enums {
            if visited.insert(EntityKey::new(EntityType::Enum, enum_id)) {
                self.collect_entity(EntityType::Enum, enum_id, None, entities, visited);
            }
        }
        for &param_id in &comp_deps.params {
            if visited.insert(EntityKey::new(EntityType::Param, param_id)) {
                self.collect_entity(EntityType::Param, param_id, None, entities, visited);
            }
        }
        for &model_id in &comp_deps.models {
            if visited.insert(EntityKey::new(EntityType::Model, model_id)) {
                self.collect_entity(EntityType::Model, model_id, None, entities, visited);
            }
        }
        for &seq_id in &comp_deps.seqs {
            if visited.insert(EntityKey::new(EntityType::Seq, seq_id)) {
                self.collect_entity(EntityType::Seq, seq_id, None, entities, visited);
            }
        }
        for &graphic_id in &comp_deps.graphics {
            if visited.insert(EntityKey::new(EntityType::Graphic, graphic_id)) {
                self.collect_entity(EntityType::Graphic, graphic_id, None, entities, visited);
            }
        }
        for &inv_id in &comp_deps.invs {
            if visited.insert(EntityKey::new(EntityType::Inv, inv_id)) {
                self.collect_entity(EntityType::Inv, inv_id, None, entities, visited);
            }
        }
        // Asset types tracked for completeness; cannot be deeply
        // compared without full archive loading.
        for &cursor_id in &comp_deps.cursors {
            if visited.insert(EntityKey::new(EntityType::Cursor, cursor_id)) {
                self.collect_entity(EntityType::Cursor, cursor_id, None, entities, visited);
            }
        }
        for &font_id in &comp_deps.fontmetrics {
            if visited.insert(EntityKey::new(EntityType::FontMetrics, font_id)) {
                self.collect_entity(EntityType::FontMetrics, font_id, None, entities, visited);
            }
        }
        for &tex_id in &comp_deps.textures {
            if visited.insert(EntityKey::new(EntityType::Texture, tex_id)) {
                self.collect_entity(EntityType::Texture, tex_id, None, entities, visited);
            }
        }
        for &ss_id in &comp_deps.stylesheets {
            if visited.insert(EntityKey::new(EntityType::Stylesheet, ss_id)) {
                self.collect_entity(EntityType::Stylesheet, ss_id, None, entities, visited);
            }
        }
        for &stat_id in &comp_deps.stats {
            if visited.insert(EntityKey::new(EntityType::Config, stat_id)) {
                self.collect_entity(EntityType::Config, stat_id, None, entities, visited);
            }
        }
    }

    fn walk_script(
        &self,
        script_id: u32,
        entities: &mut Vec<ConflictEntry>,
        visited: &mut HashSet<EntityKey>,
    ) {
        if let Some(script) = self.source.decoded_scripts.get(&script_id) {
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
        let source_exists = self.source.interfaces.values().any(|g| g.contains_key(&id));
        let target_exists = self.target.interfaces.values().any(|g| g.contains_key(&id));

        match (source_exists, target_exists) {
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
        match (s, t) {
            (Some(_), None) => (ConflictStatus::Missing, None),
            (None, Some(_)) => (ConflictStatus::IdConflict, None),
            (None, None) => (ConflictStatus::Missing, None),
            (Some(s), Some(t)) => {
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
            self.source.decoded_scripts.get(&id),
            self.target.decoded_scripts.get(&id),
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
                let s = self.source.decoded_scripts.get(&id);
                let t = self.target.decoded_scripts.get(&id);
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
                .source
                .decoded_scripts
                .get(&id)
                .and_then(|s| s.name.clone()),
            _ => None,
        }
    }

    fn build_report(&self, group_id: u32, entities: Vec<ConflictEntry>) -> ConflictReport {
        let mut summary = ConflictSummary::default();
        for e in &entities {
            match e.status {
                ConflictStatus::Safe => summary.safe += 1,
                ConflictStatus::Missing => summary.missing += 1,
                ConflictStatus::IdConflict => summary.id_conflict += 1,
                ConflictStatus::Changed => summary.changed += 1,
                ConflictStatus::ScriptChanged => summary.script_changed += 1,
                ConflictStatus::Unknown => summary.unknown += 1,
                ConflictStatus::Asset => summary.asset += 1,
            }
        }
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

        let mut summary = ConflictSummary::default();
        for e in &entities {
            match e.status {
                ConflictStatus::Safe => summary.safe += 1,
                ConflictStatus::Missing => summary.missing += 1,
                ConflictStatus::IdConflict => summary.id_conflict += 1,
                ConflictStatus::Changed => summary.changed += 1,
                ConflictStatus::ScriptChanged => summary.script_changed += 1,
                ConflictStatus::Unknown => summary.unknown += 1,
                ConflictStatus::Asset => summary.asset += 1,
            }
        }

        let script_name = self
            .source
            .decoded_scripts
            .get(&script_id)
            .and_then(|s| s.name.clone());

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
                .target
                .decoded_scripts
                .keys()
                .copied()
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
                for &script_id in &comp_deps.scripts {
                    self.collect_script_ref_updates(script_id, remap, &mut updates, &mut visited);
                }
            }
        }

        // Walk components in the dependency tree
        if let Some(comps) = self.source.parsed_components.get(&group_id) {
            for (&comp_id, comp_deps) in comps {
                let mut comp_updates = Vec::new();

                for &script_id in &comp_deps.scripts {
                    if let Some(&new_id) = remap.scripts.get(&script_id) {
                        comp_updates.push(RefUpdateEntry {
                            location: format!("scripts[{script_id}]"),
                            from: format!("script {script_id}"),
                            to: format!("script {new_id}"),
                        });
                    }
                }

                for var_ref in &comp_deps.varps {
                    let (_, id) = var_transmit_to_entity(var_ref);
                    let domain = Self::var_ref_domain(var_ref);
                    let key = format!("{}:{}", domain.as_label(), id);
                    if let Some(target) = remap.varps.get(&key) {
                        comp_updates.push(RefUpdateEntry {
                            location: format!("varps[{key}]"),
                            from: format!("varp {key}"),
                            to: format!("varp {}:{}", target.domain, target.id),
                        });
                    }
                }

                for &varbit_id in &comp_deps.varbits {
                    if let Some(&new_id) = remap.varbits.get(&varbit_id) {
                        comp_updates.push(RefUpdateEntry {
                            location: format!("varbits[{varbit_id}]"),
                            from: format!("varbit {varbit_id}"),
                            to: format!("varbit {new_id}"),
                        });
                    }
                }

                if !comp_updates.is_empty() {
                    updates.push(ReferenceUpdate {
                        entity_type: "component".to_string(),
                        id: comp_id,
                        updates: comp_updates,
                    });
                }
            }
        }

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

        if let Some(script) = self.source.decoded_scripts.get(&script_id) {
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
