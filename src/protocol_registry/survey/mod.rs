//! `survey-payloads` — protocol payload classification + schema survey
//! (the Stage 7/8 Part A audit, ported from `scripts/protocol-payload-survey.py`).
//!
//! Parses every `ServerProt.<NAME>.encode = function (...) {...}` override in the
//! server's `ServerProt.ts` and the matching `ServerProt.NAME == arg0.packetType`
//! decode branch in the client's `Client.java`, classifies each end as
//! simple/complex per the DSL rules, and emits:
//!
//!   * `data/protocol/910/payload-classification.json`   (Part A)
//!   * `data/protocol/910/payloads.json`                 (Part B, tranche only)
//!
//! DSL v1 (Stage 7) admits straight-line `buf.<codec>(<param|literal>)` bodies.
//! DSL v2 (Stage 8) additionally admits, classified as server `v2-simple`:
//! integer-expression args over the [`super::expr`] grammar, `int[]` params with
//! literal-index access, single-assignment in-grammar `const` locals (inlined at
//! use), and loop-free computed allocation sizes.
//!
//! Deterministic, no timestamps, sorted by name. Read-only over the source trees.
//! The emitted JSON is byte-identical to the retired Python's `json.dumps`
//! output (`tests/protocol_survey_oracle.rs` is the regression gate).
//!
//! Split by concern: [`server`] (TS encode-body parse + `classify_server`),
//! [`client`] (Java decode-branch parse + `classify_client` + the mirror check),
//! and this module (the shared model, codec tables, JSON serde types, and the
//! orchestration that drives both ends and renders the two outputs).

mod client;
mod server;

use crate::error::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// DSL v1 codec vocabulary (spec §1.2)
// ---------------------------------------------------------------------------

/// DSL-v1 codec → byte width (`Some`), or variable-width (`None` for the two
/// admitted variable codecs). Any codec not listed here is a non-v1 codec.
fn fixed_codec_width(codec: &str) -> Option<u32> {
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

/// Is this codec admitted by DSL v1 (a fixed-width codec or one of the two
/// variable-width codecs `pjstr` / `pSmart1or2`)?
fn is_v1_codec(codec: &str) -> bool {
    fixed_codec_width(codec).is_some() || codec == "pjstr" || codec == "pSmart1or2"
}

/// Mirror table (spec §1.4): write codec → the set of acceptable client reads.
/// Width + alt variant must match; signedness and string charset are read-side
/// choices, so signed (`g..s` / `g..b`) variants are accepted.
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

/// Param TS type a codec implies for its bare scalar arg (Part B `params`
/// typing). Codecs not listed leave the param `int`.
fn codec_param_type(codec: &str) -> &'static str {
    match codec {
        "p5" | "p6" | "p8" => "bigint",
        "pbool" => "boolean",
        "pjstr" => "string",
        _ => "int",
    }
}

// ---------------------------------------------------------------------------
// Survey data model (local — see `super::types` for the schema-doc model)
// ---------------------------------------------------------------------------

/// One TS encode-signature parameter: `(name, ts_type, default?)` verbatim.
#[derive(Debug, Clone)]
struct Param {
    name: String,
    ty: String,
    default: Option<String>,
}

/// One ordered codec field of a server encode body.
#[derive(Debug, Clone)]
struct Field {
    codec: String,
    /// Param name, integer literal, or canonical expression (schema form).
    arg: String,
}

/// The server encode body classification for one packet.
#[derive(Debug, Clone)]
struct ServerEncoder {
    name: String,
    params: Vec<Param>,
    fields: Vec<Field>,
    simple: bool,
    reason: String,
    /// DSL tier: `false` = v1 (straight-line), `true` = v2.
    is_v2: bool,
    /// Computed allocation expression (schema form) for variable-size v2
    /// packets, or `None` for fixed-size packets.
    alloc: Option<String>,
}

