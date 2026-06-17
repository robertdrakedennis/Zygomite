//! Input manifest schema for `overlay-plan` (concern 1): the deserialized
//! cacheoverlay manifest plus the resolved `OverlayRoots` output struct.
//!
//! Moved verbatim from the former flat `overlay_plan.rs`; serde field names,
//! order, attributes, and types are unchanged so emitted JSON bytes are stable.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheOverlayManifest {
    #[serde(default)]
    pub roots: OverlayRootOverrides,
    #[serde(default)]
    pub base_raw_root: Option<PathBuf>,
    #[serde(default)]
    pub donor_raw_root: Option<PathBuf>,
    #[serde(default)]
    pub base_semantic_root: Option<PathBuf>,
    #[serde(default)]
    pub donor_semantic_root: Option<PathBuf>,
    #[serde(default)]
    pub base_pack_root: Option<PathBuf>,
    #[serde(default)]
    pub output_pack_root: Option<PathBuf>,
    #[serde(default)]
    pub client_output_pack_root: Option<PathBuf>,
    #[serde(default)]
    pub imports: OverlayImports,
    #[serde(default)]
    pub archive_modes: BTreeMap<String, ArchiveMode>,
    #[serde(default)]
    pub conflict_policy: Option<String>,
    #[serde(default)]
    pub allow: OverlayAllow,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_field_names,
    reason = "overlay manifest JSON contract uses *_root field names"
)]
pub struct OverlayRootOverrides {
    #[serde(default)]
    pub base_raw_root: Option<PathBuf>,
    #[serde(default)]
    pub donor_raw_root: Option<PathBuf>,
    #[serde(default)]
    pub base_semantic_root: Option<PathBuf>,
    #[serde(default)]
    pub donor_semantic_root: Option<PathBuf>,
    #[serde(default)]
    pub base_pack_root: Option<PathBuf>,
    #[serde(default)]
    pub output_pack_root: Option<PathBuf>,
    #[serde(default)]
    pub client_output_pack_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayImports {
    #[serde(default)]
    pub map_archive: Option<String>,
    #[serde(default)]
    pub full_archives: Vec<ArchiveRef>,
    #[serde(default)]
    pub config_groups: Vec<u32>,
    #[serde(default)]
    pub maps: Vec<u32>,
    #[serde(default)]
    pub regions: Vec<RegionSpec>,
    #[serde(default)]
    pub objs: Vec<u32>,
    #[serde(default)]
    pub npcs: Vec<u32>,
    #[serde(default)]
    pub locs: Vec<u32>,
    #[serde(default)]
    pub seqs: Vec<u32>,
    #[serde(default)]
    pub bas: Vec<u32>,
    #[serde(default)]
    pub spots: Vec<u32>,
    #[serde(default)]
    pub structs: Vec<u32>,
    #[serde(default)]
    pub quests: Vec<u32>,
    #[serde(default)]
    pub enums: Vec<u32>,
    #[serde(default)]
    pub varbits: Vec<u32>,
    #[serde(default)]
    pub varps: Vec<u32>,
    #[serde(default)]
    pub db_tables: Vec<u32>,
    #[serde(default)]
    pub db_rows: Vec<u32>,
    #[serde(default)]
    pub interfaces: Vec<u32>,
    #[serde(default)]
    pub scripts: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArchiveRef {
    Name(String),
    Id(u32),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RegionSpec {
    Id(u32),
    Text(String),
    Coord { x: u32, z: u32 },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ArchiveMode {
    Auto,
    Patch,
    HardSwap,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayAllow {
    #[serde(default)]
    pub db_table_schema_changes: Vec<u32>,
    #[serde(default)]
    pub enum_ids: Vec<u32>,
    #[serde(default)]
    pub varbit_ids: Vec<u32>,
    #[serde(default)]
    pub varp_ids: Vec<u32>,
    #[serde(default)]
    pub varbit_conflict_ids: Vec<u32>,
    #[serde(default)]
    pub varp_conflict_ids: Vec<u32>,
    #[serde(default)]
    pub hard_swap_archives: Vec<ArchiveRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_field_names,
    reason = "overlay plan JSON contract uses *_root field names"
)]
pub struct OverlayRoots {
    pub base_raw_root: String,
    pub donor_raw_root: String,
    pub base_semantic_root: String,
    pub donor_semantic_root: String,
    pub base_pack_root: String,
    pub output_pack_root: String,
    pub client_output_pack_root: String,
}
