//! `unpack` — dump the full artifact tree (interfaces, configs, scripts, models,
//! audio, and the top-level binary/font/map/worldmap/defaults exports).
//!
//! The leaf dumps (`interfaces`, `varps`, `varbits`, `configs`, `cs2`, `models`,
//! `audio`) are delegated to their own command modules; the top-level export web
//! (worldmap, font metrics, UI-anim, defaults, …) lives here as it is used by no
//! other command.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgb};
use serde::Serialize;

use crate::animator::decode as decode_animator_controller;
use crate::cache::FlatCache;
use crate::cli::VarDomainArg;
use crate::cli::context::CommandContext;
use crate::cli::shared::{
    format_colour, format_coordgrid, format_map_element, java_string_hash, load_hash_name_map,
    write_binary, write_json, write_text,
};
use crate::commands::{audio, config_extract, cs2, models};
use crate::constants::{
    ARCHIVE_ANIMATOR, ARCHIVE_BINARY, ARCHIVE_CHUNK_INSTANCES, ARCHIVE_CUTSCENE2D,
    ARCHIVE_DEFAULTS, ARCHIVE_FONTMETRICS, ARCHIVE_MAPSQUARES, ARCHIVE_TTF, ARCHIVE_UI_ANIM,
    ARCHIVE_VFX, ARCHIVE_WORLDMAP, DEFAULTS_GROUP_AUDIO, DEFAULTS_GROUP_GRAPHICS,
    DEFAULTS_GROUP_TITLE, DEFAULTS_GROUP_WEARPOS, DEFAULTS_GROUP_WORLDMAP,
};
use crate::cutscene2d::decode as decode_cutscene2d;
use crate::fixture::ensure_archive_complete;
use crate::map::{decode_chunk_instance_stream, decode_map_square, decode_map_square_best_effort};
use crate::vfx::decode as decode_vfx;

/// Options for `unpack`.
#[derive(Clone, Debug)]
pub struct UnpackOpts {
    pub out_dir: PathBuf,
    pub sample_models: bool,
    pub skip_audio: bool,
    pub best_effort_maps: bool,
    pub max_audio_files: Option<usize>,
}

/// `unpack` — write the full artifact tree.
pub fn run(ctx: &CommandContext, opts: UnpackOpts) -> Result<()> {
    let UnpackOpts {
        out_dir,
        sample_models,
        skip_audio,
        best_effort_maps,
        max_audio_files,
    } = opts;
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let data_dir = ctx.data_dir();
    let build = ctx.build();

    let interface_dir = out_dir.join("interface");
    let config_dir = out_dir.join("config");
    let script_dir = out_dir.join("script");
    let model_dir = out_dir.join("model");
    let audio_dir = out_dir.join("audio");

    config_extract::run_interfaces(
        ctx,
        config_extract::InterfacesOpts {
            out_dir: Some(interface_dir),
        },
    )?;
    config_extract::run_varps(
        ctx,
        config_extract::VarpsOpts {
            out_file: Some(config_dir.join("varps.json")),
            domain: VarDomainArg::All,
        },
    )?;
    config_extract::run_varbits(
        ctx,
        config_extract::VarbitsOpts {
            out_file: Some(config_dir.join("varbits.json")),
        },
    )?;
    config_extract::run_configs(
        ctx,
        config_extract::ConfigsOpts {
            out_dir: Some(config_dir),
        },
    )?;
    cs2::run_dump(
        ctx,
        cs2::Cs2DumpOpts {
            out_file: Some(script_dir.join("scripts.json")),
            out_dir: Some(script_dir.join("decompiled")),
        },
    )?;

    if sample_models {
        models::run(
            ctx,
            models::ModelsOpts {
                out_file: Some(model_dir.join("models_sample.json")),
                out_dir: Some(model_dir.join("decoded")),
                sample_only: true,
            },
        )?;
    } else {
        models::run(
            ctx,
            models::ModelsOpts {
                out_file: Some(model_dir.join("models.json")),
                out_dir: Some(model_dir.join("decoded")),
                sample_only: false,
            },
        )?;
    }

    if !skip_audio {
        audio::run(
            ctx,
            audio::AudioOpts {
                out_dir: Some(audio_dir),
                max_files: max_audio_files,
            },
        )?;
    }

    run_top_level_exports(cache, tar_path, data_dir, &out_dir, build, best_effort_maps)?;

    Ok(())
}

