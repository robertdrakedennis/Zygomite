//! Generic `decode --archive A --group G --format F` — pretty-print any group
//! in any known cache format, reading the runtime single-file `.js5` packs.
//!
//! This replaces the ad-hoc `bun -e` byte-probes the relic work leaned on. Every
//! format routes to an EXISTING decoder: the font toolkit
//! ([`crate::font::bitmap`] / [`crate::font::modern`]), the interface component
//! port ([`crate::interface`]), or the config parsers ([`crate::config`]). New
//! group formats are a single arm in [`Format`] + [`decode_payload`].

use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{Context, Result};
use crate::font::bitmap::{FontMetrics, GlyphAtlasSprite};
use crate::font::modern::{ExtractedFace, FaceKind, ModernFontRef};
use crate::js5pack::PackArchive;

/// Default runtime pack root, mirroring [`crate::explain::DEFAULT_PACK_ROOT`].
pub const DEFAULT_PACK_ROOT: &str = "../../server/data/pack-910-base-948-overlay";

/// The list of concrete `--format` values, for help text + error suggestions.
/// Order mirrors [`Format::from_str`].
pub const FORMAT_NAMES: &[&str] = &[
    "auto",
    "sprite",
    "fontmetrics",
    "fontmetrics2",
    "ttf",
    "interface",
    "dbtable",
    "dbrow",
    "enum",
    "struct",
    "param",
    "npc",
    "obj",
];

/// Known group formats the generic decoder can pretty-print.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    /// Infer the format from the archive id.
    Auto,
    /// Glyph-atlas sprite (archive 8).
    Sprite,
    /// 910 bitmap `FontMetrics` (archive 13).
    FontMetrics,
    /// 948 modern font reference (archive 58).
    FontMetrics2,
    /// Extracted TTF/OTF face header (archive 59).
    Ttf,
    /// Interface component group (archive 3).
    Interface,
    /// `DbTable` config (flat archive 40 / config archive-2 group 40).
    DbTable,
    /// `DbRow` config (flat archive 41 / config archive-2 group 41).
    DbRow,
    /// Enum config (archive 17).
    Enum,
    /// Struct config (archive 22).
    Struct,
    /// Param config (config archive-2 group 11).
    Param,
    /// NPC config (archive 18).
    Npc,
    /// Obj/item config (archive 19).
    Obj,
}

impl FromStr for Format {
    type Err = crate::error::CacheError;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "sprite" => Self::Sprite,
            "fontmetrics" => Self::FontMetrics,
            "fontmetrics2" => Self::FontMetrics2,
            "ttf" => Self::Ttf,
            "interface" => Self::Interface,
            "dbtable" => Self::DbTable,
            "dbrow" => Self::DbRow,
            "enum" => Self::Enum,
            "struct" => Self::Struct,
            "param" => Self::Param,
            "npc" => Self::Npc,
            "obj" | "item" => Self::Obj,
            other => {
                return Err(crate::error::CacheError::message(format!(
                    "unknown decode format '{other}' (use {})",
                    FORMAT_NAMES.join("|")
                )));
            }
        })
    }
}

impl Format {
    /// Resolve `auto` to a concrete format from the archive id, else identity.
    ///
    /// The archive id is the **948 flat-cache archive number** (the same number
    /// the `--cache-dir` flat tree uses), mapped here to the concrete format /
    /// `.js5` pack the group lives in. On a genuine unknown, the error lists the
    /// archive's likely type (when known) plus the full set of valid `--format`
    /// values to pass, instead of a bare "cannot infer".
    fn resolve(self, archive: u32) -> Result<Self> {
        if self != Self::Auto {
            return Ok(self);
        }
        if let Some(format) = Self::auto_for_archive(archive) {
            return Ok(format);
        }
        Err(crate::error::CacheError::message(format!(
            "decode --format auto cannot infer a format for archive {archive}{}; \
             pass --format <{}>",
            archive_hint(archive),
            FORMAT_NAMES
                .iter()
                .filter(|&&n| n != "auto")
                .copied()
                .collect::<Vec<_>>()
                .join("|"),
        )))
    }

