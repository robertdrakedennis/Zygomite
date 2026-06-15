//! `cs2 lint-splice` — diff a spliced donor CS2 script's `// @cs2` listing
//! against a target opcode book and flag (or `--fix`) the known port rewrites.
//!
//! These rules mirror the routine 948→910 port rewrites the relic splice applied
//! (the relic porter `build-relic-scripts.py` was retired 2026-06-14 — the port
//! layer's `cs2 port --closure-of-interface 691` now reproduces the committed relic
//! listings byte-for-byte; see `plans/tooling/semantic-port-layer.md` and the
//! oracle `tests/ritual_port_oracle.rs`). Every rewrite is expressed here as a DATA
//! rule, keyed off the crate's 910 / 948 opcode-book registries
//! ([`crate::script::OpcodeBook`]) so the diff stays in sync with the books:
//!
//!   * `push_constant_string int:N; sub` → `push_constant_string int:-N; add`
//!     (910's interpreter has no `sub`; the donor RHS is always a constant).
//!   * `enum` → `_enum` (910 names the enum-lookup command `_enum`; the donor
//!     book carries the bare `enum`, which trips the assembler's round-trip
//!     fidelity gate).
//!   * db-field constants `table<<12|column<<4` → `table<<8|column` (`>>4`): the
//!     910 client packs `DBUtils` field ids as `table<<8|column`; the donor
//!     scripts carry the `<<12|<<4` form (NPE in `db_find`/`db_getfield`).
//!   * `db_find` arity: the dangling tuple-index `push_constant_string int:0`
//!     before a `db_find` becomes a zero-shift `branch` fall-through (910's
//!     `db_find` pops only `(field, key)`).
//!   * `gosub_with_params 7924` → `gosub_with_params 24924` (948's shared
//!     cc-icon-draw relocated to a free 910 id; signature drift).
//!   * per-script signature-drift stubs (14611/14620/14587) — the 3092 / 1858 /
//!     13022 callees mean something different on 910, so the donor calls are
//!     replaced with stack-shape-preserving no-ops.
//!
//! The lint reads the listing as text lines (the same representation the python
//! builder rewrites), so it neither compiles the script nor needs a cache. Run
//! on the COMMITTED (already-ported) relic listings it reports clean — proving
//! the builder left no un-ported opcode behind and the lint raises no false
//! positives (the plan's validation contract).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::script::OpcodeBook;

/// Default data dir (crate-relative), mirroring the CLI's `--data-dir` default.
pub const DEFAULT_DATA_DIR: &str = "data";
/// The donor build the relic splice set was lifted from.
pub const DONOR_BUILD: u32 = 948;
/// The base/target build the overlay assembles against.
pub const TARGET_BUILD: u32 = 910;

/// 948-format db-field constants used by the relic splice set (tables
/// 66/90/92/94/287). Mirrors `DB_FIELDS_948` in `build-relic-scripts.py`. A
/// `push_constant_string int:N` operand equal to one of these is the donor
/// `table<<12|column<<4` packing and is translated to `N >> 4`.
pub const DB_FIELDS_948: [i64; 16] = [
    270_352, 270_400, 368_640, 376_896, 376_912, 385_024, 385_040, 385_056, 385_072, 385_088,
    385_104, 385_120, 385_136, 385_152, 385_168, 1_175_552,
];

/// One `// @cs2` instruction line plus any trailing `//   @cs2 case …` lines
/// that ride along with a `switch` (mirrors the python `instructions` list,
/// where case lines are appended to their switch entry with a newline).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Instr {
    /// 0-based index in the original instruction stream (branch-target space).
    index: usize,
    /// 1-based source line in the file (for diagnostics).
    line_no: usize,
    /// The opcode mnemonic, e.g. `sub`, `push_constant_string`, `db_find`.
    op: String,
    /// The raw operand text after the opcode (e.g. `int:-1`, `0`, `94`), or
    /// empty when the instruction has none.
    operand: String,
    /// Trailing `case` lines (verbatim, including leading `//   @cs2 case`).
    cases: Vec<String>,
}

impl Instr {
    /// Reconstruct the primary `// @cs2 <op> <operand>` line (no case lines).
    fn render_primary(&self) -> String {
        if self.operand.is_empty() {
            format!("// @cs2 {}", self.op)
        } else {
            format!("// @cs2 {} {}", self.op, self.operand)
        }
    }
}

/// A header line (`// @cs2 name|locals|args …`) preserved verbatim.
#[derive(Clone, Debug)]
struct Header(String);

/// The parsed listing: leading description/comment lines, headers, and the
/// instruction stream. Non-`@cs2` comment lines and blanks are kept as-is in
/// `preamble` so `--fix` re-emits a faithful file.
#[derive(Clone, Debug)]
struct Listing {
    /// Lines before the first header/instruction (description banner etc.).
    preamble: Vec<String>,
    headers: Vec<Header>,
    instrs: Vec<Instr>,
}

/// Severity of a finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A mismatch the `--fix` rules can resolve (a known port rewrite).
    Fixable,
    /// A mismatch with no known automatic rewrite (manual review needed).
    Manual,
}

