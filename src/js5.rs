use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::Packet;
use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use std::collections::BTreeMap;
use std::io::{Cursor, Read};

#[derive(Clone, Debug)]
pub struct ArchiveIndex {
    pub version: i32,
    pub group_count: usize,
    pub group_id: Vec<u32>,
    pub group_array_size: usize,
    pub group_name_hash: Option<Vec<i32>>,
    pub group_size: Vec<u32>,
    pub group_file_ids: Vec<Option<Vec<u32>>>,
    pub group_file_names: Option<Vec<Option<Vec<i32>>>>,
    pub group_max_file_id: Vec<u32>,
    pub group_length: Option<Vec<u32>>,
}

impl ArchiveIndex {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut packet = Packet::new(data);
        let protocol = packet.g1()?;
        if !(5..=7).contains(&protocol) {
            bail!("unsupported archive index protocol: {protocol}");
        }

        let version = if protocol >= 6 { packet.g4s()? } else { 0 };
        let flags = packet.g1()?;
        let has_names = (flags & 1) != 0;
        let has_digests = (flags & 2) != 0;
        let has_lengths = (flags & 4) != 0;
        let has_uncompressed_checksums = (flags & 8) != 0;

        let group_count = if protocol >= 7 {
            usize::try_from(packet.gsmart2or4null()?)
                .context("negative group count in archive index")?
        } else {
            usize::from(packet.g2()?)
        };

        let mut last_group = 0_u32;
        let mut max_group = 0_u32;
        let mut group_id = Vec::with_capacity(group_count);
        for _ in 0..group_count {
            let delta = if protocol >= 7 {
                u32::try_from(packet.gsmart2or4null()?).context("negative group delta")?
            } else {
                u32::from(packet.g2()?)
            };
            last_group = last_group
                .checked_add(delta)
                .context("group id overflow while decoding archive index")?;
            max_group = max_group.max(last_group);
            group_id.push(last_group);
        }

        let group_array_size = usize::try_from(max_group)
            .context("group id too large")?
            .saturating_add(1);

        let mut group_name_hash = has_names.then(|| vec![-1_i32; group_array_size]);
        if has_names {
            for group in &group_id {
                let idx = usize::try_from(*group).context("group index overflow")?;
                if let Some(names) = group_name_hash.as_mut() {
                    names[idx] = packet.g4s()?;
                }
            }
        }

        // checksums
        for _ in 0..group_count {
            let _ = packet.g4s()?;
        }

        if has_uncompressed_checksums {
            for _ in 0..group_count {
                let _ = packet.g4s()?;
            }
        }

        if has_digests {
            for _ in 0..group_count {
                let _ = packet.gdata(64)?;
            }
        }

        let mut group_length = has_lengths.then(|| vec![0_u32; group_array_size]);
        if has_lengths {
            for group in &group_id {
                let idx = usize::try_from(*group).context("group index overflow")?;
                let compressed_len =
                    u32::try_from(packet.g4s()?).context("negative group length")?;
                let _ = packet.g4s()?;
                if let Some(lengths) = group_length.as_mut() {
                    lengths[idx] = compressed_len;
                }
            }
        }

        // versions
        for _ in 0..group_count {
            let _ = packet.g4s()?;
        }

        let mut group_size = vec![0_u32; group_array_size];
        if protocol >= 7 {
            for group in &group_id {
                let idx = usize::try_from(*group).context("group index overflow")?;
                group_size[idx] =
                    u32::try_from(packet.gsmart2or4null()?).context("negative group size")?;
            }
        } else {
            for group in &group_id {
                let idx = usize::try_from(*group).context("group index overflow")?;
                group_size[idx] = u32::from(packet.g2()?);
            }
        }

        let mut group_file_ids = vec![None; group_array_size];
        let mut group_max_file_id = vec![0_u32; group_array_size];
        for group in &group_id {
            let idx = usize::try_from(*group).context("group index overflow")?;
            let size = usize::try_from(group_size[idx]).context("group file count too large")?;
            let mut ids = Vec::with_capacity(size);
            let mut last = 0_u32;
            let mut max_id = 0_u32;
            for _ in 0..size {
                let delta = if protocol >= 7 {
                    u32::try_from(packet.gsmart2or4null()?).context("negative file id delta")?
                } else {
                    u32::from(packet.g2()?)
                };
                last = last
                    .checked_add(delta)
                    .context("file id overflow while decoding archive index")?;
                max_id = max_id.max(last);
                ids.push(last);
            }
            group_max_file_id[idx] = max_id.saturating_add(1);
            if usize::try_from(max_id)
                .context("max file id too large")?
                .saturating_add(1)
                != size
            {
                group_file_ids[idx] = Some(ids);
            }
        }