    /// The default concrete format for a 948 flat-cache archive id, or `None`
    /// when the archive has no generic decoder yet. Centralised so `auto`
    /// inference and the help text stay in sync.
    #[must_use]
    pub fn auto_for_archive(archive: u32) -> Option<Self> {
        Some(match archive {
            8 => Self::Sprite,
            13 => Self::FontMetrics,
            58 => Self::FontMetrics2,
            59 => Self::Ttf,
            3 => Self::Interface,
            17 => Self::Enum,
            22 => Self::Struct,
            18 => Self::Npc,
            19 => Self::Obj,
            40 => Self::DbTable,
            41 => Self::DbRow,
            _ => return None,
        })
    }

    /// The `.js5` pack file (under a pack root) holding this format's groups.
    /// Distinct from the flat-cache archive id: e.g. dbtable/dbrow/param all
    /// live inside `client.config.js5` (as config groups 40/41/11), while npc
    /// and obj have their own `client.{npc,obj}.config.js5` packs.
    fn pack_file_name(self) -> Result<&'static str> {
        Ok(match self {
            Self::Sprite => "client.sprites.js5",
            Self::FontMetrics => "client.fontmetrics.js5",
            Self::FontMetrics2 => "client.fontmetrics2.js5",
            Self::Ttf => "client.ttf.js5",
            Self::Interface => "client.interfaces.js5",
            Self::DbTable | Self::DbRow | Self::Param => "client.config.js5",
            Self::Enum => "client.enum.config.js5",
            Self::Struct => "client.struct.config.js5",
            Self::Npc => "client.npc.config.js5",
            Self::Obj => "client.obj.config.js5",
            Self::Auto => {
                return Err(crate::error::CacheError::message(
                    "decode: format must be resolved before selecting a pack file",
                ));
            }
        })
    }

    /// For config formats packed inside `client.config.js5` keyed by config
    /// group (dbtable/dbrow/param), the canonical group id to read regardless of
    /// what the user passed as `--group` (so `--archive 40 --group 40` and
    /// `--archive 40` both find the dbtables). Other formats key by `--group`
    /// directly, so this returns `None` and the caller uses the requested group.
    #[must_use]
    fn fixed_config_group(self) -> Option<u32> {
        match self {
            Self::DbTable => Some(crate::constants::CONFIG_GROUP_DBTABLE),
            Self::DbRow => Some(crate::constants::CONFIG_GROUP_DBROW),
            Self::Param => Some(crate::constants::CONFIG_GROUP_PARAM),
            _ => None,
        }
    }

    /// Bit-shift packing an entity id into `(group, file)` for archives whose
    /// `.js5` packs split ids across groups (npc/obj/struct). The real entity id
    /// is `group << shift | file`; `0` means the file id IS the id (flat group).
    #[must_use]
    const fn id_shift(self) -> u32 {
        match self {
            Self::Npc => 7,    // 128 files/group
            Self::Obj => 8,    // 256 files/group
            Self::Struct => 5, // 32 files/group
            _ => 0,
        }
    }
}

/// A human hint for a 948 flat-cache archive id with no generic decoder,
/// appended to the `auto` failure so the user knows what the archive holds (and
/// thus whether a format exists / is worth adding). Empty for truly unknown ids.
fn archive_hint(archive: u32) -> String {
    let kind = match archive {
        2 => Some("config (use --group <config group: 40 dbtable / 41 dbrow / 11 param / …>)"),
        16 => Some("loc config (no generic decoder yet)"),
        20 => Some("seq config (no generic decoder yet)"),
        21 => Some("spot config (no generic decoder yet)"),
        12 => Some("clientscripts (use the `cs2` / `explain-interface` tooling)"),
        5 => Some("map squares (use the `unpack` map tooling)"),
        _ => None,
    };
    kind.map_or_else(String::new, |k| format!(" (archive {archive} is {k})"))
}

/// Options for [`run`].
pub struct DecodeOptions<'a> {
    /// Cache archive id (used for `auto` inference and reporting).
    pub archive: u32,
    /// Group id within the archive.
    pub group: u32,
    /// Requested format (or `auto`).
    pub format: Format,
    /// Runtime pack root.
    pub pack_root: &'a Path,
    /// Emit JSON instead of the human dump.
    pub json: bool,
}

/// Decode a group and print either a JSON value or a human dump. Any donor
/// pack-root fallback note is printed to stderr first (so JSON stdout stays
/// clean for piping).
pub fn run(opts: &DecodeOptions<'_>) -> Result<()> {
    let DecodeOutput { value, pack_note } = decode_with_note(opts)?;
    if let Some(note) = pack_note {
        eprintln!("{note}");
    }
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).context("encode decode JSON")?
        );
    } else {
        print!("{}", render_human(opts, &value));
    }
    Ok(())
}