/// One reported opcode / db-field / arity mismatch vs the target book.
#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    /// 0-based instruction index (branch-target space).
    pub instr: usize,
    /// 1-based source line.
    pub line: usize,
    /// Stable rule id (e.g. `sub_to_add`, `enum_to_underscore`).
    pub rule: &'static str,
    /// Severity.
    pub severity: Severity,
    /// Human description of what is wrong and how it is (or would be) rewritten.
    pub detail: String,
}

/// Per-script lint report.
#[derive(Clone, Debug, Serialize)]
pub struct ScriptReport {
    /// Script id parsed from the filename (`scriptNNNN.asm.ts`), if any.
    pub script: Option<u32>,
    /// Source filename (basename).
    pub file: String,
    /// Findings, in instruction order.
    pub findings: Vec<Finding>,
    /// Whether `--fix` changed the file content.
    pub fixed: bool,
}

impl ScriptReport {
    /// Count of findings at [`Severity::Manual`].
    #[must_use]
    pub fn manual_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Manual)
            .count()
    }
}

/// Options for [`run`].
pub struct LintOptions<'a> {
    /// Directory of `*.asm.ts` listings to lint.
    pub scripts_dir: &'a Path,
    /// Data dir holding the opcode-book registries / txt files.
    pub data_dir: &'a Path,
    /// Target opcode book build (only 910 is meaningful today).
    pub target_book: u32,
    /// Donor opcode book build the listings were lifted from.
    pub donor_book: u32,
    /// Apply the table-driven rewrites in place.
    pub fix: bool,
    /// Emit JSON instead of the human report.
    pub json: bool,
}

/// Result of linting one listing's text without touching the filesystem.
#[derive(Clone, Debug)]
pub struct TextLintResult {
    /// Findings in instruction order.
    pub findings: Vec<Finding>,
    /// The rewritten listing text (only populated when `fix` was requested).
    pub fixed_text: Option<String>,
}

/// Lint (and optionally `--fix`) a single listing given as TEXT, against books
/// loaded from `data_dir`. This is the filesystem-free core the CLI wraps and
/// the oracle test drives. `script` is the donor script id (drives the
/// per-script signature-drift rules); pass `None` for unidentified listings.
pub fn lint_text(
    text: &str,
    script: Option<u32>,
    data_dir: &Path,
    target_book: u32,
    donor_book: u32,
    fix: bool,
) -> Result<TextLintResult> {
    let books = Books::load(data_dir, target_book, donor_book)?;
    let listing = parse_listing(text)?;
    let findings = diagnose(&listing, &books, script);
    // Only rewrite when there is something to fix (mirrors `lint_file`); on an
    // already-ported listing the rewrites would be no-ops anyway, but skipping
    // keeps the contract identical to the CLI path.
    let fixed_text = if fix && !findings.is_empty() {
        let rewritten = apply_fixes(&listing, &books, script)?;
        Some(render_listing(&rewritten))
    } else {
        None
    };
    Ok(TextLintResult {
        findings,
        fixed_text,
    })
}

/// Net stack effect (int/obj/long pop & push counts) for one opcode, read from
/// `data/stack-effects.txt` (the same table the client ScriptRunner enforces).
#[derive(Clone, Copy, Debug, Default)]
struct StackDelta {
    int_pops: i64,
    obj_pops: i64,
    long_pops: i64,
    int_pushes: i64,
    obj_pushes: i64,
    long_pushes: i64,
}

/// Loaded opcode books for the diff.
struct Books {
    target: OpcodeBook,
    donor: OpcodeBook,
    /// Static per-opcode stack effects (target book), keyed by canonical name.
    /// `None` for an opcode whose effect is operand-/callee-dependent (variadic:
    /// `gosub_with_params`, `join_string`, `db_getfield`, `switch`, …) — the
    /// net-balance check treats those as unresolvable and skips the script.
    stack: std::collections::HashMap<String, StackDelta>,
}

impl Books {
    fn load(data_dir: &Path, target: u32, donor: u32) -> Result<Self> {
        let target_book = OpcodeBook::load(data_dir, target, 0)
            .with_context(|| format!("load target opcode book (build {target})"))?;
        let donor_book = OpcodeBook::load(data_dir, donor, 0)
            .with_context(|| format!("load donor opcode book (build {donor})"))?;
        let stack = load_stack_effects(data_dir).unwrap_or_default();
        Ok(Self {
            target: target_book,
            donor: donor_book,
            stack,
        })
    }

    /// Whether the target book knows `name` (directly or via an alias).
    fn target_has(&self, name: &str) -> bool {
        self.target.opcode_for(name).is_ok()
    }

    /// Whether the target book knows `name` as a CANONICAL command (i.e. not
    /// only via an alias). A name like `enum` resolves through the alias map but
    /// is not canonical (the canonical is `_enum`), so this returns `false` —
    /// the assembler's round-trip fidelity gate rejects the non-canonical form.
    fn target_has_canonical(&self, name: &str) -> bool {
        self.target.by_name().contains_key(name)
    }

    /// The target book's canonical name for whatever opcode `name` resolves to
    /// (following aliases), or `None` if unknown.
    fn target_canonical_of(&self, name: &str) -> Option<String> {
        let opcode = self.target.opcode_for(name).ok()?;
        self.target.name(opcode).ok().map(ToString::to_string)
    }

