//! Source parsing: the client `*Prot.java` tables (the source of truth) and
//! the server `*Prot.ts` mirrors, parsed into the schema model in `super::types`.

use super::types::{JavaPacket, JavaParse, TsPacket};
use crate::error::{Context, Result};
use std::path::Path;

// ---------------------------------------------------------------------------
// Java parsing (the source of truth)
// ---------------------------------------------------------------------------

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