fn run_top_level_exports(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    build: u32,
    best_effort_maps: bool,
) -> Result<()> {
    let hash_names = load_other_names_map(data_dir)?;

    export_archive_raw(
        cache,
        tar_path,
        ARCHIVE_BINARY,
        &out_dir.join("binary"),
        ".dat",
        &hash_names,
    )
    .context("export binary archive")?;
    export_archive_raw(
        cache,
        tar_path,
        ARCHIVE_TTF,
        &out_dir.join("ttf"),
        ".ttf",
        &hash_names,
    )
    .context("export ttf archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_FONTMETRICS,
        &out_dir.join("fontmetrics"),
        ".json",
        &hash_names,
        |_, _, data| parse_fontmetrics(data),
    )
    .context("export fontmetrics archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_VFX,
        &out_dir.join("vfx"),
        ".json",
        &hash_names,
        |_, _, data| Ok(decode_vfx(data)?),
    )
    .context("export vfx archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_ANIMATOR,
        &out_dir.join("animator"),
        ".json",
        &hash_names,
        |_, _, data| Ok(decode_animator_controller(data)?),
    )
    .context("export animator archive")?;
    export_archive_json(
        cache,
        tar_path,
        ARCHIVE_CUTSCENE2D,
        &out_dir.join("cutscene2d"),
        ".json",
        &hash_names,
        |_, _, data| Ok(decode_cutscene2d(data)?),
    )
    .context("export cutscene2d archive")?;

    export_group_json(
        cache,
        tar_path,
        ARCHIVE_UI_ANIM,
        0,
        &out_dir.join("uianimcurve"),
        ".json",
        |_, _, data| parse_uianimcurve(data),
    )
    .context("export uianimcurve group")?;
    export_group_json(
        cache,
        tar_path,
        ARCHIVE_UI_ANIM,
        1,
        &out_dir.join("uianim"),
        ".json",
        |_, _, data| parse_uianim(data),
    )
    .context("export uianim group")?;

    export_mapsquares_json(
        cache,
        tar_path,
        &out_dir.join("maps"),
        build,
        best_effort_maps,
    )
    .context("export mapsquares")?;
    export_chunk_instances_json(
        cache,
        tar_path,
        &out_dir.join("chunk-instances"),
        best_effort_maps,
    )
    .context("export chunk instances")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_GRAPHICS,
        &out_dir.join("config/graphics.defaults"),
        |id, data| parse_graphics_defaults(id, data, build),
    )
    .context("export graphics defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_AUDIO,
        &out_dir.join("config/audio.defaults"),
        |id, data| parse_audio_defaults(id, data, build),
    )
    .context("export audio defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_WEARPOS,
        &out_dir.join("config/wearpos.defaults"),
        parse_wearpos_defaults,
    )
    .context("export wearpos defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_WORLDMAP,
        &out_dir.join("config/worldmap.defaults"),
        parse_worldmap_defaults,
    )
    .context("export worldmap defaults")?;
    export_defaults_text(
        cache,
        tar_path,
        DEFAULTS_GROUP_TITLE,
        &out_dir.join("config/title.defaults"),
        parse_title_defaults,
    )
    .context("export title defaults")?;
    export_worldmap_dump(cache, tar_path, &out_dir.join("worldmap"))
        .context("export worldmap dump")?;
    export_worldarea_png(cache, tar_path, &out_dir.join("areas.png"))
        .context("export worldarea png")?;
    Ok(())
}

fn load_other_names_map(data_dir: &Path) -> Result<HashMap<i32, String>> {
    let other = data_dir.join("names/other.txt");
    if !other.is_file() {
        return Ok(HashMap::new());
    }
    load_hash_name_map(&other)
}

fn export_archive_raw(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    out_dir: &Path,
    extension: &str,
    hash_names: &HashMap<i32, String>,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let mut count = 0_usize;

    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, archive, *group)?;
        let group_name = resolve_group_name(&index, *group, hash_names);
        if files.len() == 1 && files.contains_key(&0) {
            let mut name = group_name
                .or_else(|| resolve_file_name(&index, *group, 0, hash_names))
                .unwrap_or_else(|| group.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            write_binary(
                &out_dir.join(sanitize_path_component(&name)),
                files[&0].as_slice(),
            )?;
            count += 1;
            continue;
        }

        let group_dir = out_dir.join(
            group_name
                .as_deref()
                .map(sanitize_path_component)
                .unwrap_or_else(|| group.to_string()),
        );
        fs::create_dir_all(&group_dir)
            .with_context(|| format!("failed creating {}", group_dir.display()))?;

        for (file, data) in files {
            let mut name = resolve_file_name(&index, *group, file, hash_names)
                .unwrap_or_else(|| file.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            write_binary(&group_dir.join(sanitize_path_component(&name)), &data)?;
            count += 1;
        }
    }

    Ok(count)
}

fn export_archive_json<T, F>(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    out_dir: &Path,
    extension: &str,
    hash_names: &HashMap<i32, String>,
    parse: F,
) -> Result<usize>
where
    T: Serialize,
    F: Fn(u32, u32, &[u8]) -> Result<T>,
{
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let mut count = 0_usize;

    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, archive, *group)?;
        let group_name = resolve_group_name(&index, *group, hash_names);
        if files.len() == 1 && files.contains_key(&0) {
            let mut name = group_name
                .or_else(|| resolve_file_name(&index, *group, 0, hash_names))
                .unwrap_or_else(|| group.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            let parsed = parse(*group, 0, files[&0].as_slice())?;
            write_json(&out_dir.join(sanitize_path_component(&name)), &parsed)?;
            count += 1;
            continue;
        }

        let group_dir = out_dir.join(
            group_name
                .as_deref()
                .map(sanitize_path_component)
                .unwrap_or_else(|| group.to_string()),
        );
        fs::create_dir_all(&group_dir)
            .with_context(|| format!("failed creating {}", group_dir.display()))?;

        for (file, data) in files {
            let mut name = resolve_file_name(&index, *group, file, hash_names)
                .unwrap_or_else(|| file.to_string());
            if !name.ends_with(extension) {
                name.push_str(extension);
            }
            let parsed = parse(*group, file, &data)?;
            write_json(&group_dir.join(sanitize_path_component(&name)), &parsed)?;
            count += 1;
        }
    }

    Ok(count)
}

