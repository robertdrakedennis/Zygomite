//! `explain-interface` support: per-component projection + closure aggregation.
//!
//! Projects the linear `Component.decode` field order (source of truth: the
//! client `com/jagex/game/config/iftype/Component.java` `decode`, ported verbatim
//! in [`super::parse_component`] / [`super::parse_component_deps`]) into the
//! explain shape, plus a self-describing raw interface-group decoder so the
//! closure can be computed from a committed `.dat` oracle with no archive index.
//!
//! This module deliberately does NOT re-derive the byte layout. It reuses the
//! existing port: [`super::parse_component`] for the human `.if3` field lines
//! (projected here into [`ExplainComponent`]) and [`super::parse_component_deps`]
//! for the dependency sets (aggregated here into [`InterfaceClosure`]).

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::{Context, Result};
use crate::interface::{ComponentDeps, VarTransmitRef, parse_component, parse_component_deps};
use crate::js5::decompress;
use crate::packet::Packet;

/// One component projected to the fields the plan calls out:
/// `{index, type, textfont, colour, ops, bounds, text}`.
///
/// Values are projected from the stable `key=value` lines emitted by
/// [`super::parse_component`]. Absent keys map to `None` / empty.
#[derive(Clone, Debug, Serialize)]
pub struct ExplainComponent {
    /// Component (file) index within the interface group.
    pub index: u32,
    /// Component type name (`layer`, `text`, `button`, …).
    pub component_type: String,
    /// Optional author-given component name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Font metrics id referenced by the component's text part, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub textfont: Option<u32>,
    /// Primary colour literal (as emitted, e.g. `0xff981f`), when non-default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
    /// Right-click op labels (`op1..opN`), in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<String>,
    /// `[x, y, width, height]` as emitted (0 where the field was suppressed).
    pub bounds: [i32; 4],
    /// Static text the component renders, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl ExplainComponent {
    /// Project the stable `.if3` field lines from [`super::parse_component`] into
    /// the explain shape. The first line is the `[comN]` header; the remainder
    /// are `key=value`.
    fn from_lines(index: u32, lines: &[String]) -> Self {
        let mut component_type = "unsupported".to_string();
        let mut name = None;
        let mut textfont = None;
        let mut colour = None;
        let mut ops: Vec<(u32, String)> = Vec::new();
        let mut bounds = [0_i32; 4];
        let mut text = None;

        for line in lines {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            // `field_leaf` is the part after the last `.`: bare `colour` and
            // prefixed `text.colour` both resolve to `colour`.
            let leaf = field_leaf(key);
            match leaf {
                "type" => component_type = value.to_string(),
                "name" => name = Some(value.to_string()),
                // textfont appears either bare (text components) or prefixed
                // (`text.textfont` on buttons/checks/inputs/lists). Take the
                // first font we see; `fontmetrics_<id>` / `graphic_<id>`.
                "textfont" if textfont.is_none() => textfont = parse_font_ref(value),
                // The component's own primary colour. Prefer the bare `colour`,
                // but fall back to the text-part colour for text-bearing widgets.
                "colour" if key == "colour" => colour = Some(value.to_string()),
                "colour" if colour.is_none() => colour = Some(value.to_string()),
                "x" if key == "x" => bounds[0] = value.parse().unwrap_or(0),
                "y" if key == "y" => bounds[1] = value.parse().unwrap_or(0),
                "width" if key == "width" => bounds[2] = value.parse().unwrap_or(0),
                "height" if key == "height" => bounds[3] = value.parse().unwrap_or(0),
                // Static text: bare `text` (type-4 text) or `text.text` (widgets).
                "text" if text.is_none() => text = Some(value.to_string()),
                _ if key.starts_with("op") => {
                    if let Some(n) = key.strip_prefix("op").and_then(|r| r.parse::<u32>().ok()) {
                        ops.push((n, value.to_string()));
                    }
                }
                _ => {}
            }
        }

        ops.sort_by_key(|(n, _)| *n);
        let ops = ops.into_iter().map(|(_, v)| v).collect();

        Self {
            index,
            component_type,
            name,
            textfont,
            colour,
            ops,
            bounds,
            text,
        }
    }
}

/// The trailing segment of a dotted `.if3` field key: `text.colour` → `colour`,
/// bare `colour` → `colour`. Lets bare and text-part fields share one match arm.
fn field_leaf(key: &str) -> &str {
    key.rsplit('.').next().unwrap_or(key)
}