        let mut group_file_names = has_names.then(|| vec![None; group_array_size]);
        if has_names {
            for group in &group_id {
                let idx = usize::try_from(*group).context("group index overflow")?;
                let size =
                    usize::try_from(group_size[idx]).context("group file count too large")?;
                let max_file =
                    usize::try_from(group_max_file_id[idx]).context("group max file too large")?;
                let mut names = vec![-1_i32; max_file];
                for i in 0..size {
                    let file_id = if let Some(ids) = &group_file_ids[idx] {
                        usize::try_from(ids[i]).context("file id overflow")?
                    } else {
                        i
                    };
                    names[file_id] = packet.g4s()?;
                }
                if let Some(all_names) = group_file_names.as_mut() {
                    all_names[idx] = Some(names);
                }
            }
        }

        Ok(Self {
            version,
            group_count,
            group_id,
            group_array_size,
            group_name_hash,
            group_size,
            group_file_ids,
            group_file_names,
            group_max_file_id,
            group_length,
        })
    }

    pub fn file_count_for_group(&self, group: u32) -> Result<usize> {
        let idx = usize::try_from(group).context("group id overflow")?;
        let count = *self
            .group_size
            .get(idx)
            .with_context(|| format!("group {group} out of range"))?;
        usize::try_from(count).context("group file count too large")
    }

    fn file_ids_for_group(&self, group: u32) -> Result<Vec<u32>> {
        let idx = usize::try_from(group).context("group id overflow")?;
        let count = self.file_count_for_group(group)?;
        if let Some(ids) = self.group_file_ids.get(idx).and_then(Clone::clone) {
            Ok(ids)
        } else {
            let mut ids = Vec::with_capacity(count);
            for i in 0..count {
                ids.push(u32::try_from(i).context("file id too large")?);
            }
            Ok(ids)
        }
    }
}

pub fn decompress(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut packet = Packet::new(compressed);
    let compression_type = packet.g1()?;
    let compressed_size = usize::try_from(packet.g4s()?).context("negative compressed size")?;
    if compressed.len() < 5 + compressed_size {
        bail!(
            "compressed payload shorter than header declares: {} < {}",
            compressed.len(),
            5 + compressed_size
        );
    }

    match compression_type {
        0 => Ok(compressed[5..5 + compressed_size].to_vec()),
        1 => {
            let uncompressed_size =
                usize::try_from(packet.g4s()?).context("negative bzip2 uncompressed size")?;
            let payload = &compressed[9..];
            let mut framed = Vec::with_capacity(4 + payload.len());
            framed.extend_from_slice(b"BZh1");
            framed.extend_from_slice(payload);
            let mut output = Vec::new();
            BzDecoder::new(Cursor::new(framed))
                .read_to_end(&mut output)
                .context("failed to decode js5 bzip2 payload")?;
            if output.len() != uncompressed_size {
                bail!(
                    "bzip2 size mismatch: decoded {} expected {}",
                    output.len(),
                    uncompressed_size
                );
            }
            Ok(output)
        }
        2 => {
            let uncompressed_size =
                usize::try_from(packet.g4s()?).context("negative gzip uncompressed size")?;
            let payload = &compressed[9..];
            let mut output = Vec::new();
            GzDecoder::new(Cursor::new(payload))
                .read_to_end(&mut output)
                .context("failed to decode js5 gzip payload")?;
            if output.len() != uncompressed_size {
                bail!(
                    "gzip size mismatch: decoded {} expected {}",
                    output.len(),
                    uncompressed_size
                );
            }
            Ok(output)
        }
        3 => {
            let uncompressed_size =
                usize::try_from(packet.g4s()?).context("negative lzma uncompressed size")?;
            let properties = packet.g1()?;
            let dictionary_le = packet.gdata(4)?;
            let payload = &compressed[14..];

            // lzma-rs expects LZMA-alone stream with 13-byte header.
            let mut lzma_alone = Vec::with_capacity(13 + payload.len());
            lzma_alone.push(properties);
            lzma_alone.extend_from_slice(&dictionary_le);
            lzma_alone.extend_from_slice(&(uncompressed_size as u64).to_le_bytes());
            lzma_alone.extend_from_slice(payload);

            let mut output = Vec::new();
            lzma_rs::lzma_decompress(&mut Cursor::new(lzma_alone), &mut output)
                .context("failed to decode js5 lzma payload")?;
            if output.len() != uncompressed_size {
                bail!(
                    "lzma size mismatch: decoded {} expected {}",
                    output.len(),
                    uncompressed_size
                );
            }
            Ok(output)
        }
        _ => bail!("unsupported js5 compression type: {compression_type}"),
    }
}