impl ServerEncoder {
    /// A rejected (complex) encoder carrying only the one-word reason.
    fn complex(name: &str, params: Vec<Param>, reason: &str) -> Self {
        Self {
            name: name.to_owned(),
            params,
            fields: Vec::new(),
            simple: false,
            reason: reason.to_owned(),
            is_v2: false,
            alloc: None,
        }
    }
}

/// The client decode-branch classification for one packet.
#[derive(Debug, Clone)]
struct ClientBranch {
    reads: Vec<String>,
    simple: bool,
    reason: String,
}

/// The param model for one encoder: scalar param names + array param lengths.
struct ParamModel<'a> {
    names: HashSet<&'a str>,
    arrays: BTreeMap<&'a str, usize>,
}

impl ParamModel<'_> {
    /// Validate every ident/array-access in `expr` against the param model.
    /// Mirrors the Python `validate_expr_idents`: bare idents must be declared
    /// scalar params; array accesses must hit an `int[]` param in bounds.
    fn validates(&self, expr: &super::expr::Expr) -> bool {
        use super::expr::Expr;
        match expr {
            Expr::Int(_) => true,
            Expr::Ident(name) => {
                self.names.contains(name.as_str()) && !self.arrays.contains_key(name.as_str())
            }
            Expr::Index(name, idx) => self
                .arrays
                .get(name.as_str())
                .is_some_and(|&len| (*idx as usize) < len),
            Expr::Paren(inner) => self.validates(inner),
            Expr::Bin(l, _, r) => self.validates(l) && self.validates(r),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared TS parameter-list + literal parsing
// ---------------------------------------------------------------------------

/// Split `text` on commas not nested in `()`, `<>`, `[]`, or `{}`.
fn split_top_level(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut cur = String::new();
    for ch in text.chars() {
        match ch {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' | '>' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Parse a TS parameter list into ordered `(name, type, default?)` triples.
/// Handles `name: type`, `name: type = default`, and bare `name` (untyped).
fn parse_params(sig: &str) -> Vec<Param> {
    let sig = sig.trim();
    if sig.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Param> = Vec::new();
    for raw in split_top_level(sig) {
        let mut part = raw.trim().to_owned();
        let mut default: Option<String> = None;
        if let Some((decl, def)) = part.split_once('=') {
            default = Some(def.trim().to_owned());
            part = decl.trim().to_owned();
        }
        if let Some((name, ty)) = part.split_once(':') {
            out.push(Param {
                name: name.trim().to_owned(),
                ty: ty.trim().to_owned(),
                default,
            });
        } else {
            out.push(Param {
                name: part.trim().to_owned(),
                ty: String::new(),
                default,
            });
        }
    }
    out
}

/// Map a verbatim TS param type to its DSL kind, or `None` if unsupported.
/// `int[]` (TS `number[]`) is the only array kind DSL v2 admits.
fn dsl_param_type(ts_type: &str) -> Option<&'static str> {
    match ts_type.trim() {
        "number[]" | "Array<number>" => Some("int[]"),
        "number" => Some("int"),
        "string" => Some("string"),
        "bigint" => Some("bigint"),
        "boolean" => Some("boolean"),
        _ => None,
    }
}

/// Parse a plain decimal/hex integer literal, or `None` when not a literal.
/// Mirrors Python's `parse_int_literal` (accepts a leading `-`, hex via `0x`).
fn parse_int_literal(text: &str) -> Option<i64> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = lower.strip_prefix("-0x") {
        return i64::from_str_radix(hex, 16).ok().map(|v| -v);
    }
    t.parse::<i64>().ok()
}

/// Is `text` a plain integer literal in the exact form the survey accepts as a
/// v1 arg: `-?(\d+|0[xX][0-9a-fA-F]+)` AND parseable. Mirrors the Python guard
/// `parse_int_literal(arg) is not None and re.fullmatch(...)`.
fn is_plain_int_literal(text: &str) -> bool {
    if parse_int_literal(text).is_none() {
        return false;
    }
    let body = text.strip_prefix('-').unwrap_or(text);
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
}

/// Render a string list the way Python's `repr(sorted(...))` does: `['a', 'b']`.
/// Used only in the stdout summary and `mirror_mismatch` diagnostics so they
/// match the Python; the JSON outputs never embed this.
fn python_str_list(items: &[&str]) -> String {
    let inner = items
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

// ---------------------------------------------------------------------------
// payloads.json model (serde — emits the exact `json.dumps(indent=2)` order)
// ---------------------------------------------------------------------------

/// One `params[]` entry. Field order `name, type, [default]` matches the Python.
#[derive(Debug, Serialize)]
struct ParamJson {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
}

/// One `fields[]` entry. Field order `codec, arg`.
#[derive(Debug, Serialize)]
struct FieldJson {
    codec: String,
    arg: String,
}

/// One packet entry. Field order `params, fields, client_reads, [alloc]`.
#[derive(Debug, Serialize)]
struct PacketJson {
    params: Vec<ParamJson>,
    fields: Vec<FieldJson>,
    client_reads: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alloc: Option<String>,
}

/// The `payloads.json` document. Top-level order `schema, packets`.
#[derive(Debug, Serialize)]
struct PayloadsDoc {
    schema: &'static str,
    packets: BTreeMap<String, PacketJson>,
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// Resolved options for the `survey-payloads` subcommand.
#[derive(Debug)]
pub struct SurveyPayloadsOpts<'a> {
    /// Root of the client checkout (holds `client/src/main/java/...`).
    pub client_root: &'a Path,
    /// Root of the server checkout (holds `src/jagex/network/protocol/...`).
    pub server_root: &'a Path,
    /// Output directory (default `<data-dir>/protocol/910`).
    pub out_dir: &'a Path,
}

/// The in-memory survey product, kept separate from disk so the oracle test and
/// the CLI share one code path and compare bytes without re-reading files.
pub struct SurveyOutput {
    /// `payload-classification.json` body (pretty, trailing newline).
    pub classification: String,
    /// `payloads.json` body (pretty, trailing newline).
    pub payloads: String,
    /// The stdout summary lines (printed by the CLI).
    pub summary: Vec<String>,
}

/// Run the full survey in memory: parse → classify → mirror-gate → required-
/// member gate → render both JSON documents. Returns an error (mapping to a
/// non-zero exit) when a required tranche member is missing or a migrated
/// packet's prior entry was clobbered — both BEFORE anything is written.
pub fn survey(opts: &SurveyPayloadsOpts<'_>) -> Result<SurveyOutput> {
    let ts_path = opts
        .server_root
        .join("src/jagex/network/protocol/ServerProt.ts");
    let java_path = opts
        .client_root
        .join("client/src/main/java/rs2/client/Client.java");

    let ts_src = fs::read_to_string(&ts_path)
        .with_context(|| format!("failed to read {}", ts_path.display()))?;
    let java_src = fs::read_to_string(&java_path)
        .with_context(|| format!("failed to read {}", java_path.display()))?;

    let encoders: Vec<ServerEncoder> = server::extract_raw_encoders(&ts_src)
        .into_iter()
        .map(|raw| server::classify_server(&raw.name, &raw.params, &raw.body))
        .collect();
    let branches = client::extract_branches(&java_src);
    let migrated = server::migrated_encoders(&ts_src);

    // Carried-forward entries: the existing payloads.json packets (verbatim).
    let existing_payloads = load_existing_payloads(opts.out_dir)?;

    // Full packet roster from the Stage-6 schema (all 195 ServerProt packets).
    let schema_path = opts.out_dir.join("server_prot.json");
    let schema_src = fs::read_to_string(&schema_path)
        .with_context(|| format!("failed to read {}", schema_path.display()))?;
    let schema: serde_json::Value = serde_json::from_str(&schema_src)
        .with_context(|| format!("failed to parse {}", schema_path.display()))?;
    let all_packets: Vec<String> = schema["packets"]
        .as_array()
        .context("server_prot.json: `packets` is not an array")?
        .iter()
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();

    let enc_by_name: HashMap<&str, &ServerEncoder> =
        encoders.iter().map(|e| (e.name.as_str(), e)).collect();

    // ----- Part A: classification -----
    let mut classification: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

    // Packets with no encode override (or carried-forward migrated ones).
    for name in &all_packets {
        if enc_by_name.contains_key(name.as_str()) {
            continue;
        }
        let client_class = branches
            .get(name)
            .map_or_else(|| "no_client_branch".to_owned(), client_class_of);
        let server_class = if migrated.contains_key(name) {
            if entry_is_v2(existing_payloads.get(name)) {
                "v2-simple".to_owned()
            } else {
                "simple".to_owned()
            }
        } else {
            "complex:no_encode_override".to_owned()
        };
        classification.insert(name.clone(), class_entry(&server_class, &client_class));
    }

    for enc in &encoders {
        let server_class = if enc.simple {
            if enc.is_v2 {
                "v2-simple".to_owned()
            } else {
                "simple".to_owned()
            }
        } else {
            format!("complex:{}", enc.reason)
        };
        let client_class = branches
            .get(&enc.name)
            .map_or_else(|| "no_client_branch".to_owned(), client_class_of);
        classification.insert(enc.name.clone(), class_entry(&server_class, &client_class));
    }

    // ----- tranche + mirror check -----
    let mut tranche: Vec<String> = Vec::new();
    let mut mirror_mismatch: BTreeMap<String, String> = BTreeMap::new();
    for (name, cls) in &classification.clone() {
        let server_ok = cls["server"] == "simple" || cls["server"] == "v2-simple";
        if !server_ok || cls["client"] != "simple" {
            continue;
        }
        let fields: Vec<Field> = if let Some(enc) = enc_by_name.get(name.as_str()) {
            enc.fields.clone()
        } else {
            // Carried-forward migrated packet: mirror-check its prior schema.
            let Some(prior) = existing_payloads.get(name) else {
                return Err(crate::error::CacheError::message(format!(
                    "FATAL: migrated packet {name} has no entry in payloads.json \
                     (file clobbered?) — restore it before re-running"
                )));
            };
            prior_fields(prior)
        };
        let reads = &branches[name].reads;
        match client::mirror_ok(&fields, reads) {
            Ok(()) => tranche.push(name.clone()),
            Err(why) => {
                mirror_mismatch.insert(name.clone(), why);
                classification
                    .get_mut(name)
                    .expect("present")
                    .insert("client".to_owned(), "complex:mirror_mismatch".to_owned());
            }
        }
    }

    // ----- Part B: payloads.json (tranche only) -----
    let mut packets: BTreeMap<String, PacketJson> = BTreeMap::new();
    for name in &tranche {
        if let Some(enc) = enc_by_name.get(name.as_str()) {
            packets.insert(name.clone(), built_packet_json(enc, &branches[name].reads));
        } else {
            // Carry the prior entry through byte-for-byte.
            packets.insert(name.clone(), prior_packet_json(&existing_payloads[name]));
        }
    }

    // ----- required-member gate (BEFORE writing anything) -----
    let required_v1 = ["VARP_SMALL", "VARP_LARGE", "VARBIT_SMALL", "VARBIT_LARGE"];
    let required_v2 = [
        "IF_OPENTOP",
        "IF_OPENSUB",
        "IF_SETEVENTS",
        "IF_SETANIM",
        "CLEAR_PLAYER_SNAPSHOT",
        "CHAT_FILTER_SETTINGS",
    ];
    let tranche_set: HashSet<&str> = tranche.iter().map(String::as_str).collect();
    let missing: Vec<&str> = required_v1
        .iter()
        .chain(&required_v2)
        .copied()
        .filter(|r| !tranche_set.contains(r))
        .collect();
    if !missing.is_empty() {
        use std::fmt::Write as _;
        let mut msg = format!("FATAL: required tranche members missing: {missing:?}");
        for r in &missing {
            let cls = classification.get(*r);
            let _ = write!(msg, "\n  {r}: {cls:?}");
        }
        return Err(crate::error::CacheError::message(msg));
    }

    // ----- render outputs -----
    let classification_json = render_classification(&classification)?;
    let payloads_doc = PayloadsDoc {
        schema: "protocol-payloads/v2",
        packets,
    };
    let mut payloads_json = serde_json::to_string_pretty(&payloads_doc)?;
    payloads_json.push('\n');

    let summary = build_summary(
        &classification,
        &tranche,
        &mirror_mismatch,
        &required_v1,
        &required_v2,
    );

    Ok(SurveyOutput {
        classification: classification_json,
        payloads: payloads_json,
        summary,
    })
}

/// The client class string for a parsed branch (`simple` / `complex:<reason>`).
fn client_class_of(br: &ClientBranch) -> String {
    if br.simple {
        "simple".to_owned()
    } else {
        format!("complex:{}", br.reason)
    }
}

/// Build a freshly-surveyed tranche packet's JSON entry from its encoder + the
/// client read sequence.
fn built_packet_json(enc: &ServerEncoder, reads: &[String]) -> PacketJson {
    let ptypes = server::refine_param_types(enc);
    let params = enc
        .params
        .iter()
        .map(|p| ParamJson {
            name: p.name.clone(),
            ty: ptypes[&p.name].clone(),
            default: p.default.clone(),
        })
        .collect();
    let fields = enc
        .fields
        .iter()
        .map(|f| FieldJson {
            codec: f.codec.clone(),
            arg: f.arg.clone(),
        })
        .collect();
    PacketJson {
        params,
        fields,
        client_reads: reads.to_vec(),
        alloc: enc.alloc.clone(),
    }
}

/// Build a `{"server":.., "client":..}` classification entry. A `BTreeMap`
/// serialises its keys sorted, so `client` precedes `server` exactly like the
/// Python `json.dumps(sort_keys=True)`.
fn class_entry(server: &str, client: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("server".to_owned(), server.to_owned());
    m.insert("client".to_owned(), client.to_owned());
    m
}

/// Render `payload-classification.json` exactly like the Python
/// `json.dumps(x, indent=2, sort_keys=True) + "\n"`.
fn render_classification(
    classification: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<String> {
    let mut s = serde_json::to_string_pretty(classification)?;
    s.push('\n');
    Ok(s)
}

/// Load the existing `payloads.json` `packets` object (verbatim JSON values) for
/// carry-forward, or an empty map when the file is absent.
fn load_existing_payloads(out_dir: &Path) -> Result<BTreeMap<String, serde_json::Value>> {
    let path = out_dir.join("payloads.json");
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let mut out: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    if let Some(obj) = doc.get("packets").and_then(serde_json::Value::as_object) {
        for (k, v) in obj {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(out)
}

/// Extract the `fields` array of a prior payloads.json entry as [`Field`]s for
/// mirror-checking a carried-forward (migrated) packet.
fn prior_fields(entry: &serde_json::Value) -> Vec<Field> {
    entry
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|f| Field {
                    codec: f["codec"].as_str().unwrap_or_default().to_owned(),
                    arg: f["arg"].as_str().unwrap_or_default().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Re-serialise a prior payloads.json entry into the typed [`PacketJson`] so it
/// is emitted byte-identically (carry-forward of an already-migrated packet).
fn prior_packet_json(entry: &serde_json::Value) -> PacketJson {
    let params = entry
        .get("params")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|p| ParamJson {
                    name: p["name"].as_str().unwrap_or_default().to_owned(),
                    ty: p["type"].as_str().unwrap_or_default().to_owned(),
                    default: p
                        .get("default")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    let fields = entry
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|f| FieldJson {
                    codec: f["codec"].as_str().unwrap_or_default().to_owned(),
                    arg: f["arg"].as_str().unwrap_or_default().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let client_reads = entry
        .get("client_reads")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let alloc = entry
        .get("alloc")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    PacketJson {
        params,
        fields,
        client_reads,
        alloc,
    }
}

/// Classify a prior payloads.json entry as v2 (vs straight-line v1). Mirrors the
/// Python `entry_is_v2`.
fn entry_is_v2(entry: Option<&serde_json::Value>) -> bool {
    let Some(entry) = entry else {
        return false;
    };
    if entry.get("alloc").is_some() {
        return true;
    }
    let params = entry.get("params").and_then(serde_json::Value::as_array);
    let param_names: HashSet<&str> = params
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p["name"].as_str())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    if let Some(arr) = params
        && arr.iter().any(|p| p["type"].as_str() == Some("int[]"))
    {
        return true;
    }
    if let Some(fields) = entry.get("fields").and_then(serde_json::Value::as_array) {
        for fld in fields {
            let arg = fld["arg"].as_str().unwrap_or_default();
            if param_names.contains(arg) || is_plain_int_literal(arg) {
                continue;
            }
            return true;
        }
    }
    false
}

/// Build the stdout summary lines, mirroring the Python `main` print block.
fn build_summary(
    classification: &BTreeMap<String, BTreeMap<String, String>>,
    tranche: &[String],
    mirror_mismatch: &BTreeMap<String, String>,
    required_v1: &[&str],
    required_v2: &[&str],
) -> Vec<String> {
    let count_server = |val: &str| {
        classification
            .values()
            .filter(|c| c["server"] == val)
            .count()
    };
    let n_total = classification.len();
    let server_simple = count_server("simple");
    let server_v2 = count_server("v2-simple");
    let no_branch = classification
        .values()
        .filter(|c| c["client"] == "no_client_branch")
        .count();
    let mut v2_tranche: Vec<&String> = tranche
        .iter()
        .filter(|n| classification[*n]["server"] == "v2-simple")
        .collect();
    v2_tranche.sort();

    let mut lines = vec![
        format!("encoders parsed: {n_total}"),
        format!("server-simple (v1): {server_simple}"),
        format!("server-v2-simple: {server_v2}"),
        format!("no_client_branch: {no_branch}"),
        format!(
            "tranche size: {} (v1 {} + v2 {})",
            tranche.len(),
            tranche.len() - v2_tranche.len(),
            v2_tranche.len()
        ),
        format!("mirror_mismatch: {}", mirror_mismatch.len()),
    ];
    for (nm, why) in mirror_mismatch {
        lines.push(format!("  MIRROR_MISMATCH {nm}: {why}"));
    }
    let v2_names: Vec<&str> = v2_tranche.iter().map(|s| s.as_str()).collect();
    lines.push(format!("v2 tranche: {}", python_str_list(&v2_names)));
    lines.push(format!(
        "required v1 present: {}",
        python_str_list(required_v1)
    ));
    lines.push(format!(
        "required v2 present: {}",
        python_str_list(required_v2)
    ));
    lines
}

/// Run the `survey-payloads` subcommand: classify, gate, write both JSON
/// documents, and print the summary.
pub fn run_survey(opts: &SurveyPayloadsOpts<'_>) -> Result<()> {
    let output = survey(opts)?;
    fs::create_dir_all(opts.out_dir)
        .with_context(|| format!("failed to create {}", opts.out_dir.display()))?;
    let class_path = opts.out_dir.join("payload-classification.json");
    fs::write(&class_path, &output.classification)
        .with_context(|| format!("failed to write {}", class_path.display()))?;
    let payloads_path = opts.out_dir.join("payloads.json");
    fs::write(&payloads_path, &output.payloads)
        .with_context(|| format!("failed to write {}", payloads_path.display()))?;
    for line in &output.summary {
        println!("{line}");
    }
    Ok(())
}
