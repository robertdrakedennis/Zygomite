//! Migration conflict report + remap/validation data model.
//!
//! The serde-serialized report types produced by [`MigrationAnalyzer`](super::MigrationAnalyzer).
//! Moved verbatim out of the flat `migrate.rs` (behavior-preserving); field
//! names/order/attrs/types are unchanged so the JSON surface is identical.

use crate::overlay_deps::DependencySite;
use crate::validate::ValidationError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub(super) fn new() -> Self {
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
