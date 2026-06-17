//! Server TS encode-body parsing + classification (DSL v1 / v2).
//!
//! Finds every `ServerProt.X.encode = function (...): Packet {` override, parses
//! its body into statement lines, and classifies it `simple` / `v2-simple` /
//! `complex:<reason>`. The control flow mirrors the Python `classify_server`
//! exactly, including reason strings and the order rejections fire.

use super::{
    Field, Param, ParamModel, ServerEncoder, codec_param_type, dsl_param_type,
    is_plain_int_literal, is_v1_codec, split_top_level,
};
use crate::protocol_registry::expr::{Expr, emit_schema, parse_expr};
use std::collections::{BTreeMap, HashMap, HashSet};

// ---------------------------------------------------------------------------
// const-local inlining (AST substitution)
// ---------------------------------------------------------------------------

/// Substitute every `ident` that names a single-assignment const with that
/// const's (already-canonical) expression. A compound (`Bin`) substitution is
/// wrapped in parens to preserve grouping — byte-equivalent to the Python
/// `inline_consts` token rewrite, which wraps any value containing a space.
fn inline_consts(expr: &Expr, consts: &HashMap<String, Expr>) -> Expr {
    match expr {
        Expr::Ident(name) => consts.get(name).map_or_else(
            || expr.clone(),
            |val| match val {
                Expr::Bin(..) => Expr::Paren(Box::new(val.clone())),
                other => other.clone(),
            },
        ),
        // An array access on a const name cannot occur (consts are scalar
        // expressions); leave it untouched, mirroring the token rewrite which
        // only replaces bare ident tokens.
        Expr::Index(name, idx) => Expr::Index(name.clone(), *idx),
        Expr::Int(_) => expr.clone(),
        Expr::Paren(inner) => Expr::Paren(Box::new(inline_consts(inner, consts))),
        Expr::Bin(l, op, r) => Expr::Bin(
            Box::new(inline_consts(l, consts)),
            op.clone(),
            Box::new(inline_consts(r, consts)),
        ),
    }
}

// ---------------------------------------------------------------------------
// TS encode override extraction
// ---------------------------------------------------------------------------

/// One parsed encode override: its name + signature params + statement lines.
pub(super) struct RawEncoder {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<String>,
}

/// Find every `ServerProt.X.encode = function (...): Packet {` override
/// (signatures may span multiple lines) and capture its name, params, and body
/// statement lines. Mirrors the Python `ENCODE_RE` (DOTALL, non-greedy sig).
pub(super) fn extract_raw_encoders(ts_src: &str) -> Vec<RawEncoder> {
    let chars: Vec<char> = ts_src.chars().collect();
    let n = chars.len();
    let mut out: Vec<RawEncoder> = Vec::new();
    let mut search = 0;
    while let Some(rel) = find_sub(&chars, search, "ServerProt.") {
        let name_start = rel + "ServerProt.".len();
        let mut i = name_start;
        while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        let name: String = chars[name_start..i].iter().collect();
        // Require the literal `.encode = function (` to follow the name.
        let marker = ".encode = function (";
        if !slice_eq(&chars, i, marker) {
            search = name_start;
            continue;
        }
        let sig_start = i + marker.len();
        // Non-greedy: scan to the first `): Packet {` after the signature.
        let Some(sig_end) = find_sub(&chars, sig_start, "): Packet {") else {
            break;
        };
        let sig: String = chars[sig_start..sig_end].iter().collect();
        let body_start = sig_end + "): Packet {".len();
        let body = collect_body(&chars, body_start);
        out.push(RawEncoder {
            name,
            params: super::parse_params(&sig),
            body,
        });
        search = body_start;
    }
    out
}

/// Find the next occurrence of `needle` in `chars[from..]`, returning its start
/// index, or `None`. (Small linear scan — these inputs are modest.)
fn find_sub(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let pat: Vec<char> = needle.chars().collect();
    if pat.is_empty() || from >= chars.len() {
        return None;
    }
    (from..=chars.len().saturating_sub(pat.len())).find(|&start| slice_eq(chars, start, needle))
}

