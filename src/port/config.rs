//! Config port driver (plan §9 step 6): re-encode a donor (948) config group onto
//! the 910 client through the typed config IR.
//!
//! ```text
//! donor schema/index → DbTable / DbTableIndex IR → encode(target) → 910 group body
//! ```
//!
//! This folds [`crate::config_transcode`] (the DBTABLETYPE group-40 + DbTableIndex
//! re-encode) into the port layer's typed [`crate::port::ir::config`] records: a
//! re-encoded relic table is `DbTable::from_entry(decoded).encode(target)`, a
//! rebuilt index is a [`DbTableIndex`] `.encode(target)`. The byte-stable artifact
//! is the DECOMPRESSED group BODY (the Node-zlib gzip container is not reproducible
//! across zlib implementations, exactly as the config-transcode oracle notes).
//!
//! The byte-exact oracle is the committed `relic-system-948/config/40-948.dat`
//! (+ `dbtableindex/94.dat`) decompressed bodies: [`port_relic_db_groups`]
//! reproduces them byte-for-byte, gating the routing of `config transcode` through
//! this path.

use std::path::Path;

use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::port::book::BuildDescriptor;

pub use crate::config_transcode::{
    CONFIG_ARCHIVE, DBTABLETYPE_GROUP, REENCODE_TABLES, RELIC_TABLE, SERVER_ONLY_TABLES,
    TranscodeInputs, TranscodedDbGroup,
};

/// Port the relic DB groups (Config group 40 + DbTableIndex group 94) through the
/// config IR, reproducing the committed relic bodies. The IR-routed equivalent of
/// [`crate::config_transcode::transcode_db_groups`]: it consumes the same donor
/// inputs and produces the same byte-stable bodies + metadata.
///
/// The relic client-read tables (90/92/94) are re-encoded through the config IR
/// (donor `SemanticTable` → [`DbTable`] → `.encode(target)` opcode 1); the
/// DbTableIndex is built as a [`crate::port::ir::config::DbTableIndex`]; everything
/// else (the 910 base files, the server-only tables 88/89, the merge/pack/metadata)
/// reuses the shared byte-faithful machinery.
pub fn port_relic_db_groups(
    inputs: &TranscodeInputs,
    base_raw: &Path,
    donor_raw: &Path,
    target: &BuildDescriptor,
) -> Result<TranscodedDbGroup> {
    crate::config_transcode::transcode_db_groups_ir(inputs, base_raw, donor_raw, target)
}

/// Guard: the config port currently supports only the relic DBTABLETYPE chain
/// (`--archive 2 --group 40 --from 948 --to 910`), the same scope as
/// `config transcode`. Other combinations error with a clear message.
pub fn check_supported(archive: u32, group: u32, from: u32, to: u32) -> Result<()> {
    if from != 948 || to != 910 {
        bail!("config port currently supports only --from 948 --to 910 (got {from} -> {to})");
    }
    if archive != CONFIG_ARCHIVE || group != DBTABLETYPE_GROUP {
        bail!(
            "config port currently supports only --archive {CONFIG_ARCHIVE} --group {DBTABLETYPE_GROUP} (DBTABLETYPE); got archive {archive} group {group}"
        );
    }
    Ok(())
}

// ── CLI orchestration (`config port`) ────────────────────────────────────────

use std::fmt::Write as _;

/// Options for [`run`].
pub struct ConfigPortOptions<'a> {
    /// Config archive id (must be 2 for DBTABLETYPE).
    pub archive: u32,
    /// Group id (must be 40 for DBTABLETYPE).
    pub group: u32,
    /// Donor build (must be 948).
    pub from: u32,
    /// Target build (must be 910).
    pub to: u32,
    /// Donor semantic config dir (`dbtables.json` / `dbrows.json`).
    pub donor_semantic: &'a Path,
    /// Donor raw-flat root.
    pub donor_raw: &'a Path,
    /// Base (910) raw-flat root.
    pub base_raw: &'a Path,
    /// Crate data dir (loads the 910 descriptor).
    pub data_dir: &'a Path,
    /// Optional output dir; writes `<group>-948.dat(+metadata)` and the
    /// DbTableIndex `94.dat(+metadata)`. READ-ONLY caches; never the oracle dir.
    pub out_dir: Option<&'a Path>,
    /// Emit a JSON summary instead of the human report.
    pub json: bool,
}

/// Run `config port`: port the relic DB groups through the config IR and
/// optionally write the re-encoded `.dat(+metadata)` files. The byte-stable
/// artifact is the decompressed body; the live-written `.dat` carries a gzip
/// container (not byte-reproducible across zlib implementations).
pub fn run(opts: &ConfigPortOptions<'_>) -> Result<()> {
    check_supported(opts.archive, opts.group, opts.from, opts.to)?;

    let target = BuildDescriptor::load(opts.data_dir, opts.to)?;
    let inputs = TranscodeInputs::load(opts.donor_semantic)?;
    let out = port_relic_db_groups(&inputs, opts.base_raw, opts.donor_raw, &target)?;

    if let Some(dir) = opts.out_dir {
        crate::config_transcode::write_transcoded_db_group(&out, dir)?;
    }

    if opts.json {
        let summary = serde_json::json!({
            "archive": opts.archive,
            "group": opts.group,
            "from": opts.from,
            "to": opts.to,
            "via": "config-ir",
            "group40": {
                "roster": out.group40_roster,
                "body_len": out.group40_body.len(),
            },
            "index94": {
                "roster": out.index94_roster,
                "body_len": out.index94_body.len(),
            },
            "wrote": opts.out_dir.is_some(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).context("encode config port summary")?
        );
    } else {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "config port — archive {} group {} ({} -> {}) via config IR",
            opts.archive, opts.group, opts.from, opts.to
        );
        let _ = writeln!(
            s,
            "  group 40: {} files (donor {} verbatim, {} re-encoded via DbTable IR), body {} bytes",
            out.group40_roster.len(),
            SERVER_ONLY_TABLES
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/"),
            REENCODE_TABLES
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/"),
            out.group40_body.len()
        );
        let _ = writeln!(
            s,
            "  index 94: files {} , body {} bytes",
            out.index94_roster
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/"),
            out.index94_body.len()
        );
        if opts.out_dir.is_some() {
            s.push_str("  wrote config/40-948.dat(+metadata) and dbtableindex/94.dat(+metadata)\n");
        }
        print!("{s}");
    }
    Ok(())
}