/// Pull the numeric id out of a `fontmetrics_<id>` / `graphic_<id>` reference.
fn parse_font_ref(value: &str) -> Option<u32> {
    value
        .rsplit_once('_')
        .and_then(|(_, id)| id.parse::<u32>().ok())
}

/// The full dependency closure an interface references.
///
/// Aggregated across all of its components via [`super::parse_component_deps`];
/// every set is an upward closure of ids the interface needs to load/run.
#[derive(Clone, Debug, Default, Serialize)]
pub struct InterfaceClosure {
    /// `FontMetrics` ids (archive 13/58) used by text parts.
    pub fonts: BTreeSet<u32>,
    /// Sprite/graphic ids (archive 8) referenced by sprite parts.
    pub sprites: BTreeSet<u32>,
    /// CS2 script ids referenced by component hooks.
    pub scripts: BTreeSet<u32>,
    /// Enum ids referenced by components.
    pub enums: BTreeSet<u32>,
    /// Model ids referenced by model components.
    pub models: BTreeSet<u32>,
    /// Seq (animation) ids referenced by model/text components.
    pub seqs: BTreeSet<u32>,
    /// Param ids referenced in component param blocks.
    pub params: BTreeSet<u32>,
    /// Inv ids referenced by inv-transmit lists.
    pub invs: BTreeSet<u32>,
    /// Stat ids referenced by stat-transmit lists.
    pub stats: BTreeSet<u32>,
    /// Cursor ids referenced by op/target cursors.
    pub cursors: BTreeSet<u32>,
    /// Stylesheet ids referenced in the common tail.
    pub stylesheets: BTreeSet<u32>,
    /// Texture ids referenced by components.
    pub textures: BTreeSet<u32>,
    /// Var references (player/client/…) transmitted by components.
    pub vars: BTreeSet<String>,
    /// Varbit ids referenced by components.
    pub varbits: BTreeSet<u32>,
    /// Child interface group ids this interface layers in.
    pub child_interfaces: BTreeSet<u32>,
}

impl InterfaceClosure {
    /// Fold one component's [`ComponentDeps`] into the running closure.
    fn absorb(&mut self, deps: &ComponentDeps) {
        self.fonts.extend(deps.fontmetrics.iter().copied());
        self.sprites.extend(deps.graphics.iter().copied());
        self.scripts.extend(deps.scripts.iter().copied());
        self.enums.extend(deps.enums.iter().copied());
        self.models.extend(deps.models.iter().copied());
        self.seqs.extend(deps.seqs.iter().copied());
        self.params.extend(deps.params.iter().copied());
        self.invs.extend(deps.invs.iter().copied());
        self.stats.extend(deps.stats.iter().copied());
        self.cursors.extend(deps.cursors.iter().copied());
        self.stylesheets.extend(deps.stylesheets.iter().copied());
        self.textures.extend(deps.textures.iter().copied());
        self.varbits.extend(deps.varbits.iter().copied());
        for var in &deps.varps {
            self.vars.insert(var_transmit_label(var));
        }
    }
}

/// Render a transmitted var reference as a stable label for the closure.
fn var_transmit_label(var: &VarTransmitRef) -> String {
    match var {
        VarTransmitRef::Player(id) => format!("varplayer_{id}"),
        VarTransmitRef::Npc(id) => format!("varnpc_{id}"),
        VarTransmitRef::Client(id) => format!("varclient_{id}"),
        VarTransmitRef::World(id) => format!("varworld_{id}"),
        VarTransmitRef::Region(id) => format!("varregion_{id}"),
        VarTransmitRef::Object(id) => format!("varobject_{id}"),
        VarTransmitRef::Clan(id) => format!("varclan_{id}"),
        VarTransmitRef::ClanSetting(id) => format!("varclansetting_{id}"),
        VarTransmitRef::Controller(id) => format!("varcontroller_{id}"),
        VarTransmitRef::Global(id) => format!("varglobal_{id}"),
        VarTransmitRef::PlayerGroup(id) => format!("varplayergroup_{id}"),
        VarTransmitRef::VarClientString(id) => format!("varclientstring_{id}"),
    }
}

/// The decoded form of one interface group: every component projected, plus the
/// aggregated dependency closure.
#[derive(Clone, Debug, Serialize)]
pub struct ExplainedInterface {
    /// Interface group id.
    pub interface: u32,
    /// Number of components (files) in the group.
    pub component_count: usize,
    /// Per-component projection, in ascending component index.
    pub components: Vec<ExplainComponent>,
    /// Aggregated upward dependency closure.
    pub requires: InterfaceClosure,
}

