//! `extract-cs2-registry` — build a canonical, name-keyed CS2 command registry
//! from the Java client's interpreter sources plus the crate's data files, and
//! cross-check every input against the Java truth.
//!
//! The command is read-only over the client tree and `data/` inputs; it writes
//! exactly two files: the registry JSON and a discrepancy report JSON.
//!
//! Example invocation:
//!
//! ```bash
//! cd tools/rs3-cache-rs
//! cargo run --release -- --data-dir data extract-cs2-registry
//! ```

use crate::cache_bail;
use crate::error::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// Resolved options for the `extract-cs2-registry` subcommand.
#[derive(Debug)]
pub struct Cs2RegistryOpts<'a> {
    /// Root of the client checkout (holds `client/src/main/java/...`).
    pub client_root: &'a Path,
    /// Global data directory holding the `opcodes-*.txt` / `stack-effects.txt` inputs.
    pub data_dir: &'a Path,
    /// Optional override for the registry output path.
    pub out_file: Option<&'a Path>,
    /// Optional override for the report output path.
    pub report_file: Option<&'a Path>,
}

const REGISTRY_SCHEMA: &str = "cs2-registry/v3";
const REPORT_SCHEMA: &str = "cs2-registry-report/v1";
const BASE_BUILD: u32 = 910;
const DONOR_BUILD: u32 = 947;

// ---------------------------------------------------------------------------
// Switch (`executeCommand`) parsing
// ---------------------------------------------------------------------------

/// One parsed dispatch statement from a switch case body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dispatch {
    /// `"call"` for an invoked handler, `"unassigned"` for a fall-through throw case.
    pub kind: String,
    /// Qualifying class (e.g. `TwitchCommands`), or `None` for a bare method call.
    pub class: Option<String>,
    /// Handler method name, or `None` for an unassigned case.
    pub method: Option<String>,
    /// The full ordered argument list of the dispatch call, with the state
    /// parameter encoded as the marker string `"$state"` and every other
    /// argument preserved verbatim. Empty for an unassigned case.
    pub args: Vec<String>,
}

impl Dispatch {
    fn unassigned() -> Self {
        Self {
            kind: "unassigned".to_owned(),
            class: None,
            method: None,
            args: Vec::new(),
        }
    }
}

/// Result of parsing the `executeCommand` switch.
#[derive(Debug, Default)]
pub struct SwitchParse {
    /// Dispatch record keyed by opcode id.
    pub dispatches: BTreeMap<u16, Dispatch>,
    /// Ids whose case falls through to the mid-switch `throw`.
    pub unassigned: BTreeSet<u16>,
    /// Total number of `case` labels parsed.
    pub case_count: usize,
    /// Duplicate ids encountered (a single id declared by more than one case label).
    pub duplicate_ids: Vec<u16>,
}

/// Parse the dispatch switch body. Prefers `Cs2Dispatch.execute` when the file
/// is the post-split dispatch class; falls back to `ScriptRunner.executeCommand`.
///
/// `path` is used only for diagnostic messages.
pub fn parse_switch(source: &str, path: &Path) -> Result<SwitchParse> {
    let lines: Vec<&str> = source.lines().collect();

    // Locate the dispatch method signature. Accept either the pre-split
    // `executeCommand(ClientScriptCommand ...)` or the post-split
    // `execute(ClientScriptCommand ...)`.
    let mut method_lines: Vec<usize> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.contains("void executeCommand(ClientScriptCommand")
            || line.contains("void execute(ClientScriptCommand")
        {
            method_lines.push(idx);
        }
    }
    if method_lines.is_empty() {
        cache_bail!(
            "{}: could not locate `void executeCommand(ClientScriptCommand` or \
             `void execute(ClientScriptCommand` signature",
            path.display()
        );
    }
    if method_lines.len() > 1 {
        cache_bail!(
            "{}: found {} dispatch method signatures; expected exactly one",
            path.display(),
            method_lines.len()
        );
    }
    let sig_idx = method_lines[0];

    // Extract the two parameter names from the signature so the switch header
    // (`<first>.index`) and the state argument (`<second>`) are not hardcoded.
    let (command_param, state_param) = parse_dispatch_signature(lines[sig_idx], path, sig_idx + 1)?;

    let switch_idx = sig_idx + 1;
    let switch_line = lines.get(switch_idx).with_context(|| {
        format!(
            "{}:{}: file ends immediately after dispatch method signature",
            path.display(),
            sig_idx + 1
        )
    })?;
    let expected_switch = format!("switch({command_param}.index) {{");
    if switch_line.trim() != expected_switch {
        cache_bail!(
            "{}:{}: expected `{}`, found `{}` (client format changed)",
            path.display(),
            switch_idx + 1,
            expected_switch,
            switch_line.trim()
        );
    }

    let mut parse = SwitchParse::default();
    let mut pending: Vec<u16> = Vec::new();
    let mut seen_ids: BTreeSet<u16> = BTreeSet::new();
    let mut depth: i32 = 1; // the switch's opening brace
    let mut line_no = switch_idx; // 0-based index of the switch line; advances below

    while depth > 0 {
        line_no += 1;
        let raw = lines.get(line_no).with_context(|| {
            format!(
                "{}: reached end of file while parsing executeCommand switch (unbalanced braces)",
                path.display()
            )
        })?;
        let trimmed = raw.trim();
        let one_based = line_no + 1;

        if trimmed.is_empty() || trimmed == "return;" {
            continue;
        }

        let is_label =
            (trimmed.starts_with("case ") && trimmed.ends_with(':')) || trimmed == "default:";

        if !is_label {
            // Only non-label lines may carry braces (verified: no literals contain braces).
            depth += i32::try_from(trimmed.matches('{').count()).unwrap_or(0);
            depth -= i32::try_from(trimmed.matches('}').count()).unwrap_or(0);
            if depth == 0 {
                break;
            }
        }

        if let Some(inner) = trimmed.strip_prefix("case ") {
            let id_text = inner.strip_suffix(':').with_context(|| {
                format!(
                    "{}:{}: malformed case label `{}`",
                    path.display(),
                    one_based,
                    trimmed
                )
            })?;
            let id: u16 = id_text.trim().parse().with_context(|| {
                format!(
                    "{}:{}: could not parse case id from `{}`",
                    path.display(),
                    one_based,
                    trimmed
                )
            })?;
            parse.case_count += 1;
            if !seen_ids.insert(id) {
                parse.duplicate_ids.push(id);
            }
            pending.push(id);
            continue;
        }

        if trimmed == "default:" {
            // The next non-empty line must be the throw.
            let throw_line = next_non_empty(&lines, line_no)?;
            let throw_text = lines[throw_line].trim();
            if throw_text != "throw new RuntimeException();" {
                cache_bail!(
                    "{}:{}: default label not followed by `throw new RuntimeException();` (found `{}`)",
                    path.display(),
                    throw_line + 1,
                    throw_text
                );
            }
            for id in std::mem::take(&mut pending) {
                parse.unassigned.insert(id);
                parse.dispatches.insert(id, Dispatch::unassigned());
            }
            line_no = throw_line; // skip the throw line on the next iteration
            continue;
        }

        // Otherwise this is a dispatch statement body for all pending ids.
        let dispatch = parse_dispatch_statement(trimmed, &state_param, path, one_based)?;
        if pending.is_empty() {
            cache_bail!(
                "{}:{}: dispatch statement `{}` with no pending case label",
                path.display(),
                one_based,
                trimmed
            );
        }
        for id in std::mem::take(&mut pending) {
            parse.dispatches.insert(id, dispatch.clone());
        }
    }

    if !pending.is_empty() {
        cache_bail!(
            "{}: switch ended with {} unresolved case label(s): {:?}",
            path.display(),
            pending.len(),
            pending
        );
    }

    Ok(parse)
}

