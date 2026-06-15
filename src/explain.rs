//! `explain-interface N` — decode an interface to a per-component table plus its
//! upward dependency closure.
//!
//! The table projects `{index, type, textfont, colour, ops, bounds, text}` per
//! component; the `requires:` closure lists fonts / sprites / scripts / enums /
//! dbtables… / child interfaces the interface references.
//!
//! Reuses the existing component port ([`crate::interface::parse_component`] /
//! [`crate::interface::parse_component_deps`]) and the self-describing raw
//! interface-group decoder in [`crate::interface::component`]. The interface
//! group is read from a runtime single-file `.js5` pack (the same source the
//! `font` subcommand reads for font discovery), so no flat cache is required.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::cache::FlatCache;
use crate::constants::ARCHIVE_CLIENTSCRIPTS;
use crate::error::{Context, Result};
use crate::explain_transitive::{
    MapScriptSource, SetRoster, TransitiveScripts, script_callees, transitive_script_closure,
};
use crate::interface::component::{ExplainedInterface, decode_interface_group_raw, explain_interface_group};
use crate::js5pack::PackArchive;
use crate::script::{OpcodeBook, ScriptArgSignature, decode_script, decode_script_arg_signature};

/// Default runtime pack root (the 910-base / 948-overlay pack the client runs).
/// Relative to the crate working directory, mirroring the `font` subcommand.
pub const DEFAULT_PACK_ROOT: &str = "../../server/data/pack-910-base-948-overlay";
/// Interfaces single-file pack name inside the runtime pack root.
const INTERFACES_PACK: &str = "client.interfaces.js5";
/// Clientscripts single-file pack name inside a pack root (the 910-base roster).
const SCRIPTS_PACK: &str = "client.scripts.js5";

/// Default 910-base pack root holding the pristine `client.scripts.js5` roster
/// used to compute the "missing from 910 base" splice burden. The overlay pack
/// is NOT a valid roster here: it down-codes only the already-spliced donor
/// scripts, so an interface's un-ported deps would look (falsely) present.
pub const DEFAULT_BASE_PACK_ROOT: &str = "../../server/data/pack-910-base";

/// Where to source the interface group bytes.
pub enum InterfaceSource<'a> {
    /// Read group `N` from the interfaces `.js5` pack under this root.
    Pack(&'a Path),
    /// Decode a raw interface-group `.dat` (JS5 raw group with version trailer).
    RawDat(&'a Path),
}

/// Options for [`run`].
pub struct ExplainInterfaceOptions<'a> {
    /// Interface group id to explain.
    pub interface: u32,
    /// Build number to decode components at (interfaces archive is build-keyed).
    pub build: u32,
    /// Group byte source.
    pub source: InterfaceSource<'a>,
    /// Emit JSON instead of the human table.
    pub json: bool,
    /// When set, additionally compute and report the FULL transitive clientscript
    /// closure of the interface's component-bound scripts and the count missing
    /// from the 910 base (the splice burden). Sourced from [`Self::transitive`].
    pub transitive: Option<TransitiveOptions<'a>>,
}

/// Where `--transitive` sources its script call graph and 910-base roster.
///
/// The donor (948) script graph comes from a flat cache archive 12 (the
/// authoritative, un-down-coded script bodies); the 910-base roster comes from a
/// `.js5` scripts pack. Both default to the in-repo locations in the CLI.
pub struct TransitiveOptions<'a> {
    /// Flat cache dir holding the donor clientscripts (archive 12) to walk.
    pub scripts_cache: &'a Path,
    /// Build to decode the donor scripts at (e.g. 948 for the donor cache).
    pub scripts_build: u32,
    /// Sub-build for the donor opcode book.
    pub scripts_subbuild: u32,
    /// `data/` dir for the opcode book the donor scripts decode under.
    pub data_dir: &'a Path,
    /// Pack root holding the pristine 910 `client.scripts.js5` roster.
    pub base_pack_root: &'a Path,
}

/// Decode + explain an interface group from the chosen source. Discards any
/// donor pack-root fallback note (see [`explain_with_note`]).
pub fn explain(opts: &ExplainInterfaceOptions<'_>) -> Result<ExplainedInterface> {
    Ok(explain_with_note(opts)?.0)
}

