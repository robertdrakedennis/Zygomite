//! Shared `--pack-root` resolution with donor auto-fallback.
//!
//! The `font` / `explain-interface` / `decode` subcommands all read groups from
//! the runtime single-file `.js5` packs and all default `--pack-root` to the
//! runtime **overlay** pack (910-base + 948 overlay). That overlay down-codes
//! only the donor groups already spliced into the live build, so donor-only ids
//! (e.g. the ritual interface 1224, or any not-yet-ported font/config group)
//! are **absent** from it — and the commands used to fail opaquely unless the
//! user happened to know to pass the donor `--pack-root`.
//!
//! This module centralises the "is the group here? else try the donor pack;
//! else say exactly what to pass" logic so the three commands behave
//! identically. The fallback is transparent and **read-only**: it only ever
//! opens existing pack files; it never writes.
//!
//! The resolver keys off [`PackArchive::has_group`] (group present in the index,
//! whether or not its container carries bytes) so an empty-but-listed group in
//! the runtime pack is still preferred over the donor — matching the client,
//! which would read the (empty) runtime group rather than the donor's.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::error::{CacheError, Result};
use crate::js5pack::PackArchive;

/// Donor (948-all) pack root, relative to the crate working dir. This is the
/// full 948 export the runtime overlay is derived from; it holds every donor
/// group (interfaces, fonts, configs) the overlay may not have spliced yet.
/// Mirrors the path the plans/tests reference (`cache/rs3-cache/948-all/pack`).
pub const DONOR_PACK_ROOT: &str = "../../cache/rs3-cache/948-all/pack";

/// Outcome of resolving which pack root actually holds a requested group.
#[derive(Debug)]
pub struct ResolvedPack {
    /// The pack file to open (`<root>/<pack_file>`), guaranteed to contain the
    /// group when [`Self::note`] is consulted — i.e. `open` + `group_files`
    /// will find it (modulo an empty container, which the caller handles).
    pub path: PathBuf,
    /// `Some(message)` when the resolver fell back from the requested root to
    /// the donor pack. Callers should surface this on stderr so the user knows
    /// which pack served the group. `None` when the requested root was used.
    pub note: Option<String>,
}

/// `true` when `pack_file` under `root` exists AND lists `group` in its index —
/// including a group present only in the sibling `.patch.js5` (the runtime
/// merges base + patch, so a patched-only group is genuinely present and must
/// NOT trigger the donor fallback). A missing pack file (the root lacks that
/// archive entirely) counts as "no".
fn root_has_group(root: &Path, pack_file: &str, group: u32) -> bool {
    let path = root.join(pack_file);
    if !path.is_file() {
        return false;
    }
    PackArchive::open_with_patch(&path).is_ok_and(|pack| pack.has_group(group))
}

/// Resolve which pack root holds `group` for the pack file `pack_file`:
///
/// 1. If the requested `primary_root` lists the group, use it (no note).
/// 2. Else, if the donor pack lists it, use the donor and return a fallback
///    note naming the donor root.
/// 3. Else error, quoting the exact `--pack-root <donor>` to pass and noting
///    that neither pack has the group (so the id is genuinely absent / wrong).
///
/// When `primary_root` IS the donor pack (the user already pointed at it), step
/// 2 is skipped — there is nothing to fall back to and the error in step 3 says
/// the donor lacks it.
pub fn resolve(primary_root: &Path, pack_file: &str, group: u32) -> Result<ResolvedPack> {
    if root_has_group(primary_root, pack_file, group) {
        return Ok(ResolvedPack {
            path: primary_root.join(pack_file),
            note: None,
        });
    }

    let donor = Path::new(DONOR_PACK_ROOT);
    let primary_is_donor = same_root(primary_root, donor);
    if !primary_is_donor && root_has_group(donor, pack_file, group) {
        return Ok(ResolvedPack {
            path: donor.join(pack_file),
            note: Some(format!(
                "note: group {group} absent from {} — fell back to donor pack {} ({})",
                primary_root.display(),
                donor.display(),
                pack_file
            )),
        });
    }

    // Neither the requested root nor the donor has it. Be precise: distinguish
    // "the requested pack file is missing entirely" from "the group is absent".
    let primary_pack = primary_root.join(pack_file);
    let detail = if primary_pack.is_file() {
        format!("group {group} is absent from {}", primary_pack.display())
    } else {
        format!("{} does not exist", primary_pack.display())
    };
    let hint = if primary_is_donor {
        // Already pointed at the donor — no further pack to suggest.
        format!("the donor pack also lacks group {group}; the id may be wrong")
    } else {
        format!(
            "and the donor pack lacks it too; if it lives in another pack, pass --pack-root <root> \
             (donor is `{DONOR_PACK_ROOT}`)"
        )
    };
    Err(CacheError::message(format!("{detail}; {hint}")))
}

/// Resolve a group's pack like [`resolve`], printing any donor-fallback note to
/// stderr **once per resolved pack path** (so a rasterize loop over many fonts
/// that all fall back to the same donor pack notes it a single time, not N).
/// Returns only the chosen pack path; the dedup state is process-global.
///
/// Use this in the `font` subcommand's per-archive readers, which call into the
/// resolver repeatedly. One-shot callers (decode / explain-interface) surface
/// the note themselves via [`resolve`].
pub fn resolve_noting(primary_root: &Path, pack_file: &str, group: u32) -> Result<PathBuf> {
    let resolved = resolve(primary_root, pack_file, group)?;
    if let Some(note) = resolved.note {
        static SEEN: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
        let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
        let first = seen
            .lock()
            .map(|mut s| s.insert(resolved.path.clone()))
            .unwrap_or(true);
        if first {
            eprintln!("{note}");
        }
    }
    Ok(resolved.path)
}

/// Compare two roots for equality, canonicalising when both exist (so
/// `./a/../pack` and `pack` match), else falling back to literal path equality.
fn same_root(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Literal-equality fallback of `same_root` when paths don't exist on disk.
    #[test]
    fn same_root_literal_when_absent() {
        assert!(same_root(
            Path::new("/no/such/pack"),
            Path::new("/no/such/pack")
        ));
        assert!(!same_root(
            Path::new("/no/such/pack"),
            Path::new("/other/pack")
        ));
    }

    /// A non-existent requested root with a non-existent donor yields a precise
    /// "does not exist" + donor hint, never a bare/opaque error.
    #[test]
    fn resolve_missing_everywhere_is_helpful() {
        // Use a temp dir guaranteed to lack the pack file so we exercise the
        // "pack file missing" branch deterministically (donor path is also
        // unlikely to exist in a clean checkout; if it does and lacks the bogus
        // group, the message still names the donor pack-root to pass).
        let root = std::env::temp_dir().join("rs3_pack_root_probe_does_not_exist");
        let err = resolve(&root, "client.interfaces.js5", 4242).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("--pack-root") || msg.contains("donor pack lacks"),
            "error should name the donor --pack-root to pass; got: {msg}"
        );
    }
}