    /// Whether the donor book knows `name` directly (no alias map for donors).
    fn donor_has(&self, name: &str) -> bool {
        self.donor.opcode_for(name).is_ok()
    }
}

/// Variadic / callee-dependent opcodes whose stack effect cannot be read off a
/// static table (it depends on an operand count, a switch, or a callee's
/// signature). When a listing contains one of these AND it is not a
/// `gosub_with_params` we can resolve against a sibling listing, the net-balance
/// check declares the listing UNVERIFIABLE and skips it (sound: never a false
/// positive). `gosub_with_params` is handled specially (sibling-signature lookup).
fn is_variadic_op(op: &str) -> bool {
    matches!(
        op,
        "join_string"
            | "switch"
            | "db_getfield"
            | "db_getfieldcount"
            | "db_find"
            | "db_findnext"
            | "db_getrowtable"
            | "cc_setonvartransmit"
            | "cc_setonstattransmit"
            | "cc_setoninvtransmit"
            | "if_setonvartransmit"
            | "if_setonstattransmit"
            | "if_setoninvtransmit"
            | "cc_setonop"
            | "if_setonop"
    )
}

/// Parse `data/stack-effects.txt` (`name int_pops obj_pops long_pops int_pushes
/// obj_pushes long_pushes`) into a name→[`StackDelta`] map.
fn load_stack_effects(data_dir: &Path) -> Result<std::collections::HashMap<String, StackDelta>> {
    let path = data_dir.join("stack-effects.txt");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read stack effects {}", path.display()))?;
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() != 7 {
            continue;
        }
        let n = |i: usize| cols[i].parse::<i64>().ok();
        if let (Some(ip), Some(op), Some(lp), Some(ipu), Some(opu), Some(lpu)) =
            (n(1), n(2), n(3), n(4), n(5), n(6))
        {
            map.insert(
                cols[0].to_string(),
                StackDelta {
                    int_pops: ip,
                    obj_pops: op,
                    long_pops: lp,
                    int_pushes: ipu,
                    obj_pushes: opu,
                    long_pushes: lpu,
                },
            );
        }
    }
    Ok(map)
}

/// Net-stack-balance check (sound, conservative). Detects the class of bug where
/// a 948→910 rewrite changes a listing's NET STACK EFFECT — e.g. neutralising a
/// 948-only opcode (pop N) with the wrong number of `pop_*_discard`, or expanding
/// a `sub` incorrectly. `cs2 lint-splice`'s opcode-availability pass does NOT
/// catch this (the opcodes are all valid); the live client then throws an
/// `ArrayIndexOutOfBounds` ("Index -K …") off the corrupted CS2 stack.
///
/// SOUNDNESS over completeness: this only runs on listings whose EVERY opcode has
/// a statically-known stack effect (i.e. NO variadic op and NO `gosub_with_params`
/// — those depend on an operand count or a callee signature this build-time lint
/// cannot resolve). On a qualifying listing it simulates the absolute (int, obj,
/// long) depths from the declared `args`, and flags a finding if any pop would
/// underflow (depth < 0) or the depths are non-zero at a `return`. On a listing
/// with any unresolvable op it emits nothing (never a false positive). Branch
/// targets are not followed — the simulation is linear over the instruction
/// stream, which is exact for the straight-line helper bodies it qualifies on.
fn net_stack_findings(listing: &Listing, books: &Books) -> Vec<Finding> {
    // Skip if any opcode is variadic or a gosub (unresolvable arity) or unknown.
    // `push_constant_string` is operand-typed (int:/str:/long:) and handled below;
    // everything else must have a static table entry.
    for instr in &listing.instrs {
        let op = instr.op.as_str();
        if op == "gosub_with_params" || is_variadic_op(op) {
            return Vec::new();
        }
        if op != "push_constant_string"
            && !books.stack.contains_key(op)
            && !is_control_op(op)
        {
            return Vec::new();
        }
    }
    let args = listing
        .headers
        .iter()
        .find_map(|Header(h)| h.strip_prefix("// @cs2 args ").and_then(parse_counts))
        .unwrap_or((0, 0, 0));
    let (mut di, mut dobj, mut dl) = args;
    let mut findings = Vec::new();
    for instr in &listing.instrs {
        let op = instr.op.as_str();
        if is_control_op(op) {
            continue; // branch/switch/label/return: no stack effect here
        }
        // `push_constant_string` pushes one value of the type its operand names:
        // `int:` → int, `str:` → obj (string), `long:` → long. The static table
        // cannot capture this (one opcode, three effects), so resolve per operand.
        let (ip, op_, lp, ipu, opu, lpu) = if op == "push_constant_string" {
            let operand = instr.operand.as_str();
            if operand.starts_with("long:") {
                (0, 0, 0, 0, 0, 1)
            } else if operand.starts_with("str:") {
                (0, 0, 0, 0, 1, 0)
            } else {
                (0, 0, 0, 1, 0, 0) // int: (and bare ints)
            }
        } else {
            let Some(d) = books.stack.get(op) else {
                return Vec::new();
            };
            (
                d.int_pops,
                d.obj_pops,
                d.long_pops,
                d.int_pushes,
                d.obj_pushes,
                d.long_pushes,
            )
        };
        di -= ip;
        dobj -= op_;
        dl -= lp;
        if di < 0 || dobj < 0 || dl < 0 {
            findings.push(Finding {
                instr: instr.index,
                line: instr.line_no,
                rule: "net_stack_underflow",
                severity: Severity::Manual,
                detail: format!(
                    "`{op}` pops more than available (depth int={di} obj={dobj} long={dl} \
                     after pop) — the listing's net stack effect is wrong (a mis-applied \
                     948→910 rewrite); the live client will throw AIOOBE off the CS2 stack"
                ),
            });
            return findings; // first underflow is enough; stop to avoid cascade.
        }
        di += ipu;
        dobj += opu;
        dl += lpu;
    }
    findings
}