fn export_group_json<T, F>(
    cache: &FlatCache,
    tar_path: &Path,
    archive: u32,
    group: u32,
    out_dir: &Path,
    extension: &str,
    parse: F,
) -> Result<usize>
where
    T: Serialize,
    F: Fn(u32, u32, &[u8]) -> Result<T>,
{
    if ensure_archive_complete(cache.root(), tar_path, archive).is_err() {
        return Ok(0);
    }
    let Some(_payload) = cache.get(archive, group)? else {
        return Ok(0);
    };

    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(archive)?;
    let files = cache.group_files_with_index(&index, archive, group)?;
    let mut count = 0_usize;
    for (file, data) in files {
        let parsed = parse(group, file, &data)?;
        let path = out_dir.join(format!("{file}{extension}"));
        write_json(&path, &parsed)?;
        count += 1;
    }
    Ok(count)
}

fn export_mapsquares_json(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    build: u32,
    best_effort: bool,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_MAPSQUARES).is_err() {
        return Ok(0);
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;

    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let mut count = 0_usize;
    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, ARCHIVE_MAPSQUARES, *group)?;
        let square_x = group & 0b111_1111;
        let square_z = group >> 7;
        let decoded = if best_effort {
            decode_map_square_best_effort(&files, build)
        } else {
            decode_map_square(&files, build).with_context(|| {
                format!("decode mapsquare group {group} ({square_x}_{square_z})")
            })?
        };
        let path = out_dir.join(format!("{square_x}_{square_z}.json"));
        write_json(&path, &decoded)?;
        count += 1;
    }

    Ok(count)
}

fn export_chunk_instances_json(
    cache: &FlatCache,
    tar_path: &Path,
    out_dir: &Path,
    best_effort: bool,
) -> Result<usize> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CHUNK_INSTANCES).is_err() {
        return Ok(0);
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;

    let index = cache.archive_index(ARCHIVE_CHUNK_INSTANCES)?;
    let mut count = 0_usize;
    for group in &index.group_id {
        let files = cache.group_files_with_index(&index, ARCHIVE_CHUNK_INSTANCES, *group)?;
        let Some(data) = files.get(&0) else {
            continue;
        };
        let decoded = match decode_chunk_instance_stream(data) {
            Ok(decoded) => decoded,
            Err(err) if best_effort => {
                eprintln!("chunk instance decode warning group {group}: {err}");
                continue;
            }
            Err(err) => return Err(err).with_context(|| format!("decode chunk instance {group}")),
        };
        let path = out_dir.join(format!("{group}.json"));
        write_json(&path, &decoded)?;
        count += 1;
    }

    Ok(count)
}

fn export_defaults_text<F>(
    cache: &FlatCache,
    tar_path: &Path,
    group: u32,
    out_file: &Path,
    parse: F,
) -> Result<usize>
where
    F: Fn(u32, &[u8]) -> Result<Vec<String>>,
{
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_DEFAULTS).is_err() {
        return Ok(0);
    }
    let index = cache.archive_index(ARCHIVE_DEFAULTS)?;
    if !index.group_id.contains(&group) {
        return Ok(0);
    }

    let files = cache.group_files_with_index(&index, ARCHIVE_DEFAULTS, group)?;
    if files.is_empty() {
        return Ok(0);
    }

    let mut file_ids = files.keys().copied().collect::<Vec<_>>();
    file_ids.sort_unstable();

    let mut lines = Vec::new();
    for file in &file_ids {
        let data = files
            .get(file)
            .with_context(|| format!("missing defaults file {file} in group {group}"))?;
        lines.extend(parse(*file, data)?);
        lines.push(String::new());
    }

    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    write_text(out_file, &lines.join("\n"))?;
    Ok(file_ids.len())
}

fn resolve_group_name(
    index: &crate::js5::ArchiveIndex,
    group: u32,
    hash_names: &HashMap<i32, String>,
) -> Option<String> {
    let names = index.group_name_hash.as_ref()?;
    let group_idx = usize::try_from(group).ok()?;
    let hash = *names.get(group_idx)?;
    if hash == -1 {
        return None;
    }
    hash_names.get(&hash).cloned()
}

