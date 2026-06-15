//! Reusable reader for single-file `.js5` runtime packs.
//!
//! A `.js5` pack (written by `Js5.packArchive` in `server/src/jagex/js5/Js5.ts`)
//! is one archive serialized as: the raw archive-index group, then every group
//! container concatenated in index order, then an `index.size` big-endian `u32`
//! trailer of per-group stored byte lengths (`0` = absent group).
//!
//! [`PackArchive`] decodes that container once and exposes the same shape as
//! [`crate::cache::FlatCache`] for a single archive: enumerate group ids, fetch a
//! group's raw container bytes, or decompress + `unpack_group` a group into its
//! files. The pure config/interface/var parsers consume the file bytes directly.
//!
//! The §1.1 sanity identity from Stage 3 is preserved: `data_start` (where the
//! concatenated group containers begin) must equal the encoded size of the index
//! container at offset 0 (accepting an optional 2-byte version trailer).

use crate::cache_bail;
use crate::error::{Context, Result};
use crate::js5::{ArchiveIndex, decompress, unpack_group};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A decoded single-file `.js5` pack: the archive index plus, for every group,
/// the byte range of its container within the file (absent groups carry `None`).
///
/// Config archives in the runtime overlay are stored in **patch mode**: a
/// pristine base `client.<name>.js5` plus a sibling `client.<name>.patch.js5`
/// holding the overlay's added/modified groups. The runtime client's `Js5`
/// loader (`server/src/jagex/js5/Js5.ts`) opens both and lists the union of base
/// and patch group ids. Open such an archive with
/// [`PackArchive::open_with_patch`] so a group present only in the patch is
/// visible to readers. A plain [`PackArchive::open`] reads only the file handed
/// to it (no patch merge).
///
/// The merge here is **additive, base-wins-on-overlap** (group and file
/// granularity): the patch contributes groups/files the base lacks, but a group
/// or file present in the base keeps its base bytes. This deliberately differs
/// from the client's `readRaw`, where the patch overrides an overlapping base
/// group: for the id-presence / config-dump tooling the curated server ids were
/// authored against the base, so the base is the source of truth on overlap and
/// only genuinely-new patch additions are folded in. See [`Self::group_files`].
#[derive(Debug)]
pub struct PackArchive {
    /// The whole pack file, retained so group containers are zero-copy slices.
    file: Vec<u8>,
    /// The decoded archive index (group ids, file layout, …).
    index: ArchiveIndex,
    /// Per group id: `Some((start, end))` byte range of its container in `file`,
    /// or `None` when the group is absent (trailer length 0).
    ranges: BTreeMap<u32, Option<(usize, usize)>>,
    /// The sibling `.patch.js5` archive, when this pack was opened with
    /// [`PackArchive::open_with_patch`] and a patch file was present. Group
    /// lookups consult it for groups/files the base lacks (additive,
    /// base-wins-on-overlap). `None` for a plain [`PackArchive::open`].
    patch: Option<Box<Self>>,
}

/// Compute the encoded size of the index container at offset 0 from its header,
/// returning the candidate sizes (without and with a 2-byte version trailer).
fn index_container_sizes(file: &[u8]) -> Result<(usize, usize)> {
    if file.len() < 5 {
        cache_bail!("pack file too short to hold an index container header");
    }
    let compression = file[0];
    let clen = u32::from_be_bytes([file[1], file[2], file[3], file[4]]);
    let clen = usize::try_from(clen).context("index container length overflow")?;
    let header: usize = match compression {
        0 => 1 + 4,
        1..=3 => 1 + 4 + 4,
        other => cache_bail!("unsupported index container compression: {other}"),
    };
    let base = header
        .checked_add(clen)
        .context("index container size overflow")?;
    let with_trailer = base.checked_add(2).context("index container size overflow")?;
    Ok((base, with_trailer))
}

impl PackArchive {
    /// Read and decode a single-file `.js5` pack from disk.
    pub fn open(path: &Path) -> Result<Self> {
        let file = fs::read(path)
            .with_context(|| format!("failed reading pack file {}", path.display()))?;
        Self::from_bytes(file)
            .with_context(|| format!("failed reading pack {}", path.display()))
    }

