//! Client Java decode-branch parsing + classification, plus the write/read
//! mirror check.
//!
//! Finds every `if (ServerProt.NAME == arg0.packetType) {` decode branch,
//! records its `var2.gXXX()` reads with their nesting depth, and classifies it
//! `simple` (all reads at branch top level) or `complex:<reason>`. The mirror
//! check pairs a tranche candidate's write codecs against its client reads.
//! Mirrors the Python `extract_branches` / `classify_client` / `mirror_ok`.

use super::{ClientBranch, Field, mirror_reads, python_str_list};
use std::collections::BTreeMap;

/// Extract every `if (ServerProt.NAME == arg0.packetType) {` decode branch and
/// classify it. Mirrors the Python `extract_branches`.
pub(super) fn extract_branches(java_src: &str) -> BTreeMap<String, ClientBranch> {
    let lines: Vec<&str> = java_src.lines().collect();
    let mut branches: BTreeMap<String, ClientBranch> = BTreeMap::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(name) = branch_name(lines[i]) else {
            i += 1;
            continue;
        };
        let (body, end) = collect_java_branch(&lines, i);
        branches.insert(name, classify_client(&body));
        i = end;
    }
    branches
}

/// If `line` contains `if (ServerProt.NAME == arg0.packetType) {`, return NAME.
fn branch_name(line: &str) -> Option<String> {
    let idx = line.find("if (ServerProt.")?;
    let after = &line[idx + "if (ServerProt.".len()..];
    let end = after.find(" == arg0.packetType) {")?;
    let name = &after[..end];
    if !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        Some(name.to_owned())
    } else {
        None
    }
}

/// Collect a decode-branch body with per-line nesting depth. `start` is the
/// branch's opening line. Returns `(rows, end_index)` where each row is
/// `(nesting_depth, stripped_line)` (depth 0 = branch top level). Mirrors the
/// Python `collect_java_branch`.
fn collect_java_branch(lines: &[&str], start: usize) -> (Vec<(i32, String)>, usize) {
    let mut out: Vec<(i32, String)> = Vec::new();
    let mut nest: i32 = 1; // the opening `{` of the branch itself
    let mut idx = start + 1;
    while idx < lines.len() {
        let stripped = lines[idx].trim();
        // At branch top level, a line that *starts* with `}` closes the branch.
        if nest == 1 && stripped.starts_with('}') {
            break;
        }
        let opens = i32::try_from(lines[idx].matches('{').count()).unwrap_or(0);
        let closes = i32::try_from(lines[idx].matches('}').count()).unwrap_or(0);
        out.push((nest - 1, stripped.to_owned()));
        nest += opens - closes;
        idx += 1;
    }
    (out, idx)
}

/// Does `line` contain a control structure (`if`/`for`/`while`/`switch` `(`)?
/// Mirrors the Python `CONTROL_RE = \b(if|for|while|switch)\b\s*\(`.
fn has_control(line: &str) -> bool {
    let bytes = line.as_bytes();
    for kw in ["if", "for", "while", "switch"] {
        let mut from = 0;
        while let Some(rel) = line[from..].find(kw) {
            let at = from + rel;
            let before_ok =
                at == 0 || (!bytes[at - 1].is_ascii_alphanumeric() && bytes[at - 1] != b'_');
            let after = at + kw.len();
            let after_byte = bytes.get(after).copied();
            let word_boundary_after =
                after_byte.is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_');
            if before_ok && word_boundary_after {
                // Skip optional whitespace, then require `(`.
                let mut j = after;
                while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                    j += 1;
                }
                if bytes.get(j) == Some(&b'(') {
                    return true;
                }
            }
            from = at + kw.len();
        }
    }
    false
}

/// Find every `var2.gXXX(` read method on a line, in order. Mirrors the Python
/// `READ_RE = var2\.(g[A-Za-z0-9_]+)\(`.
fn find_reads(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut reads: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(rel) = line[from..].find("var2.g") {
        let at = from + rel;
        // The method name starts at the `g`.
        let start = at + "var2.".len();
        let mut j = start;
        // First char already known `g`; consume `[A-Za-z0-9_]+`.
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        // Require a `(` immediately after the identifier.
        if j > start && bytes.get(j) == Some(&b'(') {
            reads.push(line[start..j].to_owned());
        }
        from = at + "var2.".len();
    }
    reads
}

/// Classify a client decode branch. Simple iff every read sits at the branch's
/// top level (no reads under or in control structures). Records the top-level
/// read sequence. Mirrors the Python `classify_client`.
fn classify_client(body: &[(i32, String)]) -> ClientBranch {
    let mut reads: Vec<String> = Vec::new();
    for (depth, line) in body {
        if line.ends_with("else {") || line == "}" {
            continue;
        }
        let line_reads = find_reads(line);
        if line_reads.is_empty() {
            continue;
        }
        if *depth > 0 {
            return rejected("read-under-control");
        }
        if has_control(line) {
            return rejected("read-in-condition");
        }
        reads.extend(line_reads);
    }
    // Detect ternary-gated reads on top-level lines (`cond ? var2.g2() : -1`).
    for (depth, line) in body {
        if *depth == 0 && line.contains('?') && !find_reads(line).is_empty() {
            return rejected("ternary-read");
        }
    }
    if reads.is_empty() {
        return rejected("no-reads");
    }
    ClientBranch {
        reads,
        simple: true,
        reason: String::new(),
    }
}

/// A rejected (complex) client branch carrying only the reason.
fn rejected(reason: &str) -> ClientBranch {
    ClientBranch {
        reads: Vec::new(),
        simple: false,
        reason: reason.to_owned(),
    }
}

/// Check the write-codec / client-read mirror symmetry for a tranche candidate.
/// Returns `Ok(())` when symmetric, else a reason string. Mirrors `mirror_ok`.
pub(super) fn mirror_ok(fields: &[Field], reads: &[String]) -> Result<(), String> {
    if fields.len() != reads.len() {
        return Err(format!("length {} != {}", fields.len(), reads.len()));
    }
    for (idx, (f, r)) in fields.iter().zip(reads).enumerate() {
        let allowed = mirror_reads(&f.codec);
        if !allowed.contains(&r.as_str()) {
            let mut sorted: Vec<&str> = allowed.to_vec();
            sorted.sort_unstable();
            return Err(format!(
                "pos {idx}: {} !~ {r} (allowed {})",
                f.codec,
                python_str_list(&sorted)
            ));
        }
    }
    Ok(())
}
