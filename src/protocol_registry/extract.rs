//! `extract-protocol`: parse the Java truth + server TS, build per-prot schema
//! JSON, cross-diff (P1–P6), and emit the report + divergence baseline.

use super::parse::{parse_java, parse_ts};
use super::types::{
    Divergence, DivergenceBaseline, Finding, JavaPacket, JavaParse, PROTS, Prot, Schema,
    SchemaPacket, TsPacket,
};
use crate::error::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: &str = "protocol-910/v1";
const REPORT_SCHEMA: &str = "protocol-report/v1";
const DIVERGENCE_SCHEMA: &str = "protocol-divergences/v1";

/// Report summary counts.
#[derive(Debug, Serialize)]
struct ReportSummary {
    errors: usize,
    warnings: usize,
    infos: usize,
}

/// The full cross-diff report.
#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    summary: ReportSummary,
    findings: Vec<Finding>,
}

// ---------------------------------------------------------------------------
// extract-protocol entry point
// ---------------------------------------------------------------------------

/// Resolved options for the `extract-protocol` subcommand.
#[derive(Debug)]
pub struct ExtractProtocolOpts<'a> {
    /// Root of the client checkout (holds `client/src/main/java/...`).
    pub client_root: &'a Path,
    /// Root of the server checkout (holds `src/jagex/network/protocol/...`).
    pub server_root: &'a Path,
    /// Output directory (default `<data-dir>/protocol/910`).
    pub out_dir: &'a Path,
}

/// The in-memory product of extraction, kept separate from disk so tests and the
/// `generate-protocol --check` path can compare bytes without re-reading files.
pub struct ExtractOutput {
    /// Schema JSON keyed by prot tag (pretty-printed, trailing newline).
    pub schemas: BTreeMap<String, String>,
    /// The report JSON (pretty-printed, trailing newline).
    pub report: String,
    /// The divergence baseline JSON (pretty-printed, trailing newline).
    pub baseline: String,
    /// Parsed report findings (for the stdout summary).
    pub findings: Vec<Finding>,
    /// Per-prot packet counts (for the stdout summary).
    pub counts: BTreeMap<String, usize>,
}


fn java_path(client_root: &Path, prot: Prot) -> PathBuf {
    client_root
        .join("client/src/main/java/com/jagex/game/network/protocol")
        .join(prot.java_file())
}

fn ts_path(server_root: &Path, prot: Prot) -> PathBuf {
    server_root
        .join("src/jagex/network/protocol")
        .join(prot.ts_file())
}

/// Build the full extraction product in memory.
pub fn extract(opts: &ExtractProtocolOpts<'_>) -> Result<ExtractOutput> {
    let mut schemas: BTreeMap<String, String> = BTreeMap::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut findings: Vec<Finding> = Vec::new();
    let mut divergences: Vec<Divergence> = Vec::new();

    for prot in PROTS {
        let jpath = java_path(opts.client_root, prot);
        let jsrc = fs::read_to_string(&jpath)
            .with_context(|| format!("failed to read {}", jpath.display()))?;
        let java = parse_java(&jsrc, prot.java_class(), &jpath)?;

        let tpath = ts_path(opts.server_root, prot);
        let tsrc = fs::read_to_string(&tpath)
            .with_context(|| format!("failed to read {}", tpath.display()))?;
        let ts = parse_ts(&tsrc, prot.java_class(), &tpath)?;

        // Schema: from the Java truth only, sorted by opcode.
        let mut packets: Vec<SchemaPacket> = java
            .packets
            .iter()
            .map(|p| SchemaPacket {
                name: p.name.clone(),
                opcode: p.opcode,
                size: p.size,
                obf: p.obf.clone(),
            })
            .collect();
        packets.sort_by_key(|p| (p.opcode, p.name.clone()));
        counts.insert(prot.tag().to_owned(), packets.len());

        let schema = Schema {
            schema: SCHEMA_VERSION.to_owned(),
            source: jpath.display().to_string(),
            packets,
        };
        schemas.insert(prot.tag().to_owned(), to_pretty(&schema)?);

        // Cross-diff against the server TS.
        diff_prot(prot, &java, &ts, &mut findings, &mut divergences);
    }

    sort_findings(&mut findings);
    divergences.sort_by(|a, b| {
        a.prot
            .cmp(&b.prot)
            .then(a.opcode.cmp(&b.opcode))
            .then(a.check.cmp(&b.check))
    });

    let summary = ReportSummary {
        errors: findings.iter().filter(|f| f.severity == "error").count(),
        warnings: findings.iter().filter(|f| f.severity == "warning").count(),
        infos: findings.iter().filter(|f| f.severity == "info").count(),
    };
    let report = Report {
        schema: REPORT_SCHEMA,
        summary,
        findings: findings.clone(),
    };
    let baseline = DivergenceBaseline {
        schema: DIVERGENCE_SCHEMA.to_owned(),
        divergences,
    };

    Ok(ExtractOutput {
        schemas,
        report: to_pretty(&report)?,
        baseline: to_pretty(&baseline)?,
        findings,
        counts,
    })
}