    /// Decode an in-memory single-file `.js5` pack. The layout is exactly as
    /// written by `Js5.packArchive`: the raw archive-index group, then every
    /// group container concatenated in index order, then the `index.size`
    /// big-endian `u32` trailer of per-group stored byte lengths (`0` = absent).
    pub fn from_bytes(file: Vec<u8>) -> Result<Self> {
        // Decode the index from the container at offset 0. `decompress` reads the
        // header (compression byte + lengths) itself and stops at the payload end,
        // so passing the whole file is safe even with trailing group bytes.
        let index_bytes =
            decompress(&file).context("failed to decompress pack archive index")?;
        let index = ArchiveIndex::decode(&index_bytes)
            .context("failed to decode pack archive index container")?;
        let n = index.group_count;

        let trailer_len = n.checked_mul(4).context("trailer length overflow")?;
        let trailer_off = file
            .len()
            .checked_sub(trailer_len)
            .context("pack file too short for its group-length trailer")?;

        let mut lengths = Vec::with_capacity(n);
        let mut total: usize = 0;
        for i in 0..n {
            let off = trailer_off + i * 4;
            let len =
                u32::from_be_bytes([file[off], file[off + 1], file[off + 2], file[off + 3]]);
            let len = usize::try_from(len).context("group length overflow")?;
            total = total.checked_add(len).context("group length sum overflow")?;
            lengths.push(len);
        }

        let data_start = trailer_off
            .checked_sub(total)
            .context("pack group lengths exceed available bytes")?;
        if data_start == 0 {
            cache_bail!("pack data start computed as zero (index container has no bytes)");
        }
        let (size_a, size_b) = index_container_sizes(&file)?;
        if data_start != size_a && data_start != size_b {
            cache_bail!(
                "pack sanity identity failed: data_start {data_start} != index container size \
                 ({size_a} or {size_b})"
            );
        }

        if index.group_id.len() != n {
            cache_bail!(
                "index group id count {} disagrees with group_count {n}",
                index.group_id.len()
            );
        }

        let mut ranges = BTreeMap::new();
        let mut cursor = data_start;
        for (i, &group) in index.group_id.iter().enumerate() {
            let len = lengths[i];
            if len == 0 {
                ranges.insert(group, None);
                continue;
            }
            let end = cursor.checked_add(len).context("group slice overflow")?;
            if end > trailer_off {
                cache_bail!("group {group} container slice exceeds trailer offset");
            }
            ranges.insert(group, Some((cursor, end)));
            cursor = end;
        }
        if cursor != trailer_off {
            cache_bail!(
                "pack group bytes did not consume up to the trailer (left {trailer_off}, at {cursor})"
            );
        }

        Ok(Self {
            file,
            index,
            ranges,
            patch: None,
        })
    }

    /// Read and decode a single-file `.js5` pack, AND merge its sibling
    /// `client.<name>.patch.js5` (same directory, `.js5` → `.patch.js5`) over it
    /// when present — exactly as the runtime client's `Js5` loader does for the
    /// patch-mode config archives. Group lookups then see the union of base and
    /// patch group ids, with the patch winning on overlap.
    ///
    /// When no sibling patch exists this is identical to [`PackArchive::open`].
    /// Use this (not `open`) for the id-presence and group-dump paths so a group
    /// present only in the patch (e.g. an overlay-added enum group) resolves the
    /// same way it does at runtime instead of reading as absent.
    pub fn open_with_patch(path: &Path) -> Result<Self> {
        let mut base = Self::open(path)?;
        if let Some(patch_path) = patch_sibling(path)
            && patch_path.is_file()
        {
            let sibling = Self::open(&patch_path)
                .with_context(|| format!("failed reading patch pack {}", patch_path.display()))?;
            base.patch = Some(Box::new(sibling));
        }
        Ok(base)
    }

