use crate::cache::FlatCache;
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub(crate) mod builder;
pub(crate) mod manifest;
pub(crate) mod plan_output;
pub(crate) mod refs;

use builder::{
    PlanBuilder, build_selections, finalize_plan, manifest_fingerprint, print_json,
    read_semantic_manifest, resolve_roots, seed_imports, write_json, write_overlay_plan_audit,
};
use manifest::CacheOverlayManifest;
use plan_output::{OverlayProofIssue, Rs3CacheManifest};
use refs::ConfigSemanticIndex;

#[cfg(test)]
use builder::{build_overlay_proof, classify_warning_issue, normalize_archive_key};
#[cfg(test)]
use plan_output::{OverlayBlockedIssue, OverlayWarning};
#[cfg(test)]
use refs::{RefGraphRepository, SemanticRefKey};

pub const OVERLAY_PLAN_VERSION: u32 = 2;

const MAX_PLAN_WARNINGS: usize = 2048;
const MAX_EDGE_SAMPLES: usize = 256;

#[derive(Debug, Clone)]
pub(crate) struct ProofState {
    pub(crate) script_checked: usize,
    pub(crate) script_blocked: usize,
    pub(crate) script_valid: usize,
    pub(crate) component_checked: usize,
    pub(crate) component_blocked: usize,
    pub(crate) component_valid: usize,
    pub(crate) blockers: Vec<OverlayProofIssue>,
}

#[derive(Debug, Clone, Copy)]
pub struct OverlayPlanCommandOptions<'a> {
    pub manifest: &'a Path,
    pub out_file: Option<&'a Path>,
    pub audit_dir: Option<&'a Path>,
    pub allow_heuristic_sites: bool,
    pub data_dir: &'a Path,
    pub base_build: u32,
    pub donor_build: u32,
    pub base_subbuild: u32,
    pub donor_subbuild: u32,
}

pub fn run_overlay_plan_command(options: OverlayPlanCommandOptions<'_>) -> Result<()> {
    let OverlayPlanCommandOptions {
        manifest,
        out_file,
        audit_dir,
        allow_heuristic_sites,
        data_dir,
        base_build,
        donor_build,
        base_subbuild,
        donor_subbuild,
    } = options;
    ensure!(
        donor_build == 947 || donor_build == 948,
        "native overlay-plan supports donor builds 947 and 948 only"
    );
    let manifest_bytes =
        fs::read(manifest).with_context(|| format!("reading {}", manifest.display()))?;
    let manifest_value: CacheOverlayManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding overlay manifest {}", manifest.display()))?;
    let roots = resolve_roots(&manifest_value)?;
    let donor_manifest = read_semantic_manifest(&PathBuf::from(&roots.donor_semantic_root))?;
    let base_manifest = read_semantic_manifest(&PathBuf::from(&roots.base_semantic_root))?;
    ensure!(
        donor_manifest.build == donor_build && donor_manifest.subbuild == donor_subbuild,
        "donor semantic tree build mismatch: expected {}.{}, found {}.{}",
        donor_build,
        donor_subbuild,
        donor_manifest.build,
        donor_manifest.subbuild
    );
    ensure!(
        base_manifest.build == base_build && base_manifest.subbuild == base_subbuild,
        "base semantic tree build mismatch: expected {}.{}, found {}.{}",
        base_build,
        base_subbuild,
        base_manifest.build,
        base_manifest.subbuild
    );
    let cache_path = if audit_dir.is_none() {
        overlay_plan_cache_path(
            &manifest_bytes,
            &donor_manifest,
            &base_manifest,
            allow_heuristic_sites,
        )
    } else {
        None
    };
    if let Some(cache_path) = cache_path.as_ref().filter(|path| path.is_file()) {
        if let Some(path) = out_file {
            fs::copy(cache_path, path)
                .with_context(|| format!("copying cached overlay plan to {}", path.display()))?;
        } else {
            print!("{}", fs::read_to_string(cache_path)?);
        }
        eprintln!("overlay plan: cached");
        return Ok(());
    }

    let semantic_index = ConfigSemanticIndex::new(Path::new(&roots.donor_semantic_root))?;
    let mut builder = PlanBuilder {
        manifest: manifest_value,
        roots: roots.clone(),
        base_cache: FlatCache::open(&roots.base_raw_root)?,
        donor_cache: FlatCache::open(&roots.donor_raw_root)?,
        data_dir,
        base_build,
        donor_build,
        base_subbuild,
        donor_subbuild,
        group_selections: BTreeMap::new(),
        file_selections: BTreeMap::new(),
        primary_maps: BTreeSet::new(),
        primary_objs: BTreeSet::new(),
        primary_npcs: BTreeSet::new(),
        primary_locs: BTreeSet::new(),
        primary_structs: BTreeSet::new(),
        primary_enums: BTreeSet::new(),
        primary_varbits: BTreeSet::new(),
        primary_varps: BTreeSet::new(),
        primary_db_tables: BTreeSet::new(),
        primary_db_rows: BTreeSet::new(),
        primary_interfaces: BTreeSet::new(),
        primary_scripts: BTreeSet::new(),
        dependencies: BTreeMap::new(),
        warnings: Vec::new(),
        blocked: Vec::new(),
        pending: VecDeque::new(),
        seen_refs: HashSet::new(),
        indexes: HashMap::new(),
        config_group_files: HashMap::new(),
        full_archive_selections: BTreeSet::new(),
        auto_allowed_missing_varbits: BTreeSet::new(),
        auto_allowed_missing_varps: BTreeSet::new(),
        semantic_index,
        db_schema_changes: BTreeSet::new(),
        warning_overflow: 0,
        proof: ProofState {
            script_checked: 0,
            script_blocked: 0,
            script_valid: 0,
            component_checked: 0,
            component_blocked: 0,
            component_valid: 0,
            blockers: Vec::new(),
        },
        analyzer: None,
        donor_manifest,
        base_manifest,
    };

    seed_imports(&mut builder)?;
    build_selections(&mut builder, allow_heuristic_sites)?;
    let mut plan = finalize_plan(builder, allow_heuristic_sites)?;
    if let Some(dir) = audit_dir {
        plan.audit = write_overlay_plan_audit(dir, &plan.proof, &plan.blocked_conflicts)?;
    }

    if let Some(path) = out_file {
        write_json(path, &serde_json::to_value(&plan)?)?;
        if let Some(cache_path) = cache_path {
            if let Some(parent) = cache_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::copy(path, &cache_path)
                .with_context(|| format!("writing overlay plan cache {}", cache_path.display()))?;
        }
    } else {
        print_json(&serde_json::to_value(&plan)?)?;
    }

    eprintln!(
        "overlay plan: {} blocked conflicts, {} unsupported proof gap(s), {} heuristic proof gap(s)",
        plan.blocked_conflicts.len(),
        plan.proof.unsupported_site_count,
        plan.proof.heuristic_site_count
    );
    Ok(())
}

