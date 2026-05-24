use anyhow::{Context, Result};
use rs3_cache_rs::animator::decode as decode_animator_controller;
use rs3_cache_rs::audio::{AudioKind, inspect_audio_file};
use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::cli::{Cli, Command, run as run_cli};
use rs3_cache_rs::config::{
    parse_achievement, parse_area, parse_bas, parse_billboard, parse_bugtemplate, parse_category,
    parse_controller, parse_cursor, parse_gamelogevent, parse_headbar, parse_hitmark, parse_hunt,
    parse_idk, parse_inv, parse_itemcode, parse_light, parse_loc, parse_material, parse_mel,
    parse_mesanim, parse_msi, parse_npc, parse_obj, parse_overlay, parse_particle_effector,
    parse_particle_emitter, parse_quest, parse_quickchatcat, parse_quickchatphrase, parse_seq,
    parse_seqgroup, parse_skybox, parse_spot, parse_struct, parse_stylesheet, parse_texture,
    parse_underlay, parse_var_client_string, parse_var_npc_bit, parse_var_shared,
    parse_var_shared_string, parse_water, parse_worldarea,
};
use rs3_cache_rs::constants::{
    ARCHIVE_ACHIEVEMENTS, ARCHIVE_ANIMATOR, ARCHIVE_AUDIOSTREAMS, ARCHIVE_BILLBOARDS,
    ARCHIVE_CLIENTSCRIPTS, ARCHIVE_CONFIG, ARCHIVE_CUTSCENE2D, ARCHIVE_INTERFACES, ARCHIVE_JINGLES,
    ARCHIVE_LOC_CONFIG, ARCHIVE_MAPSQUARES, ARCHIVE_MATERIALS, ARCHIVE_MODELS_RT7,
    ARCHIVE_NPC_CONFIG, ARCHIVE_OBJ_CONFIG, ARCHIVE_PARTICLES, ARCHIVE_QUICKCHAT_CONFIG,
    ARCHIVE_SEQ_CONFIG, ARCHIVE_SONGS, ARCHIVE_SPOT_CONFIG, ARCHIVE_STRUCT_CONFIG,
    ARCHIVE_STYLESHEETS, ARCHIVE_SYNTH, ARCHIVE_TEXTURES, ARCHIVE_VFX, ARCHIVE_VORBIS, BUILD,
    CONFIG_GROUP_ACHIEVEMENT_ARCHIVE57, CONFIG_GROUP_AREA, CONFIG_GROUP_BAS,
    CONFIG_GROUP_BILLBOARD_ARCHIVE29, CONFIG_GROUP_BUGTEMPLATE, CONFIG_GROUP_CATEGORY,
    CONFIG_GROUP_CONTROLLER, CONFIG_GROUP_CURSOR, CONFIG_GROUP_GAMELOGEVENT, CONFIG_GROUP_HEADBAR,
    CONFIG_GROUP_HITMARK, CONFIG_GROUP_HUNT, CONFIG_GROUP_IDK, CONFIG_GROUP_INV,
    CONFIG_GROUP_ITEMCODE, CONFIG_GROUP_LIGHT, CONFIG_GROUP_LOC_LEGACY, CONFIG_GROUP_MEL,
    CONFIG_GROUP_MESANIM, CONFIG_GROUP_MSI, CONFIG_GROUP_NPC_LEGACY, CONFIG_GROUP_OBJ_LEGACY,
    CONFIG_GROUP_OVERLAY, CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27,
    CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27, CONFIG_GROUP_QUEST,
    CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24, CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24, CONFIG_GROUP_SEQ,
    CONFIG_GROUP_SEQGROUP, CONFIG_GROUP_SKYBOX, CONFIG_GROUP_SPOT, CONFIG_GROUP_STRUCT,
    CONFIG_GROUP_UNDERLAY, CONFIG_GROUP_VAR_BIT, CONFIG_GROUP_VAR_CLAN,
    CONFIG_GROUP_VAR_CLAN_SETTING, CONFIG_GROUP_VAR_CLIENT, CONFIG_GROUP_VAR_CLIENT_STRING,
    CONFIG_GROUP_VAR_CONTROLLER, CONFIG_GROUP_VAR_GLOBAL, CONFIG_GROUP_VAR_NPC,
    CONFIG_GROUP_VAR_NPC_BIT, CONFIG_GROUP_VAR_OBJECT, CONFIG_GROUP_VAR_PLAYER,
    CONFIG_GROUP_VAR_PLAYER_GROUP, CONFIG_GROUP_VAR_REGION, CONFIG_GROUP_VAR_SHARED,
    CONFIG_GROUP_VAR_SHARED_STRING, CONFIG_GROUP_VAR_WORLD, CONFIG_GROUP_WATER,
    CONFIG_GROUP_WORLDAREA, SUBBUILD,
};
use rs3_cache_rs::cutscene2d::decode as decode_cutscene2d;
use rs3_cache_rs::fixture::{
    default_cache_dir, default_tar_path, ensure_archive_complete, ensure_archive_groups,
};
use rs3_cache_rs::interface::render_interface_group;
use rs3_cache_rs::map::decode_map_square;
use rs3_cache_rs::model::Model;
use rs3_cache_rs::script::{
    OpcodeBook, decode_script, encode_script, parse_cs2_asm, script_to_asm,
};
use rs3_cache_rs::vars::{VarDomain, parse_var, parse_varbit};
use rs3_cache_rs::vfx::decode as decode_vfx;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn extraction_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn cache_dir() -> PathBuf {
    if let Ok(path) = std::env::var("RS3_CACHE_DIR") {
        return PathBuf::from(path);
    }
    default_cache_dir()
}

fn tar_path() -> PathBuf {
    if let Ok(path) = std::env::var("RS3_CACHE_TAR") {
        return PathBuf::from(path);
    }
    default_tar_path()
}

fn data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("RS3_DATA_DIR") {
        return PathBuf::from(path);
    }
    PathBuf::from("data")
}

