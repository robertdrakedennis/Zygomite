//! `discover` (gap #1): find a feature in the cache by text, and flag what is NEW
//! between two builds — discovery as a command instead of an ad-hoc `grep` + `comm`.
//!
//! Finding "the new store 947/948 shipped" took two manual steps: string-searching the
//! interface dumps for a distinctive word ("Marketplace"), and an id-set `comm` of the
//! 910 vs 948 dumps to confirm the hits were new. This subcommand does both natively
//! over the runtime flat caches the tool already reads:
//!
//! * `--query <substr>` — case-insensitively search **interface** component text /
//!   names / op labels and (by default) **CS2** script string constants for the term.
//! * `--against-cache <dir>` — a baseline build's flat cache; every hit is tagged
//!   `NEW` when its interface / script id is absent from that baseline. With no
//!   `--query`, the command instead lists every interface / script id new vs baseline
//!   (the pure `comm` diff).
//!
//! So `discover --query marketplace --against-cache <910>` finds interface 1494/1498,
//! marks them NEW, and shows the matching "The Marketplace is currently initialising"
//! text — the whole manual find, in one command.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::cache::FlatCache;
use crate::constants::{ARCHIVE_CLIENTSCRIPTS, ARCHIVE_INTERFACES};
use crate::error::{Context, Result};
use crate::interface::component::explain_interface_group;
use crate::script::{OpcodeBook, Operand, decode_script};

/// Cap on matching snippets shown per hit (the full set is in the cache; this keeps
/// the report readable).
const MAX_SNIPPETS: usize = 6;

/// Options for [`run`].
pub struct DiscoverOptions<'a> {
    /// Case-insensitive search term. `None` runs the pure new-id diff (requires
    /// `against_cache`).
    pub query: Option<String>,
    /// Build the search cache is decoded at (interfaces are build-keyed; scripts need
    /// the build's opcode book).
    pub build: u32,
    /// Sub-build for the opcode book.
    pub subbuild: u32,
    /// `data/` dir for the opcode book.
    pub data_dir: &'a Path,
    /// Flat cache dir to search (the "to"/donor build, e.g. 948).
    pub cache_dir: &'a Path,
    /// Baseline flat cache (the "from" build, e.g. 910); when set, hits are tagged
    /// `new` by id-absence and the no-query mode lists new ids.
    pub against_cache: Option<&'a Path>,
    /// Also scan CS2 script string constants (in addition to interface text).
    pub search_scripts: bool,
    /// Emit JSON instead of the human report.
    pub json: bool,
}

/// An interface whose text matched (or, in no-query mode, that is new vs baseline).
#[derive(Debug, Clone, Serialize)]
pub struct InterfaceHit {
    pub id: u32,
    pub new: bool,
    /// `comN: <text>` snippets that matched (empty in no-query mode).
    pub matches: Vec<String>,
}

/// A script whose string constants matched (or that is new vs baseline).
#[derive(Debug, Clone, Serialize)]
pub struct ScriptHit {
    pub id: u32,
    pub new: bool,
    /// Matching string constants (empty in no-query mode).
    pub strings: Vec<String>,
}

/// The discovery result.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoverResult {
    pub query: Option<String>,
    pub interfaces: Vec<InterfaceHit>,
    pub scripts: Vec<ScriptHit>,
}

/// The set of group ids present in an archive of a flat cache (for the baseline
/// presence check / new-id diff).
fn archive_group_ids(cache_dir: &Path, archive: u32) -> Result<BTreeSet<u32>> {
    let cache = FlatCache::open(cache_dir)
        .with_context(|| format!("open cache {} for discovery", cache_dir.display()))?;
    let index = cache.archive_index(archive)?;
    Ok(index.group_id.iter().copied().collect())
}

/// Search a build's interfaces: match `query` against component text / name / op
/// labels (case-insensitive), or — when `query` is `None` — list ids new vs baseline.
fn search_interfaces(
    cache_dir: &Path,
    build: u32,
    query: Option<&str>,
    baseline: Option<&BTreeSet<u32>>,
) -> Result<Vec<InterfaceHit>> {
    let cache = FlatCache::open(cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let mut hits = Vec::new();
    for &group in &index.group_id {
        let is_new = baseline.is_some_and(|b| !b.contains(&group));
        let Some(query) = query else {
            if is_new {
                hits.push(InterfaceHit {
                    id: group,
                    new: true,
                    matches: Vec::new(),
                });
            }
            continue;
        };
        let files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, group)?;
        // A group that fails to decode is skipped (best-effort search).
        let Ok(explained) = explain_interface_group(group, &files, build) else {
            continue;
        };
        let mut matches = Vec::new();
        for component in &explained.components {
            let strings = component
                .text
                .iter()
                .chain(component.name.iter())
                .chain(component.ops.iter());
            for value in strings {
                if value.to_lowercase().contains(query) {
                    matches.push(format!("com{}: {value}", component.index));
                }
            }
        }
        if !matches.is_empty() {
            matches.truncate(MAX_SNIPPETS);
            hits.push(InterfaceHit {
                id: group,
                new: is_new,
                matches,
            });
        }
    }
    Ok(hits)
}