/// Explain a whole interface group from its component file map.
///
/// Takes `component id → raw component bytes` at the given build. Components that
/// fail to decode are projected from whatever partial lines
/// [`super::parse_component`] produced and their (partial) deps still contribute
/// to the closure, matching the lenient behaviour of
/// [`super::parse_component_deps`].
pub fn explain_interface_group(
    interface: u32,
    files: &BTreeMap<u32, Vec<u8>>,
    build: u32,
) -> Result<ExplainedInterface> {
    let mut components = Vec::with_capacity(files.len());
    let mut requires = InterfaceClosure::default();

    for (&component_id, bytes) in files {
        // Human field lines → explain projection (lenient: keep partial lines).
        let lines = parse_component(component_id, bytes, build).unwrap_or_else(|err| {
            vec![
                format!("[com{}]", component_id & 0xFFFF),
                "type=unsupported".to_string(),
                format!("parse_error={}", err.to_string().replace('\n', " ")),
            ]
        });
        components.push(ExplainComponent::from_lines(component_id, &lines));

        // Dependency sets → closure (already lenient internally).
        // NB: `child_interfaces` is intentionally left empty — interfaces
        // compose by group, not by embedding a child group id in a component, so
        // there is no child-group reference to recover from one component's
        // bytes. The field documents the category for a future cross-group walk.
        let deps = parse_component_deps(component_id, bytes, build)?;
        requires.absorb(&deps);
    }

    Ok(ExplainedInterface {
        interface,
        component_count: components.len(),
        components,
        requires,
    })
}

/// Decode a raw interface-group `.dat` into its component file map.
///
/// The `.dat` is a JS5 raw group (a JS5 container plus a trailing 2-byte
/// big-endian version) and is decoded WITHOUT an archive index, by inferring the
/// file count from the self-consistent chunk footer.
///
/// A multi-file JS5 group payload is `concat(file_bytes…) ++ footer ++ marker`,
/// where `footer` is `marker × file_count` big-endian `i32` size deltas and
/// `marker` (the last byte) is the chunk count. For a candidate `file_count`,
/// the cumulative deltas must be non-negative and sum to exactly the body length
/// (`payload_len − 1 − footer_len`). Exactly one `file_count` satisfies this for
/// a real group, so the index is recoverable. Component ids are taken as the
/// contiguous range `0..file_count` (the interfaces archive numbers components
/// densely, as every shipped interface group does).
pub fn decode_interface_group_raw(raw_group: &[u8], build: u32) -> Result<ExplainedGroupRaw> {
    if raw_group.len() < 2 {
        return Err(crate::error::CacheError::message(
            "raw interface group too short to hold a version trailer",
        ));
    }
    // Strip the 2-byte version trailer, then JS5-decompress the container.
    let payload = decompress(&raw_group[..raw_group.len() - 2])
        .context("decompress raw interface group container")?;
    let files = unpack_indexless_group(&payload)?;
    Ok(ExplainedGroupRaw { build, files })
}

/// A raw interface group decoded into `component id → bytes`, deferring the
/// per-build component parse to [`Self::explain`].
pub struct ExplainedGroupRaw {
    build: u32,
    files: BTreeMap<u32, Vec<u8>>,
}

impl ExplainedGroupRaw {
    /// Component file map (component id → raw bytes).
    #[must_use]
    pub const fn files(&self) -> &BTreeMap<u32, Vec<u8>> {
        &self.files
    }

    /// Run the explain projection + closure over the decoded components.
    pub fn explain(&self, interface: u32) -> Result<ExplainedInterface> {
        explain_interface_group(interface, &self.files, self.build)
    }
}