fn lock_guard() -> std::sync::MutexGuard<'static, ()> {
    match extraction_lock().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn parse_optional_config_group<T, F>(
    cache: &FlatCache,
    config_index: &rs3_cache_rs::js5::ArchiveIndex,
    group: u32,
    mut parse: F,
) -> Result<usize>
where
    F: FnMut(u32, &[u8]) -> Result<T>,
{
    let Some(payload) = cache.get(ARCHIVE_CONFIG, group)? else {
        return Ok(0);
    };
    let files = rs3_cache_rs::js5::unpack_group(config_index, group, &payload)?;
    let mut count = 0_usize;
    for (id, data) in files {
        let _parsed = parse(id, &data)?;
        count += 1;
    }
    Ok(count)
}

fn parse_optional_archive_group<T, F>(
    cache: &FlatCache,
    archive: u32,
    index: &rs3_cache_rs::js5::ArchiveIndex,
    group: u32,
    mut parse: F,
) -> Result<usize>
where
    F: FnMut(u32, &[u8]) -> Result<T>,
{
    let Some(payload) = cache.get(archive, group)? else {
        return Ok(0);
    };
    let files = rs3_cache_rs::js5::unpack_group(index, group, &payload)?;
    let mut count = 0_usize;
    for (id, data) in files {
        let _parsed = parse(id, &data)?;
        count += 1;
    }
    Ok(count)
}

fn parse_archive_with_bits<T, F>(
    cache: &FlatCache,
    archive: u32,
    index: &rs3_cache_rs::js5::ArchiveIndex,
    bits: u32,
    mut parse: F,
) -> Result<usize>
where
    F: FnMut(u32, &[u8]) -> Result<T>,
{
    let mut count = 0_usize;
    for group in &index.group_id {
        let files = cache.group_files_with_index(index, archive, *group)?;
        for (file, data) in files {
            let id = (group << bits) | file;
            let _parsed = parse(id, &data)?;
            count += 1;
        }
    }
    Ok(count)
}

fn parse_struct_count(
    cache: &FlatCache,
    config_index: &rs3_cache_rs::js5::ArchiveIndex,
    has_struct_archive: bool,
) -> Result<usize> {
    if has_struct_archive {
        let struct_index = cache.archive_index(ARCHIVE_STRUCT_CONFIG)?;
        let mut count = 0_usize;
        for group in &struct_index.group_id {
            let files =
                cache.group_files_with_index(&struct_index, ARCHIVE_STRUCT_CONFIG, *group)?;
            for (file, data) in files {
                let id = (group << 5) | file;
                let parsed = parse_struct(id, &data)?;
                assert_eq!(id, parsed.id);
                count += 1;
            }
        }
        Ok(count)
    } else {
        parse_optional_config_group(cache, config_index, CONFIG_GROUP_STRUCT, |id, data| {
            let parsed = parse_struct(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })
    }
}

#[test]
fn cache_indexes_match_build947_snapshot() -> Result<()> {
    let _guard = lock_guard();
    let cache = FlatCache::open(cache_dir())?;
    let config = cache.archive_index(ARCHIVE_CONFIG)?;
    let interfaces = cache.archive_index(ARCHIVE_INTERFACES)?;
    let scripts = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let enums = cache.archive_index(17)?;
    let models = cache.archive_index(ARCHIVE_MODELS_RT7)?;

    assert_eq!(39, config.group_count);
    assert_eq!(1869, interfaces.group_count);
    assert_eq!(20577, scripts.group_count);
    assert_eq!(69, enums.group_count);
    assert_eq!(140_131, models.group_count);
    assert_eq!(0, scripts.group_id[0]);
    assert_eq!(
        20707,
        *scripts.group_id.last().expect("script id list empty")
    );
    assert_eq!(0, models.group_id[0]);
    assert_eq!(
        140_130,
        *models.group_id.last().expect("model id list empty")
    );
    Ok(())
}

#[test]
fn parses_every_interface_file() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    ensure_archive_complete(&cache_dir, &tar_path(), ARCHIVE_INTERFACES)?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let expected_files: usize = index
        .group_id
        .iter()
        .map(|group| index.group_size[*group as usize] as usize)
        .sum();

    let mut files = 0usize;
    let mut bytes = 0usize;
    for group in &index.group_id {
        let unpacked = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, *group)?;
        for data in unpacked.values() {
            files += 1;
            bytes += data.len();
        }
    }

    assert_eq!(expected_files, files);
    assert!(bytes > files);
    Ok(())
}

#[test]
fn interface_group_zero_model_layer_shape_matches_gold() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    ensure_archive_groups(&cache_dir, &tar_path(), ARCHIVE_INTERFACES, &[0])?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, 0)?;
    let lines = render_interface_group(0, &files, BUILD);
    let source = lines.join("\n");
    let component_count = source.matches("[com").count();

    assert_eq!(21, component_count);
    assert!(source.contains("[com0]\ntype=model"));
    assert!(source.contains("model=model_13369"));
    assert!(source.contains("modelanim=seq_3453"));
    assert!(source.contains("[com20]\ntype=layer"));
    assert!(source.contains("hide=yes"));
    Ok(())
}

#[test]
fn interface_group_569_text_rectangle_and_hook_fields_match_gold() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    ensure_archive_groups(&cache_dir, &tar_path(), ARCHIVE_INTERFACES, &[569])?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, 569)?;
    let lines = render_interface_group(569, &files, BUILD);
    let source = lines.join("\n");

    assert_eq!(31, source.matches("[com").count());
    assert!(source.contains("[com8]\ntype=rectangle"));
    assert!(source.contains("colour=0xff7600"));
    assert!(source.contains("fill=yes"));
    assert!(source.contains("[com10]\ntype=text"));
    assert!(source.contains("textfont=fontmetrics_26"));
    assert!(source.contains("text=Salt solution (Level 74)"));
    assert!(source.contains("textshadow=yes"));
    assert!(source.contains("transmitop1=yes"));
    assert!(source.contains("onload=clientscript_6233(event_com, -1, 28556, \"\")"));
    assert!(source.contains("onload=clientscript_4242(event_com, event_comsubid, 26, 28, 3)"));
    assert!(!source.contains("type=unsupported"));
    assert!(!source.contains("parse_error="));
    Ok(())
}

