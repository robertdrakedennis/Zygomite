//! Script-migration methods on [`MigrationAnalyzer`](super::MigrationAnalyzer).
//!
//! A second inherent `impl MigrationAnalyzer` block holding the clearly
//! script-specific walk/compare/remap/rewrite/validate methods. Split out of the
//! flat `migrate.rs` (behavior-preserving); method bodies are unchanged.

use super::{
    ConflictEntry, ConflictStatus, FieldDiff, MigrationAnalyzer, PreparedScriptOverlay,
    RefUpdateEntry, ReferenceUpdate, RemapTable, ScriptReport, ScriptTargetValidation,
    TargetValidationReport, asset_command_info, non_empty, push_diff, push_diff_opt,
    sort_ref_update_entries, summarize_target_validation,
};
use crate::dep_tree::{EntityKey, EntityType, ResolverContext};
use crate::overlay_deps::{DependencyConfidence, collect_script_dependency_sites};
use crate::script::{CompiledScript, Operand, decode_script, encode_script};
use crate::transpile::ScriptCatalog;
use crate::validate::Cs2Validator;
use rayon::prelude::*;
use std::collections::HashSet;

impl MigrationAnalyzer {
    pub(super) fn walk_script(
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

    pub(super) fn compare_script(&self, id: u32) -> (ConflictStatus, Option<Vec<FieldDiff>>) {
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

    pub(super) fn validate_target_scripts_from_overlays(
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

    pub(super) fn prepare_script_overlay(
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

    pub(super) fn collect_script_ref_updates(
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
}