/// Unpack a JS5 group payload into its files when the archive index is not
/// available, inferring the file count from the self-consistent chunk footer.
/// See [`decode_interface_group_raw`] for the layout contract.
fn unpack_indexless_group(payload: &[u8]) -> Result<BTreeMap<u32, Vec<u8>>> {
    let marker = usize::from(
        *payload
            .last()
            .context("interface group payload missing chunk marker")?,
    );
    if marker == 0 {
        return Err(crate::error::CacheError::message(
            "interface group chunk marker is zero",
        ));
    }
    let body = &payload[..payload.len() - 1];

    let file_count = infer_file_count(body, marker)?;
    // Reconstruct per-file sizes from the footer (cumulative-delta encoded).
    let footer_len = marker * file_count * 4;
    let footer_off = body.len() - footer_len;

    let mut sizes = vec![0_usize; file_count];
    let mut size_packet = Packet::with_pos(body, footer_off)?;
    for _ in 0..marker {
        let mut running = 0_i64;
        for size in &mut sizes {
            running += i64::from(size_packet.g4s()?);
            *size += usize::try_from(running).context("negative interface chunk size")?;
        }
    }

    // Slice the body (chunk-major, file-minor) into per-file buffers.
    let mut files: Vec<Vec<u8>> = sizes.iter().map(|&n| Vec::with_capacity(n)).collect();
    let mut data_packet = Packet::with_pos(body, footer_off)?;
    let mut read_pos = 0_usize;
    for _ in 0..marker {
        let mut running = 0_i64;
        for file in &mut files {
            running += i64::from(data_packet.g4s()?);
            let chunk = usize::try_from(running).context("negative interface chunk length")?;
            let end = read_pos + chunk;
            if end > footer_off {
                return Err(crate::error::CacheError::message(
                    "interface group chunk exceeds body length",
                ));
            }
            file.extend_from_slice(&body[read_pos..end]);
            read_pos = end;
        }
    }

    let mut out = BTreeMap::new();
    for (id, bytes) in files.into_iter().enumerate() {
        out.insert(u32::try_from(id).context("component id overflow")?, bytes);
    }
    Ok(out)
}

/// Find the unique file count whose cumulative chunk deltas are all
/// non-negative and sum exactly to the body length (body = payload minus the
/// trailing marker byte).
fn infer_file_count(body: &[u8], marker: usize) -> Result<usize> {
    let mut found: Option<usize> = None;
    let mut file_count = 1_usize;
    loop {
        let footer_len = marker
            .checked_mul(file_count)
            .and_then(|v| v.checked_mul(4))
            .context("interface footer size overflow")?;
        if footer_len > body.len() {
            break;
        }
        if footer_consistent(body, marker, file_count, footer_len) {
            if found.is_some() {
                return Err(crate::error::CacheError::message(
                    "interface group file count is ambiguous",
                ));
            }
            found = Some(file_count);
        }
        file_count += 1;
    }
    found.context("could not infer interface group file count from chunk footer")
}

/// Whether a candidate `file_count` yields a footer whose cumulative deltas are
/// non-negative and sum to exactly the body-minus-footer length.
fn footer_consistent(body: &[u8], marker: usize, file_count: usize, footer_len: usize) -> bool {
    let footer_off = body.len() - footer_len;
    let Ok(mut packet) = Packet::with_pos(body, footer_off) else {
        return false;
    };
    let mut total: i64 = 0;
    for _ in 0..marker {
        let mut running = 0_i64;
        for _ in 0..file_count {
            let Ok(delta) = packet.g4s() else {
                return false;
            };
            running += i64::from(delta);
            if running < 0 {
                return false;
            }
            total += running;
        }
    }
    total == i64::try_from(footer_off).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_component_projects_fields() {
        let lines = vec![
            "[com5]".to_string(),
            "type=text".to_string(),
            "name=title".to_string(),
            "x=12".to_string(),
            "y=4".to_string(),
            "width=100".to_string(),
            "height=20".to_string(),
            "textfont=fontmetrics_206".to_string(),
            "text=Relic Powers".to_string(),
            "colour=0xff981f".to_string(),
            "op2=Examine".to_string(),
            "op1=Activate".to_string(),
        ];
        let c = ExplainComponent::from_lines(5, &lines);
        assert_eq!(c.index, 5);
        assert_eq!(c.component_type, "text");
        assert_eq!(c.name.as_deref(), Some("title"));
        assert_eq!(c.textfont, Some(206));
        assert_eq!(c.colour.as_deref(), Some("0xff981f"));
        assert_eq!(c.bounds, [12, 4, 100, 20]);
        assert_eq!(c.text.as_deref(), Some("Relic Powers"));
        assert_eq!(c.ops, vec!["Activate".to_string(), "Examine".to_string()]);
    }

    #[test]
    fn explain_component_prefers_textpart_colour_fallback() {
        let lines = vec![
            "[com1]".to_string(),
            "type=button".to_string(),
            "text.textfont=fontmetrics_57".to_string(),
            "text.colour=0x00ff00".to_string(),
        ];
        let c = ExplainComponent::from_lines(1, &lines);
        assert_eq!(c.textfont, Some(57));
        assert_eq!(c.colour.as_deref(), Some("0x00ff00"));
    }
}