/// Control-flow / pseudo opcodes that carry no entry in `stack-effects.txt` but
/// are stack-neutral for the linear simulation (their operands are not stack
/// values in the `@cs2` listing form). `branch*`/`switch`/`return`/`label` move
/// control; `_enum` has a known effect (in the table) so is not listed here.
fn is_control_op(op: &str) -> bool {
    op == "return" || op.starts_with("branch") || op == "switch" || op == "label"
}

/// Parse a `// @cs2 (locals|args) int=A obj=B long=C` header into `(A, B, C)`.
fn parse_counts(header: &str) -> Option<(i64, i64, i64)> {
    let int = parse_kv(header, "int=")?;
    let obj = parse_kv(header, "obj=")?;
    let long = parse_kv(header, "long=")?;
    Some((int, obj, long))
}

fn parse_kv(s: &str, key: &str) -> Option<i64> {
    let rest = s.split(key).nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Run `cs2 lint-splice` over a directory of listings.
pub fn run(opts: &LintOptions<'_>) -> Result<()> {
    let books = Books::load(opts.data_dir, opts.target_book, opts.donor_book)?;
    let mut files = collect_listings(opts.scripts_dir)?;
    files.sort();

    let mut reports = Vec::with_capacity(files.len());
    for path in &files {
        reports.push(lint_file(path, &books, opts.fix)?);
    }

    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&reports).context("encode lint report JSON")?
        );
    } else {
        print!("{}", render_human(&reports, opts));
    }

    // Non-zero exit on any MANUAL finding (an un-ported opcode the rules cannot
    // resolve) so a build step can gate on it. Fixable findings without `--fix`
    // are reported but do not fail (they are the expected pre-`--fix` state).
    let manual: usize = reports.iter().map(ScriptReport::manual_count).sum();
    if manual > 0 {
        std::process::exit(3);
    }
    Ok(())
}

/// Collect `*.asm.ts` listing paths under `dir`.
fn collect_listings(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("read scripts dir {}", dir.display()))?;
    for entry in entries {
        let entry = entry.context("read dir entry")?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".asm.ts"))
        {
            out.push(path);
        }
    }
    Ok(out)
}

/// Lint a single listing file, optionally rewriting it in place.
fn lint_file(path: &Path, books: &Books, fix: bool) -> Result<ScriptReport> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read listing {}", path.display()))?;
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let script = parse_script_id(&file);

    let listing = parse_listing(&text)
        .with_context(|| format!("parse listing {}", path.display()))?;
    let findings = diagnose(&listing, books, script);

    let mut fixed = false;
    if fix && !findings.is_empty() {
        let rewritten = apply_fixes(&listing, books, script)
            .with_context(|| format!("apply fixes to {}", path.display()))?;
        let new_text = render_listing(&rewritten);
        if new_text != text {
            std::fs::write(path, &new_text)
                .with_context(|| format!("write fixed listing {}", path.display()))?;
            fixed = true;
        }
    }

    Ok(ScriptReport {
        script,
        file,
        findings,
        fixed,
    })
}

/// Parse `scriptNNNN.asm.ts` → `Some(NNNN)`; other names → `None`.
#[must_use]
pub fn parse_script_id(file: &str) -> Option<u32> {
    let stem = file.strip_suffix(".asm.ts")?;
    let digits = stem.strip_prefix("script")?;
    digits.parse::<u32>().ok()
}

// ── Listing parser ───────────────────────────────────────────────────────────

/// Parse a `// @cs2` listing into preamble / headers / instruction stream.
/// Mirrors `build-relic-scripts.py::load_asm` plus a preserved preamble.
fn parse_listing(text: &str) -> Result<Listing> {
    let mut preamble = Vec::new();
    let mut headers = Vec::new();
    let mut instrs: Vec<Instr> = Vec::new();
    let mut seen_body = false;

    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = raw;
        if is_case_line(line) {
            let last = instrs.last_mut().with_context(|| {
                format!("case line at {line_no} before any instruction: {line:?}")
            })?;
            last.cases.push(line.to_string());
            seen_body = true;
            continue;
        }
        if let Some(rest) = header_payload(line) {
            headers.push(Header(line.to_string()));
            let _ = rest;
            seen_body = true;
            continue;
        }
        if let Some((op, operand)) = instruction_payload(line) {
            instrs.push(Instr {
                index: instrs.len(),
                line_no,
                op,
                operand,
                cases: Vec::new(),
            });
            seen_body = true;
            continue;
        }
        // Non-@cs2 line: part of the preamble if we have not started the body,
        // otherwise an interleaved comment we preserve in place by attaching it
        // to the preceding instruction's trailing block.
        if seen_body {
            // Attach stray comments to the last instruction so ordering is kept.
            if let Some(last) = instrs.last_mut() {
                last.cases.push(line.to_string());
            } else {
                preamble.push(line.to_string());
            }
        } else {
            preamble.push(line.to_string());
        }
    }

    Ok(Listing {
        preamble,
        headers,
        instrs,
    })
}