fn resolve_file_name(
    index: &crate::js5::ArchiveIndex,
    group: u32,
    file: u32,
    hash_names: &HashMap<i32, String>,
) -> Option<String> {
    let group_names = index.group_file_names.as_ref()?;
    let group_idx = usize::try_from(group).ok()?;
    let file_idx = usize::try_from(file).ok()?;
    let file_hashes = group_names.get(group_idx)?.as_ref()?;
    let hash = *file_hashes.get(file_idx)?;
    if hash == -1 {
        return None;
    }
    hash_names.get(&hash).cloned()
}

fn sanitize_path_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']' | ',') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        String::from("unnamed")
    } else {
        out
    }
}

fn export_worldmap_dump(cache: &FlatCache, tar_path: &Path, out_dir: &Path) -> Result<()> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_WORLDMAP).is_err() {
        return Ok(());
    }
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed creating {}", out_dir.display()))?;
    let index = cache.archive_index(ARCHIVE_WORLDMAP)?;
    if let Some(main_group) = find_group_by_name(&index, "main")
        && let Some(details_file) = find_file_by_name(&index, main_group, "details.dat")
        && let Some(labels_file) = find_file_by_name(&index, main_group, "labels.dat")
    {
        let lines = export_worldmap_legacy(cache, &index, main_group, details_file, labels_file)?;
        write_text(&out_dir.join("dump.wma"), &lines.join("\n"))?;
        return Ok(());
    }

    let details_group = find_group_by_name(&index, "details").unwrap_or(0);
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, details_group)? else {
        return Ok(());
    };
    let details_files = crate::js5::unpack_group(&index, details_group, &payload)?;
    let mut lines = Vec::new();
    for (id, data) in details_files {
        let debug_name = unpack_worldmap_details(id, &data, &mut lines)?;
        unpack_worldmap_static_elements(cache, &index, &debug_name, &mut lines)?;
        unpack_worldmap_labels(cache, &index, &debug_name, &mut lines)?;
        lines.push(String::new());
    }

    write_text(&out_dir.join("dump.wma"), &lines.join("\n"))?;
    Ok(())
}

fn export_worldmap_legacy(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    group: u32,
    details_file: u32,
    labels_file: u32,
) -> Result<Vec<String>> {
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(Vec::new());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    let details = files.get(&details_file).with_context(|| {
        format!("legacy worldmap missing details file {details_file} in group {group}")
    })?;
    let labels = files.get(&labels_file).with_context(|| {
        format!("legacy worldmap missing labels file {labels_file} in group {group}")
    })?;

    let mut detail_packet = crate::packet::Packet::new(details);
    let mut lines = Vec::new();
    lines.push(String::from("[main]"));
    lines.push(format!(
        "origin={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(format!(
        "min={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(format!(
        "max={},{}",
        detail_packet.g2()?,
        detail_packet.g2()?
    ));
    lines.push(String::new());

    let mut label_packet = crate::packet::Packet::new(labels);
    let label_count = usize::from(label_packet.g2()?);
    for _ in 0..label_count {
        let text = label_packet.gjstr()?;
        let x = label_packet.g2()?;
        let y = label_packet.g2()?;
        let kind = label_packet.g1()?;
        lines.push(format!("label={x},{y},{text},{kind}"));
    }
    Ok(lines)
}

fn unpack_worldmap_details(id: u32, data: &[u8], lines: &mut Vec<String>) -> Result<String> {
    let mut packet = crate::packet::Packet::new(data);
    let debug_name = packet.gjstr()?;
    lines.push(format!("[{debug_name}]"));
    lines.push(format!("name={}", packet.gjstr()?));
    lines.push(format!("origin={}", format_coordgrid(packet.g4s()?)));
    lines.push(format!("background={}", format_colour(packet.g4s()?)));
    lines.push(format!("listed={}", yes_no(packet.g1()? == 1)));
    let default_zoom = packet.g1()?;
    lines.push(if default_zoom == u8::MAX {
        String::from("zoom=default")
    } else {
        format!("zoom={default_zoom}")
    });
    lines.push(format!("buildarea={}", packet.g1()?));
    let count = usize::from(packet.g1()?);
    for _ in 0..count {
        lines.push(format!(
            "subarea={},{},{},{},{},{},{},{},{}",
            packet.g1()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?,
            packet.g2()?
        ));
    }
    if !packet.is_done() {
        bail!("worldmap details {id} did not consume full payload");
    }
    Ok(debug_name)
}

fn unpack_worldmap_static_elements(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    debug_name: &str,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(group) = find_group_by_name(index, &format!("{debug_name}_staticelements")) else {
        return Ok(());
    };
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    for (_, data) in files {
        let mut packet = crate::packet::Packet::new(&data);
        lines.push(format!(
            "element={},{},{}",
            format_coordgrid(packet.g4s()?),
            format_map_element(packet.g2()?),
            packet.g1()?
        ));
    }
    Ok(())
}

fn unpack_worldmap_labels(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    debug_name: &str,
    lines: &mut Vec<String>,
) -> Result<()> {
    let Some(group) = find_group_by_name(index, &format!("{debug_name}_labels")) else {
        return Ok(());
    };
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, group)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(index, group, &payload)?;
    for (_, data) in files {
        let mut packet = crate::packet::Packet::new(&data);
        lines.push(format!(
            "label={},{},{}",
            format_coordgrid(packet.g4s()?),
            format_map_element(packet.g2()?),
            packet.g1()?
        ));
    }
    Ok(())
}

