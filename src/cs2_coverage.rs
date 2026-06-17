//! `cs2-coverage` — prove every CS2 opcode used by the runtime pack's
//! clientscripts maps to a registry command with a real dispatch handler.
//!
//! The command reads the runtime JS5 pack's single-file clientscript container
//! (`client.scripts.js5`), decodes every group with the 910 opcode book, and
//! gates that every used opcode resolves to a registry command whose
//! `dispatch.kind == "call"`. Decode failures, unassigned opcodes, and
//! unknown opcodes are reported as error-severity findings.
//!
//! The command is read-only over the pack and registry; it writes exactly one
//! file (the coverage report JSON) and never touches the pack, client, or
//! server.
//!
//! Example invocation:
//!
//! ```bash
//! cd tools/rs3-cache-rs
//! cargo run --release -- --data-dir data cs2-coverage
//! ```

use crate::cache_bail;
use crate::error::{Context, Result};
use crate::js5::ArchiveIndex;
use crate::js5pack::PackArchive;
use crate::script::{OpcodeBook, decode_script};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Resolved options for the `cs2-coverage` subcommand.
#[derive(Debug)]
pub struct Cs2CoverageOpts<'a> {
    /// Runtime pack root (holds `client.scripts.js5`).
    pub pack_root: &'a Path,
    /// Optional override for the clientscript pack file.
    pub pack_file: Option<&'a Path>,
    /// Optional override for the registry JSON path.
    pub registry: Option<&'a Path>,
    /// Optional override for the report output path.
    pub out_file: Option<&'a Path>,
    /// Global data directory holding `opcodes-910.txt` and `cs2/registry-910.json`.
    pub data_dir: &'a Path,
}

const REPORT_SCHEMA: &str = "cs2-coverage/v1";
const BASE_BUILD: u32 = 910;
const BASE_SUBBUILD: u32 = 0;

// ---------------------------------------------------------------------------
// Registry (consumer side)
// ---------------------------------------------------------------------------

/// One registry command, deserialized to only the fields coverage needs.
#[derive(Debug, Clone, serde::Deserialize)]
struct RegistryCommand {
    name: String,
    id_910: u32,
    dispatch: RegistryDispatch,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RegistryDispatch {
    kind: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RegistryFile {
    commands: Vec<RegistryCommand>,
}

/// The registry indexed for opcode lookups.
#[derive(Debug)]
struct Registry {
    /// Command name by opcode id.
    name_by_id: BTreeMap<u32, String>,
    /// Opcode ids whose dispatch handler exists (`dispatch.kind == "call"`).
    callable: BTreeSet<u32>,
    /// Opcode ids whose dispatch is `unassigned` (the throwing ids).
    unassigned: BTreeSet<u32>,
}

impl Registry {
    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed reading registry {}", path.display()))?;
        let file: RegistryFile = serde_json::from_str(&text)
            .with_context(|| format!("failed parsing registry json {}", path.display()))?;
        let mut name_by_id = BTreeMap::new();
        let mut callable = BTreeSet::new();
        let mut unassigned = BTreeSet::new();
        for command in file.commands {
            name_by_id.insert(command.id_910, command.name);
            match command.dispatch.kind.as_str() {
                "call" => {
                    callable.insert(command.id_910);
                }
                "unassigned" => {
                    unassigned.insert(command.id_910);
                }
                other => {
                    cache_bail!(
                        "unexpected dispatch kind '{other}' for opcode {} in {}",
                        command.id_910,
                        path.display()
                    );
                }
            }
        }
        if name_by_id.is_empty() {
            cache_bail!("registry {} declares no commands", path.display());
        }
        Ok(Self {
            name_by_id,
            callable,
            unassigned,
        })
    }

    fn name(&self, id: u32) -> Option<&str> {
        self.name_by_id.get(&id).map(String::as_str)
    }
}

// The single-file `.js5` pack reader lives in [`crate::js5pack::PackArchive`]
// (extracted in Stage 5 Part A); the scan below consumes it directly.

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// A single error-severity finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Finding kind: `decode_failure`, `unassigned_opcode`, or `unknown_opcode`.
    pub kind: String,
    /// The group id the finding pertains to.
    pub group: u32,
    /// The decoded script name, when available (may be empty); `None` for decode failures.
    pub script_name: Option<String>,
    /// The offending opcode id; `None` for decode failures.
    pub opcode: Option<u16>,
    /// The registry command name for the opcode; `None` when unknown or a decode failure.
    pub name: Option<String>,
    /// Zero-based instruction index for opcode findings; `None` for decode failures.
    pub instruction_index: Option<u32>,
    /// The error message for decode failures; `None` for opcode findings.
    pub message: Option<String>,
}