/// Extract the two parameter names from the dispatch method signature.
///
/// Accepts both pre-split (`executeCommand`) and post-split (`execute`) forms;
/// the parameters are `(ClientScriptCommand <command>, ClientScriptState <state>)`.
/// Returns `(command_param, state_param)`.
fn parse_dispatch_signature(
    sig_line: &str,
    path: &Path,
    line_no: usize,
) -> Result<(String, String)> {
    let open = sig_line.find('(').with_context(|| {
        format!(
            "{}:{}: dispatch signature missing `(`: `{}`",
            path.display(),
            line_no,
            sig_line.trim()
        )
    })?;
    let after = &sig_line[open + 1..];
    let close = after.find(')').with_context(|| {
        format!(
            "{}:{}: dispatch signature missing `)`: `{}`",
            path.display(),
            line_no,
            sig_line.trim()
        )
    })?;
    let params = &after[..close];
    let mut iter = params.split(',');
    let first = iter.next().with_context(|| {
        format!(
            "{}:{}: dispatch signature has no parameters: `{}`",
            path.display(),
            line_no,
            sig_line.trim()
        )
    })?;
    let second = iter.next().with_context(|| {
        format!(
            "{}:{}: dispatch signature has fewer than two parameters: `{}`",
            path.display(),
            line_no,
            sig_line.trim()
        )
    })?;
    let command_param = param_name(first, "ClientScriptCommand", path, line_no)?;
    let state_param = param_name(second, "ClientScriptState", path, line_no)?;
    Ok((command_param, state_param))
}

/// Pull the identifier (last whitespace-separated token) out of `<Type> <name>`,
/// asserting the declared type matches `expected_type`.
fn param_name(param: &str, expected_type: &str, path: &Path, line_no: usize) -> Result<String> {
    let trimmed = param.trim();
    let (ty, name) = trimmed.rsplit_once(char::is_whitespace).with_context(|| {
        format!(
            "{}:{}: malformed dispatch parameter `{}`",
            path.display(),
            line_no,
            trimmed
        )
    })?;
    if ty.trim() != expected_type {
        cache_bail!(
            "{}:{}: expected dispatch parameter of type `{}`, found `{}`",
            path.display(),
            line_no,
            expected_type,
            ty.trim()
        );
    }
    Ok(name.trim().to_owned())
}

/// Return the index of the next non-empty line strictly after `from`.
fn next_non_empty(lines: &[&str], from: usize) -> Result<usize> {
    let mut idx = from + 1;
    while let Some(line) = lines.get(idx) {
        if !line.trim().is_empty() {
            return Ok(idx);
        }
        idx += 1;
    }
    cache_bail!("reached end of file looking for non-empty line after line {from}");
}

/// Parse `[<Class>.]<method>(<args>);` into a [`Dispatch`].
///
/// `state_param` is the name of the interpreter-state argument (today `arg1`,
/// after the split `state`); each occurrence is replaced by the `$state` marker
/// in the recorded full argument list.
fn parse_dispatch_statement(
    stmt: &str,
    state_param: &str,
    path: &Path,
    line_no: usize,
) -> Result<Dispatch> {
    let body = stmt.strip_suffix(';').with_context(|| {
        format!(
            "{}:{}: dispatch statement missing trailing `;`: `{}`",
            path.display(),
            line_no,
            stmt
        )
    })?;

    let open = body.find('(').with_context(|| {
        format!(
            "{}:{}: dispatch statement missing `(`: `{}`",
            path.display(),
            line_no,
            stmt
        )
    })?;
    let callee = &body[..open];
    let args_part = body[open + 1..].strip_suffix(')').with_context(|| {
        format!(
            "{}:{}: dispatch statement missing closing `)`: `{}`",
            path.display(),
            line_no,
            stmt
        )
    })?;

    let (class, method) = match callee.rsplit_once('.') {
        Some((class, method)) => (Some(class.trim().to_owned()), method.trim().to_owned()),
        None => (None, callee.trim().to_owned()),
    };
    if method.is_empty() {
        cache_bail!(
            "{}:{}: empty method name in dispatch statement `{}`",
            path.display(),
            line_no,
            stmt
        );
    }

    let mut args: Vec<String> = Vec::new();
    let mut state_count = 0_u32;
    if !args_part.trim().is_empty() {
        for arg in args_part.split(", ") {
            let arg = arg.trim();
            if arg == state_param {
                state_count += 1;
                args.push("$state".to_owned());
                continue;
            }
            if !is_valid_extra_arg(arg) {
                cache_bail!(
                    "{}:{}: unexpected dispatch argument `{}` in `{}`",
                    path.display(),
                    line_no,
                    arg,
                    stmt
                );
            }
            args.push(arg.to_owned());
        }
    }
    if state_count != 1 {
        cache_bail!(
            "{}:{}: dispatch statement must contain exactly one `{}`, found {}: `{}`",
            path.display(),
            line_no,
            state_param,
            state_count,
            stmt
        );
    }

    Ok(Dispatch {
        kind: "call".to_owned(),
        class,
        method: Some(method),
        args,
    })
}

/// A non-`arg1` argument must be `true`, `false`, or an optionally-cast integer literal.
fn is_valid_extra_arg(arg: &str) -> bool {
    if arg == "true" || arg == "false" {
        return true;
    }
    let literal = match arg.strip_prefix("(short)") {
        Some(rest) => rest.trim(),
        None => arg,
    };
    let digits = literal.strip_prefix('-').unwrap_or(literal);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// `ClientScriptCommand.java` (enum) parsing
// ---------------------------------------------------------------------------

/// One parsed `ClientScriptCommand` static field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumField {
    /// Field identifier (mixed case, preserved verbatim).
    pub field: String,
    /// 910 opcode id from the constructor.
    pub id: u16,
    /// `true` when constructed with a `true` large-operand flag.
    pub large_operand: bool,
    /// `@ObfuscatedName` value when present directly above the field.
    pub obf: Option<String>,
    /// Zero-based position of this field's declaration in the source file.
    pub enum_order: u32,
    /// `true` when the constructor was written with an explicit second argument
    /// (`new ClientScriptCommand(id, true|false)`) rather than the 1-arg form.
    /// Needed for byte-faithful regeneration: `(id, false)` and `(id)` are
    /// semantically equal but textually distinct.
    pub ctor_explicit_operand: bool,
}