fn find_group_by_name(index: &crate::js5::ArchiveIndex, name: &str) -> Option<u32> {
    let hash = java_string_hash(name);
    let hashes = index.group_name_hash.as_ref()?;
    index.group_id.iter().copied().find(|group| {
        usize::try_from(*group)
            .ok()
            .and_then(|idx| hashes.get(idx))
            .is_some_and(|value| *value == hash)
    })
}

fn find_file_by_name(index: &crate::js5::ArchiveIndex, group: u32, name: &str) -> Option<u32> {
    let hash = java_string_hash(name);
    let group_idx = usize::try_from(group).ok()?;
    let names = index.group_file_names.as_ref()?.get(group_idx)?.as_ref()?;
    names.iter().enumerate().find_map(|(file, entry_hash)| {
        if *entry_hash == hash {
            u32::try_from(file).ok()
        } else {
            None
        }
    })
}

fn export_worldarea_png(cache: &FlatCache, tar_path: &Path, out_file: &Path) -> Result<()> {
    if ensure_archive_complete(cache.root(), tar_path, ARCHIVE_WORLDMAP).is_err() {
        return Ok(());
    }
    let index = cache.archive_index(ARCHIVE_WORLDMAP)?;
    let Some(payload) = cache.get(ARCHIVE_WORLDMAP, 3)? else {
        return Ok(());
    };
    let files = crate::js5::unpack_group(&index, 3, &payload)?;

    let width = 128_usize * 8;
    let height = 256_usize * 8;
    let mut image = vec![0_u8; width * height * 3];

    for (file, data) in files {
        let square_x = usize::try_from(file & 0x7f).context("square_x overflow")?;
        let square_z = usize::try_from(file >> 7).context("square_z overflow")?;
        let colors = decode_worldmap_color(&data)?;
        for zone_x in 0..8_usize {
            for zone_z in 0..8_usize {
                let x = 8 * square_x + zone_x;
                let z = 8 * square_z + zone_z;
                if x >= width || z >= height {
                    continue;
                }
                let color = colors[8 * zone_x + zone_z];
                let offset = ((height - 1 - z) * width + x) * 3;
                image[offset] = u8::try_from((color >> 16) & 0xff).context("red overflow")?;
                image[offset + 1] = u8::try_from((color >> 8) & 0xff).context("green overflow")?;
                image[offset + 2] = u8::try_from(color & 0xff).context("blue overflow")?;
            }
        }
    }

    let width_u32 = u32::try_from(width).context("worldarea png width overflow")?;
    let height_u32 = u32::try_from(height).context("worldarea png height overflow")?;
    let Some(buffer) = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width_u32, height_u32, image)
    else {
        bail!("failed to build worldarea image buffer");
    };
    if let Some(parent) = out_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed creating {}", parent.display()))?;
    }
    buffer
        .save(out_file)
        .with_context(|| format!("failed writing {}", out_file.display()))?;
    Ok(())
}

fn decode_worldmap_color(data: &[u8]) -> Result<[u32; 64]> {
    let mut result = [0_u32; 64];
    let mut packet = crate::packet::Packet::new(data);
    let mut index = 0_usize;
    let mut target = 0_usize;

    while target < 64 {
        let value = packet.g3()?;
        if packet.is_done() {
            target = 64;
        } else {
            target = target
                .checked_add(usize::from(packet.g1()?))
                .context("worldmap color run overflow")?;
        }
        while index < target && index < 64 {
            result[index] = value;
            index += 1;
        }
    }

    Ok(result)
}

fn parse_defaults_eof(kind: &str, id: u32, packet: &crate::packet::Packet<'_>) -> Result<()> {
    if packet.is_done() {
        return Ok(());
    }
    bail!("{kind}_{id} end of file not reached")
}

fn parse_audio_defaults(id: u32, data: &[u8], build: u32) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[audiodefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("audiodefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                let song = if build >= 912 {
                    packet.g4s()?
                } else {
                    i32::from(packet.g2()?)
                };
                lines.push(format!("titlescreensong={song}"));
            }
            opcode => bail!("audiodefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn format_wearpos(slot: u8) -> Result<&'static str> {
    let value = match slot {
        0 => "hat",
        1 => "back",
        2 => "front",
        3 => "righthand",
        4 => "torso",
        5 => "lefthand",
        6 => "arms",
        7 => "legs",
        8 => "head",
        9 => "hands",
        10 => "feet",
        11 => "jaw",
        12 => "ring",
        13 => "quiver",
        14 => "aura",
        15 => "wearpos_15",
        16 => "wearpos_16",
        17 => "pocket",
        18 => "wings",
        value => bail!("wearpos {value}"),
    };
    Ok(value)
}

