//! `migrate-check` / `migrate-script` — migration impact analysis of an
//! interface group or a single CS2 script from a source build onto the target.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cache::FlatCache;
use crate::cli::context::CommandContext;
use crate::cli::shared::write_jsonl_file;
use crate::dep_tree::ResolverContext;

/// The source build/cache the migration analysis pulls entities from.
#[derive(Clone, Debug)]
pub struct MigrateSource {
    pub cache_tar: Option<PathBuf>,
    pub build: u32,
    pub subbuild: u32,
}

/// ID-remap planning options.
#[derive(Clone, Copy, Debug)]
pub struct RemapOpts {
    pub enabled: bool,
    pub buffer: u32,
}

/// Options for `migrate-check`.
#[derive(Clone, Debug)]
pub struct MigrateCheckOpts {
    pub interface_group: u32,
    pub out_file: PathBuf,
    pub audit_dir: Option<PathBuf>,
    pub source: MigrateSource,
    pub remap: RemapOpts,
    pub validate_target: bool,
    pub allow_heuristic_sites: bool,
}

/// Options for `migrate-script`.
#[derive(Clone, Debug)]
pub struct MigrateScriptOpts {
    pub script_id: u32,
    pub out_file: PathBuf,
    pub audit_dir: Option<PathBuf>,
    pub source: MigrateSource,
    pub remap: RemapOpts,
    pub validate_target: bool,
    pub allow_heuristic_sites: bool,
}

fn load_source_resolver_context(
    target_cache: &FlatCache,
    target_tar_path: &Path,
    data_dir: &Path,
    source_cache_tar: Option<&Path>,
    source_build: u32,
    source_subbuild: u32,
) -> Result<(ResolverContext, Option<tempfile::TempDir>)> {
    let Some(source_tar) = source_cache_tar else {
        return Ok((
            ResolverContext::load_lazy(
                target_cache,
                target_tar_path,
                data_dir,
                source_build,
                source_subbuild,
            )?,
            None,
        ));
    };

    let temp_dir = tempfile::Builder::new()
        .prefix("rs3-cache-rs-source-")
        .tempdir()
        .context("creating isolated source cache")?;
    let source_cache_root = temp_dir.path().join("cache");
    fs::create_dir_all(&source_cache_root)
        .with_context(|| format!("creating {}", source_cache_root.display()))?;
    let source_cache = FlatCache::open(&source_cache_root)?;
    let source_ctx = ResolverContext::load_lazy(
        &source_cache,
        source_tar,
        data_dir,
        source_build,
        source_subbuild,
    )?;
    Ok((source_ctx, Some(temp_dir)))
}

/// `migrate-check` — loads both source and target caches for migration impact
/// analysis of an interface group.
pub fn run_check(ctx: &CommandContext, opts: MigrateCheckOpts) -> Result<()> {
    let MigrateCheckOpts {
        interface_group,
        out_file,
        audit_dir,
        source,
        remap,
        validate_target,
        allow_heuristic_sites,
    } = opts;
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let data_dir = ctx.data_dir();

    let target_ctx =
        ResolverContext::load_lazy(cache, tar_path, data_dir, ctx.build(), ctx.subbuild())?;

    let (source_ctx, _source_cache_temp) = load_source_resolver_context(
        cache,
        tar_path,
        data_dir,
        source.cache_tar.as_deref(),
        source.build,
        source.subbuild,
    )?;

    let analyzer = crate::migrate::MigrationAnalyzer::new(source_ctx, target_ctx);
    let mut report = if remap.enabled {
        analyzer.remap_interface(interface_group, remap.buffer)
    } else {
        analyzer.analyze_interface(interface_group)
    };
    if validate_target {
        report.target_validation = Some(analyzer.validate_interface_target(
            interface_group,
            &report.entities,
            report.remap.as_ref(),
            allow_heuristic_sites,
        ));
    }

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&out_file, &json)?;

    eprintln!(
        "migration report: {} entities ({} safe, {} missing, {} id_conflict, {} changed, {} script_changed) written to {}",
        report.total_entities,
        report.summary.safe,
        report.summary.missing,
        report.summary.id_conflict,
        report.summary.changed,
        report.summary.script_changed,
        out_file.display()
    );
    if let Some(target_validation) = &report.target_validation {
        eprintln!(
            "target validation: {} components ({} blocked), {} scripts checked, {} encoded, {} valid, {} with errors, {} blocked, {} heuristic sites, {} unsupported sites",
            target_validation.summary.components_checked,
            target_validation.summary.components_blocked,
            target_validation.summary.scripts_checked,
            target_validation.summary.scripts_encoded,
            target_validation.summary.scripts_valid,
            target_validation.summary.scripts_with_errors,
            target_validation.summary.scripts_blocked,
            target_validation.summary.heuristic_sites,
            target_validation.summary.unsupported_sites,
        );
    }
    if let Some(audit_dir) = audit_dir.as_deref() {
        write_conflict_audit(&report, audit_dir)?;
    }
    Ok(())
}

