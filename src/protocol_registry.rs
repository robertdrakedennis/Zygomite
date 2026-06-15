//! `extract-protocol` / `generate-protocol`.
//!
//! Extract the canonical game-protocol schema from the Java client (the source
//! of truth), cross-diff the server's TypeScript mirrors, and emit the
//! parity-gate artifacts both ends consume.
//!
//! The client's `ServerProt.java` / `ClientProt.java` / `LoginProt.java` define
//! every packet's name, opcode, and size. `extract-protocol` parses those into
//! schema JSON, diffs the three server TS tables against them (checks P1–P6),
//! and writes a findings report plus a checked-in divergence baseline.
//! `generate-protocol` then turns the schema + baseline into the server's
//! `protocol910.ts` tables and the client's `protocol-910.tsv` resource.
//!
//! Both commands are read-only over the client/server source trees; field
//! layouts and encode generation are out of scope for this stage.
//!
//! Example invocations:
//!
//! ```bash
//! cd tools/rs3-cache-rs
//! cargo run --release -- --data-dir data extract-protocol
//! cargo run --release -- --data-dir data generate-protocol --check
//! ```

use crate::error::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: &str = "protocol-910/v1";
const REPORT_SCHEMA: &str = "protocol-report/v1";
const DIVERGENCE_SCHEMA: &str = "protocol-divergences/v1";

/// The three protocol tables, in their stable emission order.
const PROTS: [Prot; 3] = [Prot::Server, Prot::Client, Prot::Login];

/// One of the three protocol tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prot {
    /// `ServerProt` — packets the server sends to the client.
    Server,
    /// `ClientProt` — packets the client sends to the server.
    Client,
    /// `LoginProt` — login/handshake packets.
    Login,
}

impl Prot {
    /// Lower-case wire tag used in the TSV, baseline, and report (`server` / `client` / `login`).
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
            Self::Login => "login",
        }
    }

    /// The Java class name backing this prot.
    #[must_use]
    pub const fn java_class(self) -> &'static str {
        match self {
            Self::Server => "ServerProt",
            Self::Client => "ClientProt",
            Self::Login => "LoginProt",
        }
    }

    /// The Java source file name (under the protocol package).
    #[must_use]
    pub const fn java_file(self) -> &'static str {
        match self {
            Self::Server => "ServerProt.java",
            Self::Client => "ClientProt.java",
            Self::Login => "LoginProt.java",
        }
    }

    /// The server TS source file name (under the protocol package).
    #[must_use]
    pub const fn ts_file(self) -> &'static str {
        match self {
            Self::Server => "ServerProt.ts",
            Self::Client => "ClientProt.ts",
            Self::Login => "LoginProt.ts",
        }
    }
}

// ---------------------------------------------------------------------------
// Java parsing (the source of truth)
// ---------------------------------------------------------------------------

/// One packet extracted from a client Java table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaPacket {
    /// Packet name (the static field identifier).
    pub name: String,
    /// Wire opcode.
    pub opcode: i32,
    /// Declared size (positive fixed, `-1` 1-byte prefix, `-2` 2-byte prefix).
    pub size: i32,
    /// `@ObfuscatedName` value attached to the declaration, when present.
    pub obf: Option<String>,
}

/// Result of parsing a client Java protocol table.
#[derive(Debug, Default)]
pub struct JavaParse {
    /// Packets in declaration order.
    pub packets: Vec<JavaPacket>,
    /// `true` when a `size` instance field is declared in the class.
    pub has_size_field: bool,
    /// `true` when the constructor body assigns `this.size`.
    pub ctor_assigns_size: bool,
    /// The raw constructor signature/body line(s) joined, for diagnostics.
    pub ctor_evidence: Option<String>,
}

/// Parse a client Java protocol table.
///
/// Every packet is declared as
/// `public static final <Class> NAME = new <Class>(<id>, <size>);`, optionally
/// preceded by an `@ObfuscatedName("…")` line (attached only when it is the
/// immediately preceding line, mirroring the Stage 1 enum parser). `size`-field
/// presence and the constructor's `this.size` assignment are also recorded so
/// the caller can verify the `LoginProt` vacuity question (spec §1.1).
pub fn parse_java(source: &str, class: &str, path: &Path) -> Result<JavaParse> {
    let field_prefix = format!("public static final {class} ");
    let ctor_prefix = format!("public {class}(");
    let new_marker = format!("new {class}(");
    let size_field_marker = "public final int size;";

    let mut parse = JavaParse::default();
    let mut pending_obf: Option<String> = None;

    let lines: Vec<&str> = source.lines().collect();
    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        let one_based = idx + 1;

        if trimmed.is_empty() {
            continue;
        }

        if let Some(obf) = parse_obfuscated_name(trimmed) {
            pending_obf = Some(obf);
            continue;
        }

        if trimmed == size_field_marker {
            parse.has_size_field = true;
            pending_obf = None;
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(field_prefix.as_str()) {
            let packet = parse_java_field(rest, &new_marker, pending_obf.take(), path, one_based)?;
            parse.packets.push(packet);
            continue;
        }

        if trimmed.starts_with(ctor_prefix.as_str()) {
            // Capture the constructor body (this line plus following lines until
            // the closing brace) to verify whether `this.size` is assigned.
            let (assigns, evidence) = scan_ctor_body(&lines, idx);
            parse.ctor_assigns_size = assigns;
            parse.ctor_evidence = Some(evidence);
            pending_obf = None;
            continue;
        }

        // Any other non-blank line clears the annotation stash.
        pending_obf = None;
    }

    Ok(parse)
}

/// Extract the quoted value of a bare `@ObfuscatedName("…")` line.
fn parse_obfuscated_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("@ObfuscatedName(\"")?;
    let value = rest.strip_suffix("\")")?;
    Some(value.to_owned())
}

/// Parse the tail after `public static final <Class> ` into a [`JavaPacket`].
fn parse_java_field(
    rest: &str,
    new_marker: &str,
    obf: Option<String>,
    path: &Path,
    line_no: usize,
) -> Result<JavaPacket> {
    let split_marker = format!(" = {new_marker}");
    let (name, ctor) = rest.split_once(split_marker.as_str()).with_context(|| {
        format!(
            "{}:{}: malformed protocol field declaration: `{}`",
            path.display(),
            line_no,
            rest
        )
    })?;
    let args = ctor.strip_suffix(");").with_context(|| {
        format!(
            "{}:{}: protocol constructor missing `);`: `{}`",
            path.display(),
            line_no,
            rest
        )
    })?;
    let (opcode, size) = parse_id_size(args, path, line_no)?;
    Ok(JavaPacket {
        name: name.trim().to_owned(),
        opcode,
        size,
        obf,
    })
}

/// Parse the `<id>, <size>` argument pair of a `(id, size)` constructor call.
fn parse_id_size(args: &str, path: &Path, line_no: usize) -> Result<(i32, i32)> {
    let (id_text, size_text) = args.split_once(',').with_context(|| {
        format!(
            "{}:{}: protocol constructor expected `id, size`, found `{}`",
            path.display(),
            line_no,
            args
        )
    })?;
    let opcode: i32 = id_text.trim().parse().with_context(|| {
        format!(
            "{}:{}: could not parse opcode from `{}`",
            path.display(),
            line_no,
            id_text.trim()
        )
    })?;
    let size: i32 = size_text.trim().parse().with_context(|| {
        format!(
            "{}:{}: could not parse size from `{}`",
            path.display(),
            line_no,
            size_text.trim()
        )
    })?;
    Ok((opcode, size))
}

/// Scan a constructor body starting at `open_idx` and report whether it assigns
/// `this.size`, plus a one-line evidence string of the body.
fn scan_ctor_body(lines: &[&str], open_idx: usize) -> (bool, String) {
    let mut assigns = false;
    let mut evidence: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut started = false;
    let mut idx = open_idx;

    while let Some(line) = lines.get(idx) {
        let trimmed = line.trim();
        evidence.push(trimmed.to_owned());
        if trimmed.contains("this.size") {
            assigns = true;
        }
        depth += i32::try_from(trimmed.matches('{').count()).unwrap_or(0);
        depth -= i32::try_from(trimmed.matches('}').count()).unwrap_or(0);
        if trimmed.contains('{') {
            started = true;
        }
        if started && depth <= 0 {
            break;
        }
        idx += 1;
        if idx - open_idx > 16 {
            break; // safety bound; constructors are tiny
        }
    }

    (assigns, evidence.join(" "))
}

// ---------------------------------------------------------------------------
// Server TS parsing (the diff target)
// ---------------------------------------------------------------------------

/// One packet extracted from a server TS table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsPacket {
    /// Packet name (the static field identifier).
    pub name: String,
    /// Wire opcode.
    pub opcode: i32,
    /// Declared size.
    pub size: i32,
}