fn parse_wearpos_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[wearposdefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("wearposdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(packet.g1()?.to_string());
                }
                lines.push(format!("unknown1={}", values.join(",")));
            }
            3 => lines.push(format!("lefthand={}", format_wearpos(packet.g1()?)?)),
            4 => lines.push(format!("righthand={}", format_wearpos(packet.g1()?)?)),
            5 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(format_wearpos(packet.g1()?)?.to_string());
                }
                lines.push(format!("lefthandextra={}", values.join(",")));
            }
            6 => {
                let count = usize::from(packet.g1()?);
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(format_wearpos(packet.g1()?)?.to_string());
                }
                lines.push(format!("righthandextra={}", values.join(",")));
            }
            opcode => bail!("wearposdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_worldmap_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[worldmapdefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("worldmapdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => lines.push(format!("unknown1={}", packet.g4s()?)),
            2 => lines.push(format!("membersfillcolour=0x{:x}", packet.g4s()? as u32)),
            3 => lines.push(format!("membersbordercolour=0x{:x}", packet.g4s()? as u32)),
            4 => lines.push(format!("membersborderthickness={}", packet.g1()?)),
            5 => lines.push(format!("memberschamferwidth={}", packet.g1()?)),
            6 => lines.push(format!("mainarea={}", packet.g4s()?)),
            7 => lines.push(format!("textshadowcolour=0x{:x}", packet.g4s()? as u32)),
            100 => lines.push(format!("font0zoom0={}", packet.g2()?)),
            101 => lines.push(format!("font1zoom0={}", packet.g2()?)),
            102 => lines.push(format!("font2zoom0={}", packet.g2()?)),
            108 => lines.push(format!("font0zoom1={}", packet.g2()?)),
            109 => lines.push(format!("font1zoom1={}", packet.g2()?)),
            110 => lines.push(format!("font2zoom1={}", packet.g2()?)),
            116 => lines.push(format!("font0zoom2={}", packet.g2()?)),
            117 => lines.push(format!("font1zoom2={}", packet.g2()?)),
            118 => lines.push(format!("font2zoom2={}", packet.g2()?)),
            124 => lines.push(format!("font0zoom3={}", packet.g2()?)),
            125 => lines.push(format!("font1zoom3={}", packet.g2()?)),
            126 => lines.push(format!("font2zoom3={}", packet.g2()?)),
            132 => lines.push(format!("font0zoom4={}", packet.g2()?)),
            133 => lines.push(format!("font1zoom4={}", packet.g2()?)),
            134 => lines.push(format!("font2zoom4={}", packet.g2()?)),
            opcode => bail!("worldmapdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_title_defaults(id: u32, data: &[u8]) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[titledefaults_{id}]")];
    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("titledefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => lines.push(format!(
                "title={},{}",
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?
            )),
            2 => {
                let count = usize::from(packet.g1()?);
                for _ in 0..count {
                    lines.push(format!("unknown2={},{}", packet.g1()?, packet.g1()?));
                }
            }
            opcode => bail!("titledefaults_{id} unknown opcode {opcode}"),
        }
    }
}

