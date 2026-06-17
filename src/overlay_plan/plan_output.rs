//! Output plan schema for `overlay-plan` (concern 2): `OverlayPlanOutput` and
//! its serde sub-structs that define the emitted JSON shape.
//!
//! Moved verbatim from the former flat `overlay_plan.rs`. The serde field
//! names, order, `#[serde(...)]` attributes, and types here ARE the JSON
//! contract — do not change them.

use super::manifest::OverlayRoots;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanOutput {
    pub roots: OverlayRoots,
    pub conflict_policy: String,
    pub hard_swap_archives: Vec<String>,
    pub patch_archives: Vec<String>,
    pub selected: OverlayPlanSelected,
    pub imports: OverlayPlanImports,
    pub dependencies: BTreeMap<String, Vec<u32>>,
    pub db: OverlayPlanDb,
    pub blocked_conflicts: Vec<OverlayBlockedIssue>,
    pub warnings: Vec<OverlayWarning>,
    pub semantic_source: &'static str,
    pub semantic_manifest: OverlaySemanticManifest,
    pub dependency_edges_sample: Vec<DependencyEdgeSample>,
    pub plan_version: u32,
    pub planner_fingerprint: String,
    pub proof: OverlayPlanProof,
    pub audit: OverlayPlanAudit,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanSelected {
    pub groups: Vec<OverlayPlanArchiveGroups>,
    pub files: Vec<OverlayPlanArchiveFiles>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanArchiveGroups {
    pub archive: String,
    pub archive_id: u32,
    pub mode: &'static str,
    pub groups: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanArchiveFiles {
    pub archive: String,
    pub archive_id: u32,
    pub mode: &'static str,
    pub group_id: u32,
    pub file_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanImports {
    pub config_groups: Vec<u32>,
    pub maps: Vec<u32>,
    pub objs: Vec<u32>,
    pub npcs: Vec<u32>,
    pub locs: Vec<u32>,
    pub structs: Vec<u32>,
    pub enums: Vec<u32>,
    pub varbits: Vec<u32>,
    pub varps: Vec<u32>,
    pub db_tables: Vec<u32>,
    pub db_rows: Vec<u32>,
    pub interfaces: Vec<u32>,
    pub scripts: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanDb {
    pub tables: Vec<u32>,
    pub rows: Vec<u32>,
    pub index_groups: Vec<u32>,
    pub schema_changes: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySemanticManifest {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub donor_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Rs3CacheManifest {
    pub tool_version: String,
    pub build: u32,
    pub subbuild: u32,
    pub cache_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayWarning {
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayBlockedIssue {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_kind: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanProof {
    pub status: &'static str,
    pub strict: bool,
    pub unsupported_site_count: usize,
    pub heuristic_site_count: usize,
    pub script_summary: OverlayProofSummary,
    pub component_summary: OverlayProofSummary,
    pub next_actions: Vec<String>,
    pub blockers: Vec<OverlayProofIssue>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayProofSummary {
    pub checked: usize,
    pub blocked: usize,
    pub valid: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayProofIssue {
    pub kind: &'static str,
    pub location: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_kind: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPlanAudit {
    pub relative_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdgeSample {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub reason: &'static str,
}
