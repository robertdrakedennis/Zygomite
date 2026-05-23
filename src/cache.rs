use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::js5::{ArchiveIndex, decompress, unpack_group};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct FlatCache {
    root: PathBuf,
}

impl FlatCache {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.is_dir() {
            bail!("cache directory not found: {}", root.display());
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn get(&self, archive: u32, group: u32) -> Result<Option<Vec<u8>>> {
        let path = self.root.join(format!("{archive}/{group}.dat"));
        if !path.is_file() {
            return Ok(None);
        }
        let bytes =
            fs::read(&path).with_context(|| format!("failed reading {}", path.display()))?;
        Ok(Some(bytes))
    }

    pub fn archive_index_bytes(&self, archive: u32) -> Result<Vec<u8>> {
        let bytes = self
            .get(255, archive)?
            .with_context(|| format!("missing archive index: 255/{archive}.dat"))?;
        decompress(&bytes).with_context(|| format!("failed to decompress archive index {archive}"))
    }

    pub fn archive_index(&self, archive: u32) -> Result<ArchiveIndex> {
        let decoded = self.archive_index_bytes(archive)?;
        ArchiveIndex::decode(&decoded)
            .with_context(|| format!("failed to decode archive index for archive {archive}"))
    }

    pub fn group_files(&self, archive: u32, group: u32) -> Result<BTreeMap<u32, Vec<u8>>> {
        let index = self.archive_index(archive)?;
        self.group_files_with_index(&index, archive, group)
    }

    pub fn group_files_with_index(
        &self,
        index: &ArchiveIndex,
        archive: u32,
        group: u32,
    ) -> Result<BTreeMap<u32, Vec<u8>>> {
        let compressed = self
            .get(archive, group)?
            .with_context(|| format!("missing group data: {archive}/{group}.dat"))?;
        unpack_group(index, group, &compressed)
            .with_context(|| format!("failed to unpack group {group} in archive {archive}"))
    }
}