/// Search a build's CS2 scripts: match `query` against string constants
/// (`push_constant_string`), or — when `query` is `None` — list ids new vs baseline.
fn search_scripts(
    cache_dir: &Path,
    data_dir: &Path,
    build: u32,
    subbuild: u32,
    query: Option<&str>,
    baseline: Option<&BTreeSet<u32>>,
) -> Result<Vec<ScriptHit>> {
    let cache = FlatCache::open(cache_dir)?;
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let book = (query.is_some())
        .then(|| OpcodeBook::load(data_dir, build, subbuild))
        .transpose()?;
    let mut hits = Vec::new();
    for &group in &index.group_id {
        let is_new = baseline.is_some_and(|b| !b.contains(&group));
        let (Some(query), Some(book)) = (query, book.as_ref()) else {
            if is_new {
                hits.push(ScriptHit {
                    id: group,
                    new: true,
                    strings: Vec::new(),
                });
            }
            continue;
        };
        let files = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, group)?;
        let Some((_, data)) = files.into_iter().min_by_key(|(file, _)| *file) else {
            continue;
        };
        let Ok(script) = decode_script(&data, book, build) else {
            continue;
        };
        let mut strings = Vec::new();
        for ins in &script.code {
            if let Operand::Str(value) = &ins.operand
                && value.to_lowercase().contains(query)
            {
                strings.push(value.clone());
            }
        }
        if !strings.is_empty() {
            strings.truncate(MAX_SNIPPETS);
            hits.push(ScriptHit {
                id: group,
                new: is_new,
                strings,
            });
        }
    }
    Ok(hits)
}

/// Compute the discovery result for `opts`.
pub fn discover(opts: &DiscoverOptions<'_>) -> Result<DiscoverResult> {
    let query = opts.query.as_deref().map(str::to_lowercase);
    let query = query.as_deref();

    let if_baseline = opts
        .against_cache
        .map(|dir| archive_group_ids(dir, ARCHIVE_INTERFACES))
        .transpose()?;
    let interfaces = search_interfaces(opts.cache_dir, opts.build, query, if_baseline.as_ref())?;

    let scripts = if opts.search_scripts || query.is_none() {
        let script_baseline = opts
            .against_cache
            .map(|dir| archive_group_ids(dir, ARCHIVE_CLIENTSCRIPTS))
            .transpose()?;
        search_scripts(
            opts.cache_dir,
            opts.data_dir,
            opts.build,
            opts.subbuild,
            query,
            script_baseline.as_ref(),
        )?
    } else {
        Vec::new()
    };

    Ok(DiscoverResult {
        query: opts.query.clone(),
        interfaces,
        scripts,
    })
}

/// Run the command: discover, then print JSON or the human report.
pub fn run(opts: &DiscoverOptions<'_>) -> Result<()> {
    let result = discover(opts)?;
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).context("encode discover JSON")?
        );
    } else {
        print!("{}", render_human(&result));
    }
    Ok(())
}

/// Render the human report.
#[must_use]
pub fn render_human(result: &DiscoverResult) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let new_ifaces = result.interfaces.iter().filter(|h| h.new).count();
    let new_scripts = result.scripts.iter().filter(|h| h.new).count();

    match &result.query {
        Some(q) => {
            let _ = writeln!(
                out,
                "discover \"{q}\" — {} interface(s) ({new_ifaces} new), {} script(s) ({new_scripts} new)",
                result.interfaces.len(),
                result.scripts.len(),
            );
        }
        None => {
            let _ = writeln!(
                out,
                "discover (new vs baseline) — {new_ifaces} new interface(s), {new_scripts} new script(s)",
            );
        }
    }

    if !result.interfaces.is_empty() {
        let _ = writeln!(out, "interfaces:");
        for hit in &result.interfaces {
            let tag = if hit.new { " NEW" } else { "" };
            let _ = writeln!(out, "  interface {}{tag}", hit.id);
            for snippet in &hit.matches {
                let _ = writeln!(out, "      {snippet}");
            }
        }
    }
    if !result.scripts.is_empty() {
        let _ = writeln!(out, "scripts:");
        for hit in &result.scripts {
            let tag = if hit.new { " NEW" } else { "" };
            if hit.strings.is_empty() {
                let _ = writeln!(out, "  script {}{tag}", hit.id);
            } else {
                let joined = hit
                    .strings
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "  script {}{tag}: {joined}", hit.id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_tags_new_hits_and_indents_snippets() {
        let result = DiscoverResult {
            query: Some("marketplace".to_string()),
            interfaces: vec![
                InterfaceHit {
                    id: 274,
                    new: false,
                    matches: vec!["com166: Marketplace".to_string()],
                },
                InterfaceHit {
                    id: 1494,
                    new: true,
                    matches: vec!["com26: The Marketplace is currently initialising.".to_string()],
                },
            ],
            scripts: vec![ScriptHit {
                id: 20072,
                new: true,
                strings: vec!["Marketplace floater setup".to_string()],
            }],
        };
        let out = render_human(&result);
        assert!(
            out.contains("2 interface(s) (1 new), 1 script(s) (1 new)"),
            "header counts new hits: {out}"
        );
        assert!(out.contains("interface 1494 NEW"), "new interface tagged");
        assert!(
            out.contains("  interface 274\n"),
            "existing interface untagged"
        );
        assert!(
            out.contains("      com26: The Marketplace"),
            "snippet indented under its interface"
        );
        assert!(
            out.contains("script 20072 NEW: \"Marketplace floater setup\""),
            "new script tagged with its matching string"
        );
    }

    #[test]
    fn render_no_query_lists_new_ids_only() {
        let result = DiscoverResult {
            query: None,
            interfaces: vec![InterfaceHit {
                id: 1494,
                new: true,
                matches: Vec::new(),
            }],
            scripts: Vec::new(),
        };
        let out = render_human(&result);
        assert!(out.contains("new vs baseline) — 1 new interface(s), 0 new script(s)"));
        assert!(out.contains("interface 1494 NEW"));
    }
}