#[test]
fn interface_group_1027_graphic_varc_and_opcursor_fields_match_gold() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    ensure_archive_groups(&cache_dir, &tar_path(), ARCHIVE_INTERFACES, &[1027])?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_INTERFACES)?;
    let files = cache.group_files_with_index(&index, ARCHIVE_INTERFACES, 1027)?;
    let lines = render_interface_group(1027, &files, BUILD);
    let source = lines.join("\n");

    assert!(source.contains("[com2]\ntype=graphic"));
    assert!(source.contains("graphic=graphic_3293"));
    assert!(source.contains("clickmask=no"));
    assert!(source.contains("onvarctransmit=clientscript_424"));
    assert!(source.contains("onvarctransmitlist=varclient_1365"));
    assert!(source.contains("[com10]\ntype=layer"));
    assert!(source.contains("op1=Select"));
    assert!(source.contains("opcursor0=cursor_46"));
    assert!(!source.contains("type=unsupported"));
    assert!(!source.contains("parse_error="));
    Ok(())
}

#[test]
fn decodes_all_cs2_scripts_and_opcodes() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    ensure_archive_complete(&cache_dir, &tar_path(), ARCHIVE_CLIENTSCRIPTS)?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let opcode_book = OpcodeBook::load(&data_dir(), BUILD, SUBBUILD)?;

    let mut scripts = 0usize;
    let mut instructions = 0usize;
    let mut unique = std::collections::BTreeSet::new();
    for group in &index.group_id {
        let unpacked = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
        for (_, data) in unpacked {
            let script = decode_script(&data, &opcode_book, BUILD)?;
            scripts += 1;
            instructions += script.code.len();
            for instruction in script.code {
                unique.insert(instruction.command);
            }
        }
    }

    assert_eq!(20577, scripts);
    assert!(instructions > 1_000_000);
    assert!(unique.len() > 300);
    assert!(unique.contains("push_varbit"));
    assert!(unique.contains("gosub_with_params"));
    assert!(unique.contains("switch"));
    Ok(())
}

#[test]
fn asm_encode_roundtrip_byte_perfect() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    ensure_archive_complete(&cache_dir, &tar_path(), ARCHIVE_CLIENTSCRIPTS)?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let opcode_book = OpcodeBook::load(&data_dir(), BUILD, SUBBUILD)?;

    let mut tested = 0usize;
    let mut total_instructions = 0usize;

    for group in &index.group_id {
        let unpacked = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
        for (_, data) in unpacked {
            if tested >= 100 {
                break;
            }

            let original = decode_script(&data, &opcode_book, BUILD)?;

            // Roundtrip 1: ASM format (decode → asm → parse → compare)
            let asm = script_to_asm(&original);
            let from_asm = parse_cs2_asm(&asm)
                .with_context(|| format!("parse_cs2_asm failed for script {:?}", original.name))?;
            assert_eq!(original.name, from_asm.name, "name mismatch via ASM");
            assert_eq!(original.local_count_int, from_asm.local_count_int);
            assert_eq!(original.local_count_object, from_asm.local_count_object);
            assert_eq!(original.local_count_long, from_asm.local_count_long);
            assert_eq!(original.argument_count_int, from_asm.argument_count_int);
            assert_eq!(
                original.argument_count_object,
                from_asm.argument_count_object
            );
            assert_eq!(original.argument_count_long, from_asm.argument_count_long);
            assert_eq!(
                original.code.len(),
                from_asm.code.len(),
                "instruction count mismatch via ASM"
            );
            for (i, (orig, parsed)) in original.code.iter().zip(from_asm.code.iter()).enumerate() {
                assert_eq!(
                    orig.command, parsed.command,
                    "command mismatch at [{i}] via ASM"
                );
                assert_operand_eq(&orig.operand, &parsed.operand, i);
            }

            // Roundtrip 2: binary encode (decode → encode → decode → compare)
            let encoded = encode_script(&original, &opcode_book, BUILD)?;
            let re_decoded = decode_script(&encoded, &opcode_book, BUILD)?;
            assert_eq!(original.name, re_decoded.name, "name mismatch via binary");
            assert_eq!(original.local_count_int, re_decoded.local_count_int);
            assert_eq!(original.local_count_object, re_decoded.local_count_object);
            assert_eq!(original.local_count_long, re_decoded.local_count_long);
            assert_eq!(original.argument_count_int, re_decoded.argument_count_int);
            assert_eq!(
                original.argument_count_object,
                re_decoded.argument_count_object
            );
            assert_eq!(original.argument_count_long, re_decoded.argument_count_long);
            assert_eq!(
                original.code.len(),
                re_decoded.code.len(),
                "instruction count mismatch via binary"
            );
            for (i, (orig, redec)) in original.code.iter().zip(re_decoded.code.iter()).enumerate() {
                assert_eq!(
                    orig.command, redec.command,
                    "command mismatch at [{i}] via binary"
                );
                assert_operand_eq(&orig.operand, &redec.operand, i);
            }

            total_instructions += original.code.len();
            tested += 1;
        }
        if tested >= 100 {
            break;
        }
    }

    eprintln!("Roundtrip OK: {tested} scripts, {total_instructions} total instructions");
    Ok(())
}

