use crate::config::ScalarValue;
use crate::dep_tree::{EntityKey, EntityRef, EntityType, ResolverContext};
use crate::interface::{ComponentDeps, VarTransmitRef};
use crate::vars::VarDomain;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ── Migration conflict report structures ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConflictStatus {
    /// Entity exists identically in both builds.
    Safe,
    /// Entity exists in source (947) but not in target (910) at all.
    Missing,
    /// A different entity occupies the same ID in the target build.
    IdConflict,
    /// Same entity exists but properties differ (name, values, type, etc.).
    Changed,
    /// Script exists in both but bytecode differs.
    ScriptChanged,
    /// Could not compare (e.g., one side failed to decode).
    Unknown,
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
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConflictSummary {
    pub safe: usize,
    pub missing: usize,
    pub id_conflict: usize,
    pub changed: usize,
    pub script_changed: usize,
    pub unknown: usize,
}

// ── Analyzer ──

pub struct MigrationAnalyzer {
    source: ResolverContext,
    target: ResolverContext,
}

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

impl MigrationAnalyzer {
    pub fn new(source: ResolverContext, target: ResolverContext) -> Self {
        Self { source, target }
    }

    pub fn analyze_interface(&self, group_id: u32) -> ConflictReport {
        let _total_components = self
            .source
            .parsed_components
            .get(&group_id)
            .map(std::collections::BTreeMap::len)
            .unwrap_or(0);

        let mut entities = Vec::new();
        let mut visited: HashSet<EntityKey> = HashSet::new();

        // Walk the full dependency tree from the source
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
        // Scripts
        for &script_id in &comp_deps.scripts {
            let key = EntityKey::new(EntityType::Script, script_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Script, script_id, None, entities, visited);
                self.walk_script(script_id, entities, visited);
            }
        }

        // Varp references
        for var_ref in &comp_deps.varps {
            let (entity_type, id) = var_transmit_to_entity(var_ref);
            let key = EntityKey::new(entity_type, id);
            if visited.insert(key) {
                let name = self
                    .source
                    .varps_by_domain
                    .get(&Self::var_ref_domain(var_ref))
                    .and_then(|vars| vars.get(&id))
                    .map(|v| v.var_name.clone());
                self.collect_entity(entity_type, id, name, entities, visited);
            }
        }

        // Varbits
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

        // Enums
        for &enum_id in &comp_deps.enums {
            let key = EntityKey::new(EntityType::Enum, enum_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Enum, enum_id, None, entities, visited);
            }
        }

        // Params
        for &param_id in &comp_deps.params {
            let key = EntityKey::new(EntityType::Param, param_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Param, param_id, None, entities, visited);
            }
        }

        // Models
        for &model_id in &comp_deps.models {
            let key = EntityKey::new(EntityType::Model, model_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Model, model_id, None, entities, visited);
            }
        }

        // Seqs
        for &seq_id in &comp_deps.seqs {
            let key = EntityKey::new(EntityType::Seq, seq_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Seq, seq_id, None, entities, visited);
            }
        }

        // Graphics
        for &graphic_id in &comp_deps.graphics {
            let key = EntityKey::new(EntityType::Graphic, graphic_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Graphic, graphic_id, None, entities, visited);
            }
        }

        // Invs
        for &inv_id in &comp_deps.invs {
            let key = EntityKey::new(EntityType::Inv, inv_id);
            if visited.insert(key) {
                self.collect_entity(EntityType::Inv, inv_id, None, entities, visited);
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
            for instruction in &script.code {
                match &instruction.operand {
                    crate::script::Operand::VarRef(var_ref) => {
                        let ref_entity = crate::dep_tree::var_ref_to_entity_ref(var_ref);
                        let (et, id) = Self::var_ref_type(&ref_entity);
                        let key = EntityKey::new(et, id);
                        if visited.insert(key) {
                            self.collect_entity(et, id, None, entities, visited);
                        }
                    }
                    crate::script::Operand::VarBitRef(vbr) => {
                        let id = u32::from(vbr.id);
                        let key = EntityKey::new(EntityType::VarBit, id);
                        if visited.insert(key) {
                            self.collect_entity(EntityType::VarBit, id, None, entities, visited);
                        }
                    }
                    crate::script::Operand::Script(called_id) => {
                        let id = *called_id as u32;
                        let key = EntityKey::new(EntityType::Script, id);
                        if visited.insert(key) {
                            self.collect_entity(EntityType::Script, id, None, entities, visited);
                            self.walk_script(id, entities, visited);
                        }
                    }
                    _ => {}
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
        let status = self.compare_entity(entity_type, id);
        let (source_summary, target_summary) = self.entity_summaries(entity_type, id);

        entities.push(ConflictEntry {
            entity_type: entity_type.as_label().to_string(),
            id,
            sub_id: None,
            name: name.or_else(|| self.lookup_name(entity_type, id)),
            status,
            source_summary,
            target_summary,
        });
    }

    fn compare_entity(&self, entity_type: EntityType, id: u32) -> ConflictStatus {
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
            _ => ConflictStatus::Unknown,
        }
    }

    fn compare_varp(&self, entity_type: EntityType, id: u32) -> ConflictStatus {
        let domain = Self::entity_type_to_domain(entity_type);
        let source_var =
            domain.and_then(|d| self.source.varps_by_domain.get(&d).and_then(|v| v.get(&id)));
        let target_var =
            domain.and_then(|d| self.target.varps_by_domain.get(&d).and_then(|v| v.get(&id)));

        match (source_var, target_var) {
            (Some(_), None) => ConflictStatus::Missing,
            (None, Some(_)) => ConflictStatus::IdConflict,
            (None, None) => ConflictStatus::Missing,
            (Some(s), Some(t)) => {
                if s.var_name == t.var_name && s.type_id == t.type_id && s.lifetime == t.lifetime {
                    ConflictStatus::Safe
                } else {
                    ConflictStatus::Changed
                }
            }
        }
    }

    fn compare_varbit(&self, id: u32) -> ConflictStatus {
        match (self.source.varbits.get(&id), self.target.varbits.get(&id)) {
            (Some(_), None) => ConflictStatus::Missing,
            (None, Some(_)) => ConflictStatus::IdConflict,
            (None, None) => ConflictStatus::Missing,
            (Some(s), Some(t)) => {
                if s.varbit_name == t.varbit_name
                    && s.base_var == t.base_var
                    && s.start_bit == t.start_bit
                    && s.end_bit == t.end_bit
                {
                    ConflictStatus::Safe
                } else {
                    ConflictStatus::Changed
                }
            }
        }
    }

    fn compare_script(&self, id: u32) -> ConflictStatus {
        match (
            self.source.decoded_scripts.get(&id),
            self.target.decoded_scripts.get(&id),
        ) {
            (Some(_), None) => ConflictStatus::Missing,
            (None, Some(_)) => ConflictStatus::IdConflict,
            (None, None) => ConflictStatus::Missing,
            (Some(s), Some(t)) => {
                if s.code.len() == t.code.len()
                    && s.argument_count_int == t.argument_count_int
                    && s.local_count_int == t.local_count_int
                    && s.name == t.name
                {
                    ConflictStatus::Safe
                } else {
                    ConflictStatus::ScriptChanged
                }
            }
        }
    }

    fn compare_enum(&self, id: u32) -> ConflictStatus {
        match (self.source.enums.get(&id), self.target.enums.get(&id)) {
            (Some(_), None) => ConflictStatus::Missing,
            (None, Some(_)) => ConflictStatus::IdConflict,
            (None, None) => ConflictStatus::Missing,
            (Some(s), Some(t)) => {
                if s.values.len() == t.values.len()
                    && s.input_type_char == t.input_type_char
                    && s.output_type_char == t.output_type_char
                {
                    ConflictStatus::Safe
                } else {
                    ConflictStatus::Changed
                }
            }
        }
    }

    fn compare_param(&self, id: u32) -> ConflictStatus {
        match (self.source.params.get(&id), self.target.params.get(&id)) {
            (Some(_), None) => ConflictStatus::Missing,
            (None, Some(_)) => ConflictStatus::IdConflict,
            (None, None) => ConflictStatus::Missing,
            (Some(s), Some(t)) => {
                if s.type_char == t.type_char
                    && s.type_id == t.type_id
                    && scalar_eq(s.default.as_ref(), t.default.as_ref())
                {
                    ConflictStatus::Safe
                } else {
                    ConflictStatus::Changed
                }
            }
        }
    }

    fn compare_seq(&self, id: u32) -> ConflictStatus {
        match (self.source.seqs.get(&id), self.target.seqs.get(&id)) {
            (Some(_), None) => ConflictStatus::Missing,
            (None, Some(_)) => ConflictStatus::IdConflict,
            (None, None) => ConflictStatus::Missing,
            (Some(s), Some(t)) => {
                if s.frames.len() == t.frames.len() && s.stretches == t.stretches {
                    ConflictStatus::Safe
                } else {
                    ConflictStatus::Changed
                }
            }
        }
    }

    fn compare_component(&self, id: u32) -> ConflictStatus {
        let source_comp = self
            .source
            .parsed_components
            .values()
            .find_map(|g| g.get(&id));
        let target_comp = self
            .target
            .parsed_components
            .values()
            .find_map(|g| g.get(&id));

        match (source_comp, target_comp) {
            (Some(_), None) => ConflictStatus::Missing,
            (None, Some(_)) => ConflictStatus::IdConflict,
            (None, None) => ConflictStatus::Missing,
            (Some(s), Some(t)) => {
                if s.component_type == t.component_type && s.name == t.name {
                    ConflictStatus::Safe
                } else {
                    ConflictStatus::Changed
                }
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
                let source = domain
                    .and_then(|d| self.source.varps_by_domain.get(&d).and_then(|v| v.get(&id)));
                let target = domain
                    .and_then(|d| self.target.varps_by_domain.get(&d).and_then(|v| v.get(&id)));
                (
                    source.map(|v| {
                        format!(
                            "name={} type={:?} lifetime={}",
                            v.var_name,
                            v.type_id,
                            v.lifetime.unwrap_or("unknown")
                        )
                    }),
                    target.map(|v| {
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
            EntityType::Enum => {
                let s = self.source.enums.get(&id);
                let t = self.target.enums.get(&id);
                (
                    s.map(|e| {
                        format!(
                            "values={} input={:?} output={:?}",
                            e.values.len(),
                            e.input_type_char.map(|c| c as char),
                            e.output_type_char.map(|c| c as char)
                        )
                    }),
                    t.map(|e| {
                        format!(
                            "values={} input={:?} output={:?}",
                            e.values.len(),
                            e.input_type_char.map(|c| c as char),
                            e.output_type_char.map(|c| c as char)
                        )
                    }),
                )
            }
            EntityType::Param => {
                let s = self.source.params.get(&id);
                let t = self.target.params.get(&id);
                (
                    s.map(|p| {
                        format!(
                            "type={:?} default={:?}",
                            p.type_char.map(|c| c as char),
                            p.default
                        )
                    }),
                    t.map(|p| {
                        format!(
                            "type={:?} default={:?}",
                            p.type_char.map(|c| c as char),
                            p.default
                        )
                    }),
                )
            }
            EntityType::Component => {
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
                (
                    s_comp.map(|c| format!("type={} name={:?}", c.component_type, c.name)),
                    t_comp.map(|c| format!("type={} name={:?}", c.component_type, c.name)),
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
            }
        }

        let components = self.source.parsed_components.get(&group_id);
        let total_components = components.map(std::collections::BTreeMap::len).unwrap_or(0);
        // Try to find a component with a name
        let interface_name =
            components.and_then(|comps| comps.values().find_map(|c| c.name.clone()));

        ConflictReport {
            source_build: self.source.build,
            target_build: self.target.build,
            interface_group: group_id,
            interface_name,
            total_components,
            total_entities: entities.len(),
            summary,
            entities,
        }
    }

    // ── Helpers ──

    fn var_ref_type(var_ref: &EntityRef) -> (EntityType, u32) {
        (var_ref.entity_type, var_ref.id)
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

fn scalar_eq(a: Option<&ScalarValue>, b: Option<&ScalarValue>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(ScalarValue::Int(ai)), Some(ScalarValue::Int(bi))) => ai == bi,
        (Some(ScalarValue::Long(al)), Some(ScalarValue::Long(bl))) => al == bl,
        (Some(ScalarValue::Str(a_str)), Some(ScalarValue::Str(b_str))) => a_str == b_str,
        _ => false,
    }
}
