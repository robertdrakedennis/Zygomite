//! Interface-migration methods on [`MigrationAnalyzer`](super::MigrationAnalyzer).
//!
//! A second inherent `impl MigrationAnalyzer` block holding the clearly
//! interface-specific analysis/remap/validation methods. Split out of the flat
//! `migrate.rs` (behavior-preserving); method bodies are unchanged.

use super::{
    ComponentTargetValidation, ConflictEntry, ConflictReport, ConflictStatus,
    DependencySiteValidation, FieldDiff, MigrationAnalyzer, RemapTable, TargetValidationReport,
    non_empty, push_diff, push_diff_opt, sorted_hashset_ids, summarize_target_validation,
    var_transmit_to_entity,
};
use crate::dep_tree::{EntityKey, EntityType, ResolverContext};
use crate::interface::ComponentDeps;
use crate::overlay_deps::{DependencyConfidence, collect_component_dependency_sites};
use crate::transpile::ScriptCatalog;
use crate::validate::extend_validation_catalog;
use rayon::prelude::*;
use std::collections::HashSet;

impl MigrationAnalyzer {
    pub(super) fn walk_component_deps(
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

    pub(super) fn compare_component(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
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
}