fn assert_operand_eq(
    a: &rs3_cache_rs::script::Operand,
    b: &rs3_cache_rs::script::Operand,
    idx: usize,
) {
    use rs3_cache_rs::script::Operand;
    match (a, b) {
        (Operand::Int(av), Operand::Int(bv)) => {
            assert_eq!(av, bv, "operand Int mismatch at [{idx}]");
        }
        (Operand::Long(av), Operand::Long(bv)) => {
            assert_eq!(av, bv, "operand Long mismatch at [{idx}]");
        }
        (Operand::Str(av), Operand::Str(bv)) => {
            assert_eq!(av, bv, "operand Str mismatch at [{idx}]");
        }
        (Operand::Local(av), Operand::Local(bv)) => {
            assert_eq!(av, bv, "operand Local mismatch at [{idx}]");
        }
        (Operand::VarRef(av), Operand::VarRef(bv)) => {
            assert_eq!(av.domain, bv.domain, "VarRef domain mismatch at [{idx}]");
            assert_eq!(av.id, bv.id, "VarRef id mismatch at [{idx}]");
            assert_eq!(
                av.transmog, bv.transmog,
                "VarRef transmog mismatch at [{idx}]"
            );
        }
        (Operand::VarBitRef(av), Operand::VarBitRef(bv)) => {
            assert_eq!(av.id, bv.id, "VarBitRef id mismatch at [{idx}]");
            assert_eq!(
                av.transmog, bv.transmog,
                "VarBitRef transmog mismatch at [{idx}]"
            );
        }
        (Operand::Branch(av), Operand::Branch(bv)) => {
            assert_eq!(av, bv, "Branch target mismatch at [{idx}]");
        }
        (Operand::Switch(av), Operand::Switch(bv)) => {
            assert_eq!(av.len(), bv.len(), "Switch case count mismatch at [{idx}]");
            for (j, (ac, bc)) in av.iter().zip(bv.iter()).enumerate() {
                assert_eq!(
                    ac.value, bc.value,
                    "Switch case[{j}] value mismatch at [{idx}]"
                );
                assert_eq!(
                    ac.target, bc.target,
                    "Switch case[{j}] target mismatch at [{idx}]"
                );
            }
        }
        (Operand::Script(av), Operand::Script(bv)) => {
            assert_eq!(av, bv, "Script operand mismatch at [{idx}]");
        }
        (Operand::Array(av), Operand::Array(bv)) => {
            assert_eq!(av, bv, "Array operand mismatch at [{idx}]");
        }
        (Operand::Count(av), Operand::Count(bv)) => {
            assert_eq!(av, bv, "Count operand mismatch at [{idx}]");
        }
        (Operand::Byte(av), Operand::Byte(bv)) => {
            assert_eq!(av, bv, "Byte operand mismatch at [{idx}]");
        }
        _ => panic!("operand type mismatch at [{idx}]: {a:?} vs {b:?}"),
    }
}

#[test]
fn asm_encode_roundtrip_byte_perfect_910() -> Result<()> {
    const BUILD_910: u32 = 910;
    const DEFAULT_910_CACHE: &str = "/tmp/rs3-cache-rs-910/cache";
    const DEFAULT_910_TAR: &str = "/Users/robert/projects/ignis/static/cache-runescape-live-en-b910-2019-12-11-00-00-00-openrs2#1730.tar";

    let cache_dir = std::env::var("RS3_CACHE_DIR_910")
        .map_or_else(|_| PathBuf::from(DEFAULT_910_CACHE), PathBuf::from);
    if !cache_dir.join("255/12.dat").is_file() {
        eprintln!("SKIP (no 910 cache at {})", cache_dir.display());
        return Ok(());
    }

    let tar = std::env::var("RS3_CACHE_TAR_910")
        .map_or_else(|_| PathBuf::from(DEFAULT_910_TAR), PathBuf::from);

    let _guard = lock_guard();
    ensure_archive_complete(&cache_dir, &tar, ARCHIVE_CLIENTSCRIPTS)?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let opcode_book = OpcodeBook::load(&data_dir(), BUILD_910, 0)?;

    let mut tested = 0usize;
    let mut total_instructions = 0usize;

    for group in &index.group_id {
        let unpacked = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
        for (_, data) in unpacked {
            if tested >= 100 {
                break;
            }

            let original = decode_script(&data, &opcode_book, BUILD_910)?;

            // ASM roundtrip
            let asm = script_to_asm(&original);
            let from_asm = parse_cs2_asm(&asm)?;
            assert_eq!(original.name, from_asm.name, "910 name mismatch via ASM");
            assert_eq!(
                original.code.len(),
                from_asm.code.len(),
                "910 instr count via ASM"
            );
            for (i, (orig, parsed)) in original.code.iter().zip(from_asm.code.iter()).enumerate() {
                assert_eq!(
                    orig.command, parsed.command,
                    "910 cmd mismatch [{i}] via ASM"
                );
                assert_operand_eq(&orig.operand, &parsed.operand, i);
            }

            // Binary roundtrip
            let encoded = encode_script(&original, &opcode_book, BUILD_910)?;
            let re_decoded = decode_script(&encoded, &opcode_book, BUILD_910)?;
            assert_eq!(
                original.name, re_decoded.name,
                "910 name mismatch via binary"
            );
            assert_eq!(
                original.code.len(),
                re_decoded.code.len(),
                "910 instr count via binary"
            );
            for (i, (orig, redec)) in original.code.iter().zip(re_decoded.code.iter()).enumerate() {
                assert_eq!(
                    orig.command, redec.command,
                    "910 cmd mismatch [{i}] via binary"
                );
                assert_operand_eq(&orig.operand, &redec.operand, i);
            }

            total_instructions += original.code.len();
            tested += 1;
        }
        if tested >= 100 {
            break;
        }
    }

    eprintln!(
        "Roundtrip OK (build 910): {tested} scripts, {total_instructions} total instructions"
    );
    Ok(())
}