/// The decoded group value plus any pack-root fallback note (for the CLI to
/// surface). Returned by [`decode_with_note`]; [`decode`] drops the note.
pub struct DecodeOutput {
    /// The decoded group as a JSON value.
    pub value: Value,
    /// `Some` when the donor pack served the group (the requested root lacked
    /// it). Already a complete human-readable message.
    pub pack_note: Option<String>,
}

/// Decode the requested group to a JSON value, discarding any fallback note.
/// Thin wrapper over [`decode_with_note`] for callers that don't need the note.
pub fn decode(opts: &DecodeOptions<'_>) -> Result<Value> {
    Ok(decode_with_note(opts)?.value)
}

/// Decode the requested group, routing on the resolved format and falling back
/// to the donor pack when the requested `--pack-root` lacks the group.
pub fn decode_with_note(opts: &DecodeOptions<'_>) -> Result<DecodeOutput> {
    let format = opts.format.resolve(opts.archive)?;
    let pack_file = format.pack_file_name()?;
    // Config formats packed by config-group (dbtable/dbrow/param) read their
    // canonical group regardless of `--group`; everything else keys by `--group`.
    let read_group = format.fixed_config_group().unwrap_or(opts.group);

    let resolved = crate::pack_root::resolve(opts.pack_root, pack_file, read_group)?;
    // Open with the sibling `.patch.js5` merged so a group present only in the
    // patch (e.g. an overlay-added enum group) decodes the same way it loads at
    // runtime, rather than reading as absent.
    let pack = PackArchive::open_with_patch(&resolved.path)
        .with_context(|| format!("open pack {}", resolved.path.display()))?;
    let files = pack.group_files(read_group)?.ok_or_else(|| {
        crate::error::CacheError::message(format!(
            "archive {} group {read_group} absent in {}",
            opts.archive,
            resolved.path.display()
        ))
    })?;

    let shift = format.id_shift();
    let mut entries = Vec::with_capacity(files.len());
    for (&file_id, bytes) in &files {
        // Reconstruct the real entity id for split archives (npc/obj/struct);
        // for flat groups the file id IS the id.
        let entity_id = if shift == 0 {
            file_id
        } else {
            (read_group << shift) | file_id
        };
        let decoded = decode_payload(format, entity_id, file_id, bytes)
            .with_context(|| format!("decode group {read_group} file {file_id}"))?;
        entries.push(decoded);
    }

    let value = json!({
        "archive": opts.archive,
        "group": read_group,
        "format": format_label(format),
        "pack": resolved.path.display().to_string(),
        "file_count": files.len(),
        "files": entries,
    });
    Ok(DecodeOutput {
        value,
        pack_note: resolved.note,
    })
}