/// Parse a server TS protocol table.
///
/// Two declaration forms are tolerated (spec §1.2):
/// - `static readonly NAME = new <Class>(<op>, <size>[, 'NAME']);`
/// - `static readonly NAME = <Class>.register(<op>, <size>, 'NAME');`
pub fn parse_ts(source: &str, class: &str, path: &Path) -> Result<Vec<TsPacket>> {
    let prefix = "static readonly ";
    let new_marker = format!("new {class}(");
    let register_marker = format!("{class}.register(");

    let mut packets = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let trimmed = raw.trim();
        let one_based = idx + 1;

        let Some(rest) = trimmed.strip_prefix(prefix) else {
            continue;
        };
        let Some((name, after_eq)) = rest.split_once(" = ") else {
            continue;
        };
        let name = name.trim();

        let args = if let Some(args) = after_eq.strip_prefix(new_marker.as_str()) {
            args
        } else if let Some(args) = after_eq.strip_prefix(register_marker.as_str()) {
            args
        } else {
            // A `static readonly` that is not a packet decl (e.g. BY_ID array).
            continue;
        };

        let args = args.strip_suffix(';').unwrap_or(args);
        let args = args.strip_suffix(')').with_context(|| {
            format!(
                "{}:{}: TS protocol declaration missing `)`: `{}`",
                path.display(),
                one_based,
                trimmed
            )
        })?;

        let (opcode, size) = parse_ts_args(args, path, one_based)?;
        packets.push(TsPacket {
            name: name.to_owned(),
            opcode,
            size,
        });
    }
    Ok(packets)
}

/// Parse the `<op>, <size>[, 'NAME']` argument list of a TS declaration.
fn parse_ts_args(args: &str, path: &Path, line_no: usize) -> Result<(i32, i32)> {
    let mut parts = args.split(',').map(str::trim);
    let op_text = parts.next().with_context(|| {
        format!("{}:{}: TS declaration missing opcode", path.display(), line_no)
    })?;
    let size_text = parts.next().with_context(|| {
        format!("{}:{}: TS declaration missing size", path.display(), line_no)
    })?;
    let opcode: i32 = op_text.parse().with_context(|| {
        format!(
            "{}:{}: could not parse TS opcode from `{}`",
            path.display(),
            line_no,
            op_text
        )
    })?;
    let size: i32 = size_text.parse().with_context(|| {
        format!(
            "{}:{}: could not parse TS size from `{}`",
            path.display(),
            line_no,
            size_text
        )
    })?;
    Ok((opcode, size))
}

// ---------------------------------------------------------------------------
// Schema documents
// ---------------------------------------------------------------------------

/// One packet row in a schema document.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SchemaPacket {
    /// Packet name.
    pub name: String,
    /// Wire opcode.
    pub opcode: i32,
    /// Declared size.
    pub size: i32,
    /// `@ObfuscatedName` value, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obf: Option<String>,
}

/// A schema document for one prot.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Schema {
    /// Schema version tag.
    pub schema: String,
    /// Resolved client Java path the schema was extracted from.
    pub source: String,
    /// Packets, sorted by opcode.
    pub packets: Vec<SchemaPacket>,
}

// ---------------------------------------------------------------------------
// Report + divergence baseline documents
// ---------------------------------------------------------------------------

/// One cross-diff finding.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Check id (`P1`..`P6`).
    pub check: String,
    /// Severity (`error` / `warning` / `info`).
    pub severity: String,
    /// Affected prot tag.
    pub prot: String,
    /// `add` / `mismatch` / `dup` — the kind of divergence, for stable sorting.
    pub kind: String,
    /// Affected opcode, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opcode: Option<i32>,
    /// Human-readable finding message.
    pub message: String,
}

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

/// One entry in the divergence baseline.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Divergence {
    /// Affected prot tag.
    pub prot: String,
    /// Affected opcode.
    pub opcode: i32,
    /// Check id that produced this divergence.
    pub check: String,
}

/// The divergence baseline document.
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct DivergenceBaseline {
    /// Schema version tag.
    pub schema: String,
    /// Divergences, sorted by `(prot, opcode, check)`.
    pub divergences: Vec<Divergence>,
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

// ---------------------------------------------------------------------------
// generate-protocol entry point
// ---------------------------------------------------------------------------

const GENERATED_TS_HEADER: &str = "// GENERATED by rs3-cache-rs `generate-protocol` from data/protocol/910 — do not edit.\n// Regenerate: cd tools/rs3-cache-rs && cargo run --release -- generate-protocol\n";

/// Resolved options for the `generate-protocol` subcommand.
#[derive(Debug)]
pub struct GenerateProtocolOpts<'a> {
    /// Schema directory (default `<data-dir>/protocol/910`).
    pub schema_dir: &'a Path,
    /// Root of the server checkout.
    pub server_root: &'a Path,
    /// Root of the client checkout.
    pub client_root: &'a Path,
    /// Compare-only mode: write nothing, return drift list (CLI maps to exit 3).
    pub check: bool,
}

/// The generated artifacts, in memory. The payload encoders artifact is
/// optional — only emitted when a `payloads.json` schema is present.
pub struct GenerateOutput {
    /// `(absolute path, body)` for the server `protocol910.ts`.
    pub server_ts: (PathBuf, String),
    /// `(absolute path, body)` for the client `protocol-910.tsv`.
    pub client_tsv: (PathBuf, String),
    /// `(absolute path, body)` for the server `encoders910.ts`, when
    /// `payloads.json` exists in the schema dir.
    pub encoders_ts: Option<(PathBuf, String)>,
}

// ---------------------------------------------------------------------------
// Payload schema (DSL v1) — `payloads.json`
// ---------------------------------------------------------------------------

/// One typed parameter of a packet encoder.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct PayloadParam {
    /// Parameter name (verbatim from the encode signature).
    pub name: String,
    /// DSL type: `int` / `string` / `bigint` / `boolean`.
    #[serde(rename = "type")]
    pub ty: String,
    /// Verbatim default-value expression, when the original signature had one
    /// (preserved so the generated encoder is a drop-in for callers that omit
    /// trailing arguments). Absent for required params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// One ordered codec field of a packet payload.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct PayloadField {
    /// Write codec name (e.g. `p2_alt2`, `pjstr`).
    pub codec: String,
    /// Argument: a param name or an integer literal.
    pub arg: String,
}

/// One packet's payload schema.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct PayloadPacket {
    /// Ordered parameters.
    pub params: Vec<PayloadParam>,
    /// Ordered codec fields (the write sequence).
    pub fields: Vec<PayloadField>,
    /// Client decode read sequence (evidence; mirror-validated).
    pub client_reads: Vec<String>,
    /// Computed (loop-free) allocation-size expression for variable-size v2
    /// packets, in the §2.2 integer grammar. Absent for fixed-size packets
    /// (their size is the schema `size`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alloc: Option<String>,
}

/// The `payloads.json` document.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Payloads {
    /// Schema version tag (`protocol-payloads/v1` or `protocol-payloads/v2`).
    pub schema: String,
    /// Packets keyed by name.
    pub packets: BTreeMap<String, PayloadPacket>,
}

// ---------------------------------------------------------------------------
// DSL v2 expression grammar (Stage 8 §2.2)
//
//   expr := term (op term)*           op := '&' | '|' | '<<' | '>>' | '+' | '-'
//   term := ident | ident '[' INT ']' | INT | HEX | '(' expr ')'
//
// String-ops only (no regex, no new deps). The parser validates an arg against
// the grammar, records identifiers + array accesses for bounds checking, and
// re-emits a canonical TS expression with `!` appended to every array access
// (strict-tsc non-null assertion). v1 plain param/literal args are the
// degenerate single-term case.
// ---------------------------------------------------------------------------

/// One token of an in-grammar expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExprTok {
    Ident(String),
    /// Integer literal, kept as written (decimal or `0x` hex).
    Int(String),
    Op(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
}

/// A parsed expression AST node.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expr {
    Ident(String),
    Int(String),
    /// `ident[index]` array access.
    Index(String, u32),
    /// `(inner)` explicit grouping.
    Paren(Box<Self>),
    /// `lhs op rhs`, left-associated as written (no precedence folding — the
    /// source parenthesizes explicitly and we reproduce that grouping).
    Bin(Box<Self>, String, Box<Self>),
}