/// Result of parsing `ClientScriptCommand.java`.
#[derive(Debug, Default)]
pub struct EnumParse {
    /// Fields in declaration order.
    pub fields: Vec<EnumField>,
    /// Duplicate ids (an id declared by more than one field).
    pub duplicate_ids: Vec<u16>,
    /// Duplicate field names.
    pub duplicate_names: Vec<String>,
}

/// Parse `ClientScriptCommand.java` static command fields.
pub fn parse_enum(source: &str, path: &Path) -> Result<EnumParse> {
    const FIELD_PREFIX: &str = "public static final ClientScriptCommand ";

    let mut parse = EnumParse::default();
    let mut pending_obf: Option<String> = None;
    let mut seen_ids: BTreeSet<u16> = BTreeSet::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut enum_order: u32 = 0;

    for (idx, raw) in source.lines().enumerate() {
        let trimmed = raw.trim();
        let one_based = idx + 1;

        if trimmed.is_empty() {
            continue;
        }

        if let Some(obf) = parse_obfuscated_name(trimmed) {
            pending_obf = Some(obf);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(FIELD_PREFIX) {
            let field = parse_enum_field(rest, pending_obf.take(), enum_order, path, one_based)?;
            enum_order += 1;
            if !seen_ids.insert(field.id) {
                parse.duplicate_ids.push(field.id);
            }
            if !seen_names.insert(field.field.clone()) {
                parse.duplicate_names.push(field.field.clone());
            }
            parse.fields.push(field);
            continue;
        }

        // Any other non-blank line clears the annotation stash.
        pending_obf = None;
    }

    Ok(parse)
}

/// Extract the quoted value of a bare `@ObfuscatedName("...")` line.
fn parse_obfuscated_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("@ObfuscatedName(\"")?;
    let value = rest.strip_suffix("\")")?;
    Some(value.to_owned())
}

/// Parse the field tail after `public static final ClientScriptCommand `.
fn parse_enum_field(
    rest: &str,
    obf: Option<String>,
    enum_order: u32,
    path: &Path,
    line_no: usize,
) -> Result<EnumField> {
    let (field, ctor) = rest
        .split_once(" = new ClientScriptCommand(")
        .with_context(|| {
            format!(
                "{}:{}: malformed ClientScriptCommand field declaration: `{}`",
                path.display(),
                line_no,
                rest
            )
        })?;
    let args = ctor.strip_suffix(");").with_context(|| {
        format!(
            "{}:{}: ClientScriptCommand constructor missing `);`: `{}`",
            path.display(),
            line_no,
            rest
        )
    })?;

    let mut parts = args.split(',').map(str::trim);
    let id_text = parts.next().with_context(|| {
        format!(
            "{}:{}: ClientScriptCommand constructor has no id argument: `{}`",
            path.display(),
            line_no,
            rest
        )
    })?;
    let id: u16 = id_text.parse().with_context(|| {
        format!(
            "{}:{}: could not parse ClientScriptCommand id `{}`",
            path.display(),
            line_no,
            id_text
        )
    })?;

    let (large_operand, ctor_explicit_operand) = match parts.next() {
        None => (false, false),
        Some("true") => (true, true),
        Some("false") => (false, true),
        Some(other) => cache_bail!(
            "{}:{}: unexpected large-operand flag `{}` in `{}`",
            path.display(),
            line_no,
            other,
            rest
        ),
    };
    if parts.next().is_some() {
        cache_bail!(
            "{}:{}: ClientScriptCommand constructor has too many arguments: `{}`",
            path.display(),
            line_no,
            rest
        );
    }

    Ok(EnumField {
        field: field.trim().to_owned(),
        id,
        large_operand,
        obf,
        enum_order,
        ctor_explicit_operand,
    })
}

// ---------------------------------------------------------------------------
// Data-file parsing
// ---------------------------------------------------------------------------

/// Stack-effect tuple for one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StackEffect {
    pub int_pops: u32,
    pub obj_pops: u32,
    pub long_pops: u32,
    pub int_pushes: u32,
    pub obj_pushes: u32,
    pub long_pushes: u32,
}

/// Read a text file with a clear I/O context message.
fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

/// Parse `opcodes-910.txt` (`name,id`) into both directions.
fn parse_opcodes_910(path: &Path) -> Result<(BTreeMap<u16, String>, Vec<String>)> {
    let text = read_text(path)?;
    let mut by_id: BTreeMap<u16, String> = BTreeMap::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut duplicate_names: Vec<String> = Vec::new();

    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let (name, id_text) = line.split_once(',').with_context(|| {
            format!(
                "{}:{}: expected `name,id`, found `{}`",
                path.display(),
                idx + 1,
                line
            )
        })?;
        let id: u16 = id_text.trim().parse().with_context(|| {
            format!(
                "{}:{}: could not parse id from `{}`",
                path.display(),
                idx + 1,
                line
            )
        })?;
        let name = name.trim().to_owned();
        if !seen_names.insert(name.clone()) {
            duplicate_names.push(name.clone());
        }
        by_id.insert(id, name);
    }
    Ok((by_id, duplicate_names))
}

/// Parse `opcodes-large-910.txt` (`id,flag`).
fn parse_opcodes_large_910(path: &Path) -> Result<BTreeMap<u16, bool>> {
    let text = read_text(path)?;
    let mut by_id: BTreeMap<u16, bool> = BTreeMap::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let (id_text, flag_text) = line.split_once(',').with_context(|| {
            format!(
                "{}:{}: expected `id,flag`, found `{}`",
                path.display(),
                idx + 1,
                line
            )
        })?;
        let id: u16 = id_text.trim().parse().with_context(|| {
            format!(
                "{}:{}: could not parse id from `{}`",
                path.display(),
                idx + 1,
                line
            )
        })?;
        let flag = match flag_text.trim() {
            "0" => false,
            "1" => true,
            other => cache_bail!(
                "{}:{}: large-flag must be 0 or 1, found `{}`",
                path.display(),
                idx + 1,
                other
            ),
        };
        by_id.insert(id, flag);
    }
    Ok(by_id)
}

/// Parse `opcodes-947.txt` (`name,id[,version_gate]`), keeping every row.
fn parse_opcodes_947(path: &Path) -> Result<HashMap<String, u32>> {
    let text = read_text(path)?;
    let mut by_name: HashMap<String, u32> = HashMap::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let mut parts = line.split(',');
        let name = parts
            .next()
            .with_context(|| format!("{}:{}: empty 947 opcode row", path.display(), idx + 1))?;
        let id_text = parts.next().with_context(|| {
            format!(
                "{}:{}: 947 opcode row missing id: `{}`",
                path.display(),
                idx + 1,
                line
            )
        })?;
        let id: u32 = id_text.trim().parse().with_context(|| {
            format!(
                "{}:{}: could not parse 947 id from `{}`",
                path.display(),
                idx + 1,
                line
            )
        })?;
        // An optional third column (version gate) is tolerated and ignored here.
        if let Some(gate_text) = parts.next() {
            let _gate: u32 = gate_text.trim().parse().with_context(|| {
                format!(
                    "{}:{}: could not parse 947 version gate from `{}`",
                    path.display(),
                    idx + 1,
                    line
                )
            })?;
        }
        by_name.insert(name.trim().to_owned(), id);
    }
    Ok(by_name)
}