/// Decode + explain an interface group, returning the explanation plus an
/// optional pack-root fallback note. When the source is the runtime pack and it
/// lacks the requested interface, the donor pack is used automatically (the note
/// records that) so donor-only ids (e.g. 1224) work without an explicit
/// `--pack-root`. The raw-`.dat` source never falls back (the bytes are given).
pub fn explain_with_note(
    opts: &ExplainInterfaceOptions<'_>,
) -> Result<(ExplainedInterface, Option<String>)> {
    match opts.source {
        InterfaceSource::Pack(root) => {
            let resolved = crate::pack_root::resolve(root, INTERFACES_PACK, opts.interface)?;
            let pack = PackArchive::open(&resolved.path)
                .with_context(|| format!("open interfaces pack {}", resolved.path.display()))?;
            let files = pack.group_files(opts.interface)?.ok_or_else(|| {
                crate::error::CacheError::message(format!(
                    "interface {} absent in {}",
                    opts.interface,
                    resolved.path.display()
                ))
            })?;
            Ok((
                explain_interface_group(opts.interface, &files, opts.build)?,
                resolved.note,
            ))
        }
        InterfaceSource::RawDat(path) => {
            let raw = std::fs::read(path)
                .with_context(|| format!("read raw interface group {}", path.display()))?;
            Ok((
                decode_interface_group_raw(&raw, opts.build)?.explain(opts.interface)?,
                None,
            ))
        }
    }
}

/// Resolve the interfaces pack path for a root (for diagnostics / callers).
#[must_use]
pub fn interfaces_pack_path(root: &Path) -> PathBuf {
    root.join(INTERFACES_PACK)
}

/// Load the 910-base script roster from a `.js5` scripts pack — the canonical
/// group ids present, each with its declared arg signature (for collision
/// detection). The signature is read book-free from each script's header (see
/// [`decode_script_arg_signature`]); a group whose header cannot be read is kept
/// in the roster with a `None` signature, so it still prunes by id but never
/// produces a false collision.
///
/// `version` is the build the headers are read at; the arg-count fields are
/// build-invariant for builds ≥ 642, so the donor `scripts_build` is used here
/// for both sides without needing a separate 910 build number.
fn base_roster_from_pack(base_pack_root: &Path, version: u32) -> Result<SetRoster> {
    let pack_path = base_pack_root.join(SCRIPTS_PACK);
    let pack = PackArchive::open(&pack_path)
        .with_context(|| format!("open 910-base scripts pack {}", pack_path.display()))?;
    // A group present in the index but with a zero-length container still ships
    // no script bytes; only count groups that actually carry a container.
    let mut groups: BTreeMap<u32, Option<ScriptArgSignature>> = BTreeMap::new();
    for group in pack.group_ids() {
        // Each clientscripts group ships exactly one script (its min file).
        let Some(files) = pack.group_files(group)? else {
            continue;
        };
        let Some((_, data)) = files.into_iter().min_by_key(|(file, _)| *file) else {
            continue;
        };
        // A header that cannot be read leaves the signature unknown (None): the
        // group still prunes by id, but the collision check stays conservative.
        let sig = decode_script_arg_signature(&data, version).ok();
        groups.insert(group, sig);
    }
    Ok(SetRoster::with_signatures(groups))
}

/// Build a [`MapScriptSource`] over a flat cache's clientscripts archive: read
/// every group's single-script bytes once up front, then decode call edges (and
/// arg signatures) on demand (memoised by [`MapScriptSource`]).
///
/// Each clientscripts group ships exactly one script, so the canonical key is the
/// group id and the raw→group re-keying in [`MapScriptSource`] is exact. The raw
/// bytes (a few MB) are shared (via [`Rc`]) by the callee and signature decoders,
/// avoiding a second copy and any re-open of the cache per visited group.
// reason: the return type carries two distinct `impl Fn` closure types that cannot
// be named in a `type` alias (no stable type-alias-impl-trait); used once here.
#[allow(clippy::type_complexity)]
fn donor_script_source(
    cache: &FlatCache,
    opcode_book: OpcodeBook,
    build: u32,
) -> Result<MapScriptSource<impl Fn(u32) -> Vec<i32>, impl Fn(u32) -> Option<ScriptArgSignature>>> {
    let index = cache
        .archive_index(ARCHIVE_CLIENTSCRIPTS)
        .context("read clientscripts archive index for transitive walk")?;
    // group id -> the group's single (min-file) raw script bytes, shared between
    // the two lazy decoders.
    let mut map: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for &group in &index.group_id {
        let files = cache
            .group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, group)
            .with_context(|| format!("read clientscripts group {group} for transitive walk"))?;
        if let Some((_, data)) = files.into_iter().min_by_key(|(file, _)| *file) {
            map.insert(group, data);
        }
    }
    let bytes = std::rc::Rc::new(map);
    let present: BTreeSet<u32> = bytes.keys().copied().collect();

    let callee_bytes = std::rc::Rc::clone(&bytes);
    let decoder = move |group: u32| -> Vec<i32> {
        let Some(data) = callee_bytes.get(&group) else {
            return Vec::new();
        };
        match decode_script(data, &opcode_book, build) {
            Ok(script) => script_callees(&script),
            Err(_) => Vec::new(),
        }
    };

    let sig_bytes = std::rc::Rc::clone(&bytes);
    let sig_decoder = move |group: u32| -> Option<ScriptArgSignature> {
        // Book-free: the arg signature is read straight from the script header.
        let data = sig_bytes.get(&group)?;
        decode_script_arg_signature(data, build).ok()
    };

    Ok(MapScriptSource::new(present, decoder, sig_decoder))
}