/// Tokenize an expression. Returns an error for any character outside the
/// grammar's alphabet (ternaries, `===`, `*`, `/`, etc.), which is how those
/// packets stay hand-written.
fn tokenize_expr(text: &str) -> Result<Vec<ExprTok>> {
    let bytes: Vec<char> = text.chars().collect();
    let mut toks: Vec<ExprTok> = Vec::new();
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        let c = bytes[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                toks.push(ExprTok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(ExprTok::RParen);
                i += 1;
            }
            '[' => {
                toks.push(ExprTok::LBracket);
                i += 1;
            }
            ']' => {
                toks.push(ExprTok::RBracket);
                i += 1;
                // Consume a TS non-null assertion `!` after `]` (no-op).
                if i < n && bytes[i] == '!' {
                    i += 1;
                }
            }
            '<' | '>' => {
                if i + 1 < n && bytes[i + 1] == c {
                    toks.push(ExprTok::Op(format!("{c}{c}")));
                    i += 2;
                } else {
                    return Err(crate::error::CacheError::message(format!(
                        "expression: stray `{c}` (only `<<`/`>>` admitted)"
                    )));
                }
            }
            '&' | '|' | '+' | '-' => {
                toks.push(ExprTok::Op(c.to_string()));
                i += 1;
            }
            '0'..='9' => {
                let start = i;
                if c == '0' && i + 1 < n && (bytes[i + 1] == 'x' || bytes[i + 1] == 'X') {
                    i += 2;
                    let hstart = i;
                    while i < n && bytes[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    if i == hstart {
                        return Err(crate::error::CacheError::message(
                            "expression: malformed hex literal".to_owned(),
                        ));
                    }
                } else {
                    while i < n && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                toks.push(ExprTok::Int(bytes[start..i].iter().collect()));
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                toks.push(ExprTok::Ident(bytes[start..i].iter().collect()));
            }
            _ => {
                return Err(crate::error::CacheError::message(format!(
                    "expression: illegal character {c:?}"
                )));
            }
        }
    }
    Ok(toks)
}

/// Recursive-descent parser over the token list. Mirrors the survey's parser.
struct ExprParser {
    toks: Vec<ExprTok>,
    pos: usize,
}

impl ExprParser {
    fn new(toks: Vec<ExprTok>) -> Self {
        Self { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&ExprTok> {
        self.toks.get(self.pos)
    }

    fn parse(&mut self) -> Result<Expr> {
        let e = self.parse_expr()?;
        if self.pos != self.toks.len() {
            return Err(crate::error::CacheError::message(format!(
                "expression: trailing tokens at {}",
                self.pos
            )));
        }
        Ok(e)
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        let mut left = self.parse_term()?;
        while let Some(ExprTok::Op(op)) = self.peek() {
            let op = op.clone();
            self.pos += 1;
            let right = self.parse_term()?;
            left = Expr::Bin(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr> {
        match self.peek().cloned() {
            Some(ExprTok::LParen) => {
                self.pos += 1;
                let inner = self.parse_expr()?;
                if self.peek() != Some(&ExprTok::RParen) {
                    return Err(crate::error::CacheError::message(
                        "expression: missing `)`".to_owned(),
                    ));
                }
                self.pos += 1;
                Ok(Expr::Paren(Box::new(inner)))
            }
            Some(ExprTok::Int(v)) => {
                self.pos += 1;
                Ok(Expr::Int(v))
            }
            Some(ExprTok::Ident(name)) => {
                self.pos += 1;
                if self.peek() == Some(&ExprTok::LBracket) {
                    self.pos += 1;
                    let idx = match self.peek().cloned() {
                        Some(ExprTok::Int(v)) if !v.starts_with("0x") && !v.starts_with("0X") => {
                            self.pos += 1;
                            v.parse::<u32>().map_err(|_| {
                                crate::error::CacheError::message(
                                    "expression: array index out of range".to_owned(),
                                )
                            })?
                        }
                        _ => {
                            return Err(crate::error::CacheError::message(
                                "expression: array index must be a decimal literal".to_owned(),
                            ));
                        }
                    };
                    if self.peek() != Some(&ExprTok::RBracket) {
                        return Err(crate::error::CacheError::message(
                            "expression: missing `]`".to_owned(),
                        ));
                    }
                    self.pos += 1;
                    Ok(Expr::Index(name, idx))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            other => Err(crate::error::CacheError::message(format!(
                "expression: unexpected token {other:?}"
            ))),
        }
    }
}

/// Parse an in-grammar expression string into an AST.
fn parse_expr(text: &str) -> Result<Expr> {
    let toks = tokenize_expr(text)?;
    if toks.is_empty() {
        return Err(crate::error::CacheError::message(
            "expression: empty".to_owned(),
        ));
    }
    ExprParser::new(toks).parse()
}

/// Validate every identifier and array access in an expression against the
/// declared params. `scalars` are non-array param names; `arrays` maps an
/// array param name to its declared element count (from the default).
fn validate_expr(
    expr: &Expr,
    scalars: &std::collections::HashSet<&str>,
    arrays: &BTreeMap<&str, usize>,
    ctx: &str,
) -> Result<()> {
    match expr {
        Expr::Int(_) => Ok(()),
        Expr::Ident(name) => {
            if scalars.contains(name.as_str()) {
                Ok(())
            } else if arrays.contains_key(name.as_str()) {
                Err(crate::error::CacheError::message(format!(
                    "{ctx}: array param `{name}` used without an index"
                )))
            } else {
                Err(crate::error::CacheError::message(format!(
                    "{ctx}: `{name}` is not a declared param"
                )))
            }
        }
        Expr::Index(name, idx) => {
            let Some(&len) = arrays.get(name.as_str()) else {
                return Err(crate::error::CacheError::message(format!(
                    "{ctx}: `{name}[{idx}]` indexes a non-array param"
                )));
            };
            if (*idx as usize) >= len {
                return Err(crate::error::CacheError::message(format!(
                    "{ctx}: `{name}[{idx}]` index out of bounds (length {len})"
                )));
            }
            Ok(())
        }
        Expr::Paren(inner) => validate_expr(inner, scalars, arrays, ctx),
        Expr::Bin(l, _, r) => {
            validate_expr(l, scalars, arrays, ctx)?;
            validate_expr(r, scalars, arrays, ctx)
        }
    }
}

/// Emit an expression as canonical TS: operators spaced, parens reproduced,
/// `!` appended to every array access for strict tsc.
fn emit_expr(expr: &Expr) -> String {
    match expr {
        Expr::Ident(name) => name.clone(),
        Expr::Int(v) => v.clone(),
        Expr::Index(name, idx) => format!("{name}[{idx}]!"),
        Expr::Paren(inner) => format!("({})", emit_expr(inner)),
        Expr::Bin(l, op, r) => format!("{} {op} {}", emit_expr(l), emit_expr(r)),
    }
}

/// Count the elements of a verbatim array default like `[0, 0, 0, 0]`.
/// Returns the element count, or an error if the default isn't a bracketed list.
fn array_default_len(default: &str) -> Result<usize> {
    let t = default.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| {
            crate::error::CacheError::message(format!(
                "array default `{default}` is not a `[...]` literal"
            ))
        })?;
    if inner.trim().is_empty() {
        return Ok(0);
    }
    Ok(inner.split(',').filter(|s| !s.trim().is_empty()).count())
}

/// Build the scalar-param set and array-param length map for one packet.
/// Hard-errors when an `int[]` param has no default.
fn param_models<'a>(
    name: &str,
    params: &'a [PayloadParam],
) -> Result<(std::collections::HashSet<&'a str>, BTreeMap<&'a str, usize>)> {
    let mut scalars: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut arrays: BTreeMap<&str, usize> = BTreeMap::new();
    for p in params {
        if p.ty == "int[]" {
            let default = p.default.as_deref().ok_or_else(|| {
                crate::error::CacheError::message(format!(
                    "payload `{name}` array param `{}` has no default",
                    p.name
                ))
            })?;
            arrays.insert(p.name.as_str(), array_default_len(default)?);
        } else {
            scalars.insert(p.name.as_str());
        }
    }
    Ok((scalars, arrays))
}

/// DSL-v1 fixed-width write codec → byte width. `None` means variable width.
fn codec_fixed_width(codec: &str) -> Option<u32> {
    match codec {
        "p1" | "p1_alt1" | "p1_alt2" | "p1_alt3" | "pbool" => Some(1),
        "p2" | "p2_alt1" | "p2_alt2" | "p2_alt3" => Some(2),
        "p3" => Some(3),
        "p4" | "p4_alt1" | "p4_alt2" | "p4_alt3" => Some(4),
        "p5" => Some(5),
        "p6" => Some(6),
        "p8" => Some(8),
        _ => None,
    }
}

/// Is this a DSL-v1 codec the generator understands at all (fixed or variable)?
fn is_v1_codec(codec: &str) -> bool {
    codec_fixed_width(codec).is_some() || codec == "pjstr" || codec == "pSmart1or2"
}

/// Mirror table (spec §1.4): the set of client read methods a write codec may
/// pair with. Width + alt variant must match; signed/byte read variants are
/// accepted (read-side signedness/charset choice).
fn mirror_reads(codec: &str) -> &'static [&'static str] {
    match codec {
        "p1" | "pbool" => &["g1", "g1b"],
        "p1_alt1" => &["g1_alt1", "g1b_alt1"],
        "p1_alt2" => &["g1_alt2", "g1b_alt2"],
        "p1_alt3" => &["g1_alt3", "g1b_alt3"],
        "p2" => &["g2", "g2s"],
        "p2_alt1" => &["g2_alt1", "g2s_alt1"],
        "p2_alt2" => &["g2_alt2", "g2s_alt2"],
        "p2_alt3" => &["g2_alt3"],
        "p3" => &["g3", "g3s"],
        "p4" => &["g4s", "g4"],
        "p4_alt1" => &["g4_alt1"],
        "p4_alt2" => &["g4_alt2"],
        "p4_alt3" => &["g4_alt3", "g3_alt3"],
        "p5" => &["g5"],
        "p6" => &["g6"],
        "p8" => &["g8"],
        "pjstr" => &["gjstr", "gjstr2"],
        "pSmart1or2" => &["gSmart1or2", "gSmart1or2s"],
        _ => &[],
    }
}

/// Map a DSL param type to its TS type for the generated function signature.
fn ts_param_type(ty: &str) -> &'static str {
    match ty {
        "string" => "string",
        "bigint" => "bigint",
        "boolean" => "boolean",
        "int[]" => "number[]",
        _ => "number",
    }
}

/// Convert a `SCREAMING_SNAKE` packet name to an `encodeCamelCase` function name.
fn encoder_fn_name(name: &str) -> String {
    let mut out = String::from("encode");
    for part in name.split('_') {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            for c in chars {
                out.push(c.to_ascii_lowercase());
            }
        }
    }
    out
}

/// Load the schema for one prot from the schema directory.
fn load_schema(schema_dir: &Path, prot: Prot) -> Result<Schema> {
    let path = schema_dir.join(format!("{}_prot.json", prot.tag()));
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// Load the divergence baseline from the schema directory.
fn load_baseline(schema_dir: &Path) -> Result<DivergenceBaseline> {
    let path = schema_dir.join("known-divergences.json");
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// Build both generated artifacts in memory from the schema + baseline.
pub fn generate(opts: &GenerateProtocolOpts<'_>) -> Result<GenerateOutput> {
    let server = load_schema(opts.schema_dir, Prot::Server)?;
    let client = load_schema(opts.schema_dir, Prot::Client)?;
    let login = load_schema(opts.schema_dir, Prot::Login)?;
    let baseline = load_baseline(opts.schema_dir)?;

    let server_ts_body = render_server_ts(&server, &client, &login, &baseline);
    let client_tsv_body = render_client_tsv(&server, &client, &login, &baseline);

    let server_ts_path = opts
        .server_root
        .join("src/generated/protocol/protocol910.ts");
    let client_tsv_path = opts
        .client_root
        .join("client/src/test/resources/protocol-910.tsv");

    // Payload encoders: only when a `payloads.json` schema is present.
    let encoders_ts = match load_payloads(opts.schema_dir)? {
        Some(payloads) => {
            let body = render_encoders_ts(&payloads, &server)?;
            let path = opts
                .server_root
                .join("src/generated/protocol/encoders910.ts");
            Some((path, body))
        }
        None => None,
    };

    Ok(GenerateOutput {
        server_ts: (server_ts_path, server_ts_body),
        client_tsv: (client_tsv_path, client_tsv_body),
        encoders_ts,
    })
}

/// Render the server `protocol910.ts` module.
fn render_server_ts(
    server: &Schema,
    client: &Schema,
    login: &Schema,
    baseline: &DivergenceBaseline,
) -> String {
    let mut out = String::from(GENERATED_TS_HEADER);
    out.push('\n');
    render_ts_table(&mut out, "SERVER_PROT_910", &server.packets);
    out.push('\n');
    render_ts_table(&mut out, "CLIENT_PROT_910", &client.packets);
    out.push('\n');
    render_ts_table(&mut out, "LOGIN_PROT_910", &login.packets);
    out.push('\n');
    out.push_str("export const PROTOCOL_KNOWN_DIVERGENCES = [\n");
    for d in &baseline.divergences {
        let _ = writeln!(
            out,
            "    {{ prot: '{}', opcode: {}, check: '{}' }},",
            d.prot, d.opcode, d.check
        );
    }
    out.push_str("] as const;\n");
    out
}

/// Render one `as const` table.
fn render_ts_table(out: &mut String, export_name: &str, packets: &[SchemaPacket]) {
    let _ = writeln!(out, "export const {export_name} = [");
    for p in packets {
        let _ = writeln!(
            out,
            "    {{ name: '{}', opcode: {}, size: {} }},",
            p.name, p.opcode, p.size
        );
    }
    out.push_str("] as const;\n");
}

/// Render the client `protocol-910.tsv` resource (sorted by kind, opcode).
fn render_client_tsv(
    server: &Schema,
    client: &Schema,
    login: &Schema,
    baseline: &DivergenceBaseline,
) -> String {
    // Build all lines as (kind, opcode, text) so we can sort deterministically.
    let mut rows: Vec<(String, i32, String)> = Vec::new();
    for (prot, schema) in [
        (Prot::Server, server),
        (Prot::Client, client),
        (Prot::Login, login),
    ] {
        for p in &schema.packets {
            rows.push((
                prot.tag().to_owned(),
                p.opcode,
                format!("{}\t{}\t{}\t{}", prot.tag(), p.name, p.opcode, p.size),
            ));
        }
    }
    for d in &baseline.divergences {
        rows.push((
            "divergence".to_owned(),
            d.opcode,
            format!("divergence\t{}\t{}\t{}", d.prot, d.opcode, d.check),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    let mut out = String::new();
    for (_, _, text) in rows {
        out.push_str(&text);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Payload encoder generation (`encoders910.ts`)
// ---------------------------------------------------------------------------

const GENERATED_ENCODERS_HEADER: &str = "// GENERATED by rs3-cache-rs `generate-protocol` from data/protocol/910/payloads.json — do not edit.\n// Regenerate: cd tools/rs3-cache-rs && cargo run --release -- generate-protocol\n";

/// Validate one payload packet against the schema-declared `size`, the mirror
/// table, and (Stage 8) the v2 expression grammar (field args + computed alloc).
/// Returns the variable-size flag (`true` when the schema size is `-1`/`-2`).
fn validate_payload(name: &str, packet: &PayloadPacket, size: i32) -> Result<bool> {
    // Mirror-symmetry: same length, each field's codec mirrors its read.
    if packet.fields.len() != packet.client_reads.len() {
        return Err(crate::error::CacheError::message(format!(
            "payload `{name}`: {} write field(s) but {} client read(s) — write/read length mismatch",
            packet.fields.len(),
            packet.client_reads.len()
        )));
    }
    for (idx, (field, read)) in packet.fields.iter().zip(&packet.client_reads).enumerate() {
        if !is_v1_codec(&field.codec) {
            return Err(crate::error::CacheError::message(format!(
                "payload `{name}` field {idx}: `{}` is not a DSL-v1 codec",
                field.codec
            )));
        }
        let allowed = mirror_reads(&field.codec);
        if !allowed.contains(&read.as_str()) {
            return Err(crate::error::CacheError::message(format!(
                "payload `{name}` field {idx}: write `{}` does not mirror client read `{}` (allowed: {})",
                field.codec,
                read,
                allowed.join(", ")
            )));
        }
    }

    // Expression args (v2): every field arg must parse in-grammar with all
    // identifiers declared and all array accesses in-bounds. v1 plain
    // param/literal args are the degenerate single-term case and pass too.
    let (scalars, arrays) = param_models(name, &packet.params)?;
    for (idx, field) in packet.fields.iter().enumerate() {
        let expr = parse_expr(&field.arg).map_err(|e| {
            crate::error::CacheError::message(format!(
                "payload `{name}` field {idx} arg `{}`: {e}",
                field.arg
            ))
        })?;
        validate_expr(&expr, &scalars, &arrays, &format!("payload `{name}` field {idx} arg"))?;
    }

    // Size: fixed packets must have sum-of-fixed-widths == schema size; variable
    // packets (-1/-2) must contain at least one variable codec OR carry a
    // computed `alloc` expression (Stage 8). Expression args do not change codec
    // widths, so the fixed-size sum check is unchanged.
    let variable = size < 0;
    let mut fixed_sum: u32 = 0;
    let mut has_variable = false;
    for field in &packet.fields {
        match codec_fixed_width(&field.codec) {
            Some(w) => fixed_sum += w,
            None => has_variable = true,
        }
    }

    // A computed `alloc` is validated against the same grammar/param model.
    if let Some(alloc) = &packet.alloc {
        let expr = parse_expr(alloc).map_err(|e| {
            crate::error::CacheError::message(format!(
                "payload `{name}` alloc `{alloc}`: {e}"
            ))
        })?;
        validate_expr(&expr, &scalars, &arrays, &format!("payload `{name}` alloc"))?;
    }

    if variable {
        if !has_variable && packet.alloc.is_none() {
            return Err(crate::error::CacheError::message(format!(
                "payload `{name}`: schema size {size} is variable but neither a variable-width codec nor a computed `alloc` is present"
            )));
        }
    } else {
        if has_variable {
            return Err(crate::error::CacheError::message(format!(
                "payload `{name}`: schema size {size} is fixed but a variable-width codec is present"
            )));
        }
        if packet.alloc.is_some() {
            return Err(crate::error::CacheError::message(format!(
                "payload `{name}`: schema size {size} is fixed but a computed `alloc` is present"
            )));
        }
        let declared = u32::try_from(size).unwrap_or(0);
        if fixed_sum != declared {
            return Err(crate::error::CacheError::message(format!(
                "payload `{name}`: fixed-size sum {fixed_sum} != schema size {declared}"
            )));
        }
    }
    Ok(variable)
}

/// Render the size expression for a packet's `new Packet(new Uint8Array(<expr>))`.
///
/// A v2 computed `alloc` (emitted canonically) wins when present. Otherwise:
/// fixed packets emit the literal byte count; variable packets emit a sum of
/// fixed-width terms plus `x.length + 1` for `pjstr` and `(v < 128 ? 1 : 2)`
/// for `pSmart1or2`.
fn render_size_expr(packet: &PayloadPacket) -> Result<String> {
    if let Some(alloc) = &packet.alloc {
        // Re-emit the alloc expression canonically (already validated).
        return Ok(emit_expr(&parse_expr(alloc)?));
    }
    let mut terms: Vec<String> = Vec::new();
    let mut fixed: u32 = 0;
    for field in &packet.fields {
        match codec_fixed_width(&field.codec) {
            Some(w) => fixed += w,
            None => match field.codec.as_str() {
                "pjstr" => terms.push(format!("{}.length + 1", field.arg)),
                "pSmart1or2" => terms.push(format!("({} < 128 ? 1 : 2)", field.arg)),
                _ => {}
            },
        }
    }
    if terms.is_empty() {
        return Ok(fixed.to_string());
    }
    // Leading fixed total first (when non-zero), then variable terms in order.
    let mut all: Vec<String> = Vec::new();
    if fixed > 0 {
        all.push(fixed.to_string());
    }
    all.extend(terms);
    Ok(all.join(" + "))
}

/// Render one exported typed encoder function. Field args are re-emitted from
/// their parsed AST so array accesses gain a strict-tsc `!` and the canonical
/// parenthesization is reproduced verbatim.
fn render_encoder_fn(out: &mut String, name: &str, packet: &PayloadPacket) -> Result<()> {
    let fn_name = encoder_fn_name(name);
    let sig_params: Vec<String> = packet
        .params
        .iter()
        .map(|p| match &p.default {
            Some(d) => format!("{}: {} = {}", p.name, ts_param_type(&p.ty), d),
            None => format!("{}: {}", p.name, ts_param_type(&p.ty)),
        })
        .collect();
    let _ = writeln!(
        out,
        "export function {fn_name}({}): Packet {{",
        sig_params.join(", ")
    );
    let _ = writeln!(
        out,
        "    const buf: Packet = new Packet(new Uint8Array({}));",
        render_size_expr(packet)?
    );
    for field in &packet.fields {
        let arg = emit_expr(&parse_expr(&field.arg)?);
        let _ = writeln!(out, "    buf.{}({});", field.codec, arg);
    }
    out.push_str("    return buf;\n}\n");
    Ok(())
}

/// Render the full `encoders910.ts` module from the payload schema (validated
/// against the server schema sizes). Returns an error on the first validation
/// failure (size sum or mirror symmetry).
fn render_encoders_ts(payloads: &Payloads, server: &Schema) -> Result<String> {
    if payloads.schema != "protocol-payloads/v1" && payloads.schema != "protocol-payloads/v2" {
        return Err(crate::error::CacheError::message(format!(
            "unexpected payloads schema `{}` (want protocol-payloads/v1 or /v2)",
            payloads.schema
        )));
    }
    let size_by_name: BTreeMap<&str, i32> =
        server.packets.iter().map(|p| (p.name.as_str(), p.size)).collect();

    // Validate every packet before emitting anything.
    for (name, packet) in &payloads.packets {
        let size = size_by_name.get(name.as_str()).copied().ok_or_else(|| {
            crate::error::CacheError::message(format!(
                "payload `{name}` has no entry in server_prot.json schema"
            ))
        })?;
        validate_payload(name, packet, size)?;
    }

    let mut out = String::from(GENERATED_ENCODERS_HEADER);
    out.push('\n');
    out.push_str("import Packet from '#jagex/bytepacking/Packet.js';\n\n");
    let mut first = true;
    for (name, packet) in &payloads.packets {
        if !first {
            out.push('\n');
        }
        first = false;
        render_encoder_fn(&mut out, name, packet)?;
    }
    Ok(out)
}

/// Load `payloads.json` from the schema dir, if present.
fn load_payloads(schema_dir: &Path) -> Result<Option<Payloads>> {
    let path = schema_dir.join("payloads.json");
    if !path.is_file() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let payloads: Payloads =
        serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(payloads))
}

/// Run the `generate-protocol` subcommand. Returns `true` when `--check` found
/// drift (the CLI maps that to exit code 3).
pub fn run_generate(opts: &GenerateProtocolOpts<'_>) -> Result<bool> {
    let output = generate(opts)?;
    let mut artifacts: Vec<&(PathBuf, String)> = vec![&output.server_ts, &output.client_tsv];
    if let Some(enc) = &output.encoders_ts {
        artifacts.push(enc);
    }

    if opts.check {
        let mut drift: Vec<&Path> = Vec::new();
        for (path, body) in &artifacts {
            let on_disk = fs::read_to_string(path).unwrap_or_default();
            if on_disk != *body {
                drift.push(path);
            }
        }
        if drift.is_empty() {
            println!("generate-protocol --check: all {} artifact(s) up to date", artifacts.len());
            return Ok(false);
        }
        println!("generate-protocol --check: {} artifact(s) differ:", drift.len());
        for path in &drift {
            println!("  - {}", path.display());
        }
        return Ok(true);
    }

    for (path, body) in &artifacts {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::{
        DivergenceBaseline, ExtractProtocolOpts, GenerateProtocolOpts, PayloadField, PayloadPacket,
        PayloadParam, Payloads, Prot, Schema, SchemaPacket, array_default_len, codec_fixed_width,
        emit_expr, encoder_fn_name, extract, generate, is_v1_codec, mirror_reads, parse_expr,
        parse_java, parse_ts, render_client_tsv, render_encoders_ts, render_server_ts,
        render_size_expr, validate_payload,
    };
    use crate::error::Result;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    /// `(client_root, server_root)` produced by [`write_tree`].
    type RootPair = (std::path::PathBuf, std::path::PathBuf);

    const SERVER_JAVA: &str = r#"
package com.jagex.game.network.protocol;

@ObfuscatedName("nz")
public class ServerProt {

	@ObfuscatedName("nz.e")
	public static final ServerProt TELEMETRY_GRID_ADD_GROUP = new ServerProt(0, 5);

	@ObfuscatedName("nz.n")
	public static final ServerProt ENVIRONMENT_OVERRIDE = new ServerProt(1, -1);

	@ObfuscatedName("nz.gg")
	public final int id;

	@ObfuscatedName("nz.gr")
	public final int size;

	public ServerProt(int id, int size) {
		this.id = id;
		this.size = size;
	}
}
"#;

    const LOGIN_JAVA: &str = r#"
package com.jagex.game.network.protocol;

@ObfuscatedName("nu")
public class LoginProt {

	@ObfuscatedName("nu.e")
	public static final LoginProt INIT_GAME_CONNECTION = new LoginProt(14, 0);

	@ObfuscatedName("nu.n")
	public static final LoginProt INIT_JS5REMOTE_CONNECTION = new LoginProt(15, -1);

	@ObfuscatedName("nu.r")
	public final int id;

	public LoginProt(int id, int size) {
		this.id = id;
	}
}
"#;

    #[test]
    fn java_parser_records_packets_and_obf() -> Result<()> {
        let parse = parse_java(SERVER_JAVA, "ServerProt", Path::new("ServerProt.java"))?;
        assert_eq!(parse.packets.len(), 2);
        assert_eq!(parse.packets[0].name, "TELEMETRY_GRID_ADD_GROUP");
        assert_eq!(parse.packets[0].opcode, 0);
        assert_eq!(parse.packets[0].size, 5);
        assert_eq!(parse.packets[0].obf.as_deref(), Some("nz.e"));
        assert_eq!(parse.packets[1].size, -1);
        assert!(parse.has_size_field);
        assert!(parse.ctor_assigns_size);
        Ok(())
    }

    #[test]
    fn java_parser_detects_login_size_vacuity() -> Result<()> {
        let parse = parse_java(LOGIN_JAVA, "LoginProt", Path::new("LoginProt.java"))?;
        // No `public final int size;` field and the constructor never assigns
        // `this.size` — the size parameter is dead.
        assert!(!parse.has_size_field);
        assert!(!parse.ctor_assigns_size);
        assert!(parse.ctor_evidence.as_deref().expect("ctor evidence").contains("this.id"));
        // Sizes still come from declaration literals.
        assert_eq!(parse.packets[0].size, 0);
        assert_eq!(parse.packets[1].size, -1);
        Ok(())
    }

    #[test]
    fn ts_parser_handles_new_and_register_forms() -> Result<()> {
        let server_ts = r"
export default class ServerProt {
    static readonly TELEMETRY_GRID_ADD_GROUP = new ServerProt(0, 5, 'TELEMETRY_GRID_ADD_GROUP');
    static readonly field3697 = new ServerProt(1, 9);
}
";
        let packets = parse_ts(server_ts, "ServerProt", Path::new("ServerProt.ts"))?;
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].name, "TELEMETRY_GRID_ADD_GROUP");
        assert_eq!(packets[1].name, "field3697");
        assert_eq!(packets[1].size, 9);

        let login_ts = r"
export default class LoginProt {
    static readonly BY_ID: LoginProt[] = new Array(32);
    static readonly INIT_GAME_CONNECTION = LoginProt.register(14, 0, 'INIT_GAME_CONNECTION');
}
";
        let lp = parse_ts(login_ts, "LoginProt", Path::new("LoginProt.ts"))?;
        // The BY_ID array static is skipped; only the register() decl is kept.
        assert_eq!(lp.len(), 1);
        assert_eq!(lp[0].name, "INIT_GAME_CONNECTION");
        assert_eq!(lp[0].opcode, 14);
        Ok(())
    }

    /// Build a minimal client+server tree and run the full extraction.
    fn write_tree(
        dir: &Path,
        server_java: &str,
        client_java: &str,
        login_java: &str,
        server_ts: &str,
        client_ts: &str,
        login_ts: &str,
    ) -> Result<RootPair> {
        let client = dir.join("client-root");
        let server = dir.join("server-root");
        let jdir = client.join("client/src/main/java/com/jagex/game/network/protocol");
        let tdir = server.join("src/jagex/network/protocol");
        fs::create_dir_all(&jdir)?;
        fs::create_dir_all(&tdir)?;
        fs::write(jdir.join("ServerProt.java"), server_java)?;
        fs::write(jdir.join("ClientProt.java"), client_java)?;
        fs::write(jdir.join("LoginProt.java"), login_java)?;
        fs::write(tdir.join("ServerProt.ts"), server_ts)?;
        fs::write(tdir.join("ClientProt.ts"), client_ts)?;
        fs::write(tdir.join("LoginProt.ts"), login_ts)?;
        Ok((client, server))
    }

    #[test]
    fn cross_diff_fires_each_check() -> Result<()> {
        let dir = tempdir()?;

        // Client ClientProt: opcode 0 = A(size 3), opcode 1 = B(size 4). (no opcode 2)
        let client_java = r"
public class ClientProt {
	public static final ClientProt A = new ClientProt(0, 3);
	public static final ClientProt B = new ClientProt(1, 4);
	public final int id;
	public final int size;
	public ClientProt(int id, int size) {
		this.id = id;
		this.size = size;
	}
}
";
        // Server ClientProt.ts: opcode 0 renamed (P1), opcode 1 size differs (P2),
        // opcode 2 server-only (P4). B is also a P-something on the client side?
        // opcode 5 client-only handled by ServerProt below; here B(1) has wrong size.
        let client_ts = r"
export default class ClientProt {
    static readonly RENAMED = new ClientProt(0, 3, 'RENAMED');
    static readonly B = new ClientProt(1, 99, 'B');
    static readonly SERVER_ONLY = new ClientProt(2, 1, 'SERVER_ONLY');
}
";

        // ServerProt: opcode 0 only in client → P3 (server missing it).
        let server_java = r"
public class ServerProt {
	public static final ServerProt S0 = new ServerProt(0, 5);
	public static final ServerProt S1 = new ServerProt(1, 6);
	public final int id;
	public final int size;
	public ServerProt(int id, int size) {
		this.id = id;
		this.size = size;
	}
}
";
        let server_ts = r"
export default class ServerProt {
    static readonly S1 = new ServerProt(1, 6, 'S1');
}
";

        // LoginProt with vacuous size → P5. Duplicate opcode within Java → P6.
        let login_java = r"
public class LoginProt {
	public static final LoginProt L0 = new LoginProt(14, 0);
	public static final LoginProt L0DUP = new LoginProt(14, 1);
	public final int id;
	public LoginProt(int id, int size) {
		this.id = id;
	}
}
";
        let login_ts = r"
export default class LoginProt {
    static readonly L0 = LoginProt.register(14, 0, 'L0');
    static readonly L0DUP = LoginProt.register(14, 1, 'L0DUP');
}
";

        let (client, server) = write_tree(
            dir.path(),
            server_java,
            client_java,
            login_java,
            server_ts,
            client_ts,
            login_ts,
        )?;
        let out_dir = dir.path().join("protocol/910");
        let output = extract(&ExtractProtocolOpts {
            client_root: &client,
            server_root: &server,
            out_dir: &out_dir,
        })?;

        let checks: BTreeSet<String> =
            output.findings.iter().map(|f| f.check.clone()).collect();
        for expected in ["P1", "P2", "P3", "P4", "P5", "P6"] {
            assert!(checks.contains(expected), "missing {expected}: {checks:?}");
        }

        // Baseline carries P1/P2/P3/P4 (not P5/P6).
        let baseline: DivergenceBaseline = serde_json::from_str(&output.baseline)?;
        let baseline_checks: BTreeSet<String> =
            baseline.divergences.iter().map(|d| d.check.clone()).collect();
        assert!(baseline_checks.contains("P1"));
        assert!(baseline_checks.contains("P2"));
        assert!(baseline_checks.contains("P3"));
        assert!(baseline_checks.contains("P4"));
        assert!(!baseline_checks.contains("P5"));
        assert!(!baseline_checks.contains("P6"));

        // Schema packet counts (client 2, server 2, login 2).
        assert_eq!(output.counts["client"], 2);
        assert_eq!(output.counts["server"], 2);
        assert_eq!(output.counts["login"], 2);
        Ok(())
    }

    #[test]
    fn emission_formats_and_round_trips() {
        let server = Schema {
            schema: "protocol-910/v1".to_owned(),
            source: "x".to_owned(),
            packets: vec![super::SchemaPacket {
                name: "TELEMETRY_GRID_ADD_GROUP".to_owned(),
                opcode: 0,
                size: 5,
                obf: Some("nz.e".to_owned()),
            }],
        };
        let empty = Schema {
            schema: "protocol-910/v1".to_owned(),
            source: "x".to_owned(),
            packets: vec![],
        };
        let baseline = DivergenceBaseline {
            schema: "protocol-divergences/v1".to_owned(),
            divergences: vec![super::Divergence {
                prot: "client".to_owned(),
                opcode: 55,
                check: "P4".to_owned(),
            }],
        };

        let ts = render_server_ts(&server, &empty, &empty, &baseline);
        assert!(ts.contains("export const SERVER_PROT_910 = ["));
        assert!(ts.contains("{ name: 'TELEMETRY_GRID_ADD_GROUP', opcode: 0, size: 5 },"));
        assert!(ts.contains("{ prot: 'client', opcode: 55, check: 'P4' },"));
        assert!(ts.ends_with("] as const;\n"));

        let tsv = render_client_tsv(&server, &empty, &empty, &baseline);
        assert!(tsv.contains("divergence\tclient\t55\tP4\n"));
        assert!(tsv.contains("server\tTELEMETRY_GRID_ADD_GROUP\t0\t5\n"));
        assert!(tsv.ends_with('\n'));
    }

    #[test]
    fn generate_check_round_trips_against_written_files() -> Result<()> {
        let dir = tempdir()?;
        // Minimal schema dir.
        let schema_dir = dir.path().join("protocol/910");
        fs::create_dir_all(&schema_dir)?;
        let schema = r#"{ "schema": "protocol-910/v1", "source": "x", "packets": [ { "name": "A", "opcode": 0, "size": 5 } ] }"#;
        fs::write(schema_dir.join("server_prot.json"), schema)?;
        fs::write(schema_dir.join("client_prot.json"), schema)?;
        fs::write(schema_dir.join("login_prot.json"), schema)?;
        fs::write(
            schema_dir.join("known-divergences.json"),
            r#"{ "schema": "protocol-divergences/v1", "divergences": [] }"#,
        )?;

        let server = dir.path().join("server-root");
        let client = dir.path().join("client-root");

        // Write, then --check must report no drift; double-write byte-identical.
        let drift = super::run_generate(&GenerateProtocolOpts {
            schema_dir: &schema_dir,
            server_root: &server,
            client_root: &client,
            check: false,
        })?;
        assert!(!drift);
        let g1 = generate(&GenerateProtocolOpts {
            schema_dir: &schema_dir,
            server_root: &server,
            client_root: &client,
            check: false,
        })?;
        let drift = super::run_generate(&GenerateProtocolOpts {
            schema_dir: &schema_dir,
            server_root: &server,
            client_root: &client,
            check: true,
        })?;
        assert!(!drift, "freshly written artifacts must not drift");
        // Byte-stable across runs.
        let g2 = generate(&GenerateProtocolOpts {
            schema_dir: &schema_dir,
            server_root: &server,
            client_root: &client,
            check: false,
        })?;
        assert_eq!(g1.server_ts.1, g2.server_ts.1);
        assert_eq!(g1.client_tsv.1, g2.client_tsv.1);
        Ok(())
    }

    #[test]
    fn extract_is_byte_stable() -> Result<()> {
        let dir = tempdir()?;
        let (client, server) = write_tree(
            dir.path(),
            SERVER_JAVA,
            r"
public class ClientProt {
	public static final ClientProt A = new ClientProt(0, 3);
	public final int id;
	public final int size;
	public ClientProt(int id, int size) { this.id = id; this.size = size; }
}
",
            LOGIN_JAVA,
            r"
export default class ServerProt {
    static readonly TELEMETRY_GRID_ADD_GROUP = new ServerProt(0, 5, 'TELEMETRY_GRID_ADD_GROUP');
    static readonly ENVIRONMENT_OVERRIDE = new ServerProt(1, -1, 'ENVIRONMENT_OVERRIDE');
}
",
            r"
export default class ClientProt {
    static readonly A = new ClientProt(0, 3, 'A');
}
",
            r"
export default class LoginProt {
    static readonly INIT_GAME_CONNECTION = LoginProt.register(14, 0, 'INIT_GAME_CONNECTION');
    static readonly INIT_JS5REMOTE_CONNECTION = LoginProt.register(15, -1, 'INIT_JS5REMOTE_CONNECTION');
}
",
        )?;
        let opts = ExtractProtocolOpts {
            client_root: &client,
            server_root: &server,
            out_dir: &dir.path().join("protocol/910"),
        };
        let a = extract(&opts)?;
        let b = extract(&opts)?;
        assert_eq!(a.report, b.report);
        assert_eq!(a.baseline, b.baseline);
        assert_eq!(a.schemas, b.schemas);
        // Server prot extracted both packets in opcode order.
        assert!(a.schemas["server"].contains("TELEMETRY_GRID_ADD_GROUP"));
        assert_eq!(Prot::Server.tag(), "server");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Stage 7 — payload encoder generation unit tests
    // -----------------------------------------------------------------------

    fn field(codec: &str, arg: &str) -> PayloadField {
        PayloadField {
            codec: codec.to_owned(),
            arg: arg.to_owned(),
        }
    }

    fn param(name: &str, ty: &str, default: Option<&str>) -> PayloadParam {
        PayloadParam {
            name: name.to_owned(),
            ty: ty.to_owned(),
            default: default.map(str::to_owned),
        }
    }

    /// Build a `PayloadPacket` with no computed alloc (fixed/v1 default).
    fn pk(
        params: Vec<PayloadParam>,
        fields: Vec<PayloadField>,
        client_reads: &[&str],
    ) -> PayloadPacket {
        PayloadPacket {
            params,
            fields,
            client_reads: client_reads.iter().map(|s| (*s).to_owned()).collect(),
            alloc: None,
        }
    }

    /// Expect `validate_payload` to succeed and return its variable-size flag.
    fn expect_variable(name: &str, packet: &PayloadPacket, size: i32) -> bool {
        match validate_payload(name, packet, size) {
            Ok(v) => v,
            Err(e) => panic!("validate_payload({name}) unexpectedly failed: {e}"),
        }
    }

    /// Expect `validate_payload` to fail and return the error message.
    fn expect_error(name: &str, packet: &PayloadPacket, size: i32) -> String {
        match validate_payload(name, packet, size) {
            Ok(v) => panic!("validate_payload({name}) unexpectedly succeeded ({v})"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn codec_width_table_matches_packet_ts() {
        // Fixed widths per Packet.ts write methods.
        assert_eq!(codec_fixed_width("p1"), Some(1));
        assert_eq!(codec_fixed_width("p1_alt3"), Some(1));
        assert_eq!(codec_fixed_width("pbool"), Some(1));
        assert_eq!(codec_fixed_width("p2_alt2"), Some(2));
        assert_eq!(codec_fixed_width("p3"), Some(3));
        assert_eq!(codec_fixed_width("p4_alt1"), Some(4));
        assert_eq!(codec_fixed_width("p5"), Some(5));
        assert_eq!(codec_fixed_width("p6"), Some(6));
        assert_eq!(codec_fixed_width("p8"), Some(8));
        // Variable / unknown codecs have no fixed width.
        assert_eq!(codec_fixed_width("pjstr"), None);
        assert_eq!(codec_fixed_width("pSmart1or2"), None);
        assert_eq!(codec_fixed_width("pdata"), None);
        // v1 admission.
        assert!(is_v1_codec("p2_alt2"));
        assert!(is_v1_codec("pjstr"));
        assert!(is_v1_codec("pSmart1or2"));
        assert!(!is_v1_codec("pdata"));
        assert!(!is_v1_codec("pSmart2or4"));
    }

    #[test]
    fn mirror_table_pairs_widths_and_alts() {
        assert!(mirror_reads("p1").contains(&"g1b"));
        assert!(mirror_reads("p2_alt2").contains(&"g2_alt2"));
        assert!(mirror_reads("p2_alt1").contains(&"g2_alt1"));
        assert!(mirror_reads("p4").contains(&"g4s"));
        assert!(mirror_reads("p4_alt1").contains(&"g4_alt1"));
        assert!(mirror_reads("pjstr").contains(&"gjstr"));
        assert!(mirror_reads("pSmart1or2").contains(&"gSmart1or2"));
        // A width-mismatched read is never accepted.
        assert!(!mirror_reads("p1").contains(&"g2"));
        assert!(!mirror_reads("p2_alt2").contains(&"g2_alt1"));
        // Unknown codec yields an empty allow-set.
        assert!(mirror_reads("pdata").is_empty());
    }

    #[test]
    fn encoder_fn_names_are_camel_cased() {
        assert_eq!(encoder_fn_name("VARP_SMALL"), "encodeVarpSmall");
        assert_eq!(encoder_fn_name("CLIENT_SETVARC_SMALL"), "encodeClientSetvarcSmall");
        assert_eq!(encoder_fn_name("IF_MOVESUB"), "encodeIfMovesub");
        assert_eq!(encoder_fn_name("SPOTANIM_SPECIFIC"), "encodeSpotanimSpecific");
    }

    #[test]
    fn fixed_size_sum_must_equal_schema_size() {
        // VARP_SMALL: p1 (1) + p2_alt2 (2) = 3 == declared 3 → ok.
        let ok = pk(
            vec![param("id", "int", None), param("value", "int", None)],
            vec![field("p1", "value"), field("p2_alt2", "id")],
            &["g1b", "g2_alt2"],
        );
        assert!(!expect_variable("VARP_SMALL", &ok, 3));

        // Wrong declared size → hard error (sum 3 != 4).
        let err = expect_error("VARP_SMALL", &ok, 4);
        assert!(
            err.contains("fixed-size sum 3 != schema size 4"),
            "got: {err}"
        );
    }

    #[test]
    fn variable_size_expression_emission() {
        // CLIENT_SETVARCSTR_SMALL: p2 (2) + pjstr → size -1 variable.
        let strpk = pk(
            vec![param("id", "int", None), param("value", "string", None)],
            vec![field("p2", "id"), field("pjstr", "value")],
            &["g2", "gjstr"],
        );
        assert!(expect_variable("CLIENT_SETVARCSTR_SMALL", &strpk, -1));
        assert_eq!(render_size_expr(&strpk).unwrap(), "2 + value.length + 1");

        // pSmart1or2 contributes a ternary term.
        let smart = pk(
            vec![param("type", "int", None)],
            vec![field("pSmart1or2", "type")],
            &["gSmart1or2"],
        );
        assert_eq!(render_size_expr(&smart).unwrap(), "(type < 128 ? 1 : 2)");

        // A fixed packet renders the literal byte count.
        let fixed = pk(
            vec![param("energy", "int", None)],
            vec![field("p1", "energy")],
            &["g1"],
        );
        assert_eq!(render_size_expr(&fixed).unwrap(), "1");

        // A variable schema size with no variable codec / no alloc is rejected.
        let bad = expect_error("X", &fixed, -1);
        assert!(
            bad.contains("neither a variable-width codec nor a computed `alloc`"),
            "got: {bad}"
        );
        // A fixed schema size with a variable codec is rejected.
        let bad2 = expect_error("Y", &strpk, 4);
        assert!(bad2.contains("variable-width codec is present"), "got: {bad2}");
    }

    #[test]
    fn mirror_symmetry_and_unknown_codec_rejected() {
        // Mirror mismatch: p1 paired with g2 (width mismatch) → error.
        let mismatch = pk(
            vec![param("v", "int", None)],
            vec![field("p1", "v")],
            &["g2"],
        );
        let err = expect_error("BAD", &mismatch, 1);
        assert!(err.contains("does not mirror"), "got: {err}");

        // Read/write length mismatch.
        let lenbad = pk(
            vec![param("a", "int", None), param("b", "int", None)],
            vec![field("p1", "a"), field("p1", "b")],
            &["g1"],
        );
        let err = expect_error("LEN", &lenbad, 2);
        assert!(err.contains("length mismatch"), "got: {err}");

        // Unknown (non-v1) codec.
        let unknown = pk(
            vec![param("v", "int", None)],
            vec![field("pdata", "v")],
            &["gdata"],
        );
        let err = expect_error("UNK", &unknown, 1);
        assert!(err.contains("not a DSL-v1 codec"), "got: {err}");
    }

    #[test]
    fn render_encoders_ts_emits_validated_functions() -> Result<()> {
        let mut packets: BTreeMap<String, PayloadPacket> = BTreeMap::new();
        packets.insert(
            "VARP_SMALL".to_owned(),
            pk(
                vec![param("id", "int", None), param("value", "int", None)],
                vec![field("p1", "value"), field("p2_alt2", "id")],
                &["g1b", "g2_alt2"],
            ),
        );
        packets.insert(
            "SPOTANIM_SPECIFIC".to_owned(),
            pk(
                vec![
                    param("targetHash", "int", None),
                    param("height", "int", Some("0")),
                ],
                vec![field("p4_alt1", "targetHash"), field("p2_alt2", "height")],
                &["g4_alt1", "g2_alt2"],
            ),
        );
        let payloads = Payloads {
            schema: "protocol-payloads/v1".to_owned(),
            packets,
        };
        let server = Schema {
            schema: "protocol-910/v1".to_owned(),
            source: "x".to_owned(),
            packets: vec![
                SchemaPacket {
                    name: "VARP_SMALL".to_owned(),
                    opcode: 157,
                    size: 3,
                    obf: None,
                },
                SchemaPacket {
                    name: "SPOTANIM_SPECIFIC".to_owned(),
                    opcode: 99,
                    size: 6,
                    obf: None,
                },
            ],
        };
        let ts = render_encoders_ts(&payloads, &server)?;
        assert!(ts.starts_with("// GENERATED"));
        assert!(ts.contains("import Packet from '#jagex/bytepacking/Packet.js';"));
        assert!(ts.contains("export function encodeVarpSmall(id: number, value: number): Packet {"));
        assert!(ts.contains("    const buf: Packet = new Packet(new Uint8Array(3));"));
        assert!(ts.contains("    buf.p2_alt2(id);"));
        // Default preserved in the generated signature.
        assert!(ts.contains("height: number = 0"));

        // A bad schema size makes generation fail (sum check fires).
        let mut bad_server = server;
        bad_server.packets[0].size = 9;
        match render_encoders_ts(&payloads, &bad_server) {
            Ok(_) => panic!("render_encoders_ts should fail on a bad fixed-size schema"),
            Err(e) => assert!(e.to_string().contains("fixed-size sum"), "got: {e}"),
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Stage 8 — DSL v2 expression + array-param + computed-alloc unit tests
    // -----------------------------------------------------------------------

    /// Parse then re-emit; assert the canonical output matches `want`.
    fn roundtrip(input: &str, want: &str) {
        let e = parse_expr(input).unwrap_or_else(|err| panic!("parse `{input}`: {err}"));
        assert_eq!(emit_expr(&e), want, "round-trip of `{input}`");
    }

    #[test]
    fn expr_parse_emit_round_trips() {
        // Plain v1 degenerate cases.
        roundtrip("value", "value");
        roundtrip("0xff", "0xff");
        roundtrip("42", "42");
        // Shift-or cuid (the IF_SET* family).
        roundtrip(
            "(interfaceId << 16) | component",
            "(interfaceId << 16) | component",
        );
        roundtrip("(topLevelInterfaceId << 16) | component", "(topLevelInterfaceId << 16) | component");
        // Mask.
        roundtrip("snapshotId & 0xff", "snapshotId & 0xff");
        roundtrip("fontId & 0xffffffff", "fontId & 0xffffffff");
        // Array access gains the `!` non-null assertion; `key[3]!` input also OK.
        roundtrip("key[3]", "key[3]!");
        roundtrip("key[3]!", "key[3]!");
        roundtrip("key[0]", "key[0]!");
    }

    #[test]
    fn expr_rejects_out_of_grammar() {
        // Ternary, equality, multiplication, nullish — all rejected.
        assert!(parse_expr("hidden ? 1 : 0").is_err());
        assert!(parse_expr("objId === -1 ? 65535 : objId").is_err());
        assert!(parse_expr("sourceX * 2").is_err());
        assert!(parse_expr("text ?? 0").is_err());
        // Unbalanced parens / brackets.
        assert!(parse_expr("(a | b").is_err());
        assert!(parse_expr("key[1").is_err());
        // Non-decimal array index.
        assert!(parse_expr("key[0x1]").is_err());
    }

    #[test]
    fn expr_validation_checks_idents_and_bounds() {
        use super::param_models;
        let params = vec![
            param("interfaceId", "int", None),
            param("component", "int", None),
            param("key", "int[]", Some("[0, 0, 0, 0]")),
        ];
        let pkt = pk(
            params.clone(),
            vec![
                field("p4_alt1", "(interfaceId << 16) | component"),
                field("p4", "key[3]"),
            ],
            &["g4_alt1", "g4s"],
        );
        // size: 4 + 4 = 8 fixed; passes full validation.
        assert!(!expect_variable("V2OK", &pkt, 8));

        // Undeclared identifier.
        let bad_ident = pk(
            params.clone(),
            vec![field("p4", "bogus | component")],
            &["g4s"],
        );
        let err = expect_error("V2BAD", &bad_ident, 4);
        assert!(err.contains("`bogus` is not a declared param"), "got: {err}");

        // Index out of bounds (default has 4 elements, index 4 is OOB).
        let oob = pk(params.clone(), vec![field("p4", "key[4]")], &["g4s"]);
        let err = expect_error("V2OOB", &oob, 4);
        assert!(err.contains("index out of bounds"), "got: {err}");

        // Indexing a scalar param.
        let scalar_index = pk(params.clone(), vec![field("p4", "interfaceId[0]")], &["g4s"]);
        let err = expect_error("V2SCALAR", &scalar_index, 4);
        assert!(err.contains("indexes a non-array param"), "got: {err}");

        // Bare array param without an index.
        let bare_array = pk(params, vec![field("p4", "key")], &["g4s"]);
        let err = expect_error("V2BARE", &bare_array, 4);
        assert!(err.contains("used without an index"), "got: {err}");

        // Array param missing its default → param-model build fails.
        let model_params = [param("a", "int", None), param("k", "int[]", Some("[0, 0]"))];
        let (scalars, arrays) = param_models("OK", &model_params).unwrap();
        assert!(scalars.contains("a"));
        assert_eq!(arrays.get("k"), Some(&2));
        let bad_params = [param("k", "int[]", None)];
        assert!(param_models("BAD", &bad_params).is_err());
    }

    #[test]
    fn array_default_len_counts_elements() {
        assert_eq!(array_default_len("[0, 0, 0, 0]").unwrap(), 4);
        assert_eq!(array_default_len("[1,2,3]").unwrap(), 3);
        assert_eq!(array_default_len("[ ]").unwrap(), 0);
        assert!(array_default_len("0, 0").is_err());
    }

    #[test]
    fn if_opensub_shaped_end_to_end_render() -> Result<()> {
        // IF_OPENSUB: array param + shift-or cuid, all fixed (size 23).
        let mut packets: BTreeMap<String, PayloadPacket> = BTreeMap::new();
        packets.insert(
            "IF_OPENSUB".to_owned(),
            PayloadPacket {
                params: vec![
                    param("topLevelInterfaceId", "int", None),
                    param("component", "int", None),
                    param("subInterfaceId", "int", None),
                    param("type", "int", None),
                    param("key", "int[]", Some("[0, 0, 0, 0]")),
                ],
                fields: vec![
                    field("p4_alt2", "key[2]"),
                    field("p4_alt1", "(topLevelInterfaceId << 16) | component"),
                    field("p1_alt2", "type"),
                    field("p4", "key[3]"),
                    field("p2", "subInterfaceId"),
                    field("p4_alt2", "key[1]"),
                    field("p4_alt2", "key[0]"),
                ],
                client_reads: ["g4_alt2", "g4_alt1", "g1_alt2", "g4s", "g2", "g4_alt2", "g4_alt2"]
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
                alloc: None,
            },
        );
        let payloads = Payloads {
            schema: "protocol-payloads/v2".to_owned(),
            packets,
        };
        let server = Schema {
            schema: "protocol-910/v1".to_owned(),
            source: "x".to_owned(),
            packets: vec![SchemaPacket {
                name: "IF_OPENSUB".to_owned(),
                opcode: 100,
                size: 23,
                obf: None,
            }],
        };
        let ts = render_encoders_ts(&payloads, &server)?;
        assert!(ts.contains(
            "export function encodeIfOpensub(topLevelInterfaceId: number, component: number, subInterfaceId: number, type: number, key: number[] = [0, 0, 0, 0]): Packet {"
        ), "signature:\n{ts}");
        assert!(ts.contains("    const buf: Packet = new Packet(new Uint8Array(23));"));
        // Array access carries `!`, shift-or is parenthesized as parsed.
        assert!(ts.contains("    buf.p4_alt2(key[2]!);"), "body:\n{ts}");
        assert!(ts.contains("    buf.p4_alt1((topLevelInterfaceId << 16) | component);"));
        assert!(ts.contains("    buf.p4(key[3]!);"));
        Ok(())
    }

    #[test]
    fn computed_alloc_validates_and_renders() -> Result<()> {
        // A contrived variable packet whose size is a computed in-grammar expr.
        let mut packets: BTreeMap<String, PayloadPacket> = BTreeMap::new();
        packets.insert(
            "VARALLOC".to_owned(),
            PayloadPacket {
                params: vec![param("a", "int", None), param("b", "int", None)],
                fields: vec![field("p2", "a")],
                client_reads: vec!["g2".to_owned()],
                alloc: Some("2 + b".to_owned()),
            },
        );
        let payloads = Payloads {
            schema: "protocol-payloads/v2".to_owned(),
            packets,
        };
        let server = Schema {
            schema: "protocol-910/v1".to_owned(),
            source: "x".to_owned(),
            packets: vec![SchemaPacket {
                name: "VARALLOC".to_owned(),
                opcode: 1,
                size: -1,
                obf: None,
            }],
        };
        let ts = render_encoders_ts(&payloads, &server)?;
        assert!(ts.contains("    const buf: Packet = new Packet(new Uint8Array(2 + b));"), "body:\n{ts}");

        // A computed alloc on a fixed-size packet is rejected.
        let fixed_with_alloc = pk(
            vec![param("a", "int", None)],
            vec![field("p2", "a")],
            &["g2"],
        );
        let mut bad = fixed_with_alloc;
        bad.alloc = Some("2".to_owned());
        let err = expect_error("FIXEDALLOC", &bad, 2);
        assert!(err.contains("fixed but a computed `alloc`"), "got: {err}");
        Ok(())
    }
}