/// Diff one prot's Java truth against its server TS mirror, emitting findings
/// (P1–P6) and the divergence baseline entries (P1/P2/P3/P4).
fn diff_prot(
    prot: Prot,
    java: &JavaParse,
    ts: &[TsPacket],
    findings: &mut Vec<Finding>,
    divergences: &mut Vec<Divergence>,
) {
    let tag = prot.tag();

    // P6 — duplicates within each single file.
    detect_dups_java(prot, java, findings);
    detect_dups_ts(prot, ts, findings);

    let java_by_opcode: BTreeMap<i32, &JavaPacket> =
        java.packets.iter().map(|p| (p.opcode, p)).collect();
    let ts_by_opcode: BTreeMap<i32, &TsPacket> = ts.iter().map(|p| (p.opcode, p)).collect();

    // P1 / P2 — opcodes present on both sides.
    for (&opcode, jp) in &java_by_opcode {
        if let Some(tp) = ts_by_opcode.get(&opcode) {
            if jp.name != tp.name {
                findings.push(Finding {
                    check: "P1".to_owned(),
                    severity: "error".to_owned(),
                    prot: tag.to_owned(),
                    kind: "mismatch".to_owned(),
                    opcode: Some(opcode),
                    message: format!(
                        "{tag} opcode {opcode}: client name `{}` != server name `{}`",
                        jp.name, tp.name
                    ),
                });
                divergences.push(Divergence {
                    prot: tag.to_owned(),
                    opcode,
                    check: "P1".to_owned(),
                });
            } else if jp.size != tp.size {
                // Same opcode+name, differing size.
                findings.push(Finding {
                    check: "P2".to_owned(),
                    severity: "error".to_owned(),
                    prot: tag.to_owned(),
                    kind: "mismatch".to_owned(),
                    opcode: Some(opcode),
                    message: format!(
                        "{tag} opcode {opcode} (`{}`): client size {} != server size {}",
                        jp.name, jp.size, tp.size
                    ),
                });
                divergences.push(Divergence {
                    prot: tag.to_owned(),
                    opcode,
                    check: "P2".to_owned(),
                });
            }
        } else {
            // P3 — client packet with no server entry.
            findings.push(Finding {
                check: "P3".to_owned(),
                severity: "warning".to_owned(),
                prot: tag.to_owned(),
                kind: "add".to_owned(),
                opcode: Some(opcode),
                message: format!(
                    "{tag} opcode {opcode} (`{}`) present in client, absent in server TS (unimplemented server-side)",
                    jp.name
                ),
            });
            divergences.push(Divergence {
                prot: tag.to_owned(),
                opcode,
                check: "P3".to_owned(),
            });
        }
    }

    // P4 — server TS entry with no client packet.
    for (&opcode, tp) in &ts_by_opcode {
        if !java_by_opcode.contains_key(&opcode) {
            findings.push(Finding {
                check: "P4".to_owned(),
                severity: "error".to_owned(),
                prot: tag.to_owned(),
                kind: "add".to_owned(),
                opcode: Some(opcode),
                message: format!(
                    "{tag} opcode {opcode} (`{}`) present in server TS, absent in client (server speaks a packet the client does not know)",
                    tp.name
                ),
            });
            divergences.push(Divergence {
                prot: tag.to_owned(),
                opcode,
                check: "P4".to_owned(),
            });
        }
    }

    // P5 — LoginProt client-size vacuity (one info finding).
    if prot == Prot::Login {
        let vacuous = !java.has_size_field || !java.ctor_assigns_size;
        let message = if vacuous {
            format!(
                "LoginProt: client size is vacuous — has_size_field={}, ctor_assigns_size={} (constructor: {}); schema sizes taken from declaration literals, Java parity test skips the size assertion",
                java.has_size_field,
                java.ctor_assigns_size,
                java.ctor_evidence.as_deref().unwrap_or("<none>")
            )
        } else {
            "LoginProt: client size is materialized (size field present and assigned in constructor)".to_owned()
        };
        findings.push(Finding {
            check: "P5".to_owned(),
            severity: "info".to_owned(),
            prot: tag.to_owned(),
            kind: "info".to_owned(),
            opcode: None,
            message,
        });
    }
}