/// Compute the transitive script closure + splice burden for an explained
/// interface, given the depth-1 component-bound script ids (raw).
pub fn compute_transitive(
    depth1_scripts: &BTreeSet<u32>,
    opts: &TransitiveOptions<'_>,
) -> Result<TransitiveScripts> {
    // Read base headers at the donor build: the arg-count fields are
    // build-invariant for builds ≥ 642, so one version serves both sides.
    let base = base_roster_from_pack(opts.base_pack_root, opts.scripts_build)?;
    let cache = FlatCache::open(opts.scripts_cache).with_context(|| {
        format!(
            "open donor scripts cache {} for transitive walk",
            opts.scripts_cache.display()
        )
    })?;
    let opcode_book = OpcodeBook::load(opts.data_dir, opts.scripts_build, opts.scripts_subbuild)
        .context("load opcode book for transitive script decode")?;
    let source = donor_script_source(&cache, opcode_book, opts.scripts_build)?;
    let seeds = depth1_scripts.iter().map(|&id| id as i32);
    Ok(transitive_script_closure(seeds, &source, &base))
}

/// Run the command: decode, then print either JSON or the human table. When
/// `--transitive` is set, also compute and append the transitive closure block.
pub fn run(opts: &ExplainInterfaceOptions<'_>) -> Result<()> {
    let (explained, pack_note) = explain_with_note(opts)?;
    if let Some(note) = pack_note {
        eprintln!("{note}");
    }
    let transitive = opts
        .transitive
        .as_ref()
        .map(|t| compute_transitive(&explained.requires.scripts, t))
        .transpose()?;

    if opts.json {
        let value = build_json(&explained, transitive.as_ref())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&value).context("encode explain JSON")?
        );
    } else {
        print!("{}", render_human(&explained));
        if let Some(transitive) = &transitive {
            print!("{}", render_transitive_human(&explained, transitive));
        }
    }
    Ok(())
}

/// Build the `--json` document: the explained interface, plus a `transitive`
/// section when `--transitive` ran. Kept additive so existing `--json` consumers
/// (which read the interface fields) are unaffected.
fn build_json(
    explained: &ExplainedInterface,
    transitive: Option<&TransitiveScripts>,
) -> Result<serde_json::Value> {
    let mut value = serde_json::to_value(explained).context("encode explained interface")?;
    if let Some(transitive) = transitive {
        let obj = value
            .as_object_mut()
            .context("explained interface JSON must be an object")?;
        obj.insert(
            "transitive".to_string(),
            serde_json::json!({
                "depth1_scripts": explained.requires.scripts.len(),
                "transitive_scripts": transitive.closure_len(),
                "missing_from_910": transitive.missing_len(),
                "missing_ids": transitive.missing_from_910.iter().copied().collect::<Vec<_>>(),
                "closure_ids": transitive.closure.iter().copied().collect::<Vec<_>>(),
                "unresolved_ids": transitive.unresolved.iter().copied().collect::<Vec<_>>(),
                "collisions": transitive.collision_len(),
                "collision_ids": transitive.collisions.keys().copied().collect::<Vec<_>>(),
                // Per-collision detail: the id plus both arg signatures so a
                // consumer can see exactly why a remap is required.
                "collision_detail": transitive
                    .collisions
                    .values()
                    .map(|c| serde_json::json!({
                        "id": c.group,
                        "donor_args": c.donor.display(),
                        "base_args": c.base.display(),
                    }))
                    .collect::<Vec<_>>(),
            }),
        );
    }
    Ok(value)
}