/// Parse `opcodes-948.txt` (`name,id[,version_gate]`), keeping every row.
///
/// Identical shape to `opcodes-947.txt`; kept as its own function so the
/// extractor's intent (948 is the active donor build) is explicit.
fn parse_opcodes_948(path: &Path) -> Result<HashMap<String, u32>> {
    parse_opcodes_947(path)
}

/// Parse `stack-effects.txt` (space-separated 7-tuple).
fn parse_stack_effects(path: &Path) -> Result<HashMap<String, StackEffect>> {
    let text = read_text(path)?;
    let mut by_name: HashMap<String, StackEffect> = HashMap::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts
            .next()
            .with_context(|| format!("{}:{}: empty stack-effect row", path.display(), idx + 1))?;
        let mut next_num = |label: &str| -> Result<u32> {
            let token = parts.next().with_context(|| {
                format!(
                    "{}:{}: stack-effect row missing {} field: `{}`",
                    path.display(),
                    idx + 1,
                    label,
                    line
                )
            })?;
            token.parse::<u32>().with_context(|| {
                format!(
                    "{}:{}: could not parse stack-effect {} `{}`",
                    path.display(),
                    idx + 1,
                    label,
                    token
                )
            })
        };
        let effect = StackEffect {
            int_pops: next_num("int_pops")?,
            obj_pops: next_num("obj_pops")?,
            long_pops: next_num("long_pops")?,
            int_pushes: next_num("int_pushes")?,
            obj_pushes: next_num("obj_pushes")?,
            long_pushes: next_num("long_pushes")?,
        };
        by_name.insert(name.to_owned(), effect);
    }
    Ok(by_name)
}

/// Parse `opcode-aliases-910.txt` (`alt_name,canonical_910_name`) into a
/// canonical→sorted-alts reverse map.
fn parse_aliases_910(path: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let text = read_text(path)?;
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let (alt, canonical) = line.split_once(',').with_context(|| {
            format!(
                "{}:{}: expected `alt_name,canonical_name`, found `{}`",
                path.display(),
                idx + 1,
                line
            )
        })?;
        reverse
            .entry(canonical.trim().to_owned())
            .or_default()
            .insert(alt.trim().to_owned());
    }
    Ok(reverse
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}

// ---------------------------------------------------------------------------
// Registry assembly
// ---------------------------------------------------------------------------

/// One command row in the registry output.
#[derive(Debug, Serialize)]
struct Command {
    name: String,
    id_910: u16,
    id_947: Option<u32>,
    id_948: Option<u32>,
    /// Exact name the 948 row carried in `opcodes-948.txt`. Equals `name` except
    /// when the match came through an alias (e.g. `enum` for canonical `_enum`);
    /// `None` when `id_948` is null. Lets the registry-backed 948 book key by the
    /// donor-file name byte-for-byte (§3.1).
    id_948_name: Option<String>,
    enum_field: Option<String>,
    enum_order: Option<u32>,
    obf: Option<String>,
    large_operand: bool,
    ctor_explicit_operand: bool,
    dispatch: Dispatch,
    stack: Option<StackEffect>,
    aliases: Vec<String>,
}

/// Top-level registry document.
#[derive(Debug, Serialize)]
struct Registry {
    schema: &'static str,
    base_build: u32,
    donor_build: u32,
    sources: Sources,
    commands: Vec<Command>,
}

/// Resolved source paths recorded in the registry.
#[derive(Debug, Serialize)]
struct Sources {
    script_runner: String,
    command_enum: String,
    opcodes_910: String,
    opcodes_947: String,
    opcodes_948: String,
    opcodes_large_910: String,
    stack_effects: String,
    aliases_910: String,
}

/// One discrepancy-report finding.
#[derive(Debug, Clone, Serialize)]
struct Finding {
    check: String,
    severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id_910: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    message: String,
}

impl Finding {
    fn new(
        check: &str,
        severity: &str,
        id: Option<u16>,
        name: Option<String>,
        msg: String,
    ) -> Self {
        Self {
            check: check.to_owned(),
            severity: severity.to_owned(),
            id_910: id,
            name,
            message: msg,
        }
    }
}

/// Report summary counts.
#[derive(Debug, Serialize)]
struct ReportSummary {
    errors: usize,
    warnings: usize,
    infos: usize,
}

/// Top-level report document.
#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    summary: ReportSummary,
    findings: Vec<Finding>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the `extract-cs2-registry` subcommand.