/// `migrate-script` — loads both source and target caches for single-script
/// migration analysis.
pub fn run_script(ctx: &CommandContext, opts: MigrateScriptOpts) -> Result<()> {
    let MigrateScriptOpts {
        script_id,
        out_file,
        audit_dir,
        source,
        remap,
        validate_target,
        allow_heuristic_sites,
    } = opts;
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let data_dir = ctx.data_dir();

    let target_ctx =
        ResolverContext::load_lazy(cache, tar_path, data_dir, ctx.build(), ctx.subbuild())?;

    let (source_ctx, _source_cache_temp) = load_source_resolver_context(
        cache,
        tar_path,
        data_dir,
        source.cache_tar.as_deref(),
        source.build,
        source.subbuild,
    )?;

    let analyzer = crate::migrate::MigrationAnalyzer::new(source_ctx, target_ctx);
    let mut report = if remap.enabled {
        analyzer.remap_script(script_id, remap.buffer)
    } else {
        analyzer.analyze_script(script_id)
    };
    if validate_target {
        report.target_validation = Some(analyzer.validate_script_target(
            &report.entities,
            report.remap.as_ref(),
            allow_heuristic_sites,
        ));
    }

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&out_file, &json)?;

    eprintln!(
        "script migration report: {} entities ({} safe, {} missing, {} id_conflict, {} changed, {} script_changed) written to {}",
        report.total_entities,
        report.summary.safe,
        report.summary.missing,
        report.summary.id_conflict,
        report.summary.changed,
        report.summary.script_changed,
        out_file.display()
    );
    if let Some(target_validation) = &report.target_validation {
        eprintln!(
            "target validation: {} scripts checked, {} encoded, {} valid, {} with errors, {} blocked, {} heuristic sites, {} unsupported sites",
            target_validation.summary.scripts_checked,
            target_validation.summary.scripts_encoded,
            target_validation.summary.scripts_valid,
            target_validation.summary.scripts_with_errors,
            target_validation.summary.scripts_blocked,
            target_validation.summary.heuristic_sites,
            target_validation.summary.unsupported_sites,
        );
    }
    if let Some(audit_dir) = audit_dir.as_deref() {
        write_script_audit(&report, audit_dir)?;
    }
    Ok(())
}

fn write_conflict_audit(report: &crate::migrate::ConflictReport, audit_dir: &Path) -> Result<()> {
    let Some(target_validation) = &report.target_validation else {
        return Ok(());
    };
    write_target_validation_audit(
        target_validation,
        report.reference_updates.as_deref().unwrap_or(&[]),
        audit_dir,
        &serde_json::json!({
            "source_build": report.source_build,
            "target_build": report.target_build,
            "interface_group": report.interface_group,
            "total_entities": report.total_entities,
            "summary": report.summary,
        }),
    )
}

fn write_script_audit(report: &crate::migrate::ScriptReport, audit_dir: &Path) -> Result<()> {
    let Some(target_validation) = &report.target_validation else {
        return Ok(());
    };
    write_target_validation_audit(
        target_validation,
        report.reference_updates.as_deref().unwrap_or(&[]),
        audit_dir,
        &serde_json::json!({
            "source_build": report.source_build,
            "target_build": report.target_build,
            "script_id": report.script_id,
            "total_entities": report.total_entities,
            "summary": report.summary,
        }),
    )
}

fn write_target_validation_audit(
    target_validation: &crate::migrate::TargetValidationReport,
    reference_updates: &[crate::migrate::ReferenceUpdate],
    audit_dir: &Path,
    migration_summary: &serde_json::Value,
) -> Result<()> {
    fs::create_dir_all(audit_dir).with_context(|| format!("creating {}", audit_dir.display()))?;

    let summary_path = audit_dir.join("summary.json");
    fs::write(
        &summary_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "migration": migration_summary,
            "target_validation": target_validation.summary,
            "remap_applied": target_validation.remap_applied,
            "target_build": target_validation.target_build,
        }))?,
    )
    .with_context(|| format!("writing {}", summary_path.display()))?;

    let failed_components = target_validation
        .components
        .iter()
        .filter(|component| {
            !component.blocking_issues.is_empty()
                || !component.heuristic_sites.is_empty()
                || !component.unsupported_sites.is_empty()
        })
        .collect::<Vec<_>>();
    write_jsonl_file(
        &audit_dir.join("components_failed.jsonl"),
        &failed_components,
    )?;

    let failed_scripts = target_validation
        .scripts
        .iter()
        .filter(|script| {
            script.failure.is_some()
                || !script.validation_errors.is_empty()
                || !script.blockers.is_empty()
                || !script.unsupported_sites.is_empty()
        })
        .collect::<Vec<_>>();
    write_jsonl_file(&audit_dir.join("scripts_failed.jsonl"), &failed_scripts)?;

    let heuristic_sites = target_validation
        .components
        .iter()
        .flat_map(|component| {
            component.heuristic_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "component",
                    "component_id": component.component_id,
                    "component_name": component.name,
                    "site": site,
                })
            })
        })
        .chain(target_validation.scripts.iter().flat_map(|script| {
            script.heuristic_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "script",
                    "source_script_id": script.source_script_id,
                    "target_script_id": script.target_script_id,
                    "script_name": script.script_name,
                    "site": site,
                })
            })
        }))
        .collect::<Vec<_>>();
    write_jsonl_file(&audit_dir.join("heuristic_sites.jsonl"), &heuristic_sites)?;

    let unsupported_sites = target_validation
        .components
        .iter()
        .flat_map(|component| {
            component.unsupported_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "component",
                    "component_id": component.component_id,
                    "component_name": component.name,
                    "site": site,
                })
            })
        })
        .chain(target_validation.scripts.iter().flat_map(|script| {
            script.unsupported_sites.iter().map(move |site| {
                serde_json::json!({
                    "owner_kind": "script",
                    "source_script_id": script.source_script_id,
                    "target_script_id": script.target_script_id,
                    "script_name": script.script_name,
                    "site": site,
                })
            })
        }))
        .collect::<Vec<_>>();
    write_jsonl_file(
        &audit_dir.join("unsupported_sites.jsonl"),
        &unsupported_sites,
    )?;

    write_jsonl_file(&audit_dir.join("rewrites_applied.jsonl"), reference_updates)?;
    Ok(())
}