/// True for a `//   @cs2 case …` line (switch case ride-along).
fn is_case_line(line: &str) -> bool {
    line.starts_with("//   @cs2 case ")
}

/// Return the header tail when `line` is a `// @cs2 (name|locals|args) …`.
fn header_payload(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("// @cs2 ")?;
    for kw in ["name ", "name=", "locals ", "args "] {
        if rest.starts_with(kw) {
            return Some(rest);
        }
    }
    None
}

/// Split a `// @cs2 <op> [operand]` instruction line into `(op, operand)`.
/// Returns `None` for non-instruction lines (and for the header keywords, which
/// the caller handles first).
fn instruction_payload(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("// @cs2 ")?;
    let rest = rest.trim_end();
    if rest.is_empty() {
        return None;
    }
    Some(match rest.split_once(' ') {
        Some((op, operand)) => (op.to_string(), operand.trim().to_string()),
        None => (rest.to_string(), String::new()),
    })
}

// ── Diagnosis (book diff + rule recognition) ─────────────────────────────────

/// Diagnose all findings for a listing without mutating it. The findings mirror
/// exactly what `apply_fixes` would rewrite, so a clean listing (the committed
/// post-rewrite oracle) reports zero findings.
fn diagnose(listing: &Listing, books: &Books, script: Option<u32>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let instrs = &listing.instrs;

    for (i, instr) in instrs.iter().enumerate() {
        let op = instr.op.as_str();

        // Rule: bare `enum` → `_enum`. The target book resolves `enum` only via
        // an alias (canonical `_enum`); the non-canonical mnemonic trips the
        // assembler's round-trip fidelity gate, so it must be rewritten even
        // though the opcode itself is known. (Mirrors the python `enum→_enum`.)
        if op == "enum"
            && !books.target_has_canonical("enum")
            && books.target_canonical_of("enum").as_deref() == Some("_enum")
        {
            findings.push(Finding {
                instr: instr.index,
                line: instr.line_no,
                rule: "enum_to_underscore",
                severity: Severity::Fixable,
                detail: "`enum` resolves only via alias; canonical target name is `_enum`"
                    .to_string(),
            });
            continue;
        }

        // Rule: `sub` (absent in target book) → negate the preceding constant +
        // `add`. The donor RHS must be a `push_constant_string int:N`.
        if op == "sub" && !books.target_has("sub") {
            let prev_const = i
                .checked_sub(1)
                .and_then(|p| instrs.get(p))
                .and_then(parse_int_constant);
            if prev_const.is_some() {
                findings.push(Finding {
                    instr: instr.index,
                    line: instr.line_no,
                    rule: "sub_to_add",
                    severity: Severity::Fixable,
                    detail: "`sub` has no target opcode; negate the preceding constant and use `add`"
                        .to_string(),
                });
            } else {
                findings.push(Finding {
                    instr: instr.index,
                    line: instr.line_no,
                    rule: "sub_to_add",
                    severity: Severity::Manual,
                    detail: "`sub` not preceded by an int constant; cannot auto-rewrite"
                        .to_string(),
                });
            }
            continue;
        }

        // Rule: relocated shared script `gosub_with_params 7924` → `24924`.
        if op == "gosub_with_params" && instr.operand == "7924" {
            findings.push(Finding {
                instr: instr.index,
                line: instr.line_no,
                rule: "relocate_7924",
                severity: Severity::Fixable,
                detail: "shared cc-icon-draw 7924 relocated to free id 24924".to_string(),
            });
            continue;
        }

        // Rule: db-field constant in the donor `<<12|<<4` packing → `>>4`.
        if op == "push_constant_string"
            && let Some(v) = parse_int_operand(&instr.operand)
            && DB_FIELDS_948.contains(&v)
        {
            findings.push(Finding {
                instr: instr.index,
                line: instr.line_no,
                rule: "db_field_shift",
                severity: Severity::Fixable,
                detail: format!(
                    "donor db-field {v} (table<<12|col<<4) → target {} (table<<8|col)",
                    v >> 4
                ),
            });
            continue;
        }

        // Rule: `db_find` arity — the preceding tuple-index push must become a
        // zero-shift fall-through branch (910 pops only field+key).
        if op == "db_find" {
            let prev_is_tuple_push = i
                .checked_sub(1)
                .and_then(|p| instrs.get(p))
                .is_some_and(|p| p.op == "push_constant_string" && p.operand == "int:0");
            if prev_is_tuple_push {
                findings.push(Finding {
                    instr: instr.index,
                    line: instr.line_no,
                    rule: "db_find_arity",
                    severity: Severity::Fixable,
                    detail: "target `db_find` pops only (field,key); drop the tuple-index push"
                        .to_string(),
                });
                continue;
            }
        }

        // Per-script signature-drift rules.
        if let Some(rule) = signature_drift_finding(script, instr) {
            findings.push(rule);
            continue;
        }

        // Catch-all book diff: any opcode the donor knows but the target does
        // NOT, and which is not covered by a rule above, is a manual finding
        // (an un-ported opcode the splice would mis-assemble). We only flag
        // names the donor book actually recognises to avoid noise on free-text.
        if !op.is_empty()
            && !books.target_has(op)
            && books.donor_has(op)
            && !is_handled_donor_only(op)
        {
            findings.push(Finding {
                instr: instr.index,
                line: instr.line_no,
                rule: "unmapped_opcode",
                severity: Severity::Manual,
                detail: format!("`{op}` exists in the donor book but has no target opcode"),
            });
        }
    }

    // Net-stack-balance pass (sound; only fires on fully-resolvable listings).
    // Catches the class of bug where a rewrite changed the net stack effect — the
    // opcode-availability pass above passes such listings clean, but the live
    // client throws AIOOBE off the corrupted CS2 stack. See `net_stack_findings`.
    findings.extend(net_stack_findings(listing, books));

    findings
}