pub fn run(opts: &Cs2RegistryOpts<'_>) -> Result<()> {
    let script_runner_path = opts
        .client_root
        .join("client/src/main/java/rs2/client/clientscript/ScriptRunner.java");
    let command_enum_path = opts
        .client_root
        .join("client/src/main/java/com/jagex/game/script/ClientScriptCommand.java");
    let opcodes_910_path = opts.data_dir.join("opcodes-910.txt");
    let opcodes_947_path = opts.data_dir.join("opcodes-947.txt");
    let opcodes_948_path = opts.data_dir.join("opcodes-948.txt");
    let opcodes_large_910_path = opts.data_dir.join("opcodes-large-910.txt");
    let stack_effects_path = opts.data_dir.join("stack-effects.txt");
    let aliases_910_path = opts.data_dir.join("opcode-aliases-910.txt");

    // Dispatch source: after the Stage 2 split the switch lives in
    // `Cs2Dispatch.java#execute`; before the split it is
    // `ScriptRunner.java#executeCommand`. Prefer the former when present.
    let dispatch_path = opts
        .client_root
        .join("client/src/main/java/rs2/client/clientscript/Cs2Dispatch.java");
    let switch_path = if dispatch_path.is_file() {
        dispatch_path
    } else {
        script_runner_path.clone()
    };
    let switch_src = read_text(&switch_path)?;
    let switch = parse_switch(&switch_src, &switch_path)?;

    let enum_src = read_text(&command_enum_path)?;
    let enums = parse_enum(&enum_src, &command_enum_path)?;

    let (names_910, dup_names_910) = parse_opcodes_910(&opcodes_910_path)?;
    let large_910 = parse_opcodes_large_910(&opcodes_large_910_path)?;
    let ids_947 = parse_opcodes_947(&opcodes_947_path)?;
    let ids_948 = parse_opcodes_948(&opcodes_948_path)?;
    let stack_effects = parse_stack_effects(&stack_effects_path)?;
    let aliases = parse_aliases_910(&aliases_910_path)?;

    let enum_by_id: HashMap<u16, &EnumField> = enums.fields.iter().map(|f| (f.id, f)).collect();

    let mut findings: Vec<Finding> = Vec::new();
    let mut commands: Vec<Command> = Vec::new();

    // Build one command per switch case id.
    for (&id, dispatch) in &switch.dispatches {
        let (name, synthesized) = match names_910.get(&id) {
            Some(name) => (name.clone(), false),
            None => (format!("unnamed_{id}"), true),
        };
        if synthesized {
            findings.push(Finding::new(
                "C1",
                "error",
                Some(id),
                None,
                format!("switch case id {id} has no opcodes-910.txt row"),
            ));
            findings.push(Finding::new(
                "C2b",
                "error",
                Some(id),
                Some(name.clone()),
                format!("name synthesized as unnamed_{id} (no opcodes-910.txt row)"),
            ));
        }

        let donor_id = resolve_947(&name, &ids_947, &aliases);
        let donor_948 = resolve_948(&name, &ids_948, &aliases);
        let (donor_id_948, donor_948_name) = match donor_948 {
            Some((did, dname)) => (Some(did), Some(dname)),
            None => (None, None),
        };

        let enum_field = enum_by_id.get(&id);
        let (enum_field_name, enum_order, obf, large_operand, ctor_explicit_operand) =
            if let Some(field) = enum_field {
                (
                    Some(field.field.clone()),
                    Some(field.enum_order),
                    field.obf.clone(),
                    field.large_operand,
                    field.ctor_explicit_operand,
                )
            } else {
                if !switch.unassigned.contains(&id) {
                    findings.push(Finding::new(
                        "C3",
                        "warning",
                        Some(id),
                        Some(name.clone()),
                        format!("switch case id {id} has no ClientScriptCommand enum constant"),
                    ));
                }
                (None, None, None, false, false)
            };

        let stack = stack_effects.get(&name).copied();
        if stack.is_none() && !switch.unassigned.contains(&id) {
            findings.push(Finding::new(
                "C5",
                "info",
                Some(id),
                Some(name.clone()),
                format!("registry name `{name}` has no stack-effects.txt row"),
            ));
        }

        if donor_id.is_none() {
            findings.push(Finding::new(
                "C7",
                "info",
                Some(id),
                Some(name.clone()),
                format!("registry name `{name}` has id_947 == null (removed/renamed in 947)"),
            ));
        }

        if donor_id_948.is_none() {
            findings.push(Finding::new(
                "C7b",
                "info",
                Some(id),
                Some(name.clone()),
                format!("registry name `{name}` has id_948 == null (removed/renamed in 948)"),
            ));
        }

        let command_aliases = aliases.get(&name).cloned().unwrap_or_default();

        commands.push(Command {
            name,
            id_910: id,
            id_947: donor_id,
            id_948: donor_id_948,
            id_948_name: donor_948_name,
            enum_field: enum_field_name,
            enum_order,
            obf,
            large_operand,
            ctor_explicit_operand,
            dispatch: dispatch.clone(),
            stack,
            aliases: command_aliases,
        });
    }

    commands.sort_by_key(|c| c.id_910);

    // ---- Cross-checks that scan inputs rather than the assembled registry ----
    let registry_ids: BTreeSet<u16> = switch.dispatches.keys().copied().collect();
    let registry_names: BTreeSet<String> = commands.iter().map(|c| c.name.clone()).collect();

    // C2: opcodes-910 row id with no switch case.
    for (&id, name) in &names_910 {
        if !registry_ids.contains(&id) {
            findings.push(Finding::new(
                "C2",
                "error",
                Some(id),
                Some(name.clone()),
                format!("opcodes-910.txt id {id} (`{name}`) has no switch case"),
            ));
        }
    }

    // C3b: enum constant id with no switch case.
    for field in &enums.fields {
        if !registry_ids.contains(&field.id) {
            findings.push(Finding::new(
                "C3b",
                "warning",
                Some(field.id),
                Some(field.field.clone()),
                format!(
                    "ClientScriptCommand `{}` (id {}) has no switch case",
                    field.field, field.id
                ),
            ));
        }
    }

    // C4: enum large flag vs opcodes-large-910 flag.
    for field in &enums.fields {
        if let Some(&large_flag) = large_910.get(&field.id)
            && field.large_operand != large_flag
        {
            findings.push(Finding::new(
                "C4",
                "error",
                Some(field.id),
                Some(field.field.clone()),
                format!(
                    "enum isLargeOperand={} != opcodes-large-910 flag={} for id {}",
                    field.large_operand, large_flag, field.id
                ),
            ));
        }
    }

    // C4b: opcodes-large-910 id set vs switch id set (both directions).
    let large_ids: BTreeSet<u16> = large_910.keys().copied().collect();
    for id in large_ids.difference(&registry_ids) {
        findings.push(Finding::new(
            "C4b",
            "error",
            Some(*id),
            None,
            format!("opcodes-large-910 id {id} has no switch case"),
        ));
    }
    for id in registry_ids.difference(&large_ids) {
        findings.push(Finding::new(
            "C4b",
            "error",
            Some(*id),
            None,
            format!("switch id {id} missing from opcodes-large-910"),
        ));
    }

    // C5b: stack-effects command name not in registry (after alias resolution).
    for name in stack_effects.keys() {
        if registry_names.contains(name) || alias_matches_registry(name, &aliases, &registry_names)
        {
            continue;
        }
        findings.push(Finding::new(
            "C5b",
            "warning",
            None,
            Some(name.clone()),
            format!("stack-effects.txt command `{name}` not in registry"),
        ));
    }

    // C6: opcodes-947 name unmatched in registry (after alias resolution).
    for name in ids_947.keys() {
        if registry_names.contains(name) || alias_matches_registry(name, &aliases, &registry_names)
        {
            continue;
        }
        findings.push(Finding::new(
            "C6",
            "info",
            None,
            Some(name.clone()),
            format!("opcodes-947.txt name `{name}` unmatched in registry (likely new 947 command)"),
        ));
    }

    // C6b: opcodes-948 name unmatched in registry (after alias resolution).
    for name in ids_948.keys() {
        if registry_names.contains(name) || alias_matches_registry(name, &aliases, &registry_names)
        {
            continue;
        }
        findings.push(Finding::new(
            "C6b",
            "info",
            None,
            Some(name.clone()),
            format!("opcodes-948.txt name `{name}` unmatched in registry (likely new 948 command)"),
        ));
    }

    // C8: duplicates within any single source.
    for &id in &switch.duplicate_ids {
        findings.push(Finding::new(
            "C8",
            "error",
            Some(id),
            None,
            format!("duplicate case id {id} in switch"),
        ));
    }
    for &id in &enums.duplicate_ids {
        findings.push(Finding::new(
            "C8",
            "error",
            Some(id),
            None,
            format!("duplicate id {id} in ClientScriptCommand fields"),
        ));
    }
    for name in &enums.duplicate_names {
        findings.push(Finding::new(
            "C8",
            "error",
            None,
            Some(name.clone()),
            format!("duplicate field name `{name}` in ClientScriptCommand"),
        ));
    }
    for name in &dup_names_910 {
        findings.push(Finding::new(
            "C8",
            "error",
            None,
            Some(name.clone()),
            format!("duplicate name `{name}` in opcodes-910.txt"),
        ));
    }

    // C9: unassigned ids and contiguity gaps.
    for &id in &switch.unassigned {
        findings.push(Finding::new(
            "C9",
            "info",
            Some(id),
            None,
            format!("unassigned (fall-through) id {id}"),
        ));
    }
    if let (Some(&min), Some(&max)) = (registry_ids.iter().next(), registry_ids.iter().next_back())
    {
        for id in min..=max {
            if !registry_ids.contains(&id) {
                findings.push(Finding::new(
                    "C9",
                    "info",
                    Some(id),
                    None,
                    format!("id-range gap: id {id} has no switch case"),
                ));
            }
        }
    }

    sort_findings(&mut findings);

    // ---- Write outputs ----
    let default_out = opts.data_dir.join("cs2").join("registry-910.json");
    let out_file: PathBuf = opts
        .out_file
        .map_or(default_out, std::path::Path::to_path_buf);
    let report_file: PathBuf = if let Some(p) = opts.report_file {
        p.to_path_buf()
    } else {
        let dir = out_file.parent().unwrap_or_else(|| Path::new("."));
        dir.join("registry-910.report.json")
    };

    let registry = Registry {
        schema: REGISTRY_SCHEMA,
        base_build: BASE_BUILD,
        donor_build: DONOR_BUILD,
        sources: Sources {
            script_runner: script_runner_path.display().to_string(),
            command_enum: command_enum_path.display().to_string(),
            opcodes_910: opcodes_910_path.display().to_string(),
            opcodes_947: opcodes_947_path.display().to_string(),
            opcodes_948: opcodes_948_path.display().to_string(),
            opcodes_large_910: opcodes_large_910_path.display().to_string(),
            stack_effects: stack_effects_path.display().to_string(),
            aliases_910: aliases_910_path.display().to_string(),
        },
        commands,
    };

    let summary = ReportSummary {
        errors: findings.iter().filter(|f| f.severity == "error").count(),
        warnings: findings.iter().filter(|f| f.severity == "warning").count(),
        infos: findings.iter().filter(|f| f.severity == "info").count(),
    };
    let report = Report {
        schema: REPORT_SCHEMA,
        summary,
        findings,
    };

    if let Some(dir) = out_file.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create output dir {}", dir.display()))?;
    }
    if let Some(dir) = report_file.parent() {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create report dir {}", dir.display()))?;
    }

    write_pretty_json(&out_file, &registry)?;
    write_pretty_json(&report_file, &report)?;

    print_summary(&report, switch.case_count, &out_file, &report_file);

    Ok(())
}