/// Decode one file payload under a concrete (non-auto) format into JSON.
/// `entity_id` is the reconstructed real id (for split archives), `file_id` the
/// raw per-group file id (reported on font/sprite outputs that key by file).
fn decode_payload(format: Format, entity_id: u32, file_id: u32, bytes: &[u8]) -> Result<Value> {
    match format {
        Format::Sprite => {
            let sprite = GlyphAtlasSprite::decode(bytes, None)?;
            let inked = sprite.alpha.iter().filter(|&&a| a != 0).count();
            Ok(json!({
                "file": file_id,
                "width": sprite.width,
                "height": sprite.height,
                "inked_px": inked,
            }))
        }
        Format::FontMetrics => {
            let m = FontMetrics::decode(bytes)?;
            let ink = m.glyph_height.iter().filter(|&&h| h > 0).count();
            Ok(json!({
                "file": file_id,
                "version": m.version,
                "has_kerning": m.has_kerning,
                "atlas_w": m.atlas_w,
                "atlas_h": m.atlas_h,
                "ascent": m.ascent,
                "descent": m.descent,
                "line_height": m.line_height,
                "divisor": m.divisor,
                "ink_glyphs": ink,
            }))
        }
        Format::FontMetrics2 => {
            let r = ModernFontRef::decode(bytes)?;
            Ok(match r {
                ModernFontRef::TtfRef { face, size_px } => json!({
                    "file": file_id,
                    "fmt": 2,
                    "face": face,
                    "size_px": size_px,
                }),
                ModernFontRef::Fmt1Bitmap {
                    target_ascent,
                    ascent,
                    descent,
                    divisor,
                    line_height,
                } => json!({
                    "file": file_id,
                    "fmt": 1,
                    "target_ascent": target_ascent,
                    "ascent": ascent,
                    "descent": descent,
                    "divisor": divisor,
                    "line_height": line_height,
                }),
            })
        }
        Format::Ttf => {
            let face = ExtractedFace::from_payload(bytes.to_vec())?;
            let kind = match face.kind {
                FaceKind::TrueType => "truetype",
                FaceKind::OpenTypeCff => "opentype-cff",
            };
            Ok(json!({
                "file": file_id,
                "kind": kind,
                "bytes": face.bytes.len(),
            }))
        }
        Format::Interface => {
            // Build 947 is the donor numbering the runtime base decodes with.
            let lines = crate::interface::parse_component(file_id, bytes, crate::constants::BUILD)
                .unwrap_or_else(|e| {
                    vec![
                        format!("[com{}]", file_id & 0xFFFF),
                        format!("parse_error={e}"),
                    ]
                });
            Ok(json!({ "component": file_id, "fields": lines }))
        }
        Format::DbTable => to_json(&crate::config::parse_dbtable(entity_id, bytes)?),
        Format::DbRow => to_json(&crate::config::parse_dbrow(entity_id, bytes)?),
        Format::Enum => to_json(&crate::config::parse_enum(entity_id, bytes)?),
        Format::Struct => to_json(&crate::config::parse_struct(entity_id, bytes)?),
        Format::Param => to_json(&crate::config::parse_param(entity_id, bytes)?),
        Format::Npc => to_json(&crate::config::parse_npc(entity_id, bytes)?),
        Format::Obj => to_json(&crate::config::parse_obj(entity_id, bytes)?),
        Format::Auto => Err(crate::error::CacheError::message(
            "decode: auto format must be resolved before decode_payload",
        )),
    }
}

/// Serialize an existing `Serialize` config entry into a `serde_json::Value`.
fn to_json<T: Serialize>(entry: &T) -> Result<Value> {
    serde_json::to_value(entry).context("serialize config entry")
}

/// Human label for a resolved format.
const fn format_label(format: Format) -> &'static str {
    match format {
        Format::Auto => "auto",
        Format::Sprite => "sprite",
        Format::FontMetrics => "fontmetrics",
        Format::FontMetrics2 => "fontmetrics2",
        Format::Ttf => "ttf",
        Format::Interface => "interface",
        Format::DbTable => "dbtable",
        Format::DbRow => "dbrow",
        Format::Enum => "enum",
        Format::Struct => "struct",
        Format::Param => "param",
        Format::Npc => "npc",
        Format::Obj => "obj",
    }
}

/// Render a decoded group value as a compact human dump.
fn render_human(opts: &DecodeOptions<'_>, value: &Value) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let format = value.get("format").and_then(Value::as_str).unwrap_or("?");
    let count = value.get("file_count").and_then(Value::as_u64).unwrap_or(0);
    // Prefer the group actually read (config formats override `--group` with
    // their canonical config group, e.g. dbtable → 40).
    let group = value
        .get("group")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| u64::from(opts.group));
    let _ = writeln!(
        out,
        "archive {} group {group} — format {format}, {count} file(s)",
        opts.archive
    );
    if let Some(files) = value.get("files").and_then(Value::as_array) {
        for f in files {
            // One compact line per file: its key/value scalars.
            let mut parts = Vec::new();
            if let Some(map) = f.as_object() {
                for (k, v) in map {
                    if k == "fields" {
                        // Interface components: print each field line indented.
                        continue;
                    }
                    parts.push(format!("{k}={}", scalar(v)));
                }
            }
            let _ = writeln!(out, "  {}", parts.join(" "));
            if let Some(fields) = f.get("fields").and_then(Value::as_array) {
                for line in fields {
                    if let Some(s) = line.as_str() {
                        let _ = writeln!(out, "      {s}");
                    }
                }
            }
        }
    }
    out
}

/// Render a JSON scalar compactly (arrays/objects fall back to JSON).
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// Convenience default pack root path for the CLI dispatch.
#[must_use]
pub fn default_pack_root() -> PathBuf {
    PathBuf::from(DEFAULT_PACK_ROOT)
}
