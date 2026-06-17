//! `audio` — inspect/extract every audio archive, optionally writing each file
//! (and any embedded OGG) plus a manifest.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::audio::{AudioKind, inspect_audio_file};
use crate::cache::FlatCache;
use crate::cli::context::CommandContext;
use crate::cli::shared::{print_json, write_binary, write_json};
use crate::constants::AUDIO_ARCHIVES;
use crate::fixture::ensure_archive_complete;

#[derive(Debug, Serialize)]
struct AudioSummary {
    archives: BTreeMap<u32, usize>,
    kinds: BTreeMap<String, usize>,
    extracted_embedded_ogg: usize,
    manifest_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct AudioManifestEntry {
    archive: u32,
    group: u32,
    file: u32,
    size: usize,
    kind: String,
    raw_extension: String,
    embedded_ogg_offset: Option<usize>,
    extracted_ogg: bool,
}

/// Options for `audio`.
#[derive(Clone, Debug, Default)]
pub struct AudioOpts {
    pub out_dir: Option<PathBuf>,
    pub max_files: Option<usize>,
}

/// `audio` — inspect/extract audio archives.
pub fn run(ctx: &CommandContext, opts: AudioOpts) -> Result<()> {
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let AudioOpts { out_dir, max_files } = opts;
    let out_dir = out_dir.as_deref();

    let mut available = Vec::new();
    for archive in AUDIO_ARCHIVES {
        if ensure_archive_complete(cache.root(), tar_path, archive).is_ok() {
            available.push(archive);
        }
    }
    let cache = FlatCache::open(cache.root())?;
    let mut archive_counts = BTreeMap::new();
    let mut kind_counts = BTreeMap::new();
    let mut extracted_embedded_ogg = 0_usize;
    let mut manifest = Vec::new();

    let mut processed = 0_usize;
    let process_limit = max_files.unwrap_or(usize::MAX);
    let mut limit_hit = false;

    for archive in available {
        let index = cache.archive_index(archive)?;
        let mut file_count = 0_usize;
        for group in &index.group_id {
            let files = cache.group_files_with_index(&index, archive, *group)?;
            for (file, data) in &files {
                if processed >= process_limit {
                    limit_hit = true;
                    break;
                }
                let inspection = inspect_audio_file(data);
                *kind_counts
                    .entry(inspection.kind.as_str().to_string())
                    .or_insert(0) += 1;
                let mut extracted_ogg = false;

                if let Some(out) = out_dir {
                    let raw_path =
                        out.join(format!("{archive}_{group}_{file}.{}", inspection.extension));
                    write_binary(&raw_path, data)?;

                    if inspection.kind == AudioKind::Jaga
                        && let Some(ogg) = inspection.embedded_ogg_slice(data)
                    {
                        let ogg_path = out.join(format!("{archive}_{group}_{file}.ogg"));
                        write_binary(&ogg_path, ogg)?;
                        extracted_ogg = true;
                        extracted_embedded_ogg += 1;
                    }
                }

                manifest.push(AudioManifestEntry {
                    archive,
                    group: *group,
                    file: *file,
                    size: data.len(),
                    kind: inspection.kind.as_str().to_string(),
                    raw_extension: inspection.extension.to_string(),
                    embedded_ogg_offset: inspection.embedded_ogg_offset,
                    extracted_ogg,
                });
                file_count += 1;
                processed += 1;
            }
            if limit_hit {
                break;
            }
        }
        archive_counts.insert(archive, file_count);
        if limit_hit {
            break;
        }
    }

    let manifest_path = if let Some(out) = out_dir {
        let manifest_path = out.join("audio_manifest.json");
        write_json(&manifest_path, &manifest)?;
        Some(manifest_path.display().to_string())
    } else {
        None
    };

    print_json(&AudioSummary {
        archives: archive_counts,
        kinds: kind_counts,
        extracted_embedded_ogg,
        manifest_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_manifest_entry_serializes() {
        let entry = AudioManifestEntry {
            archive: 14,
            group: 1,
            file: 0,
            size: 123,
            kind: "jaga".to_string(),
            raw_extension: "jaga".to_string(),
            embedded_ogg_offset: Some(32),
            extracted_ogg: true,
        };
        let json = serde_json::to_string(&entry).expect("serialize manifest entry");
        assert!(json.contains("\"archive\":14"));
        assert!(json.contains("\"kind\":\"jaga\""));
    }
}
