//! Interface port driver (plan §9 step 5): re-encode a donor (948) interface
//! group onto the 910 client through the typed interface IR.
//!
//! ```text
//! donor component bytes ─decode→ InterfaceIr ─lower(list_to_server_driven)→ IR′ ─encode(validate)→ 910 .dat
//! ```
//!
//! This is the interface analogue of [`crate::port::ritual`]: the front-end
//! ([`InterfaceIr::from_donor_files`]) lifts the donor components to typed IR, the
//! named lowering ([`lower::list_to_server_driven`]) downcodes the composite
//! widgets the 910 client lacks, and the back-end ([`encode::encode_group`])
//! validates representability + re-decodes every component through the 910 mirror,
//! then re-packs the group `.dat`.
//!
//! The byte-exact oracle is the committed `1224-910.dat` (sha-pinned in
//! `CacheOverlay.ts`): `port_interface_group` reproduces it byte-for-byte, gating
//! the routing of `interface transcode` through this path.

use std::collections::BTreeMap;

use crate::error::Result;
use crate::port::book::BuildDescriptor;
use crate::port::encode::interface::{self as encode, EncodedGroup};
use crate::port::ir::interface::{ComponentKind, InterfaceIr};
use crate::port::lower::interface::{self as lower, Downcoded};

/// The result of porting one interface group: the encoded 910 group plus the
/// per-component downcode report.
pub struct PortedInterface {
    /// The encoded group (per-component bytes, body, `.dat`, roster).
    pub group: EncodedGroup,
    /// Per-component downcode disposition, in ascending component id.
    pub downcodes: Vec<(u32, Downcoded)>,
    /// The component count.
    pub component_count: usize,
}

impl PortedInterface {
    /// The components that were downcoded (rewritten composite widgets).
    #[must_use]
    pub fn rewritten(&self) -> Vec<(u32, Downcoded)> {
        self.downcodes
            .iter()
            .copied()
            .filter(|(_, d)| !matches!(d, Downcoded::Kept))
            .collect()
    }
}

/// Port a whole donor interface group's component file map through the IR layer.
///
/// * `group` — the interface group id (for diagnostics / output naming).
/// * `files` — the donor component file map (dense `0..n` ids).
/// * `decode_build` — the layout the donor components decode at (947/948).
/// * `target` — the 910 build descriptor (capabilities gate the lowering/encode).
/// * `version` — the JS5 group version written into the `.dat` trailer (9).
pub fn port_interface_group(
    group: u32,
    files: &BTreeMap<u32, Vec<u8>>,
    decode_build: u32,
    target: &BuildDescriptor,
    version: u16,
) -> Result<PortedInterface> {
    let component_count = files.len();
    // 1) decode: donor bytes → typed IR.
    let mut ir = InterfaceIr::from_donor_files(group, files, decode_build)?;
    // 2) lower: composite widgets the target lacks → server-driven primitives.
    let report = lower::list_to_server_driven(&mut ir, target)?;
    // 3) encode: IR → 910 bytes (validating) + re-pack `.dat`.
    let encoded = encode::encode_group(&ir, target, version)?;

    let downcodes: Vec<(u32, Downcoded)> = report
        .into_iter()
        .enumerate()
        .map(|(i, d)| {
            #[allow(clippy::cast_possible_truncation)]
            (i as u32, d)
        })
        .collect();

    Ok(PortedInterface {
        group: encoded,
        downcodes,
        component_count,
    })
}

/// The original (composite) kind a downcode came from, for a report row.
#[must_use]
pub fn downcode_from(d: Downcoded) -> Option<ComponentKind> {
    match d {
        Downcoded::Kept => None,
        Downcoded::ToLayer { from, .. } | Downcoded::ToText { from, .. } => Some(from),
    }
}

// ── CLI orchestration (`interface port`) ─────────────────────────────────────

use std::fmt::Write as _;
use std::path::Path;

use crate::error::Context;