/// Render the human-readable transitive closure block appended after the
/// component table when `--transitive` is set.
#[must_use]
pub fn render_transitive_human(
    explained: &ExplainedInterface,
    transitive: &TransitiveScripts,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "transitive scripts:");
    let _ = writeln!(
        out,
        "  depth-1 component-bound: {}",
        explained.requires.scripts.len()
    );
    let _ = writeln!(
        out,
        "  transitive closure:      {} ({} missing from 910 base, {} collision(s))",
        transitive.closure_len(),
        transitive.missing_len(),
        transitive.collision_len()
    );
    if !transitive.missing_from_910.is_empty() {
        let ids = transitive
            .missing_from_910
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  missing ids [{}]: {}", transitive.missing_len(), ids);
    }
    // Proc-id collisions: same id in 910 but a DIFFERENT proc (arg signature
    // differs). These are NOT pruned — they need an explicit remap, so call each
    // one out with both signatures (the silent root cause of stack-underflow
    // saga when a `gosub_with_params <id>` resolves the wrong 910 proc).
    if !transitive.collisions.is_empty() {
        let _ = writeln!(
            out,
            "  collisions [{}] (present in 910 but signature differs — REMAP required):",
            transitive.collision_len()
        );
        for c in transitive.collisions.values() {
            let _ = writeln!(
                out,
                "    {}: present in 910 but signature differs — 948 args={} vs 910 args={} → COLLISION, remap required",
                c.group,
                c.donor.display(),
                c.base.display()
            );
        }
    }
    if !transitive.unresolved.is_empty() {
        let ids = transitive
            .unresolved
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "  unresolved refs [{}]: {}",
            transitive.unresolved.len(),
            ids
        );
    }
    out
}

/// Render the human component table + `requires:` closure block.
#[must_use]
pub fn render_human(explained: &ExplainedInterface) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "interface {} — {} component(s)",
        explained.interface, explained.component_count
    );
    // Header row, pre-aligned to the same column widths used per component
    // below ({:>5}  {:<14} {:>8}  {:<10} {:<22} …). Written as one literal so
    // there are no format arguments (clippy::write_literal).
    out.push_str(
        "  idx  type               font  colour     bounds(x,y,w,h)        text/ops\n",
    );
    for c in &explained.components {
        // Skip empty/legacy components (no type body) to keep the table focused
        // on the meaningful rows, matching how the relic scan read the group.
        if c.component_type == "unsupported" && c.text.is_none() && c.ops.is_empty() {
            continue;
        }
        let font = c.textfont.map_or_else(|| "-".to_string(), |f| f.to_string());
        let colour = c.colour.clone().unwrap_or_else(|| "-".to_string());
        let bounds = format!(
            "{},{},{},{}",
            c.bounds[0], c.bounds[1], c.bounds[2], c.bounds[3]
        );
        let mut tail = String::new();
        if let Some(text) = &c.text {
            let _ = write!(tail, "\"{}\"", truncate(text, 32));
        }
        if !c.ops.is_empty() {
            if !tail.is_empty() {
                tail.push(' ');
            }
            let _ = write!(tail, "ops[{}]", c.ops.join("|"));
        }
        if let Some(name) = &c.name {
            if !tail.is_empty() {
                tail.push(' ');
            }
            let _ = write!(tail, "<{name}>");
        }
        let _ = writeln!(
            out,
            "{:>5}  {:<14} {:>8}  {:<10} {:<22} {}",
            c.index, c.component_type, font, colour, bounds, tail
        );
    }

    let r = &explained.requires;
    out.push_str("requires:\n");
    write_id_line(&mut out, "fonts", &r.fonts);
    write_id_line(&mut out, "sprites", &r.sprites);
    write_id_line(&mut out, "scripts", &r.scripts);
    write_id_line(&mut out, "enums", &r.enums);
    write_id_line(&mut out, "models", &r.models);
    write_id_line(&mut out, "seqs", &r.seqs);
    write_id_line(&mut out, "params", &r.params);
    write_id_line(&mut out, "invs", &r.invs);
    write_id_line(&mut out, "stats", &r.stats);
    write_id_line(&mut out, "cursors", &r.cursors);
    write_id_line(&mut out, "stylesheets", &r.stylesheets);
    write_id_line(&mut out, "textures", &r.textures);
    write_id_line(&mut out, "varbits", &r.varbits);
    write_id_line(&mut out, "child_interfaces", &r.child_interfaces);
    if !r.vars.is_empty() {
        let _ = writeln!(
            out,
            "  {:<16} [{}] {}",
            "vars",
            r.vars.len(),
            r.vars.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    out
}

/// Emit one `requires:` line for a non-empty id set.
fn write_id_line(out: &mut String, label: &str, ids: &std::collections::BTreeSet<u32>) {
    if ids.is_empty() {
        return;
    }
    let joined = ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "  {:<16} [{}] {}", label, ids.len(), joined);
}

/// Truncate a string to `max` chars with an ellipsis when longer.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.replace('\n', " ");
    }
    let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", truncated.replace('\n', " "))
}

/// Convenience: default pack-rooted options for interface `id` at the runtime
/// build, JSON off. Used by the CLI dispatch.
#[must_use]
pub fn default_pack_root() -> PathBuf {
    PathBuf::from(DEFAULT_PACK_ROOT)
}

/// The default 910-base scripts pack root (the splice-burden roster source).
#[must_use]
pub fn default_base_pack_root() -> PathBuf {
    PathBuf::from(DEFAULT_BASE_PACK_ROOT)
}