/// Resolve a registry name to its 947 opcode id, retrying via aliases on a miss.
fn resolve_947(
    name: &str,
    ids_947: &HashMap<String, u32>,
    aliases: &BTreeMap<String, Vec<String>>,
) -> Option<u32> {
    if let Some(&id) = ids_947.get(name) {
        return Some(id);
    }
    if let Some(alts) = aliases.get(name) {
        for alt in alts {
            if let Some(&id) = ids_947.get(alt) {
                return Some(id);
            }
        }
    }
    None
}

/// Resolve a registry name to its 948 opcode id, retrying via aliases on a miss.
///
/// Returns `(id, donor_name)` where `donor_name` is the exact name the matching
/// row carried in `opcodes-948.txt` — the canonical name on a direct hit, or the
/// alias on an alias hit. Recording the donor name lets the registry-backed 948
/// `OpcodeBook` key the command exactly as the txt-driven book did (§3.1 caveat).
fn resolve_948(
    name: &str,
    ids_948: &HashMap<String, u32>,
    aliases: &BTreeMap<String, Vec<String>>,
) -> Option<(u32, String)> {
    if let Some(&id) = ids_948.get(name) {
        return Some((id, name.to_owned()));
    }
    if let Some(alts) = aliases.get(name) {
        for alt in alts {
            if let Some(&id) = ids_948.get(alt) {
                return Some((id, alt.clone()));
            }
        }
    }
    None
}

/// Return `true` when `name`, or one of its registered aliases' canonical sides, is in the registry.
fn alias_matches_registry(
    name: &str,
    aliases: &BTreeMap<String, Vec<String>>,
    registry_names: &BTreeSet<String>,
) -> bool {
    // `aliases` maps canonical_910_name -> [alt_name]. A foreign `name` matches
    // the registry when it is an alt of some canonical name that is in the registry.
    for (canonical, alts) in aliases {
        if alts.iter().any(|alt| alt == name) && registry_names.contains(canonical) {
            return true;
        }
    }
    false
}

/// Sort findings deterministically by (check, id, name, message).
fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        a.check
            .cmp(&b.check)
            .then(a.id_910.cmp(&b.id_910))
            .then(a.name.cmp(&b.name))
            .then(a.message.cmp(&b.message))
    });
}

/// Serialize `value` as pretty JSON to `path`.
fn write_pretty_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    std::io::Write::write_all(&mut writer, b"\n")
        .with_context(|| format!("failed to finalize {}", path.display()))?;
    Ok(())
}