pub fn unpack_group(
    index: &ArchiveIndex,
    group: u32,
    compressed_data: &[u8],
) -> Result<BTreeMap<u32, Vec<u8>>> {
    let group_data = decompress(compressed_data)?;
    let file_count = index.file_count_for_group(group)?;
    if file_count == 1 {
        let mut out = BTreeMap::new();
        let file_id = index
            .file_ids_for_group(group)?
            .first()
            .copied()
            .unwrap_or(0);
        out.insert(file_id, group_data);
        return Ok(out);
    }

    let marker = usize::from(
        *group_data
            .last()
            .context("group payload missing chunk marker")?,
    );
    let footer_len = marker
        .checked_mul(file_count)
        .and_then(|v| v.checked_mul(4))
        .context("group footer size overflow")?;
    if group_data.len() < 1 + footer_len {
        bail!("group payload too short for chunk footer");
    }
    let file_sizes_pos = group_data.len() - 1 - footer_len;
    let mut size_packet = Packet::with_pos(&group_data, file_sizes_pos)?;
    let mut file_positions = vec![0_i64; file_count];
    for _ in 0..marker {
        let mut file_start = 0_i64;
        for position in &mut file_positions {
            let delta = i64::from(size_packet.g4s()?);
            file_start = file_start
                .checked_add(delta)
                .context("file chunk size overflow")?;
            if file_start < 0 {
                bail!("negative cumulative file chunk size");
            }
            *position = position
                .checked_add(file_start)
                .context("file size overflow while expanding chunks")?;
        }
    }

    let mut files = Vec::with_capacity(file_count);
    for size in &file_positions {
        files.push(vec![
            0_u8;
            usize::try_from(*size)
                .context("negative expanded file size")?
        ]);
    }
    file_positions.fill(0_i64);

    let mut size_packet = Packet::with_pos(&group_data, file_sizes_pos)?;
    let mut payload_offset = 0_i64;
    for _ in 0..marker {
        let mut chunk_len = 0_i64;
        for file in 0..file_count {
            let delta = i64::from(size_packet.g4s()?);
            chunk_len = chunk_len
                .checked_add(delta)
                .context("chunk length overflow")?;
            if chunk_len < 0 {
                bail!("negative cumulative chunk length");
            }
            let end_i64 = payload_offset
                .checked_add(chunk_len)
                .context("payload offset overflow")?;
            if end_i64 < 0 {
                bail!("negative payload offset");
            }
            let end = usize::try_from(end_i64).context("payload offset conversion failed")?;
            if end > file_sizes_pos {
                bail!("group payload chunk exceeds body length");
            }
            let src_start = usize::try_from(payload_offset).context("negative source start")?;
            let dst_start =
                usize::try_from(file_positions[file]).context("negative destination start")?;
            let dst_end_i64 = file_positions[file]
                .checked_add(chunk_len)
                .context("destination slice overflow")?;
            let dst_end = usize::try_from(dst_end_i64).context("negative destination end")?;
            files[file][dst_start..dst_end].copy_from_slice(&group_data[src_start..end]);
            file_positions[file] = dst_end_i64;
            payload_offset = end_i64;
        }
    }

    let mut map = BTreeMap::new();
    for (idx, file) in files.into_iter().enumerate() {
        let file_id = index.file_ids_for_group(group)?[idx];
        map.insert(file_id, file);
    }
    Ok(map)
}