/// Emit P6 findings for duplicate opcodes or names within a Java table.
fn detect_dups_java(prot: Prot, java: &JavaParse, findings: &mut Vec<Finding>) {
    let tag = prot.tag();
    let mut seen_op: BTreeSet<i32> = BTreeSet::new();
    let mut seen_name: BTreeSet<&str> = BTreeSet::new();
    for p in &java.packets {
        if !seen_op.insert(p.opcode) {
            findings.push(dup_finding(tag, Some(p.opcode), format!(
                "{tag} (client {}): duplicate opcode {}",
                prot.java_file(),
                p.opcode
            )));
        }
        if !seen_name.insert(p.name.as_str()) {
            findings.push(dup_finding(tag, Some(p.opcode), format!(
                "{tag} (client {}): duplicate name `{}`",
                prot.java_file(),
                p.name
            )));
        }
    }
}

/// Emit P6 findings for duplicate opcodes or names within a TS table.
fn detect_dups_ts(prot: Prot, ts: &[TsPacket], findings: &mut Vec<Finding>) {
    let tag = prot.tag();
    let mut seen_op: BTreeSet<i32> = BTreeSet::new();
    let mut seen_name: BTreeSet<&str> = BTreeSet::new();
    for p in ts {
        if !seen_op.insert(p.opcode) {
            findings.push(dup_finding(tag, Some(p.opcode), format!(
                "{tag} (server {}): duplicate opcode {}",
                prot.ts_file(),
                p.opcode
            )));
        }
        if !seen_name.insert(p.name.as_str()) {
            findings.push(dup_finding(tag, Some(p.opcode), format!(
                "{tag} (server {}): duplicate name `{}`",
                prot.ts_file(),
                p.name
            )));
        }
    }
}

fn dup_finding(tag: &str, opcode: Option<i32>, message: String) -> Finding {
    Finding {
        check: "P6".to_owned(),
        severity: "error".to_owned(),
        prot: tag.to_owned(),
        kind: "dup".to_owned(),
        opcode,
        message,
    }
}

/// Sort findings deterministically by `(check, kind, opcode, prot, message)`.
fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        a.check
            .cmp(&b.check)
            .then(a.kind.cmp(&b.kind))
            .then(a.opcode.cmp(&b.opcode))
            .then(a.prot.cmp(&b.prot))
            .then(a.message.cmp(&b.message))
    });
}

/// Serialize a value as pretty JSON with a trailing newline.
fn to_pretty<T: Serialize>(value: &T) -> Result<String> {
    let mut s = serde_json::to_string_pretty(value)?;
    s.push('\n');
    Ok(s)
}

/// Run the `extract-protocol` subcommand: write schema, report, and baseline.
pub fn run_extract(opts: &ExtractProtocolOpts<'_>) -> Result<()> {
    let output = extract(opts)?;

    fs::create_dir_all(opts.out_dir)
        .with_context(|| format!("failed to create {}", opts.out_dir.display()))?;

    for prot in PROTS {
        let body = &output.schemas[prot.tag()];
        let path = opts.out_dir.join(format!("{}_prot.json", prot.tag()));
        fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    }
    let report_path = opts.out_dir.join("protocol-910.report.json");
    fs::write(&report_path, &output.report)
        .with_context(|| format!("failed to write {}", report_path.display()))?;
    let baseline_path = opts.out_dir.join("known-divergences.json");
    fs::write(&baseline_path, &output.baseline)
        .with_context(|| format!("failed to write {}", baseline_path.display()))?;

    print_extract_summary(&output, opts.out_dir);
    Ok(())
}

/// Print the human summary: per-prot counts, per-check finding counts, up to 20 each.
fn print_extract_summary(output: &ExtractOutput, out_dir: &Path) {
    println!("extract-protocol: out-dir {}", out_dir.display());
    for prot in PROTS {
        println!(
            "  {} packets: {}",
            prot.tag(),
            output.counts.get(prot.tag()).copied().unwrap_or(0)
        );
    }
    for check in ["P1", "P2", "P3", "P4", "P5", "P6"] {
        let matching: Vec<&Finding> =
            output.findings.iter().filter(|f| f.check == check).collect();
        println!("{check}: {} finding(s)", matching.len());
        for finding in matching.iter().take(20) {
            println!("  - {}", finding.message);
        }
        if matching.len() > 20 {
            println!("  ... and {} more", matching.len() - 20);
        }
    }
}