/// Donor-only opcodes that have a dedicated rewrite rule above (so the
/// catch-all does not double-report them).
fn is_handled_donor_only(op: &str) -> bool {
    matches!(op, "sub" | "enum")
}

/// Per-script signature-drift findings (14611 / 14620 / 14587). These mirror the
/// `if sid == …` blocks in `build-relic-scripts.py`.
fn signature_drift_finding(script: Option<u32>, instr: &Instr) -> Option<Finding> {
    let sid = script?;
    let make = |rule: &'static str, detail: &str| Finding {
        instr: instr.index,
        line: instr.line_no,
        rule,
        severity: Severity::Fixable,
        detail: detail.to_string(),
    };
    match (sid, instr.op.as_str(), instr.operand.as_str()) {
        (14611, "gosub_with_params", "3092") => Some(make(
            "drift_3092",
            "948 script3092 (chronote discount) != 910 script3092; replace call with push 0",
        )),
        (14620, "gosub_with_params", "1858") => Some(make(
            "drift_1858_call",
            "948 script1858 != 910 script1858; replace call with bitcount (stack-shape no-op)",
        )),
        (14587, "gosub_with_params", "13022") => Some(make(
            "drift_13022",
            "948 script13022 != 910 script13022; replace call with bitcount (stack-shape no-op)",
        )),
        _ => None,
    }
}

// ── Operand parsing ──────────────────────────────────────────────────────────

/// If `instr` is `push_constant_string int:N`, return `N`.
fn parse_int_constant(instr: &Instr) -> Option<i64> {
    if instr.op != "push_constant_string" {
        return None;
    }
    parse_int_operand(&instr.operand)
}

/// Parse an `int:N` operand into `N` (signed). Returns `None` for other forms.
fn parse_int_operand(operand: &str) -> Option<i64> {
    operand.strip_prefix("int:")?.trim().parse::<i64>().ok()
}

// ── Fix application (port of apply_rewrites) ─────────────────────────────────