#[test]
fn parses_varps_and_varbits() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    ensure_archive_complete(&cache_dir, &tar_path(), ARCHIVE_CONFIG)?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_CONFIG)?;

    let groups = [
        (CONFIG_GROUP_VAR_PLAYER, VarDomain::Player),
        (CONFIG_GROUP_VAR_NPC, VarDomain::Npc),
        (CONFIG_GROUP_VAR_CLIENT, VarDomain::Client),
        (CONFIG_GROUP_VAR_WORLD, VarDomain::World),
        (CONFIG_GROUP_VAR_REGION, VarDomain::Region),
        (CONFIG_GROUP_VAR_OBJECT, VarDomain::Object),
        (CONFIG_GROUP_VAR_CLAN, VarDomain::Clan),
        (CONFIG_GROUP_VAR_CLAN_SETTING, VarDomain::ClanSetting),
        (CONFIG_GROUP_VAR_CONTROLLER, VarDomain::Controller),
        (CONFIG_GROUP_VAR_GLOBAL, VarDomain::Global),
        (CONFIG_GROUP_VAR_PLAYER_GROUP, VarDomain::PlayerGroup),
    ];

    let mut var_count = 0usize;
    for (group, domain) in groups {
        let payload = cache
            .get(ARCHIVE_CONFIG, group)?
            .unwrap_or_else(|| panic!("missing config group {group}"));
        let files = rs3_cache_rs::js5::unpack_group(&index, group, &payload)?;
        for (id, bytes) in files {
            let parsed = parse_var(domain, id, &bytes)?;
            assert_eq!(parsed.id, id);
            var_count += 1;
        }
    }
    assert!(var_count > 10_000);

    let varbit_payload = cache
        .get(ARCHIVE_CONFIG, CONFIG_GROUP_VAR_BIT)?
        .unwrap_or_else(|| panic!("missing varbit group {CONFIG_GROUP_VAR_BIT}"));
    let varbits = rs3_cache_rs::js5::unpack_group(&index, CONFIG_GROUP_VAR_BIT, &varbit_payload)?;
    let mut parsed = 0usize;
    let mut with_domain = 0usize;
    for (id, bytes) in varbits {
        let varbit = parse_varbit(id, &bytes)?;
        parsed += 1;
        if varbit.domain.is_some() {
            with_domain += 1;
        }
    }
    assert!(parsed > 6_000);
    assert_eq!(parsed, with_domain);
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn parses_additional_config_groups() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    let tar = tar_path();
    ensure_archive_complete(&cache_dir, &tar, ARCHIVE_CONFIG)?;
    let has_struct_archive =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_STRUCT_CONFIG).is_ok();
    let has_loc_archive = ensure_archive_complete(&cache_dir, &tar, ARCHIVE_LOC_CONFIG).is_ok();
    let has_npc_archive = ensure_archive_complete(&cache_dir, &tar, ARCHIVE_NPC_CONFIG).is_ok();
    let has_obj_archive = ensure_archive_complete(&cache_dir, &tar, ARCHIVE_OBJ_CONFIG).is_ok();
    let has_seq_archive = ensure_archive_complete(&cache_dir, &tar, ARCHIVE_SEQ_CONFIG).is_ok();
    let has_spot_archive = ensure_archive_complete(&cache_dir, &tar, ARCHIVE_SPOT_CONFIG).is_ok();
    let has_achievements_archive =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_ACHIEVEMENTS).is_ok();
    let has_materials_archive =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_MATERIALS).is_ok();
    let cache = FlatCache::open(&cache_dir)?;
    let config_index = cache.archive_index(ARCHIVE_CONFIG)?;
    let idk_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_IDK, |id, data| {
            let parsed = parse_idk(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let loc_count = if has_loc_archive {
        let loc_index = cache.archive_index(ARCHIVE_LOC_CONFIG)?;
        parse_archive_with_bits(&cache, ARCHIVE_LOC_CONFIG, &loc_index, 8, |id, data| {
            let parsed = parse_loc(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    } else {
        parse_optional_config_group(
            &cache,
            &config_index,
            CONFIG_GROUP_LOC_LEGACY,
            |id, data| {
                let parsed = parse_loc(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    };
    let npc_count = if has_npc_archive {
        let npc_index = cache.archive_index(ARCHIVE_NPC_CONFIG)?;
        parse_archive_with_bits(&cache, ARCHIVE_NPC_CONFIG, &npc_index, 7, |id, data| {
            let parsed = parse_npc(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    } else {
        parse_optional_config_group(
            &cache,
            &config_index,
            CONFIG_GROUP_NPC_LEGACY,
            |id, data| {
                let parsed = parse_npc(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    };
    let obj_count = if has_obj_archive {
        let obj_index = cache.archive_index(ARCHIVE_OBJ_CONFIG)?;
        parse_archive_with_bits(&cache, ARCHIVE_OBJ_CONFIG, &obj_index, 8, |id, data| {
            let parsed = parse_obj(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    } else {
        parse_optional_config_group(
            &cache,
            &config_index,
            CONFIG_GROUP_OBJ_LEGACY,
            |id, data| {
                let parsed = parse_obj(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    };
    let seq_count = if has_seq_archive {
        let seq_index = cache.archive_index(ARCHIVE_SEQ_CONFIG)?;
        parse_archive_with_bits(&cache, ARCHIVE_SEQ_CONFIG, &seq_index, 7, |id, data| {
            let parsed = parse_seq(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    } else {
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_SEQ, |id, data| {
            let parsed = parse_seq(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    };
    let spot_count = if has_spot_archive {
        let spot_index = cache.archive_index(ARCHIVE_SPOT_CONFIG)?;
        parse_archive_with_bits(&cache, ARCHIVE_SPOT_CONFIG, &spot_index, 8, |id, data| {
            let parsed = parse_spot(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    } else {
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_SPOT, |id, data| {
            let parsed = parse_spot(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    };
    let bas_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_BAS, |id, data| {
            let parsed = parse_bas(id, data, BUILD)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let quest_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_QUEST, |id, data| {
            let parsed = parse_quest(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let mel_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_MEL, |id, data| {
            let parsed = parse_mel(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let water_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_WATER, |id, data| {
            let parsed = parse_water(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let achievement_count = if has_achievements_archive {
        let achievement_index = cache.archive_index(ARCHIVE_ACHIEVEMENTS)?;
        parse_archive_with_bits(
            &cache,
            ARCHIVE_ACHIEVEMENTS,
            &achievement_index,
            CONFIG_GROUP_ACHIEVEMENT_ARCHIVE57,
            |id, data| {
                let parsed = parse_achievement(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    } else {
        0
    };
    let material_count = if has_materials_archive {
        let material_index = cache.archive_index(ARCHIVE_MATERIALS)?;
        parse_archive_with_bits(&cache, ARCHIVE_MATERIALS, &material_index, 0, |id, data| {
            let parsed = parse_material(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?
    } else {
        0
    };
    let inv_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_INV, |id, data| {
            let parsed = parse_inv(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let cursor_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_CURSOR, |id, data| {
            let parsed = parse_cursor(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let seqgroup_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_SEQGROUP, |id, data| {
            let parsed = parse_seqgroup(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let category_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_CATEGORY, |id, data| {
            let parsed = parse_category(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let controller_count = parse_optional_config_group(
        &cache,
        &config_index,
        CONFIG_GROUP_CONTROLLER,
        |id, data| {
            let parsed = parse_controller(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        },
    )?;
    let struct_count = parse_struct_count(&cache, &config_index, has_struct_archive)?;
    let area_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_AREA, |id, data| {
            let parsed = parse_area(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let hunt_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_HUNT, |id, data| {
            let parsed = parse_hunt(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let mesanim_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_MESANIM, |id, data| {
            let parsed = parse_mesanim(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let itemcode_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_ITEMCODE, |id, data| {
            let parsed = parse_itemcode(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let gamelogevent_count = parse_optional_config_group(
        &cache,
        &config_index,
        CONFIG_GROUP_GAMELOGEVENT,
        |id, data| {
            let parsed = parse_gamelogevent(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        },
    )?;
    let bugtemplate_count = parse_optional_config_group(
        &cache,
        &config_index,
        CONFIG_GROUP_BUGTEMPLATE,
        |id, data| {
            let parsed = parse_bugtemplate(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        },
    )?;
    let var_client_string_count = parse_optional_config_group(
        &cache,
        &config_index,
        CONFIG_GROUP_VAR_CLIENT_STRING,
        |id, data| {
            let parsed = parse_var_client_string(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        },
    )?;
    let var_npc_bit_count = parse_optional_config_group(
        &cache,
        &config_index,
        CONFIG_GROUP_VAR_NPC_BIT,
        |id, data| {
            let parsed = parse_var_npc_bit(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        },
    )?;
    let var_shared_count = parse_optional_config_group(
        &cache,
        &config_index,
        CONFIG_GROUP_VAR_SHARED,
        |id, data| {
            let parsed = parse_var_shared(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        },
    )?;
    let var_shared_string_count = parse_optional_config_group(
        &cache,
        &config_index,
        CONFIG_GROUP_VAR_SHARED_STRING,
        |id, data| {
            let parsed = parse_var_shared_string(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        },
    )?;
    let underlay_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_UNDERLAY, |id, data| {
            let parsed = parse_underlay(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let overlay_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_OVERLAY, |id, data| {
            let parsed = parse_overlay(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let msi_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_MSI, |id, data| {
            let parsed = parse_msi(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let skybox_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_SKYBOX, |id, data| {
            let parsed = parse_skybox(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let worldarea_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_WORLDAREA, |id, data| {
            let parsed = parse_worldarea(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let quickchat_archive_available =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_QUICKCHAT_CONFIG).is_ok();
    let quickchatcat_count = if quickchat_archive_available {
        let quickchat_index = cache.archive_index(ARCHIVE_QUICKCHAT_CONFIG)?;
        parse_optional_archive_group(
            &cache,
            ARCHIVE_QUICKCHAT_CONFIG,
            &quickchat_index,
            CONFIG_GROUP_QUICKCHATCAT_ARCHIVE24,
            |id, data| {
                let parsed = parse_quickchatcat(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    } else {
        0
    };
    let quickchatphrase_count = if quickchat_archive_available {
        let quickchat_index = cache.archive_index(ARCHIVE_QUICKCHAT_CONFIG)?;
        parse_optional_archive_group(
            &cache,
            ARCHIVE_QUICKCHAT_CONFIG,
            &quickchat_index,
            CONFIG_GROUP_QUICKCHATPHRASE_ARCHIVE24,
            |id, data| {
                let parsed = parse_quickchatphrase(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    } else {
        0
    };
    let headbar_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_HEADBAR, |id, data| {
            let parsed = parse_headbar(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let hitmark_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_HITMARK, |id, data| {
            let parsed = parse_hitmark(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let light_count =
        parse_optional_config_group(&cache, &config_index, CONFIG_GROUP_LIGHT, |id, data| {
            let parsed = parse_light(id, data)?;
            assert_eq!(id, parsed.id);
            Ok(parsed)
        })?;
    let particles_archive_available =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_PARTICLES).is_ok();
    let particleeffector_count = if particles_archive_available {
        let particle_index = cache.archive_index(ARCHIVE_PARTICLES)?;
        parse_optional_archive_group(
            &cache,
            ARCHIVE_PARTICLES,
            &particle_index,
            CONFIG_GROUP_PARTICLE_EFFECTOR_ARCHIVE27,
            |id, data| {
                let parsed = parse_particle_effector(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    } else {
        0
    };
    let particleemitter_count = if particles_archive_available {
        let particle_index = cache.archive_index(ARCHIVE_PARTICLES)?;
        parse_optional_archive_group(
            &cache,
            ARCHIVE_PARTICLES,
            &particle_index,
            CONFIG_GROUP_PARTICLE_EMITTER_ARCHIVE27,
            |id, data| {
                let parsed = parse_particle_emitter(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    } else {
        0
    };
    let billboards_archive_available =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_BILLBOARDS).is_ok();
    let billboard_count = if billboards_archive_available {
        let billboard_index = cache.archive_index(ARCHIVE_BILLBOARDS)?;
        parse_optional_archive_group(
            &cache,
            ARCHIVE_BILLBOARDS,
            &billboard_index,
            CONFIG_GROUP_BILLBOARD_ARCHIVE29,
            |id, data| {
                let parsed = parse_billboard(id, data)?;
                assert_eq!(id, parsed.id);
                Ok(parsed)
            },
        )?
    } else {
        0
    };
    let textures_archive_available =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_TEXTURES).is_ok();
    let texture_count = if textures_archive_available {
        let texture_index = cache.archive_index(ARCHIVE_TEXTURES)?;
        let mut count = 0_usize;
        for group in &texture_index.group_id {
            let files = cache.group_files_with_index(&texture_index, ARCHIVE_TEXTURES, *group)?;
            for (file, data) in files {
                let parsed = parse_texture(group + file, &data)?;
                assert_eq!(group + file, parsed.id);
                count += 1;
            }
        }
        count
    } else {
        0
    };
    let stylesheets_archive_available =
        ensure_archive_complete(&cache_dir, &tar, ARCHIVE_STYLESHEETS).is_ok();
    let stylesheet_count = if stylesheets_archive_available {
        let stylesheet_index = cache.archive_index(ARCHIVE_STYLESHEETS)?;
        let mut count = 0_usize;
        for group in &stylesheet_index.group_id {
            let files =
                cache.group_files_with_index(&stylesheet_index, ARCHIVE_STYLESHEETS, *group)?;
            for (file, data) in files {
                let parsed = parse_stylesheet(group + file, &data)?;
                assert_eq!(group + file, parsed.id);
                count += 1;
            }
        }
        count
    } else {
        0
    };

    assert!(idk_count > 0);
    assert!(loc_count > 0);
    assert!(npc_count > 0);
    assert!(obj_count > 0);
    assert!(seq_count > 0);
    assert!(spot_count > 0);
    assert!(inv_count > 0);
    assert!(cursor_count > 0);
    assert!(seqgroup_count > 0);
    assert!(category_count > 0);
    assert!(controller_count > 0);
    assert!(struct_count > 0);
    assert!(
        area_count
            + bas_count
            + quest_count
            + mel_count
            + water_count
            + achievement_count
            + material_count
            + hunt_count
            + mesanim_count
            + itemcode_count
            + gamelogevent_count
            + bugtemplate_count
            + var_client_string_count
            + var_npc_bit_count
            + var_shared_count
            + var_shared_string_count
            + underlay_count
            + overlay_count
            + msi_count
            + skybox_count
            + worldarea_count
            + quickchatcat_count
            + quickchatphrase_count
            + headbar_count
            + hitmark_count
            + light_count
            + particleeffector_count
            + particleemitter_count
            + billboard_count
            + texture_count
            + stylesheet_count
            > 0
    );
    Ok(())
}

#[test]
fn parses_gold_model_sample_set() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    let sample_groups: Vec<u32> = (0..=100)
        .chain([1_000, 5_000, 10_000, 50_000, 100_000, 140_130])
        .collect();
    ensure_archive_groups(&cache_dir, &tar_path(), ARCHIVE_MODELS_RT7, &sample_groups)?;
    let cache = FlatCache::open(&cache_dir)?;
    let index = cache.archive_index(ARCHIVE_MODELS_RT7)?;

    let mut decoded_models = Vec::new();
    for group in &sample_groups {
        let payload = cache
            .get(ARCHIVE_MODELS_RT7, *group)?
            .unwrap_or_else(|| panic!("missing model group {group}"));
        let files = rs3_cache_rs::js5::unpack_group(&index, *group, &payload)?;
        let model = Model::decode(
            files
                .get(&0)
                .unwrap_or_else(|| panic!("model file 0 missing for group {group}")),
            BUILD,
        )?;
        assert_eq!(2, model.format);
        assert_eq!(5, model.version);
        decoded_models.push(model);
    }

    assert_eq!(107, decoded_models.len());
    assert!(decoded_models.iter().any(|m| m.unk_count1 > 0));
    assert!(decoded_models.iter().any(|m| m.unk_count2 > 0));
    assert!(decoded_models.iter().any(|m| {
        m.meshdata
            .as_ref()
            .is_some_and(|mesh| usize::try_from(mesh.vertex_count).is_ok_and(|c| c > 8_000))
    }));

    let first_model = &decoded_models[0];
    assert_eq!(15, first_model.always_0f);
    assert_eq!(3, first_model.mesh_count);
    let meshdata = first_model
        .meshdata
        .as_ref()
        .expect("first_model missing meshdata");
    assert_eq!(222, meshdata.vertex_count);
    assert_eq!(3, meshdata.renders.len());
    let pos = meshdata
        .position_buffer
        .as_ref()
        .expect("first_model missing position buffer");
    assert_eq!(256, i32::from(pos[0][0]));
    assert_eq!(11, i32::from(pos[0][1]));
    assert_eq!(-256, i32::from(pos[0][2]));
    Ok(())
}

#[test]
fn parses_animator_and_cutscene2d_samples() -> Result<()> {
    let _guard = lock_guard();
    let cache_root = cache_dir();
    let tar = tar_path();
    ensure_archive_complete(&cache_root, &tar, ARCHIVE_ANIMATOR)?;
    ensure_archive_complete(&cache_root, &tar, ARCHIVE_CUTSCENE2D)?;
    let cache = FlatCache::open(&cache_root)?;

    let animator_index = cache.archive_index(ARCHIVE_ANIMATOR)?;
    let mut animator_decoded = 0_usize;
    for group in animator_index.group_id.iter().take(5) {
        let files = cache.group_files_with_index(&animator_index, ARCHIVE_ANIMATOR, *group)?;
        for (_, data) in files {
            let _decoded = decode_animator_controller(&data)?;
            animator_decoded += 1;
        }
    }
    assert!(animator_decoded > 0);

    let cutscene_index = cache.archive_index(ARCHIVE_CUTSCENE2D)?;
    let mut cutscene_decoded = 0_usize;
    for group in cutscene_index.group_id.iter().take(5) {
        let files = cache.group_files_with_index(&cutscene_index, ARCHIVE_CUTSCENE2D, *group)?;
        for (_, data) in files {
            let _decoded = decode_cutscene2d(&data)?;
            cutscene_decoded += 1;
        }
    }
    assert!(cutscene_decoded > 0);

    Ok(())
}

#[test]
fn parses_vfx_samples() -> Result<()> {
    let _guard = lock_guard();
    let cache_root = cache_dir();
    let tar = tar_path();
    ensure_archive_complete(&cache_root, &tar, ARCHIVE_VFX)?;
    let cache = FlatCache::open(&cache_root)?;

    let index = cache.archive_index(ARCHIVE_VFX)?;
    let mut parsed = 0_usize;
    for group in index.group_id.iter().take(20) {
        let files = cache.group_files_with_index(&index, ARCHIVE_VFX, *group)?;
        for (_, data) in files {
            let _decoded = decode_vfx(&data)?;
            parsed += 1;
        }
    }
    assert!(parsed > 0);

    Ok(())
}

#[test]
fn parses_mapsquare_samples() -> Result<()> {
    let _guard = lock_guard();
    let cache_root = cache_dir();
    let tar = tar_path();
    ensure_archive_complete(&cache_root, &tar, ARCHIVE_MAPSQUARES)?;
    let cache = FlatCache::open(&cache_root)?;

    let index = cache.archive_index(ARCHIVE_MAPSQUARES)?;
    let mut parsed = 0_usize;
    for group in index.group_id.iter().take(20) {
        let files = cache.group_files_with_index(&index, ARCHIVE_MAPSQUARES, *group)?;
        let _decoded = decode_map_square(&files, BUILD)?;
        parsed += 1;
    }
    assert!(parsed > 0);

    Ok(())
}

#[test]
fn cli_smoke_unpack_and_audio_write_expected_artifacts() -> Result<()> {
    let _guard = lock_guard();
    let cache_root = cache_dir();
    let tar = tar_path();
    ensure_archive_complete(&cache_root, &tar, ARCHIVE_INTERFACES)?;

    let temp = tempfile::tempdir()?;
    let unpack_out = temp.path().join("unpack");
    run_cli(Cli {
        cache_dir: Some(cache_root.clone()),
        cache_tar: Some(tar.clone()),
        data_dir: data_dir(),
        build: BUILD,
        subbuild: SUBBUILD,
        command: Command::Unpack {
            out_dir: unpack_out.clone(),
            sample_models: true,
            skip_audio: true,
            best_effort_maps: false,
            max_audio_files: None,
        },
    })?;

    for rel in [
        "interface",
        "config/varps.json",
        "config/varbits.json",
        "script/scripts.json",
        "script/decompiled",
        "model/models_sample.json",
        "model/decoded",
        "binary",
        "ttf",
        "fontmetrics",
        "maps",
        "vfx",
        "animator",
        "cutscene2d",
        "uianim",
        "uianimcurve",
        "worldmap/dump.wma",
        "areas.png",
    ] {
        assert!(unpack_out.join(rel).exists(), "missing {rel}");
    }
    assert!(!unpack_out.join("audio").exists());

    let audio_out = temp.path().join("audio");
    run_cli(Cli {
        cache_dir: Some(cache_root),
        cache_tar: Some(tar),
        data_dir: data_dir(),
        build: BUILD,
        subbuild: SUBBUILD,
        command: Command::Audio {
            out_dir: Some(audio_out.clone()),
            max_files: Some(256),
        },
    })?;

    let manifest_path = audio_out.join("audio_manifest.json");
    assert!(manifest_path.is_file());
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    let entries = manifest.as_array().expect("audio manifest array");
    assert!(!entries.is_empty());
    assert!(audio_out.read_dir()?.count() > 1);

    Ok(())
}

#[test]
fn grabs_audio_archives() -> Result<()> {
    let _guard = lock_guard();
    let cache_dir = cache_dir();
    let tar = tar_path();
    let mut available = Vec::new();
    for archive in [
        ARCHIVE_SYNTH,
        ARCHIVE_SONGS,
        ARCHIVE_JINGLES,
        ARCHIVE_VORBIS,
        ARCHIVE_AUDIOSTREAMS,
    ] {
        if ensure_archive_groups(&cache_dir, &tar, archive, &[]).is_ok() {
            available.push(archive);
        }
    }

    assert!(!available.is_empty());
    let cache = FlatCache::open(&cache_dir)?;
    let mut scanned = 0_usize;
    let mut jaga = 0_usize;
    let mut direct_ogg = 0_usize;
    let mut with_embedded_ogg = 0_usize;
    let mut unknown = 0_usize;

    for archive in available {
        let index = cache.archive_index(archive)?;
        assert!(!index.group_id.is_empty());
        assert!(index.group_count > 0);

        'groups: for group in &index.group_id {
            let files = cache.group_files_with_index(&index, archive, *group)?;
            for data in files.values() {
                let inspection = inspect_audio_file(data);
                match inspection.kind {
                    AudioKind::Jaga => {
                        jaga += 1;
                        if inspection.embedded_ogg_offset.is_some() {
                            with_embedded_ogg += 1;
                        }
                    }
                    AudioKind::Ogg => direct_ogg += 1,
                    AudioKind::Unknown => unknown += 1,
                    AudioKind::Midi | AudioKind::Wav | AudioKind::Flac => {}
                }
                scanned += 1;
                if scanned >= 2_000 {
                    break 'groups;
                }
            }
        }
    }
    assert!(scanned > 0);
    assert!(jaga + direct_ogg > 0);
    if jaga > 0 {
        assert_eq!(jaga, with_embedded_ogg);
    }
    assert!(unknown < scanned);
    Ok(())
}