impl Finding {
    fn sort_key(&self) -> (&str, u32, u16) {
        (self.kind.as_str(), self.group, self.opcode.unwrap_or(0))
    }
}

/// Per-group classification outcome.
#[derive(Debug)]
struct GroupOutcome {
    /// Whether at least one script in the group decoded cleanly.
    decoded: bool,
    /// Findings raised for this group.
    findings: Vec<Finding>,
    /// Distinct opcode ids used in this group.
    opcodes_used: BTreeSet<u16>,
}

/// Classify one present group's container bytes against the registry.
fn classify_group(
    group: u32,
    container: &[u8],
    opcode_book: &OpcodeBook,
    registry: &Registry,
    index: &ArchiveIndex,
) -> GroupOutcome {
    let mut findings = Vec::new();
    let mut opcodes_used = BTreeSet::new();
    let mut decoded = false;

    // Mirror the `cs2` dump path: unpack the group container into its file(s),
    // then decode each file's script bytes with the 910 opcode book.
    let files = match crate::js5::unpack_group(index, group, container) {
        Ok(files) => files,
        Err(e) => {
            findings.push(decode_failure(group, &format!("unpack failed: {e}")));
            return GroupOutcome {
                decoded,
                findings,
                opcodes_used,
            };
        }
    };

    for (_file, bytes) in files {
        let script = match decode_script(&bytes, opcode_book, BASE_BUILD) {
            Ok(script) => script,
            Err(e) => {
                findings.push(decode_failure(group, &e.to_string()));
                continue;
            }
        };
        decoded = true;
        let script_name = script.name.clone().unwrap_or_default();
        for (idx, instruction) in script.code.iter().enumerate() {
            let opcode = instruction.opcode;
            opcodes_used.insert(opcode);
            let id = u32::from(opcode);
            let instruction_index = u32::try_from(idx).ok();
            if registry.callable.contains(&id) {
                continue;
            }
            if registry.unassigned.contains(&id) {
                findings.push(Finding {
                    kind: "unassigned_opcode".to_owned(),
                    group,
                    script_name: Some(script_name.clone()),
                    opcode: Some(opcode),
                    name: registry.name(id).map(str::to_owned),
                    instruction_index,
                    message: None,
                });
            } else {
                findings.push(Finding {
                    kind: "unknown_opcode".to_owned(),
                    group,
                    script_name: Some(script_name.clone()),
                    opcode: Some(opcode),
                    name: None,
                    instruction_index,
                    message: None,
                });
            }
        }
    }

    GroupOutcome {
        decoded,
        findings,
        opcodes_used,
    }
}