fn parse_graphics_defaults(id: u32, data: &[u8], build: u32) -> Result<Vec<String>> {
    let mut packet = crate::packet::Packet::new(data);
    let mut lines = vec![format!("[graphicsdefaults_{id}]")];
    let mut hitmark_count = 4_u8;

    loop {
        match packet.g1()? {
            0 => {
                parse_defaults_eof("graphicsdefaults", id, &packet)?;
                return Ok(lines);
            }
            1 => {
                for i in 0..hitmark_count {
                    lines.push(format!("hitmark{i}pos={},{}", packet.g2s()?, packet.g2s()?));
                }
            }
            2 => {
                let model = if build < 681 {
                    packet.g2null()?
                } else {
                    packet.gsmart2or4null()?
                };
                lines.push(format!("performancemetricsmodel={model}"));
            }
            3 => {
                hitmark_count = packet.g1()?;
                lines.push(format!("hitmarkcount={hitmark_count}"));
            }
            4 => lines.push(String::from("unknown4=no")),
            5 => lines.push(format!("titleinterface={}", packet.g3()?)),
            6 => lines.push(format!("lobbyinterface={}", packet.g3()?)),
            7 => {
                for i in 0..10_u8 {
                    for j in 0..4_u8 {
                        lines.push(format!("playerrecol{i}s{j}={}", packet.g2null()?));
                        let count = usize::from(packet.g2()?);
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(packet.g2null()?.to_string());
                        }
                        lines.push(format!("playerrecol{i}d{j}={}", values.join(",")));
                    }
                }
            }
            8 => lines.push(String::from("npcchatline=no")),
            9 => lines.push(format!("npcchatlineduration={}", packet.g1()?)),
            10 => lines.push(String::from("playerchatline=no")),
            11 => lines.push(format!("playerchatlineduration={}", packet.g1()?)),
            12 => lines.push(format!("initialsize={},{}", packet.g2()?, packet.g2()?)),
            13 => lines.push(format!("headbarcount={}", packet.g1()?)),
            14 => lines.push(format!("headbarupdatecount={}", packet.g1()?)),
            15 => lines.push(format!("entityoverlayoffset={}", packet.g1()?)),
            16 => lines.push(String::from("somethingcamera=yes")),
            17 => lines.push(format!("objnumcolour=0x{:x}", packet.g4s()? as u32)),
            18 => lines.push(format!("objnumcolourk=0x{:x}", packet.g4s()? as u32)),
            19 => lines.push(format!("objnumcolourm=0x{:x}", packet.g4s()? as u32)),
            20 => lines.push(format!(
                "spotshadowtexture={},{}",
                packet.g2()?,
                packet.g1()?
            )),
            21 => lines.push(format!("minimapscale={}", packet.g1()?)),
            22 => {
                let p11full = packet.gsmart2or4null()?;
                let p12full = packet.gsmart2or4null()?;
                let b12full = packet.gsmart2or4null()?;
                let hintheadicon = packet.gsmart2or4null()?;
                let hintmapmarker = packet.gsmart2or4null()?;
                let mapflag = packet.gsmart2or4null()?;
                let mapflag_origin = (packet.g1s()?, packet.g1s()?);
                let cross = packet.gsmart2or4null()?;
                let mapdot = packet.gsmart2or4null()?;
                let nameicon = packet.gsmart2or4null()?;
                let floorshadow = packet.gsmart2or4null()?;
                let compass = packet.gsmart2or4null()?;
                let otherlevel = packet.gsmart2or4null()?;
                let mapedge = packet.gsmart2or4null()?;
                lines.push(format!(
                    "sprites={p11full},{p12full},{b12full},{hintheadicon},{hintmapmarker},{mapflag},{},{},{cross},{mapdot},{nameicon},{floorshadow},{compass},{otherlevel},{mapedge}",
                    mapflag_origin.0, mapflag_origin.1
                ));
            }
            23 => {
                for i in 0..10_u8 {
                    for j in 0..4_u8 {
                        lines.push(format!("playerretex{i}s{j}={}", packet.g2null()?));
                        let count = usize::from(packet.g2()?);
                        let mut values = Vec::with_capacity(count);
                        for _ in 0..count {
                            values.push(packet.g2null()?.to_string());
                        }
                        lines.push(format!("playerretex{i}d{j}={}", values.join(",")));
                    }
                }
            }
            24 => lines.push(format!("unknown24={}", packet.g4s()?)),
            25 => lines.push(format!(
                "unknown25={},{},{},{},{},{}",
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?,
                packet.gsmart2or4null()?
            )),
            26 => lines.push(format!("objnumcolourb=0x{:x}", packet.g4s()? as u32)),
            27 => lines.push(format!("objnumcolourt=0x{:x}", packet.g4s()? as u32)),
            28 => lines.push(format!("objnumcolourq=0x{:x}", packet.g4s()? as u32)),
            29 => lines.push(format!("unknown29={},{}", packet.g4s()?, packet.g4s()?)),
            opcode => bail!("graphicsdefaults_{id} unknown opcode {opcode}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct UiAnimCurveEntry {
    keyframes: Vec<[f32; 4]>,
}

fn parse_uianimcurve(data: &[u8]) -> Result<UiAnimCurveEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let count = usize::from(packet.g1()?);
    let mut keyframes = Vec::with_capacity(count);
    for _ in 0..count {
        keyframes.push([
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
            read_f32_be(&mut packet)?,
        ]);
    }
    if !packet.is_done() {
        bail!("uianimcurve did not consume full payload");
    }
    Ok(UiAnimCurveEntry { keyframes })
}

#[derive(Clone, Debug, Serialize)]
struct UiAnimEntry {
    mode: u8,
    curve: Option<i32>,
    easing_type: Option<i32>,
    easing_unknown: bool,
    target: u8,
    target_mode: u8,
    values: Vec<Vec<i32>>,
}

fn parse_uianim(data: &[u8]) -> Result<UiAnimEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let mode = packet.g1()?;
    let (curve, easing_type, easing_unknown) = match mode {
        1 => (Some(packet.g4s()?), None, false),
        2 => (None, Some(packet.g4s()?), packet.g1()? == 1),
        value => bail!("unknown uianim mode {value}"),
    };

    let target = packet.g1()?;
    let target_mode = packet.g1()?;
    let count = usize::from(packet.g2()?);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        if target == 0 || target == 3 {
            values.push(vec![packet.g4s()?, packet.g4s()?]);
        } else if target == 6 {
            values.push(vec![packet.g4s()?, packet.g4s()?, packet.g4s()?]);
        } else {
            values.push(vec![packet.g4s()?]);
        }
    }
    if !packet.is_done() {
        bail!("uianim did not consume full payload");
    }

    Ok(UiAnimEntry {
        mode,
        curve,
        easing_type,
        easing_unknown,
        target,
        target_mode,
        values,
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum FontSourceType {
    SpriteBitmap,
    SpriteFontsheet,
    Vector,
}

#[derive(Clone, Debug, Serialize)]
struct FontGlyphInfo {
    width: u8,
    height: u8,
    bearing_y: u8,
}

#[derive(Clone, Debug, Serialize)]
struct FontSheetPosition {
    x: u16,
    y: u16,
}

#[derive(Clone, Debug, Serialize)]
struct FontKerningData {
    left_kern: Vec<Vec<i8>>,
    right_kern: Vec<Vec<i8>>,
}

#[derive(Clone, Debug, Serialize)]
struct FontMetricsEntry {
    source_type: FontSourceType,
    source_pack_id: Option<i32>,
    pixel_size: Option<u8>,
    glyph_info: Vec<FontGlyphInfo>,
    font_sheet_width: Option<u16>,
    font_sheet_height: Option<u16>,
    font_sheet_position: Vec<FontSheetPosition>,
    base_line: Option<u8>,
    upper_case_ascent: Option<u8>,
    byte3049: Option<u8>,
    max_ascent: Option<u8>,
    max_descent: Option<u8>,
    scale: Option<u8>,
    kerning_data: Option<FontKerningData>,
}

fn parse_fontmetrics(data: &[u8]) -> Result<FontMetricsEntry> {
    let mut packet = crate::packet::Packet::new(data);
    let source_type = match packet.g1()? {
        0 => FontSourceType::SpriteBitmap,
        1 => FontSourceType::SpriteFontsheet,
        2 => FontSourceType::Vector,
        value => bail!("invalid font source type id {value}"),
    };

    match source_type {
        FontSourceType::Vector => {
            let entry = FontMetricsEntry {
                source_type,
                source_pack_id: Some(packet.g4s()?),
                pixel_size: Some(packet.g1()?),
                glyph_info: Vec::new(),
                font_sheet_width: None,
                font_sheet_height: None,
                font_sheet_position: Vec::new(),
                base_line: None,
                upper_case_ascent: None,
                byte3049: None,
                max_ascent: None,
                max_descent: None,
                scale: None,
                kerning_data: None,
            };
            if !packet.is_done() {
                bail!("fontmetrics vector did not consume full payload");
            }
            Ok(entry)
        }
        FontSourceType::SpriteBitmap | FontSourceType::SpriteFontsheet => {
            let complex_kerning = packet.g1()? == 1;
            let source_pack_id = match source_type {
                FontSourceType::SpriteFontsheet => Some(packet.g4s()?),
                FontSourceType::SpriteBitmap | FontSourceType::Vector => None,
            };

            let mut glyph_info = vec![
                FontGlyphInfo {
                    width: 0,
                    height: 0,
                    bearing_y: 0,
                };
                256
            ];
            for glyph in &mut glyph_info {
                glyph.width = packet.g1()?;
            }
            for glyph in &mut glyph_info {
                glyph.height = packet.g1()?;
            }
            for glyph in &mut glyph_info {
                glyph.bearing_y = packet.g1()?;
            }

            let font_sheet_width = packet.g2()?;
            let font_sheet_height = packet.g2()?;
            let mut positions = vec![FontSheetPosition { x: 0, y: 0 }; 256];
            for item in &mut positions {
                item.x = packet.g2()?;
            }
            for item in &mut positions {
                item.y = packet.g2()?;
            }

            let kerning_data = if complex_kerning {
                Some(parse_font_kerning(&mut packet)?)
            } else {
                None
            };
            let base_line = if complex_kerning {
                Some(0)
            } else {
                Some(packet.g1()?)
            };

            let entry = FontMetricsEntry {
                source_type,
                source_pack_id,
                pixel_size: None,
                glyph_info,
                font_sheet_width: Some(font_sheet_width),
                font_sheet_height: Some(font_sheet_height),
                font_sheet_position: positions,
                base_line,
                upper_case_ascent: Some(packet.g1()?),
                byte3049: Some(packet.g1()?),
                max_ascent: Some(packet.g1()?),
                max_descent: Some(packet.g1()?),
                scale: Some(packet.g1()?),
                kerning_data,
            };

            if !packet.is_done() {
                bail!("fontmetrics sprite did not consume full payload");
            }
            Ok(entry)
        }
    }
}

fn parse_font_kerning(packet: &mut crate::packet::Packet<'_>) -> Result<FontKerningData> {
    let mut right_kern = Vec::with_capacity(256);
    for _ in 0..256_usize {
        let mut kerns = Vec::with_capacity(256);
        let mut kern = 0_i32;
        for _ in 0..256_usize {
            kern += i32::from(packet.g1s()?);
            kerns.push(kern as i8);
        }
        right_kern.push(kerns);
    }

    let mut left_kern = Vec::with_capacity(256);
    for _ in 0..256_usize {
        let mut kerns = Vec::with_capacity(256);
        let mut kern = 0_i32;
        for _ in 0..256_usize {
            kern += i32::from(packet.g1s()?);
            kerns.push(kern as i8);
        }
        left_kern.push(kerns);
    }

    Ok(FontKerningData {
        left_kern,
        right_kern,
    })
}

fn read_f32_be(packet: &mut crate::packet::Packet<'_>) -> Result<f32> {
    Ok(f32::from_bits(packet.g4s()? as u32))
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