/// Apply the table-driven rewrites to a listing, returning the rewritten
/// listing. This reproduces `build-relic-scripts.py::apply_rewrites` step for
/// step, preserving instruction count so branch targets never move.
fn apply_fixes(listing: &Listing, books: &Books, script: Option<u32>) -> Result<Listing> {
    let mut instrs = listing.instrs.clone();

    // 1) sub → negate preceding constant + add. (rewrite_sub)
    for i in 0..instrs.len() {
        if instrs[i].op == "sub" && !books.target_has("sub") {
            let prev = i
                .checked_sub(1)
                .with_context(|| format!("sub at instr {i} with no predecessor"))?;
            let n = parse_int_constant(&instrs[prev]).with_context(|| {
                format!(
                    "sub at instr {i} not preceded by an int constant: {:?}",
                    instrs[prev].render_primary()
                )
            })?;
            instrs[prev].operand = format!("int:{}", -n);
            instrs[i].op = "add".to_string();
            instrs[i].operand = "0".to_string();
        } else if instrs[i].op == "sub" {
            bail!(
                "instr {i}: unsupported sub form: {:?}",
                instrs[i].render_primary()
            );
        }
    }

    // 2) gosub_with_params 7924 → 24924.
    for instr in &mut instrs {
        if instr.op == "gosub_with_params" && instr.operand == "7924" {
            instr.operand = "24924".to_string();
        }
    }

    // 3) enum → _enum.
    for instr in &mut instrs {
        if instr.op == "enum" {
            instr.op = "_enum".to_string();
            instr.operand = "0".to_string();
        }
    }

    // 4) db-field constants >> 4.
    for instr in &mut instrs {
        if instr.op == "push_constant_string"
            && let Some(v) = parse_int_operand(&instr.operand)
            && DB_FIELDS_948.contains(&v)
        {
            instr.operand = format!("int:{}", v >> 4);
        }
    }

    // 5) db_find arity: preceding tuple-index push → branch <i> fall-through.
    // Only rewrite when the donor tuple-index push is actually present; an
    // already-ported `db_find` (predecessor is the `branch`) is left untouched
    // so `--fix` is idempotent. (`diagnose` likewise only flags the push form.)
    for i in 0..instrs.len() {
        if instrs[i].op == "db_find" && i >= 1 {
            let prev = i - 1;
            if instrs[prev].op == "push_constant_string" && instrs[prev].operand == "int:0" {
                instrs[prev].op = "branch".to_string();
                instrs[prev].operand = i.to_string();
            }
        }
    }

    // 6) per-script signature-drift rewrites.
    if let Some(sid) = script {
        if sid == 14611 {
            rewrite_unique(&mut instrs, "gosub_with_params", "3092", |instr| {
                instr.op = "push_constant_string".to_string();
                instr.operand = "int:0".to_string();
            })?;
        }
        if sid == 14620 {
            rewrite_unique(&mut instrs, "push_constant_string", "int:6", |instr| {
                instr.operand = "int:0".to_string();
            })?;
            rewrite_unique(&mut instrs, "gosub_with_params", "1858", |instr| {
                instr.op = "bitcount".to_string();
                instr.operand = "0".to_string();
            })?;
        }
        if sid == 14587 {
            rewrite_unique(&mut instrs, "gosub_with_params", "13022", |instr| {
                instr.op = "bitcount".to_string();
                instr.operand = "0".to_string();
            })?;
        }
        if sid == 24924 {
            let leftovers = instrs
                .iter()
                .filter(|x| x.op == "gosub_with_params" && x.operand == "24924")
                .count();
            if leftovers > 0 {
                bail!("script 24924 must not call itself");
            }
        }
    }

    Ok(Listing {
        preamble: listing.preamble.clone(),
        headers: listing.headers.clone(),
        instrs,
    })
}

/// Replace AT MOST one instruction matching `(op, operand)` (zero-shift),
/// mirroring `rewrite_line` but tolerating zero matches so `--fix` is idempotent
/// on an already-ported listing (the python builder runs once on donor input
/// where the match is always present; this lint may re-run on ported files).
/// Still errors on more than one match (an ambiguous rewrite).
fn rewrite_unique(
    instrs: &mut [Instr],
    op: &str,
    operand: &str,
    mut edit: impl FnMut(&mut Instr),
) -> Result<()> {
    let hits: Vec<usize> = instrs
        .iter()
        .enumerate()
        .filter(|(_, x)| x.op == op && x.operand == operand)
        .map(|(i, _)| i)
        .collect();
    if hits.len() > 1 {
        bail!(
            "expected at most one `// @cs2 {op} {operand}`, found {}",
            hits.len()
        );
    }
    if let Some(&idx) = hits.first() {
        edit(&mut instrs[idx]);
    }
    Ok(())
}

// ── Rendering ────────────────────────────────────────────────────────────────

/// Re-emit a listing to text: preamble, headers, then each instruction's
/// primary line followed by its ride-along (case / comment) lines.
fn render_listing(listing: &Listing) -> String {
    let mut out = String::new();
    for line in &listing.preamble {
        out.push_str(line);
        out.push('\n');
    }
    for Header(h) in &listing.headers {
        out.push_str(h);
        out.push('\n');
    }
    for instr in &listing.instrs {
        out.push_str(&instr.render_primary());
        out.push('\n');
        for c in &instr.cases {
            out.push_str(c);
            out.push('\n');
        }
    }
    out
}

/// Render the human report across all scripts.
fn render_human(reports: &[ScriptReport], opts: &LintOptions<'_>) -> String {
    let mut out = String::new();
    let total_findings: usize = reports.iter().map(|r| r.findings.len()).sum();
    let manual: usize = reports.iter().map(ScriptReport::manual_count).sum();
    let fixed = reports.iter().filter(|r| r.fixed).count();
    let _ = writeln!(
        out,
        "cs2 lint-splice — {} listing(s) vs book {} (donor {}){}",
        reports.len(),
        opts.target_book,
        opts.donor_book,
        if opts.fix { ", --fix" } else { "" }
    );
    for r in reports {
        if r.findings.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "  {} — {} finding(s){}",
            r.file,
            r.findings.len(),
            if r.fixed { " [FIXED]" } else { "" }
        );
        for f in &r.findings {
            let sev = match f.severity {
                Severity::Fixable => "fixable",
                Severity::Manual => "MANUAL",
            };
            let _ = writeln!(
                out,
                "      instr {:>4} (line {:>4})  {:<18} {:<8} {}",
                f.instr, f.line, f.rule, sev, f.detail
            );
        }
    }
    let _ = writeln!(
        out,
        "summary: {total_findings} finding(s), {manual} manual, {fixed} file(s) rewritten"
    );
    out
}