fn overlay_plan_cache_path(
    manifest_bytes: &[u8],
    donor_manifest: &Rs3CacheManifest,
    base_manifest: &Rs3CacheManifest,
    allow_heuristic_sites: bool,
) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    OVERLAY_PLAN_VERSION.hash(&mut hasher);
    allow_heuristic_sites.hash(&mut hasher);
    manifest_bytes.hash(&mut hasher);
    manifest_fingerprint(donor_manifest).hash(&mut hasher);
    manifest_fingerprint(base_manifest).hash(&mut hasher);
    Some(
        PathBuf::from(home)
            .join(".cache/alerion/rs3-cache-rs-overlay-plans")
            .join(format!("{:016x}.json", hasher.finish())),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        OverlayBlockedIssue, OverlayWarning, RefGraphRepository, build_overlay_proof,
        classify_warning_issue, normalize_archive_key, write_overlay_plan_audit,
    };
    use std::fs;

    #[test]
    fn normalizes_archive_key() {
        assert_eq!(
            normalize_archive_key("textures.png.mipped"),
            "texturespngmipped"
        );
        assert_eq!(normalize_archive_key("vfx2"), "vfx2");
    }

    #[test]
    fn fallback_warning_is_heuristic_gap() {
        let issue = classify_warning_issue(
            1,
            &OverlayWarning {
                kind: "risk".to_string(),
                message:
                    "refs/loc.json has no entry for loc_1; falling back to donor binary dependency scan where supported."
                        .to_string(),
                ref_kind: Some("loc".to_string()),
                archive: None,
                id: None,
            },
        )
        .expect("proof issue");
        assert_eq!(issue.kind, "heuristic");
        assert_eq!(issue.location, "warning[1]");
    }

    #[test]
    fn heuristic_gap_blocks_when_not_allowed() {
        let proof = build_overlay_proof(
            &[OverlayWarning {
                kind: "risk".to_string(),
                message: "refs/loc.json missing under semantic root; run cache:semantic:sync-947."
                    .to_string(),
                ref_kind: Some("loc".to_string()),
                archive: None,
                id: None,
            }],
            &[],
            super::ProofState {
                script_checked: 0,
                script_blocked: 0,
                script_valid: 0,
                component_checked: 0,
                component_blocked: 0,
                component_valid: 0,
                blockers: Vec::new(),
            },
            false,
            910,
            947,
        );
        assert_eq!(proof.status, "blocked");
        assert_eq!(proof.heuristic_site_count, 1);
    }

    #[test]
    fn heuristic_gap_is_allowed_when_enabled() {
        let proof = build_overlay_proof(
            &[OverlayWarning {
                kind: "risk".to_string(),
                message: "refs/loc.json missing under semantic root; run cache:semantic:sync-947."
                    .to_string(),
                ref_kind: Some("loc".to_string()),
                archive: None,
                id: None,
            }],
            &[],
            super::ProofState {
                script_checked: 0,
                script_blocked: 0,
                script_valid: 0,
                component_checked: 0,
                component_blocked: 0,
                component_valid: 0,
                blockers: Vec::new(),
            },
            true,
            910,
            947,
        );
        assert_eq!(proof.status, "ok");
        assert_eq!(proof.heuristic_site_count, 1);
        assert!(!proof.strict);
    }

    #[test]
    fn audit_writes_expected_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let proof = build_overlay_proof(
            &[],
            &[OverlayBlockedIssue {
                kind: "conflict".to_string(),
                archive: None,
                archive_id: None,
                group_id: None,
                file_id: None,
                id: Some(7),
                ref_kind: Some("varbit".to_string()),
                message: "varbit_7 differs".to_string(),
            }],
            super::ProofState {
                script_checked: 1,
                script_blocked: 1,
                script_valid: 0,
                component_checked: 0,
                component_blocked: 0,
                component_valid: 0,
                blockers: vec![super::OverlayProofIssue {
                    kind: "unsupported",
                    location: "script_42".to_string(),
                    ref_kind: Some("script".to_string()),
                    message: "script_42 target validation failed".to_string(),
                }],
            },
            false,
            910,
            947,
        );

        let audit = write_overlay_plan_audit(temp.path(), &proof, &[]).expect("write audit");
        assert!(audit.relative_paths.contains(&"summary.json".to_string()));
        assert!(temp.path().join("summary.json").is_file());
        assert!(temp.path().join("unsupported_sites.jsonl").is_file());
        assert!(temp.path().join("scripts_failed.jsonl").is_file());
    }

    #[test]
    fn manifest_deserializes_script_and_interface_imports() {
        let manifest: super::CacheOverlayManifest = serde_json::from_value(serde_json::json!({
            "roots": {
                "baseRawRoot": "/tmp/base-raw",
                "donorRawRoot": "/tmp/donor-raw",
                "baseSemanticRoot": "/tmp/base-semantic",
                "donorSemanticRoot": "/tmp/donor-semantic",
                "basePackRoot": "/tmp/base-pack",
                "outputPackRoot": "/tmp/output-pack",
                "clientOutputPackRoot": "/tmp/client-pack"
            },
            "imports": {
                "interfaces": [1213, 1218],
                "scripts": [548, 5690]
            }
        }))
        .expect("manifest");

        assert_eq!(manifest.imports.interfaces, vec![1213, 1218]);
        assert_eq!(manifest.imports.scripts, vec![548, 5690]);
    }

    #[test]
    fn ref_repository_keeps_empty_entries_and_loads_varp_kind() {
        let temp = tempfile::tempdir().expect("tempdir");
        let refs = temp.path().join("refs");
        fs::create_dir_all(&refs).expect("refs dir");
        fs::write(refs.join("bas.json"), "{\n  \"1159\": {}\n}\n").expect("bas refs");
        fs::write(
            refs.join("varp.json"),
            "{\n  \"player\": {\n    \"42\": {\"varbit\": [7]}\n  }\n}\n",
        )
        .expect("varp refs");

        let repo = RefGraphRepository::new(temp.path()).expect("repo");
        assert!(repo.has_kind("bas"));
        assert!(repo.get_refs("bas", 1159).is_some());
        assert!(repo.has_kind("varp"));
        assert_eq!(
            repo.get_refs("varp", 42)
                .and_then(|refs| refs.get(&super::SemanticRefKey::VarBit))
                .expect("varp refs"),
            &vec![7]
        );
    }
}