/// Print the human summary to stdout: one line per check then up to 20 findings each.
fn print_summary(report: &Report, case_count: usize, out_file: &Path, report_file: &Path) {
    let order = [
        "C1", "C2", "C2b", "C3", "C3b", "C4", "C4b", "C5", "C5b", "C6", "C6b", "C7", "C7b", "C8",
        "C9",
    ];
    println!("extract-cs2-registry: {case_count} switch cases");
    println!("registry: {}", out_file.display());
    println!("report:   {}", report_file.display());
    println!(
        "summary: errors={} warnings={} infos={}",
        report.summary.errors, report.summary.warnings, report.summary.infos
    );
    for check in order {
        let matching: Vec<&Finding> = report
            .findings
            .iter()
            .filter(|f| f.check == check)
            .collect();
        println!("{check}: {} finding(s)", matching.len());
        for finding in matching.iter().take(20) {
            println!("  - {}", finding.message);
        }
        if matching.len() > 20 {
            println!("  ... and {} more", matching.len() - 20);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Dispatch, parse_aliases_910, parse_enum, parse_opcodes_910, parse_opcodes_947,
        parse_opcodes_948, parse_opcodes_large_910, parse_stack_effects, parse_switch, resolve_947,
        resolve_948,
    };
    use crate::error::Result;
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    const SWITCH_FIXTURE: &str = r"
	public static final void executeCommand(ClientScriptCommand arg0, ClientScriptState arg1) throws CameraException, VarBitOverflowException {
		switch(arg0.index) {
			case 0:
				telemetry_get_group_count(arg1);
				return;
			case 1:
				TwitchCommands.ttv_setdebugoutput(arg1);
				return;
			case 2:
				if_sendto(true, arg1);
				return;
			case 3:
			case 4:
			case 5:
			default:
				throw new RuntimeException();
			case 6:
				push_array(arg1, true, false);
				return;
			case 7:
				QuickChatDynamicCommand.add(arg1, (short) -32146);
				return;
			case 8:
				cc_getparentlayer(arg1);
		}
	}

	public static final void other(ClientScriptState arg0) {
		switch (arg0.kind) {
			case 0:
				return;
		}
	}
";

    #[test]
    fn switch_parses_all_six_shapes() -> Result<()> {
        let parse = parse_switch(SWITCH_FIXTURE, Path::new("fixture.java"))?;
        assert_eq!(parse.case_count, 9);

        // Simple
        assert_eq!(
            parse.dispatches[&0],
            Dispatch {
                kind: "call".to_owned(),
                class: None,
                method: Some("telemetry_get_group_count".to_owned()),
                args: vec!["$state".to_owned()],
            }
        );
        // Qualified
        assert_eq!(
            parse.dispatches[&1],
            Dispatch {
                kind: "call".to_owned(),
                class: Some("TwitchCommands".to_owned()),
                method: Some("ttv_setdebugoutput".to_owned()),
                args: vec!["$state".to_owned()],
            }
        );
        // Boolean variant — state-after marker keeps its position.
        assert_eq!(
            parse.dispatches[&2],
            Dispatch {
                kind: "call".to_owned(),
                class: None,
                method: Some("if_sendto".to_owned()),
                args: vec!["true".to_owned(), "$state".to_owned()],
            }
        );
        // Fall-through unassigned
        assert_eq!(
            parse.unassigned,
            [3_u16, 4, 5].into_iter().collect::<BTreeSet<_>>()
        );
        assert_eq!(parse.dispatches[&3].kind, "unassigned");
        assert_eq!(parse.dispatches[&3].method, None);
        // Extra trailing args — state-first marker then literals.
        assert_eq!(
            parse.dispatches[&6].args,
            vec!["$state".to_owned(), "true".to_owned(), "false".to_owned()]
        );
        // Cast literal
        assert_eq!(
            parse.dispatches[&7],
            Dispatch {
                kind: "call".to_owned(),
                class: Some("QuickChatDynamicCommand".to_owned()),
                method: Some("add".to_owned()),
                args: vec!["$state".to_owned(), "(short) -32146".to_owned()],
            }
        );
        // Final case without return
        assert_eq!(
            parse.dispatches[&8].method.as_deref(),
            Some("cc_getparentlayer")
        );
        Ok(())
    }

    #[test]
    fn switch_parsing_bounds_to_method() -> Result<()> {
        let parse = parse_switch(SWITCH_FIXTURE, Path::new("fixture.java"))?;
        // The trailing `other` switch's `case 0:` must not overwrite ours; id 0 stays our dispatch.
        assert_eq!(
            parse.dispatches[&0].method.as_deref(),
            Some("telemetry_get_group_count")
        );
        // Only ids 0..=8 captured (the second switch is excluded).
        assert_eq!(parse.dispatches.keys().copied().max(), Some(8));
        assert_eq!(parse.case_count, 9);
        Ok(())
    }

    const POST_SPLIT_FIXTURE: &str = r"
	public static void execute(ClientScriptCommand command, ClientScriptState state) throws CameraException, VarBitOverflowException {
		switch(command.index) {
			case 0:
				MiscOps.telemetry_get_group_count(state);
				return;
			case 1:
				IfOps.if_sendto(true, state);
				return;
			case 2:
			default:
				throw new RuntimeException();
		}
	}
";

    #[test]
    fn switch_accepts_post_split_signature() -> Result<()> {
        let parse = parse_switch(POST_SPLIT_FIXTURE, Path::new("fixture.java"))?;
        // Parameter names `command`/`state` and `command.index` header accepted;
        // `state` becomes the `$state` marker.
        assert_eq!(
            parse.dispatches[&0],
            Dispatch {
                kind: "call".to_owned(),
                class: Some("MiscOps".to_owned()),
                method: Some("telemetry_get_group_count".to_owned()),
                args: vec!["$state".to_owned()],
            }
        );
        assert_eq!(
            parse.dispatches[&1].args,
            vec!["true".to_owned(), "$state".to_owned()]
        );
        assert!(parse.unassigned.contains(&2));
        Ok(())
    }

    const ENUM_FIXTURE: &str = r#"
package com.jagex.game.script;

@ObfuscatedName("ss")
public class ClientScriptCommand implements ScriptCommand {

	public static final ClientScriptCommand add = new ClientScriptCommand(842);

	@ObfuscatedName("ss.e")
	public static final ClientScriptCommand PUSH_CONSTANT_INT = new ClientScriptCommand(204, true);

	@ObfuscatedName("ss.x")
	private int unrelated;
	public static final ClientScriptCommand POP_DISCARD = new ClientScriptCommand(11, false);

	public static final ClientScriptCommand DUP_A = new ClientScriptCommand(99);
	public static final ClientScriptCommand DUP_B = new ClientScriptCommand(99);

	public ClientScriptCommand(int arg0) {
	}
}
"#;

    #[test]
    fn enum_parses_fields_and_duplicates() -> Result<()> {
        let parse = parse_enum(ENUM_FIXTURE, Path::new("fixture.java"))?;
        assert_eq!(parse.fields.len(), 5);

        // Unannotated 1-arg
        let add = &parse.fields[0];
        assert_eq!(add.field, "add");
        assert_eq!(add.id, 842);
        assert!(!add.large_operand);
        assert_eq!(add.obf, None);
        assert_eq!(add.enum_order, 0);

        // Annotated 2-arg true
        let pci = &parse.fields[1];
        assert_eq!(pci.field, "PUSH_CONSTANT_INT");
        assert!(pci.large_operand);
        assert_eq!(pci.obf.as_deref(), Some("ss.e"));
        assert_eq!(pci.enum_order, 1);
        // Declaration order is contiguous regardless of source line gaps.
        assert_eq!(parse.fields[2].enum_order, 2);
        assert_eq!(parse.fields[3].enum_order, 3);
        assert_eq!(parse.fields[4].enum_order, 4);

        // The interleaved unrelated line cleared the stash → POP_DISCARD has no obf.
        let pop = &parse.fields[2];
        assert_eq!(pop.field, "POP_DISCARD");
        assert_eq!(pop.obf, None);
        assert!(!pop.large_operand);

        // Duplicate id captured for C8.
        assert_eq!(parse.duplicate_ids, vec![99]);
        Ok(())
    }

    fn write(dir: &Path, name: &str, body: &str) -> Result<std::path::PathBuf> {
        let path = dir.join(name);
        fs::write(&path, body)?;
        Ok(path)
    }

    #[test]
    fn data_file_parsers() -> Result<()> {
        let dir = tempdir()?;

        let p910 = write(
            dir.path(),
            "opcodes-910.txt",
            "// header a\n// header b\n_enum,810\nadd,842\n\n",
        )?;
        let (by_id, dups) = parse_opcodes_910(&p910)?;
        assert_eq!(by_id[&810], "_enum");
        assert_eq!(by_id[&842], "add");
        assert!(dups.is_empty());

        let plarge = write(
            dir.path(),
            "opcodes-large-910.txt",
            "// header\n810,0\n842,1\n",
        )?;
        let large = parse_opcodes_large_910(&plarge)?;
        assert!(!large[&810]);
        assert!(large[&842]);

        let p947 = write(dir.path(), "opcodes-947.txt", "enum,1660\nadd,900,742\n")?;
        let ids = parse_opcodes_947(&p947)?;
        assert_eq!(ids["enum"], 1660);
        assert_eq!(ids["add"], 900); // optional 3rd column ignored, row kept

        let p948 = write(dir.path(), "opcodes-948.txt", "enum,2002\nadd,901\n")?;
        let ids948 = parse_opcodes_948(&p948)?;
        assert_eq!(ids948["enum"], 2002);
        assert_eq!(ids948["add"], 901);

        let pstack = write(
            dir.path(),
            "stack-effects.txt",
            "# header\n_enum 4 0 0 1 1 0\nadd 2 0 0 1 0 0\n",
        )?;
        let stack = parse_stack_effects(&pstack)?;
        assert_eq!(stack["_enum"].int_pops, 4);
        assert_eq!(stack["_enum"].obj_pushes, 1);
        assert_eq!(stack["add"].int_pushes, 1);

        let palias = write(
            dir.path(),
            "opcode-aliases-910.txt",
            "// header\nenum,_enum\n",
        )?;
        let aliases = parse_aliases_910(&palias)?;
        assert_eq!(aliases["_enum"], vec!["enum".to_owned()]);

        // alias reverse-mapping used in 947 resolution
        assert_eq!(resolve_947("_enum", &ids, &aliases), Some(1660));
        assert_eq!(resolve_947("add", &ids, &aliases), Some(900));
        assert_eq!(resolve_947("missing", &ids, &aliases), None);

        // 948 resolution returns the donor-file name alongside the id. A direct
        // hit returns the canonical name; an alias hit returns the alias.
        assert_eq!(
            resolve_948("add", &ids948, &aliases),
            Some((901, "add".to_owned()))
        );
        assert_eq!(
            resolve_948("_enum", &ids948, &aliases),
            Some((2002, "enum".to_owned()))
        );
        assert_eq!(resolve_948("missing", &ids948, &aliases), None);

        Ok(())
    }

    #[test]
    fn cross_check_findings_via_full_run() -> Result<()> {
        let dir = tempdir()?;
        let client = dir.path().join("client-root");
        let java = client.join("client/src/main/java");
        let sr = java.join("rs2/client/clientscript");
        let csc = java.join("com/jagex/game/script");
        fs::create_dir_all(&sr)?;
        fs::create_dir_all(&csc)?;

        // Switch: id 0 (named, large), id 1 (named, not large, no enum -> C3),
        // id 2 (unnamed -> C1/C2b), id 3 (unassigned -> C9).
        let switch = "\tpublic static final void executeCommand(ClientScriptCommand arg0, ClientScriptState arg1) throws CameraException, VarBitOverflowException {\n\t\tswitch(arg0.index) {\n\t\t\tcase 0:\n\t\t\t\tfoo(arg1);\n\t\t\t\treturn;\n\t\t\tcase 1:\n\t\t\t\tbar(arg1);\n\t\t\t\treturn;\n\t\t\tcase 2:\n\t\t\t\tbaz(arg1);\n\t\t\t\treturn;\n\t\t\tcase 3:\n\t\t\tdefault:\n\t\t\t\tthrow new RuntimeException();\n\t\t\tcase 4:\n\t\t\t\tqux(arg1);\n\t\t}\n\t}\n";
        fs::write(sr.join("ScriptRunner.java"), switch)?;

        // Enum: id 0 large=true; id 7 has no switch case (C3b); duplicate id 0? no.
        // Also include an enum for id 0 with isLargeOperand mismatch handling.
        let enum_src = "\tpublic static final ClientScriptCommand FOO = new ClientScriptCommand(0, true);\n\tpublic static final ClientScriptCommand STRAY = new ClientScriptCommand(7, false);\n";
        fs::write(csc.join("ClientScriptCommand.java"), enum_src)?;

        // Data files. opcodes-910: ids 0,1,4 named; id 5 named but no switch (C2);
        // id 2 intentionally absent (-> C1/C2b). Note id 3 is unassigned, present here as well.
        fs::write(
            dir.path().join("opcodes-910.txt"),
            "// h1\n// h2\n// h3\nfoo,0\nbar,1\nthree,3\nqux,4\nstray910,5\n",
        )?;
        // large-910: id 0 flag=0 (mismatch with enum true -> C4); plus full id set for C4b.
        fs::write(
            dir.path().join("opcodes-large-910.txt"),
            "// h\n0,0\n1,0\n2,0\n3,0\n4,0\n",
        )?;
        // 947: foo present; orphan947 unmatched (C6).
        fs::write(
            dir.path().join("opcodes-947.txt"),
            "foo,100\norphan947,200\n",
        )?;
        // 948: foo present; orphan948 unmatched (C6b). bar/qux/three absent -> C7b.
        fs::write(
            dir.path().join("opcodes-948.txt"),
            "foo,300\norphan948,400\n",
        )?;
        // stack-effects: foo present; ghoststack not in registry (C5b). bar/qux missing -> C5.
        fs::write(
            dir.path().join("stack-effects.txt"),
            "# h\nfoo 0 0 0 1 0 0\nghoststack 1 0 0 0 0 0\n",
        )?;
        fs::write(dir.path().join("opcode-aliases-910.txt"), "// h\n")?;

        let out = dir.path().join("cs2/registry-910.json");
        let report_path = dir.path().join("cs2/registry-910.report.json");
        super::run(&super::Cs2RegistryOpts {
            client_root: &client,
            data_dir: dir.path(),
            out_file: Some(&out),
            report_file: Some(&report_path),
        })?;

        let report: serde_json::Value = serde_json::from_str(&fs::read_to_string(&report_path)?)?;
        let findings = report["findings"].as_array().expect("findings array");
        let checks: BTreeSet<String> = findings
            .iter()
            .map(|f| f["check"].as_str().expect("check string").to_owned())
            .collect();

        for expected in [
            "C1", "C2", "C2b", "C3", "C3b", "C4", "C5", "C5b", "C6", "C6b", "C7", "C7b", "C9",
        ] {
            assert!(
                checks.contains(expected),
                "missing finding {expected}: {checks:?}"
            );
        }

        // Determinism: a second run is byte identical.
        let first = fs::read(&out)?;
        let first_report = fs::read(&report_path)?;
        super::run(&super::Cs2RegistryOpts {
            client_root: &client,
            data_dir: dir.path(),
            out_file: Some(&out),
            report_file: Some(&report_path),
        })?;
        assert_eq!(first, fs::read(&out)?);
        assert_eq!(first_report, fs::read(&report_path)?);

        Ok(())
    }

    #[test]
    fn resolve_947_direct_and_alias() {
        let mut ids: HashMap<String, u32> = HashMap::new();
        ids.insert("enum".to_owned(), 1660);
        let mut aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        aliases.insert("_enum".to_owned(), vec!["enum".to_owned()]);
        assert_eq!(resolve_947("_enum", &ids, &aliases), Some(1660));
        assert_eq!(resolve_947("nope", &ids, &aliases), None);
    }
}