    /// The decoded archive index (the BASE index; the patch overlay is not
    /// folded into it — query group presence/content via the accessors, which
    /// consult the patch).
    #[must_use]
    pub const fn index(&self) -> &ArchiveIndex {
        &self.index
    }

    /// All group ids in the archive index (the union of base and any merged
    /// patch groups), in ascending order with no duplicates.
    pub fn group_ids(&self) -> impl Iterator<Item = u32> + '_ {
        // BTreeMap keys are already sorted; chain base + patch ids and dedup the
        // overlap (a group present in both). Collect into a BTreeSet so the
        // result stays sorted+unique regardless of which layer carries an id.
        let mut ids: std::collections::BTreeSet<u32> = self.ranges.keys().copied().collect();
        if let Some(patch) = &self.patch {
            ids.extend(patch.ranges.keys().copied());
        }
        ids.into_iter()
    }

    /// `true` when the group id is present in the merged index — listed in the
    /// patch or the base (whether or not its container carries bytes). Matches
    /// the runtime, which lists the union of base and patch group ids.
    #[must_use]
    pub fn has_group(&self, group: u32) -> bool {
        self.ranges.contains_key(&group)
            || self.patch.as_ref().is_some_and(|p| p.ranges.contains_key(&group))
    }

    /// The raw container bytes (`<archive>/<group>.dat` form) for a group, or
    /// `None` when the group is absent or its container length was zero.
    ///
    /// Merge semantics are **additive, base-wins-on-overlap**: a group present in
    /// the base is served from the base; a group present ONLY in the sibling
    /// patch (an overlay-*added* group) is served from the patch. This makes a
    /// patched-only group visible — the exact defect Bug A fixed — without
    /// re-deriving the *content* of groups the curated server ids were authored
    /// against from the base.
    ///
    /// This deliberately differs from the runtime client's `Js5.readRaw`, which
    /// lets the patch override an overlapping base group (so the *client* renders
    /// the donor variant). For the id-presence / config-dump tools the base is
    /// the curated source of truth on overlap; only genuinely-new patch groups
    /// are merged in. (See `group_files` for the per-file analogue.)
    #[must_use]
    pub fn group_container(&self, group: u32) -> Option<&[u8]> {
        if let Some(bytes) = self.group_container_local(group) {
            return Some(bytes);
        }
        // Absent from the base: fall through to a patch-only group.
        self.patch
            .as_ref()
            .and_then(|patch| patch.group_container_local(group))
    }

    /// Container bytes for a group from THIS pack's own file only (no patch
    /// consultation). `None` when the group is absent from this layer's index or
    /// its trailer length was zero.
    #[must_use]
    fn group_container_local(&self, group: u32) -> Option<&[u8]> {
        let (start, end) = (*self.ranges.get(&group)?)?;
        Some(&self.file[start..end])
    }

    /// Decompress and `unpack_group` a present group into its file map, mirroring
    /// [`crate::cache::FlatCache::group_files`]. Returns `Ok(None)` when the group
    /// is absent (so callers can distinguish "no such group" from a parse error).
    ///
    /// Additive, base-wins-on-overlap at **file** granularity (see
    /// [`Self::group_container`]): the base file map is the source of truth, and a
    /// merged patch contributes only files the base does NOT already carry — both
    /// files of a patch-only group (the base lacks the whole group) and any new
    /// files a patch adds to a group that also exists in the base. A file present
    /// in both keeps its base bytes (the curated ids were validated against the
    /// base), so this never silently re-derives existing base content from the
    /// patch.
    pub fn group_files(&self, group: u32) -> Result<Option<BTreeMap<u32, Vec<u8>>>> {
        let base = match self.group_container_local(group) {
            Some(container) => Some(
                unpack_group(&self.index, group, container)
                    .with_context(|| format!("failed to unpack group {group}"))?,
            ),
            None => None,
        };

        // Files the patch adds for this group (its own index/layout), if any.
        let patch_files = match &self.patch {
            Some(patch) => match patch.group_container_local(group) {
                Some(container) => Some(
                    unpack_group(&patch.index, group, container)
                        .with_context(|| format!("failed to unpack patch group {group}"))?,
                ),
                None => None,
            },
            None => None,
        };

        match (base, patch_files) {
            (None, None) => Ok(None),
            (Some(files), None) => Ok(Some(files)),
            (None, Some(files)) => Ok(Some(files)),
            (Some(mut files), Some(patch_files)) => {
                // Union: keep every base file, add only patch files the base
                // lacks (base wins on overlap).
                for (file_id, bytes) in patch_files {
                    files.entry(file_id).or_insert(bytes);
                }
                Ok(Some(files))
            }
        }
    }
}