/// Options for [`run`].
pub struct InterfacePortOptions<'a> {
    /// Interface group id.
    pub group: u32,
    /// Donor build (must be 948).
    pub from: u32,
    /// Target build (must be 910).
    pub to: u32,
    /// Build the donor components decode at (947/948 layout).
    pub decode_build: u32,
    /// Donor group byte source (raw `.dat` or runtime pack).
    pub source: crate::interface::transcode::GroupSource<'a>,
    /// Crate data dir (loads the 910 descriptor).
    pub data_dir: &'a Path,
    /// Optional output dir; writes `interfaces/<group>-910.dat`. READ-ONLY caches
    /// otherwise — never point this at a protected oracle dir.
    pub out_dir: Option<&'a Path>,
    /// Emit a JSON summary instead of the human report.
    pub json: bool,
}

/// One downcoded component, summarised for the report.
#[derive(serde::Serialize)]
struct RewriteSummary {
    component: u32,
    from_type: u8,
    to_type: &'static str,
    dropped_body_bytes: usize,
}

/// Run `interface port`. Reads the donor group, ports it through the IR layer
/// (decode → lower → encode, validating every component through the 910 mirror),
/// optionally writes the `.dat`, and prints a summary.
pub fn run(opts: &InterfacePortOptions<'_>) -> Result<()> {
    if opts.from != 948 || opts.to != 910 {
        crate::cache_bail!(
            "interface port currently supports only --from 948 --to 910 (got {} -> {})",
            opts.from,
            opts.to
        );
    }

    let target = BuildDescriptor::load(opts.data_dir, opts.to)?;
    let files = crate::interface::transcode::read_group_files_pub(
        &opts.source,
        opts.group,
        opts.decode_build,
    )?;
    let version = u16::from(crate::interface::transcode::TARGET_VERSION);
    let ported = port_interface_group(opts.group, &files, opts.decode_build, &target, version)?;

    let rewrites: Vec<RewriteSummary> = ported
        .downcodes
        .iter()
        .filter_map(|(id, d)| match d {
            Downcoded::Kept => None,
            Downcoded::ToLayer { from, dropped } => Some(RewriteSummary {
                component: *id,
                from_type: from.type_id(),
                to_type: "layer",
                dropped_body_bytes: *dropped,
            }),
            Downcoded::ToText { from, dropped } => Some(RewriteSummary {
                component: *id,
                from_type: from.type_id(),
                to_type: "text",
                dropped_body_bytes: *dropped,
            }),
        })
        .collect();

    if let Some(dir) = opts.out_dir {
        let iface_dir = dir.join("interfaces");
        std::fs::create_dir_all(&iface_dir)
            .with_context(|| format!("create {}", iface_dir.display()))?;
        let out_path = iface_dir.join(format!("{}-910.dat", opts.group));
        std::fs::write(&out_path, &ported.group.dat)
            .with_context(|| format!("write {}", out_path.display()))?;
    }

    if opts.json {
        let summary = serde_json::json!({
            "group": opts.group,
            "from": opts.from,
            "to": opts.to,
            "target_version": crate::interface::transcode::TARGET_VERSION,
            "components": ported.component_count,
            "kept": ported.component_count - rewrites.len(),
            "rewritten": rewrites,
            "validated_through_910_mirror": ported.group.components.len(),
            "dat_bytes": ported.group.dat.len(),
            "wrote": opts.out_dir.is_some(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).context("encode interface port summary")?
        );
    } else {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "interface port — group {} ({} -> {}, version {})",
            opts.group,
            opts.from,
            opts.to,
            crate::interface::transcode::TARGET_VERSION
        );
        let _ = writeln!(
            s,
            "  {} components: {} kept, {} downcoded; all {} validated through the 910 mirror",
            ported.component_count,
            ported.component_count - rewrites.len(),
            rewrites.len(),
            ported.group.components.len()
        );
        for r in &rewrites {
            let _ = writeln!(
                s,
                "    com{}: type {} -> {} (dropped {} body bytes)",
                r.component, r.from_type, r.to_type, r.dropped_body_bytes
            );
        }
        if opts.out_dir.is_some() {
            let _ = writeln!(s, "  wrote interfaces/{}-910.dat", opts.group);
        }
        print!("{s}");
    }
    Ok(())
}