/// Convenience default data dir for the CLI dispatch.
#[must_use]
pub fn default_data_dir() -> PathBuf {
    PathBuf::from(DEFAULT_DATA_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn books() -> Books {
        Books::load(Path::new(DEFAULT_DATA_DIR), 910, 948).expect("load books")
    }

    #[test]
    fn parses_op_and_operand() {
        let (op, operand) = instruction_payload("// @cs2 push_constant_string int:-1").unwrap();
        assert_eq!(op, "push_constant_string");
        assert_eq!(operand, "int:-1");
        let (op, operand) = instruction_payload("// @cs2 return 0").unwrap();
        assert_eq!(op, "return");
        assert_eq!(operand, "0");
    }

    #[test]
    fn sub_rewrites_to_negate_and_add() {
        let text = "// @cs2 push_constant_string int:5\n// @cs2 sub 0\n// @cs2 return 0\n";
        let listing = parse_listing(text).unwrap();
        let fixed = apply_fixes(&listing, &books(), None).unwrap();
        let out = render_listing(&fixed);
        assert!(out.contains("// @cs2 push_constant_string int:-5"));
        assert!(out.contains("// @cs2 add 0"));
        assert!(!out.contains("// @cs2 sub"));
    }

    #[test]
    fn enum_rewrites_to_underscore() {
        let text = "// @cs2 enum 0\n";
        let listing = parse_listing(text).unwrap();
        let fixed = apply_fixes(&listing, &books(), None).unwrap();
        assert_eq!(render_listing(&fixed), "// @cs2 _enum 0\n");
    }

    #[test]
    fn db_field_shifts_right_four() {
        let text = "// @cs2 push_constant_string int:385024\n";
        let listing = parse_listing(text).unwrap();
        let fixed = apply_fixes(&listing, &books(), None).unwrap();
        // 385024 >> 4 == 24064
        assert_eq!(render_listing(&fixed), "// @cs2 push_constant_string int:24064\n");
    }

    #[test]
    fn db_find_arity_becomes_branch() {
        let text = "// @cs2 push_constant_string int:0\n// @cs2 db_find 0\n";
        let listing = parse_listing(text).unwrap();
        let fixed = apply_fixes(&listing, &books(), None).unwrap();
        let out = render_listing(&fixed);
        assert!(out.contains("// @cs2 branch 1"));
        assert!(out.contains("// @cs2 db_find 0"));
    }

    #[test]
    fn net_stack_flags_int_underflow() {
        // push ONE int, then `add` (pops TWO) → underflow.
        let text = "// @cs2 locals int=0 obj=0 long=0\n// @cs2 args int=0 obj=0 long=0\n\
                    // @cs2 push_constant_string int:1\n// @cs2 add 0\n// @cs2 return 0\n";
        let listing = parse_listing(text).unwrap();
        let findings = net_stack_findings(&listing, &books());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "net_stack_underflow");
    }

    #[test]
    fn net_stack_flags_wrong_type_pop() {
        // a neutralisation that pops the WRONG stack type: an int is on the stack
        // but `pop_long_discard` pops the (empty) long stack → underflow.
        let text = "// @cs2 locals int=1 obj=0 long=0\n// @cs2 args int=1 obj=0 long=0\n\
                    // @cs2 push_int_local 0\n// @cs2 pop_long_discard 0\n// @cs2 return 0\n";
        let listing = parse_listing(text).unwrap();
        let findings = net_stack_findings(&listing, &books());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "net_stack_underflow");
    }

    #[test]
    fn net_stack_clean_balanced_body_has_no_findings() {
        // push int + int, add (pop2 push1), pop into a local: balanced, no finding.
        let text = "// @cs2 locals int=1 obj=0 long=0\n// @cs2 args int=0 obj=0 long=0\n\
                    // @cs2 push_constant_string int:1\n// @cs2 push_constant_string int:2\n\
                    // @cs2 add 0\n// @cs2 pop_int_local 0\n// @cs2 return 0\n";
        let listing = parse_listing(text).unwrap();
        assert!(net_stack_findings(&listing, &books()).is_empty());
    }

    #[test]
    fn net_stack_skips_unresolvable_gosub() {
        // a listing with a gosub is UNVERIFIABLE (callee arity unknown) → no findings
        // even though the local view looks like an underflow.
        let text = "// @cs2 locals int=0 obj=0 long=0\n// @cs2 args int=0 obj=0 long=0\n\
                    // @cs2 add 0\n// @cs2 gosub_with_params 1234\n// @cs2 return 0\n";
        let listing = parse_listing(text).unwrap();
        assert!(net_stack_findings(&listing, &books()).is_empty());
    }

    #[test]
    fn net_stack_string_push_is_obj_not_int() {
        // `push_constant_string str:"x"` pushes an OBJ; popping it as an obj is fine,
        // but popping an int after it must underflow (proves operand-typing works).
        let ok = "// @cs2 locals int=0 obj=1 long=0\n// @cs2 args int=0 obj=0 long=0\n\
                  // @cs2 push_constant_string str:\"x\"\n// @cs2 pop_obj_discard 0\n// @cs2 return 0\n";
        let listing = parse_listing(ok).unwrap();
        assert!(
            net_stack_findings(&listing, &books()).is_empty(),
            "string push then obj pop is balanced"
        );
    }
}