/// The sibling patch path for a base `.js5` pack: `…/client.<name>.js5` →
/// `…/client.<name>.patch.js5`. Mirrors the runtime client's
/// `file.replace('.js5', '.patch.js5')`. Returns `None` when `path` does not end
/// in `.js5` (so a non-pack path is never mistaken for one).
fn patch_sibling(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".js5")?;
    // Guard against double-patching: a `.patch.js5` has no further sibling.
    if stem.ends_with(".patch") {
        return None;
    }
    Some(path.with_file_name(format!("{stem}.patch.js5")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::ByteWriter;

    /// Wrap raw bytes in a compression-0 JS5 container (no version trailer).
    fn container0(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(0u8);
        out.extend_from_slice(&u32::try_from(payload.len()).expect("len fits u32").to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Build a protocol-6 archive index container for the given group ids with a
    /// single file each (no names, no lengths).
    fn build_index(group_ids: &[u32]) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.p1(6); // protocol
        w.p4s(0); // version
        w.p1(0); // flags: none
        w.p2(u16::try_from(group_ids.len()).expect("group count fits u16")); // group count
        let mut last = 0u32;
        for &g in group_ids {
            w.p2(u16::try_from(g - last).expect("group delta fits u16"));
            last = g;
        }
        for _ in group_ids {
            w.p4s(0); // checksums
        }
        for _ in group_ids {
            w.p4s(0); // versions
        }
        for _ in group_ids {
            w.p2(1); // group sizes (file count per group = 1)
        }
        for _ in group_ids {
            w.p2(1); // file id deltas: one file at delta 1 -> file id 0
        }
        container0(&w.data)
    }

    /// Assemble a full single-file pack from an index container and ordered group
    /// containers (None = absent group, trailer length 0).
    fn build_pack(index_container: &[u8], groups: &[Option<Vec<u8>>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(index_container);
        let mut lengths = Vec::new();
        for g in groups {
            match g {
                Some(bytes) => {
                    out.extend_from_slice(bytes);
                    lengths.push(u32::try_from(bytes.len()).expect("group len fits u32"));
                }
                None => lengths.push(0),
            }
        }
        for len in lengths {
            out.extend_from_slice(&len.to_be_bytes());
        }
        out
    }

    #[test]
    fn pack_reader_extracts_groups_and_handles_absent_and_version_trailer() {
        let index = build_index(&[0, 1, 2]);
        // group 0 present, group 1 absent, group 2 present with a 2-byte version
        // trailer appended to its container bytes.
        let g0 = container0(b"AAAA");
        let mut g2 = container0(b"CCCC");
        g2.extend_from_slice(&[0x12, 0x34]);
        let pack = build_pack(&index, &[Some(g0.clone()), None, Some(g2.clone())]);

        let decoded = PackArchive::from_bytes(pack).expect("pack decodes");
        assert_eq!(decoded.group_ids().collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(decoded.group_container(0), Some(g0.as_slice()));
        assert_eq!(decoded.group_container(1), None);
        assert_eq!(decoded.group_container(2), Some(g2.as_slice()));
        assert!(decoded.has_group(1));
        assert!(!decoded.has_group(9));
        // group_files for present group 0 yields one file; the index encodes a
        // single file at delta 1, so its file id is 1.
        let files = decoded.group_files(0).expect("unpack ok").expect("present");
        assert_eq!(files.keys().copied().collect::<Vec<_>>(), vec![1]);
        // group_files for absent group 1 is Ok(None).
        assert!(decoded.group_files(1).expect("absent ok").is_none());
    }

    #[test]
    fn pack_reader_rejects_broken_sanity_identity() {
        // Insert one extra byte between the index container and the group data, so
        // data_start no longer equals the index container size.
        let index = build_index(&[0]);
        let g0 = container0(b"AAAA");
        let mut pack = Vec::new();
        pack.extend_from_slice(&index);
        pack.push(0xFF); // stray byte breaking the identity
        pack.extend_from_slice(&g0);
        pack.extend_from_slice(&u32::try_from(g0.len()).expect("len fits u32").to_be_bytes());

        let err = PackArchive::from_bytes(pack).expect_err("identity must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("sanity identity"), "unexpected error: {msg}");
    }

    // --- patch-merge (Bug A) -------------------------------------------------

    /// Build a protocol-6 index for groups that each hold a single file at the
    /// given file id (so base and patch can map the SAME group id to DIFFERENT
    /// file ids, exercising the per-file union). `(group_id, file_id)` pairs.
    fn build_index_single_files(groups: &[(u32, u32)]) -> Vec<u8> {
        let mut w = ByteWriter::new();
        w.p1(6); // protocol
        w.p4s(0); // version
        w.p1(0); // flags: none
        w.p2(u16::try_from(groups.len()).expect("group count fits u16"));
        let mut last = 0u32;
        for &(g, _) in groups {
            w.p2(u16::try_from(g - last).expect("group delta fits u16"));
            last = g;
        }
        for _ in groups {
            w.p4s(0); // checksums
        }
        for _ in groups {
            w.p4s(0); // versions
        }
        for _ in groups {
            w.p2(1); // group sizes: one file per group
        }
        for &(_, file_id) in groups {
            // Single file id deltas: a leading delta of `file_id` -> that id.
            w.p2(u16::try_from(file_id).expect("file id fits u16"));
        }
        container0(&w.data)
    }

    /// Decode a base pack and a patch pack from in-memory bytes and wire the
    /// patch as the base's sibling, mirroring `open_with_patch` without disk I/O.
    fn merged(base_pack: Vec<u8>, patch_pack: Vec<u8>) -> PackArchive {
        let mut base = PackArchive::from_bytes(base_pack).expect("base decodes");
        let patch = PackArchive::from_bytes(patch_pack).expect("patch decodes");
        base.patch = Some(Box::new(patch));
        base
    }

    #[test]
    fn merge_exposes_patch_only_group_and_keeps_base() {
        // Base has groups 0 and 1; the patch ADDS group 2 (a patched-only group,
        // the Bug A shape). The merged view must list all three and serve group 2
        // from the patch while groups 0/1 keep their base bytes.
        let base = build_pack(
            &build_index(&[0, 1]),
            &[Some(container0(b"BASE0")), Some(container0(b"BASE1"))],
        );
        let patch = build_pack(&build_index(&[2]), &[Some(container0(b"PATCH2"))]);
        let m = merged(base, patch);

        assert_eq!(m.group_ids().collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(m.has_group(2), "patched-only group must be present");
        assert_eq!(m.group_container(0), Some(container0(b"BASE0").as_slice()));
        assert_eq!(m.group_container(2), Some(container0(b"PATCH2").as_slice()));
        // group_files routes group 2 to the patch's layout (file id 1).
        let files2 = m.group_files(2).expect("ok").expect("present");
        assert_eq!(files2.get(&1).map(Vec::as_slice), Some(b"PATCH2".as_ref()));
    }

    #[test]
    fn merge_base_wins_on_overlapping_group() {
        // Group 5 exists in BOTH base and patch with DIFFERENT bytes (the enums
        // 1479/1480 shape). The id-presence/dump tooling must keep the BASE bytes
        // on overlap so curated ids validated against the base stay stable — the
        // regression guard for the generate-ts-ids drift gate.
        let base = build_pack(&build_index(&[5]), &[Some(container0(b"BASE_FIVE"))]);
        let patch = build_pack(&build_index(&[5]), &[Some(container0(b"PATCH_FIVE"))]);
        let m = merged(base, patch);

        assert_eq!(m.group_ids().collect::<Vec<_>>(), vec![5]);
        assert_eq!(
            m.group_container(5),
            Some(container0(b"BASE_FIVE").as_slice()),
            "base must win on an overlapping group"
        );
        let files = m.group_files(5).expect("ok").expect("present");
        assert_eq!(files.get(&1).map(Vec::as_slice), Some(b"BASE_FIVE".as_ref()));
    }

    #[test]
    fn merge_unions_new_patch_files_but_keeps_base_files() {
        // Same group id in base and patch, but the patch declares a DIFFERENT
        // single file id. The merged group_files must contain BOTH the base file
        // and the patch-only file (per-file union), proving a patch can add a new
        // file to an existing group without dropping or overriding base files.
        let base = build_pack(
            &build_index_single_files(&[(7, 10)]),
            &[Some(container0(b"BASEFILE"))],
        );
        let patch = build_pack(
            &build_index_single_files(&[(7, 20)]),
            &[Some(container0(b"PATCHFILE"))],
        );
        let m = merged(base, patch);

        let files = m.group_files(7).expect("ok").expect("present");
        assert_eq!(files.get(&10).map(Vec::as_slice), Some(b"BASEFILE".as_ref()));
        assert_eq!(files.get(&20).map(Vec::as_slice), Some(b"PATCHFILE".as_ref()));
        assert_eq!(files.len(), 2, "union should hold both files");
    }

    #[test]
    fn merge_per_file_base_wins_on_same_file_id() {
        // Same group AND same single file id in both layers, but different bytes:
        // the base file must win (the per-file analogue of the overlap rule).
        let base = build_pack(
            &build_index_single_files(&[(7, 10)]),
            &[Some(container0(b"BASEFILE"))],
        );
        let patch = build_pack(
            &build_index_single_files(&[(7, 10)]),
            &[Some(container0(b"PATCHFILE"))],
        );
        let m = merged(base, patch);

        let files = m.group_files(7).expect("ok").expect("present");
        assert_eq!(files.len(), 1);
        assert_eq!(files.get(&10).map(Vec::as_slice), Some(b"BASEFILE".as_ref()));
    }

    #[test]
    fn open_with_patch_round_trips_on_disk() {
        // End-to-end: write a base `.js5` and a sibling `.patch.js5` to a tempdir
        // and confirm `open_with_patch` finds the patched-only group (exercising
        // `patch_sibling` path derivation + the disk read), while a plain `open`
        // of the base alone does not.
        let dir = std::env::temp_dir().join(format!(
            "rs3_pack_patch_roundtrip_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir tempdir");
        let base_path = dir.join("client.enum.config.js5");
        let patch_path = dir.join("client.enum.config.patch.js5");

        let base = build_pack(&build_index(&[0]), &[Some(container0(b"BASE0"))]);
        let patch = build_pack(&build_index(&[2]), &[Some(container0(b"PATCH2"))]);
        std::fs::write(&base_path, &base).expect("write base");
        std::fs::write(&patch_path, &patch).expect("write patch");

        let plain = PackArchive::open(&base_path).expect("open base");
        assert!(!plain.has_group(2), "plain open must NOT see the patch group");

        let merged = PackArchive::open_with_patch(&base_path).expect("open_with_patch");
        assert!(merged.has_group(2), "open_with_patch must see the patch group");
        assert_eq!(merged.group_ids().collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(merged.group_container(2), Some(container0(b"PATCH2").as_slice()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn patch_sibling_derivation() {
        // `.js5` -> `.patch.js5`; a `.patch.js5` has no further sibling; a
        // non-`.js5` path yields None.
        assert_eq!(
            patch_sibling(Path::new("/p/client.enum.config.js5")),
            Some(PathBuf::from("/p/client.enum.config.patch.js5"))
        );
        assert_eq!(patch_sibling(Path::new("/p/client.enum.config.patch.js5")), None);
        assert_eq!(patch_sibling(Path::new("/p/notapack.bin")), None);
    }
}