fn decode_failure(group: u32, message: &str) -> Finding {
    Finding {
        kind: "decode_failure".to_owned(),
        group,
        script_name: None,
        opcode: None,
        name: None,
        instruction_index: None,
        message: Some(message.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// The coverage report summary block.
#[derive(Debug, Serialize)]
pub struct Summary {
    /// Number of groups in the archive index.
    pub groups_indexed: usize,
    /// Groups with a non-zero trailer length (present in the pack).
    pub groups_present: usize,
    /// Groups where at least one script decoded cleanly.
    pub groups_decoded: usize,
    /// Number of `decode_failure` findings.
    pub decode_failures: usize,
    /// Number of `unassigned_opcode` findings.
    pub unassigned_opcode_findings: usize,
    /// Number of `unknown_opcode` findings.
    pub unknown_opcode_findings: usize,
    /// Distinct opcode ids used across the whole pack.
    pub distinct_opcodes_used: usize,
    /// Count of callable registry commands never used by any script.
    pub implemented_unused: usize,
}

/// Per-opcode usage aggregate.
#[derive(Debug, Serialize)]
pub struct OpcodeUsage {
    /// Opcode id.
    pub id: u16,
    /// Registry command name for this opcode, when known.
    pub name: Option<String>,
    /// Number of script groups that used this opcode.
    pub scripts: usize,
}

/// The full coverage report.
#[derive(Debug, Serialize)]
pub struct Report {
    /// Report schema identifier (`cs2-coverage/v1`).
    pub schema: &'static str,
    /// Resolved pack file path.
    pub pack_file: String,
    /// Resolved registry path.
    pub registry: String,
    /// Summary counters.
    pub summary: Summary,
    /// Error-severity findings, sorted by (kind, group, opcode).
    pub findings: Vec<Finding>,
    /// Per-opcode usage, sorted by opcode id.
    pub opcode_usage: Vec<OpcodeUsage>,
    /// Callable registry command names never used, sorted.
    pub implemented_unused: Vec<String>,
}

impl Report {
    /// Total error-severity finding count.
    #[must_use]
    pub fn error_findings(&self) -> usize {
        self.summary.decode_failures
            + self.summary.unassigned_opcode_findings
            + self.summary.unknown_opcode_findings
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the `cs2-coverage` scan. Returns `Ok(true)` when error-severity findings
/// exist (the caller maps that to exit code 4), `Ok(false)` for a clean scan.
pub fn run(opts: &Cs2CoverageOpts<'_>) -> Result<bool> {
    let out_file: PathBuf = opts
        .out_file
        .map(Path::to_path_buf)
        .unwrap_or_else(|| opts.data_dir.join("cs2").join("coverage-910.report.json"));

    let report = scan(opts)?;

    write_report(&out_file, &report)?;
    print_human_summary(&report, &out_file);

    Ok(report.error_findings() > 0)
}

/// Run the coverage scan and return the in-memory report without writing it.
/// Exposed for integration tests.
pub fn scan(opts: &Cs2CoverageOpts<'_>) -> Result<Report> {
    let pack_file: PathBuf = opts
        .pack_file
        .map(Path::to_path_buf)
        .unwrap_or_else(|| opts.pack_root.join("client.scripts.js5"));
    let registry_path: PathBuf = opts
        .registry
        .map(Path::to_path_buf)
        .unwrap_or_else(|| opts.data_dir.join("cs2").join("registry-910.json"));

    let registry = Registry::load(&registry_path)?;
    let opcode_book = OpcodeBook::load(opts.data_dir, BASE_BUILD, BASE_SUBBUILD)?;

    let pack = PackArchive::open(&pack_file)?;

    // Collect present (group, container) pairs in index order; this preserves
    // the exact group-iteration order of the original reader so the merged
    // report is byte-identical.
    let present: Vec<(u32, &[u8])> = pack
        .group_ids()
        .filter_map(|group| pack.group_container(group).map(|bytes| (group, bytes)))
        .collect();

    let groups_indexed = pack.group_ids().count();
    let groups_present = present.len();

    let index = pack.index();
    let outcomes: Vec<GroupOutcome> = present
        .par_iter()
        .map(|&(group, bytes)| classify_group(group, bytes, &opcode_book, &registry, index))
        .collect();

    // Merge deterministically.
    let mut findings: Vec<Finding> = Vec::new();
    let mut groups_decoded = 0_usize;
    // opcode id -> number of scripts (groups, here one group == one script
    // family) that used it. We count distinct groups using each opcode.
    let mut opcode_group_counts: BTreeMap<u16, usize> = BTreeMap::new();
    for outcome in &outcomes {
        if outcome.decoded {
            groups_decoded += 1;
        }
        for opcode in &outcome.opcodes_used {
            *opcode_group_counts.entry(*opcode).or_insert(0) += 1;
        }
    }
    for outcome in outcomes {
        findings.extend(outcome.findings);
    }
    findings.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    let decode_failures = findings
        .iter()
        .filter(|f| f.kind == "decode_failure")
        .count();
    let unassigned_opcode_findings = findings
        .iter()
        .filter(|f| f.kind == "unassigned_opcode")
        .count();
    let unknown_opcode_findings = findings
        .iter()
        .filter(|f| f.kind == "unknown_opcode")
        .count();

    let opcode_usage: Vec<OpcodeUsage> = opcode_group_counts
        .iter()
        .map(|(&id, &scripts)| OpcodeUsage {
            id,
            name: registry.name(u32::from(id)).map(str::to_owned),
            scripts,
        })
        .collect();

    // implemented_unused: callable registry commands whose id never appears.
    let used_ids: BTreeSet<u32> = opcode_group_counts
        .keys()
        .map(|&id| u32::from(id))
        .collect();
    let mut implemented_unused: Vec<String> = registry
        .callable
        .iter()
        .filter(|id| !used_ids.contains(id))
        .filter_map(|id| registry.name(*id).map(str::to_owned))
        .collect();
    implemented_unused.sort();

    let report = Report {
        schema: REPORT_SCHEMA,
        pack_file: pack_file.display().to_string(),
        registry: registry_path.display().to_string(),
        summary: Summary {
            groups_indexed,
            groups_present,
            groups_decoded,
            decode_failures,
            unassigned_opcode_findings,
            unknown_opcode_findings,
            distinct_opcodes_used: opcode_group_counts.len(),
            implemented_unused: implemented_unused.len(),
        },
        findings,
        opcode_usage,
        implemented_unused,
    };

    Ok(report)
}

fn write_report(path: &Path, report: &Report) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(report).context("failed to encode coverage report")?;
    fs::write(path, json).with_context(|| format!("failed writing {}", path.display()))
}

fn print_human_summary(report: &Report, out_file: &Path) {
    let s = &report.summary;
    println!("cs2-coverage report: {}", out_file.display());
    println!("groups_indexed: {}", s.groups_indexed);
    println!("groups_present: {}", s.groups_present);
    println!("groups_decoded: {}", s.groups_decoded);
    println!("decode_failures: {}", s.decode_failures);
    println!(
        "unassigned_opcode_findings: {}",
        s.unassigned_opcode_findings
    );
    println!("unknown_opcode_findings: {}", s.unknown_opcode_findings);
    println!("distinct_opcodes_used: {}", s.distinct_opcodes_used);
    println!("implemented_unused: {}", s.implemented_unused);

    let shown = report.findings.iter().take(20);
    for finding in shown {
        match finding.kind.as_str() {
            "decode_failure" => println!(
                "  [{}] group {}: {}",
                finding.kind,
                finding.group,
                finding.message.as_deref().unwrap_or("")
            ),
            _ => println!(
                "  [{}] group {} opcode {} ({}) instr {}",
                finding.kind,
                finding.group,
                finding.opcode.unwrap_or(0),
                finding.name.as_deref().unwrap_or("?"),
                finding
                    .instruction_index
                    .map_or_else(|| "?".to_owned(), |i| i.to_string())
            ),
        }
    }
    if report.findings.len() > 20 {
        println!("  … and {} more", report.findings.len() - 20);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The single-file `.js5` pack reader and its sanity-identity tests now live
    // in `crate::js5pack` (Stage 5 Part A); the classification-level tests below
    // stay here.

    fn test_registry() -> Registry {
        let mut name_by_id = BTreeMap::new();
        let mut callable = BTreeSet::new();
        let mut unassigned = BTreeSet::new();
        // 0 = callable, 79 = unassigned, 5000 unknown (no entry).
        name_by_id.insert(0, "good_op".to_owned());
        name_by_id.insert(79, "thrower".to_owned());
        callable.insert(0);
        unassigned.insert(79);
        Registry {
            name_by_id,
            callable,
            unassigned,
        }
    }

    fn instr(opcode: u16) -> crate::script::Instruction {
        crate::script::Instruction {
            opcode,
            command: format!("op_{opcode}"),
            operand: crate::script::Operand::Int(0),
        }
    }

    fn make_script(name: Option<&str>, opcodes: &[u16]) -> crate::script::CompiledScript {
        crate::script::CompiledScript {
            name: name.map(str::to_owned),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: opcodes.iter().map(|&o| instr(o)).collect(),
        }
    }

    /// Classify a pre-decoded script against the registry (mirrors the inner
    /// loop of `classify_group`, exercising classification without the JS5
    /// decode layer).
    fn classify_decoded(
        group: u32,
        script: &crate::script::CompiledScript,
        registry: &Registry,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();
        let script_name = script.name.clone().unwrap_or_default();
        for (idx, instruction) in script.code.iter().enumerate() {
            let id = u32::from(instruction.opcode);
            let instruction_index = u32::try_from(idx).ok();
            if registry.callable.contains(&id) {
                continue;
            }
            if registry.unassigned.contains(&id) {
                findings.push(Finding {
                    kind: "unassigned_opcode".to_owned(),
                    group,
                    script_name: Some(script_name.clone()),
                    opcode: Some(instruction.opcode),
                    name: registry.name(id).map(str::to_owned),
                    instruction_index,
                    message: None,
                });
            } else {
                findings.push(Finding {
                    kind: "unknown_opcode".to_owned(),
                    group,
                    script_name: Some(script_name.clone()),
                    opcode: Some(instruction.opcode),
                    name: None,
                    instruction_index,
                    message: None,
                });
            }
        }
        findings
    }

    #[test]
    fn classification_covers_each_kind() {
        let registry = test_registry();
        // ok path: only callable opcode 0.
        let ok = make_script(Some("ok"), &[0, 0]);
        assert!(classify_decoded(7, &ok, &registry).is_empty());

        // unassigned opcode 79.
        let unassigned = make_script(Some("u"), &[79]);
        let f = classify_decoded(8, &unassigned, &registry);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "unassigned_opcode");
        assert_eq!(f[0].opcode, Some(79));
        assert_eq!(f[0].name.as_deref(), Some("thrower"));
        assert_eq!(f[0].instruction_index, Some(0));

        // unknown opcode 5000 (no registry entry).
        let unknown = make_script(None, &[5000]);
        let f = classify_decoded(9, &unknown, &registry);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "unknown_opcode");
        assert_eq!(f[0].opcode, Some(5000));
        assert_eq!(f[0].name, None);
        assert_eq!(f[0].script_name.as_deref(), Some(""));
    }

    #[test]
    fn implemented_unused_math() {
        let registry = test_registry();
        // Only opcode 0 used.
        let mut used_ids: BTreeSet<u32> = BTreeSet::new();
        used_ids.insert(0);
        let unused_count = registry
            .callable
            .iter()
            .filter(|id| !used_ids.contains(id))
            .filter_map(|id| registry.name(*id).map(str::to_owned))
            .count();
        // opcode 0 is the only callable command and it is used.
        assert_eq!(unused_count, 0);

        // Now no opcodes used: the one callable command (0) is unused.
        let used_ids: BTreeSet<u32> = BTreeSet::new();
        let mut unused: Vec<String> = registry
            .callable
            .iter()
            .filter(|id| !used_ids.contains(id))
            .filter_map(|id| registry.name(*id).map(str::to_owned))
            .collect();
        unused.sort();
        assert_eq!(unused, vec!["good_op".to_owned()]);
    }

    #[test]
    fn report_serialization_is_deterministic() {
        let report = Report {
            schema: REPORT_SCHEMA,
            pack_file: "/tmp/pack".to_owned(),
            registry: "/tmp/reg".to_owned(),
            summary: Summary {
                groups_indexed: 3,
                groups_present: 2,
                groups_decoded: 2,
                decode_failures: 1,
                unassigned_opcode_findings: 0,
                unknown_opcode_findings: 0,
                distinct_opcodes_used: 4,
                implemented_unused: 1,
            },
            findings: vec![decode_failure(14490, "boom")],
            opcode_usage: vec![OpcodeUsage {
                id: 0,
                name: Some("good_op".to_owned()),
                scripts: 5,
            }],
            implemented_unused: vec!["a".to_owned()],
        };
        let a = serde_json::to_vec_pretty(&report).expect("encode a");
        let b = serde_json::to_vec_pretty(&report).expect("encode b");
        assert_eq!(a, b);
    }
}