/// Does `chars[at..]` start with `needle`?
fn slice_eq(chars: &[char], at: usize, needle: &str) -> bool {
    let mut idx = at;
    for c in needle.chars() {
        if chars.get(idx) != Some(&c) {
            return false;
        }
        idx += 1;
    }
    true
}

/// Collect the encoder body (from just after the opening `{`) as trimmed,
/// non-empty statement lines, reading to the matching closing brace.
fn collect_body(chars: &[char], body_start: usize) -> Vec<String> {
    let mut depth = 1;
    let mut i = body_start;
    while i < chars.len() && depth > 0 {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let body_text: String = chars[body_start..i.min(chars.len())].iter().collect();
    body_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Already-migrated packets: `ServerProt.NAME.encode = encodeName;`. Returns a
/// map `NAME -> encodeName`. These have no hand-written body left to parse.
pub(super) fn migrated_encoders(ts_src: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for line in ts_src.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("ServerProt.") else {
            continue;
        };
        let Some((name, after)) = rest.split_once(".encode = ") else {
            continue;
        };
        let Some(func) = after.strip_suffix(';') else {
            continue;
        };
        // The whole line must have been `...encode = encodeName;` with the name
        // and func both bare identifiers (the function-assignment form).
        if !name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            continue;
        }
        if func.starts_with("encode")
            && func.len() > "encode".len()
            && func.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            out.insert(name.to_owned(), func.to_owned());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Statement matchers
// ---------------------------------------------------------------------------

/// Parse a fixed-alloc `const buf: Packet = new Packet(new Uint8Array(<X>));`.
fn match_alloc_fixed(s: &str) -> Option<&str> {
    s.strip_prefix("const buf: Packet = new Packet(new Uint8Array(")?
        .strip_suffix("));")
}

/// Parse a dynamic-alloc `const buf: Packet = Packet.alloc(<X>);`.
fn match_alloc_dynamic(s: &str) -> Option<&str> {
    s.strip_prefix("const buf: Packet = Packet.alloc(")?
        .strip_suffix(");")
}

/// Parse a `buf.<codec>(<arg>);` statement into `(codec, arg)`.
fn match_buf_call(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix("buf.")?.strip_suffix(");")?;
    let (codec, arg) = inner.split_once('(')?;
    Some((codec, arg))
}

/// Parse a `const <name>[: type] = <rhs>;` local (not `const buf...`).
fn match_const_decl(s: &str) -> Option<(String, String)> {
    let rest = s.strip_prefix("const ")?;
    let (lhs, rhs) = rest.split_once(" = ")?;
    let rhs = rhs.strip_suffix(';')?;
    // lhs is `name` or `name: type`.
    let name = lhs.split(':').next().unwrap_or(lhs).trim();
    if name.is_empty() {
        return None;
    }
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !first_ok || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((name.to_owned(), rhs.trim().to_owned()))
}

/// The outcome of resolving a single `buf.<codec>(<arg>)` argument.
enum ArgResolve {
    /// Accepted: canonical arg text + whether it required the v2 grammar.
    Ok { canon: String, is_v2: bool },
    /// Rejected (`computed-arg`).
    Reject,
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify a server encoder body for DSL v1 / v2 (Stage 7 §1.1 / Stage 8 §3).
/// The statement loop mirrors the Python `classify_server` exactly.
pub(super) fn classify_server(name: &str, params: &[Param], body: &[String]) -> ServerEncoder {
    // Build the param model. Array params carry their default's element count.
    let mut names: HashSet<&str> = HashSet::new();
    let mut arrays: BTreeMap<&str, usize> = BTreeMap::new();
    for p in params {
        names.insert(p.name.as_str());
        if dsl_param_type(&p.ty) == Some("int[]") {
            let Some(default) = &p.default else {
                return ServerEncoder::complex(name, params.to_vec(), "array-param-no-default");
            };
            let inner = default.trim().trim_start_matches('[').trim_end_matches(']');
            let count = split_top_level(inner)
                .into_iter()
                .filter(|e| !e.trim().is_empty())
                .count();
            arrays.insert(p.name.as_str(), count);
        }
    }
    let model = ParamModel { names, arrays };

    let mut fields: Vec<Field> = Vec::new();
    let mut saw_alloc = false;
    let mut saw_return = false;
    let mut alloc_expr: Option<String> = None;
    let mut is_v2 = false;
    let mut consts: HashMap<String, Expr> = HashMap::new();

    for s in body {
        if saw_return {
            return ServerEncoder::complex(name, params.to_vec(), "code-after-return");
        }
        // Fixed allocation: `new Packet(new Uint8Array(<X>))`.
        if let Some(raw) = match_alloc_fixed(s) {
            if saw_alloc {
                return ServerEncoder::complex(name, params.to_vec(), "multi-alloc");
            }
            saw_alloc = true;
            let raw = raw.trim();
            if !is_plain_int_literal(raw) {
                // A computed `new Uint8Array(<expr>)` — admit as a v2 alloc.
                let Ok(parsed) = parse_expr(raw) else {
                    return ServerEncoder::complex(
                        name,
                        params.to_vec(),
                        "alloc-computed-nongrammar",
                    );
                };
                let inlined = inline_consts(&parsed, &consts);
                if !model.validates(&inlined) {
                    return ServerEncoder::complex(
                        name,
                        params.to_vec(),
                        "alloc-computed-badident",
                    );
                }
                alloc_expr = Some(emit_schema(&inlined));
                is_v2 = true;
            }
            continue;
        }
        // Dynamic allocation: `Packet.alloc(<X>)`.
        if let Some(raw) = match_alloc_dynamic(s) {
            if saw_alloc {
                return ServerEncoder::complex(name, params.to_vec(), "multi-alloc");
            }
            saw_alloc = true;
            match resolve_alloc_size(raw.trim(), &consts, &model) {
                Some(canon) => {
                    alloc_expr = Some(canon);
                    is_v2 = true;
                }
                None => return ServerEncoder::complex(name, params.to_vec(), "alloc-dynamic"),
            }
            continue;
        }
        if s == "return buf;" {
            saw_return = true;
            continue;
        }
        // A `buf.<codec>(<arg>);` write.
        if let Some((codec, arg)) = match_buf_call(s) {
            if !is_v1_codec(codec) {
                return ServerEncoder::complex(
                    name,
                    params.to_vec(),
                    &format!("non-v1-codec:{codec}"),
                );
            }
            match resolve_arg(arg.trim(), &consts, &model) {
                ArgResolve::Ok { canon, is_v2: v2 } => {
                    if v2 {
                        is_v2 = true;
                    }
                    fields.push(Field {
                        codec: codec.to_owned(),
                        arg: canon,
                    });
                }
                ArgResolve::Reject => {
                    return ServerEncoder::complex(name, params.to_vec(), "computed-arg");
                }
            }
            continue;
        }
        // A single-assignment in-grammar `const` local (not `const buf`).
        if !s.starts_with("const buf")
            && let Some((local, rhs)) = match_const_decl(s)
        {
            let Ok(parsed) = parse_expr(&rhs) else {
                return ServerEncoder::complex(name, params.to_vec(), "local");
            };
            let inlined = inline_consts(&parsed, &consts);
            if !model.validates(&inlined) {
                return ServerEncoder::complex(name, params.to_vec(), "local");
            }
            consts.insert(local, inlined);
            is_v2 = true;
            continue;
        }
        // Anything else: conditionals, loops, helper calls, let-locals, etc.
        if s.starts_with("if ") || s.starts_with("if(") {
            return ServerEncoder::complex(name, params.to_vec(), "conditional");
        }
        if s.starts_with("for ") || s.starts_with("for(") || s.starts_with("while") {
            return ServerEncoder::complex(name, params.to_vec(), "loop");
        }
        if s.contains("Packet.alloc") {
            return ServerEncoder::complex(name, params.to_vec(), "alloc-dynamic");
        }
        if s.starts_with("const ") || s.starts_with("let ") {
            return ServerEncoder::complex(name, params.to_vec(), "local");
        }
        if s.starts_with("buf.") {
            return ServerEncoder::complex(name, params.to_vec(), "non-codec-call");
        }
        return ServerEncoder::complex(name, params.to_vec(), "other");
    }

    if !saw_alloc {
        return ServerEncoder::complex(name, params.to_vec(), "no-alloc");
    }
    if !saw_return {
        return ServerEncoder::complex(name, params.to_vec(), "no-return");
    }
    if fields.is_empty() {
        return ServerEncoder::complex(name, params.to_vec(), "empty");
    }
    ServerEncoder {
        name: name.to_owned(),
        params: params.to_vec(),
        fields,
        simple: true,
        reason: String::new(),
        is_v2,
        alloc: alloc_expr,
    }
}

/// Validate one `buf.<codec>(<arg>)` arg as v1-plain or a v2 expression after
/// const-inlining. Mirrors the Python `resolve_arg`.
fn resolve_arg(arg: &str, consts: &HashMap<String, Expr>, model: &ParamModel<'_>) -> ArgResolve {
    let arg = arg.trim();
    // v1: a bare scalar param name.
    if model.names.contains(arg) && !model.arrays.contains_key(arg) {
        return ArgResolve::Ok {
            canon: arg.to_owned(),
            is_v2: false,
        };
    }
    // v1: a plain integer literal.
    if is_plain_int_literal(arg) {
        return ArgResolve::Ok {
            canon: arg.to_owned(),
            is_v2: false,
        };
    }
    // A bare const reference inlines to its canonical expression; its tier is v1
    // only when the const resolves to a bare scalar param name.
    if let Some(val) = consts.get(arg) {
        let canon = emit_schema(val);
        let is_v2 = !matches!(val, Expr::Ident(n) if model.names.contains(n.as_str()));
        return ArgResolve::Ok { canon, is_v2 };
    }
    // v2: parse the expression, inlining consts first.
    let Ok(parsed) = parse_expr(arg) else {
        return ArgResolve::Reject;
    };
    let inlined = inline_consts(&parsed, consts);
    if !model.validates(&inlined) {
        return ArgResolve::Reject;
    }
    ArgResolve::Ok {
        canon: emit_schema(&inlined),
        is_v2: true,
    }
}

/// Validate a `Packet.alloc(<expr>)` size as a loop-free computed size in the
/// pure-integer grammar. Returns the canonical size text or `None`. Mirrors the
/// Python `resolve_alloc_size`.
fn resolve_alloc_size(
    raw: &str,
    consts: &HashMap<String, Expr>,
    model: &ParamModel<'_>,
) -> Option<String> {
    let parsed = parse_expr(raw).ok()?;
    let inlined = inline_consts(&parsed, consts);
    if !model.validates(&inlined) {
        return None;
    }
    Some(emit_schema(&inlined))
}

/// Map each param name to its DSL type using its TS type + codec usage. Mirrors
/// the Python `refine_param_types`.
pub(super) fn refine_param_types(enc: &ServerEncoder) -> HashMap<String, String> {
    let mut types: HashMap<String, String> = HashMap::new();
    let param_names: HashSet<&str> = enc.params.iter().map(|p| p.name.as_str()).collect();
    for p in &enc.params {
        let kind = dsl_param_type(&p.ty).unwrap_or("int");
        types.insert(p.name.clone(), kind.to_owned());
    }
    for fld in &enc.fields {
        if !param_names.contains(fld.arg.as_str()) {
            continue;
        }
        if types.get(&fld.arg).map(String::as_str) == Some("int") {
            types.insert(fld.arg.clone(), codec_param_type(&fld.codec).to_owned());
        }
    }
    types
}
