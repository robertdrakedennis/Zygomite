//! Ref / dependency model for `overlay-plan` (concern 3): `RefKind`,
//! `SemanticRefKey`, the archive/target descriptors, and the semantic
//! reference-graph repositories used by the planner.
//!
//! Moved verbatim from the former flat `overlay_plan.rs`.

use super::MAX_EDGE_SAMPLES;
use super::plan_output::{DependencyEdgeSample, OverlayWarning};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub type SemanticRefBuckets = HashMap<u32, HashMap<SemanticRefKey, Vec<u32>>>;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RefKind {
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
    pub fn as_str(self) -> &'static str {
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

    pub fn from_entity_type(entity_type: &str) -> Option<Self> {
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
pub struct ArchiveDef {
    pub id: u32,
    pub donor_id: u32,
    pub name: &'static str,
}

#[derive(Debug, Clone)]
pub struct ConfigTarget {
    pub archive: ArchiveDef,
    pub group_id: u32,
    pub file_id: u32,
    pub id: u32,
    pub kind: RefKind,
}

#[derive(Debug, Clone)]
pub struct RawGroupTarget {
    pub archive: ArchiveDef,
    pub group_id: u32,
    pub id: u32,
    pub kind: RefKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Primary,
    Dependency,
}

#[derive(Debug, Clone)]
pub struct PendingRef {
    pub kind: RefKind,
    pub id: u32,
    pub source: String,
    pub mode: SelectionMode,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum RootKind {
    Base,
    Donor,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum SemanticRefKey {
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
    pub fn from_label(label: &str) -> Self {
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

    pub fn as_label(&self) -> &str {
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
pub struct RefGraphRepository {
    pub graphs: HashMap<String, HashMap<u32, HashMap<SemanticRefKey, Vec<u32>>>>,
}

#[derive(Debug, Clone)]
pub struct ConfigSemanticIndex {
    pub ref_graph: RefGraphRepository,
    pub dependency_edges_sample: Vec<DependencyEdgeSample>,
    pub edge_sample_overflow: usize,
    pub missing_refs_kinds_logged: HashSet<String>,
    pub partial_refs_kinds_logged: HashSet<String>,
}

impl ConfigSemanticIndex {
    pub fn new(semantic_root: &Path) -> Result<Self> {
        Ok(Self {
            ref_graph: RefGraphRepository::new(semantic_root)?,
            dependency_edges_sample: Vec::new(),
            edge_sample_overflow: 0,
            missing_refs_kinds_logged: HashSet::new(),
            partial_refs_kinds_logged: HashSet::new(),
        })
    }

    pub fn record_dependency_edge(&mut self, edge: DependencyEdgeSample) {
        if self.dependency_edges_sample.len() < MAX_EDGE_SAMPLES {
            self.dependency_edges_sample.push(edge);
        } else {
            self.edge_sample_overflow += 1;
        }
    }

    pub fn edge_sample_warning(&self) -> Option<OverlayWarning> {
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
    pub fn new(semantic_root: &Path) -> Result<Self> {
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

    pub fn has_kind(&self, kind: &str) -> bool {
        self.graphs.contains_key(semantic_refs_file_name(kind))
    }

    pub fn get_refs(&self, kind: &str, id: u32) -> Option<&HashMap<SemanticRefKey, Vec<u32>>> {
        // Semantic kinds and refs file names differ for spotanims ("spotanim" config kind,
        // refs/spot.json); normalise so spot configs get dependency closure instead of a
        // missing-refs heuristic gap.
        self.graphs.get(semantic_refs_file_name(kind))?.get(&id)
    }

    pub fn get_dbrow_table_id(&self, row_id: u32) -> Option<u32> {
        self.get_refs("dbrow", row_id)?
            .get(&SemanticRefKey::DbTable)?
            .first()
            .copied()
    }
}

pub fn normalize_graph_ref_kind(kind: &SemanticRefKey) -> Option<RefKind> {
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

pub fn semantic_refs_file_name(kind: &str) -> &str {
    if kind == "spotanim" { "spot" } else { kind }
}
