//! `cs2` family: the legacy clientscript dump (`cs2` with no subcommand), the
//! semantic 948→910 port (`cs2 port`), and the `port plan` representability
//! dry-run dispatch.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use rayon::prelude::*;
use serde::Serialize;

use crate::cache::FlatCache;
use crate::cli::PortCommand;
use crate::cli::context::CommandContext;
use crate::cli::shared::{
    format_script_source, load_script_group_names, print_json, sanitize_file_component, write_json,
    write_text,
};
use crate::constants::ARCHIVE_CLIENTSCRIPTS;
use crate::fixture::ensure_archive_complete;
use crate::script::{CompiledScript, MIN_SCRIPT_BUILD, OpcodeBook, decode_script};

#[derive(Debug, Serialize)]
struct Cs2Summary {
    scripts: usize,
    instructions: usize,
    unique_opcodes: usize,
}

/// Options for the legacy `cs2` dump.
#[derive(Clone, Debug, Default)]
pub struct Cs2DumpOpts {
    pub out_file: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
}

/// Options for `cs2 port`.
#[derive(Clone, Debug)]
pub struct Cs2PortOpts {
    pub from: u32,
    pub to: u32,
    pub closure_of_interface: u32,
    pub base_cache_dir: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub check_oracle: bool,
    pub json: bool,
}

/// `cs2` (no subcommand) — dump every clientscript group.
pub fn run_dump(ctx: &CommandContext, opts: Cs2DumpOpts) -> Result<()> {
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let data_dir = ctx.data_dir();
    let Cs2DumpOpts { out_file, out_dir } = opts;
    let out_file = out_file.as_deref();
    let out_dir = out_dir.as_deref();

    ensure_archive_complete(cache.root(), tar_path, ARCHIVE_CLIENTSCRIPTS)?;
    let cache = FlatCache::open(cache.root())?;
    let opcode_book = OpcodeBook::load(data_dir, ctx.build(), ctx.subbuild())?;
    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let keep_decoded = out_file.is_some();
    let script_group_names = load_script_group_names(&index, data_dir)?;
    let build = ctx.build();

    if let Some(path) = out_dir {
        fs::create_dir_all(path).with_context(|| format!("failed creating {}", path.display()))?;
    }

    let mut scripts = 0_usize;
    let mut instructions = 0_usize;
    let mut opcode_names = HashMap::<String, usize>::new();
    let mut decoded_all = Vec::new();

    struct GroupCs2Result {
        scripts: usize,
        instructions: usize,
        opcode_counts: HashMap<String, usize>,
        decoded: Vec<CompiledScript>,
    }

    let group_results = index
        .group_id
        .par_iter()
        .map(|group| -> Result<GroupCs2Result> {
            let files = cache.group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, *group)?;
            let single_file_group = files.len() == 1;
            let mut scripts = 0_usize;
            let mut instructions = 0_usize;
            let mut opcode_counts = HashMap::<String, usize>::new();
            let mut decoded = Vec::new();

            for (file, bytes) in files {
                let script = match decode_script(&bytes, &opcode_book, build) {
                    Ok(s) => s,
                    Err(e) => {
                        if build < MIN_SCRIPT_BUILD {
                            eprintln!("warning: skipping script {file} in group {group}: {e}");
                            continue;
                        }
                        return Err(e.into());
                    }
                };
                scripts += 1;
                instructions += script.code.len();
                for instruction in &script.code {
                    *opcode_counts
                        .entry(instruction.command.clone())
                        .or_insert(0) += 1;
                }

                if let Some(dir) = out_dir {
                    let hint = script_group_names
                        .get(group)
                        .map(String::as_str)
                        .or(script.name.as_deref())
                        .unwrap_or("script");
                    let source_name = sanitize_file_component(hint);
                    let file_name = if single_file_group {
                        format!("{group}_{source_name}.cs2")
                    } else {
                        format!("{group}_{file}_{source_name}.cs2")
                    };
                    let path = dir.join(file_name);
                    write_text(&path, &format_script_source(*group, file, &script))?;
                }

                if keep_decoded {
                    decoded.push(script);
                }
            }

            Ok(GroupCs2Result {
                scripts,
                instructions,
                opcode_counts,
                decoded,
            })
        })
        .collect::<Vec<_>>();

    for result in group_results {
        let result = result?;
        scripts += result.scripts;
        instructions += result.instructions;
        for (opcode, count) in result.opcode_counts {
            *opcode_names.entry(opcode).or_insert(0) += count;
        }
        if keep_decoded {
            decoded_all.extend(result.decoded);
        }
    }

    if let Some(path) = out_file {
        write_json(path, &decoded_all)?;
    }
    print_json(&Cs2Summary {
        scripts,
        instructions,
        unique_opcodes: opcode_names.len(),
    })
}

/// `cs2 port` — the semantic 948→910 CS2 port (plan §9/§10). Decodes the donor
/// closure from `cache` (the 948 flat cache), runs it through the port layer, and
/// (optionally) writes the `.asm.ts` listings + checks them byte-for-byte against
/// the committed oracle. Today only `--closure-of-interface 1224` is wired (the
/// ritual driver, the byte-exact oracle).
pub fn run_port(ctx: &CommandContext, opts: Cs2PortOpts) -> Result<()> {
    use crate::port::book::BuildDescriptor;
    use crate::port::{lodestone, material_storage, relic, ritual};

    let cache = ctx.cache();
    let data_dir = ctx.data_dir();
    let Cs2PortOpts {
        from,
        to,
        closure_of_interface,
        base_cache_dir,
        out_dir,
        check_oracle,
        json,
    } = opts;
    let base_cache_dir = base_cache_dir.as_deref();
    let out_dir = out_dir.as_deref();

    ensure!(
        from == 948 && to == 910,
        "cs2 port currently supports only --from 948 --to 910 (got {from} → {to})"
    );
    ensure!(
        matches!(closure_of_interface, 1224 | 691 | 660 | 1092),
        "cs2 port currently supports --closure-of-interface 1224 (ritual selection), 691 \
         (relic powers), 660 (material storage), or 1092 (lodestone); got {closure_of_interface}"
    );

    let index = cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let book_948 = OpcodeBook::load(data_dir, 948, 1)?;
    let d948 = BuildDescriptor::load(data_dir, 948)?;
    let d910 = BuildDescriptor::load(data_dir, 910)?;
    let source = ritual::cache_source(cache, &index, &book_948);
    let ported = match closure_of_interface {
        1224 => ritual::port_ritual_scripts(&source, &d948, &d910)?,
        691 => relic::port_relic_scripts(&source, &d948, &d910)?,
        660 => {
            // The 9239 base augmentation needs a 910-base cache.
            let base_dir = base_cache_dir
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("../../cache/unpacked/910"));
            let base_cache = FlatCache::open(&base_dir)?;
            let base_index = base_cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
            let book_910 = OpcodeBook::load(data_dir, 910, 0)?;
            let base_source = ritual::flat_cache_source(&base_cache, &base_index, &book_910, 910);
            material_storage::port_material_storage_scripts(&source, &base_source, &d948, &d910)?
        }
        1092 => {
            // Lodestone patches augment 910-base scripts only.
            let base_dir = base_cache_dir
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("../../cache/unpacked/910"));
            let base_cache = FlatCache::open(&base_dir)?;
            let base_index = base_cache.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
            let book_910 = OpcodeBook::load(data_dir, 910, 0)?;
            let base_source = ritual::flat_cache_source(&base_cache, &base_index, &book_910, 910);
            lodestone::port_lodestone_scripts(&base_source, &d910)?
        }
        other => bail!("unsupported interface {other}"),
    };

    // Optionally write the listings.
    if let Some(dir) = out_dir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create out dir {}", dir.display()))?;
        for p in &ported {
            let path = dir.join(format!("script{}.asm.ts", p.out_id));
            std::fs::write(&path, &p.text).with_context(|| format!("write {}", path.display()))?;
        }
    }

    // The byte-exact oracle: diff each produced listing against the committed one.
    let mut mismatches: Vec<i32> = Vec::new();
    let mut checked = 0_usize;
    if check_oracle {
        let oracle_family = match closure_of_interface {
            1224 => "ritual-pedestal-948",
            691 => "relic-system-948",
            660 => "material-storage-948",
            1092 => "lodestone-948",
            other => bail!("no committed oracle for interface {other}"),
        };
        let oracle_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../server/cache-patches")
            .join(oracle_family)
            .join("scripts");
        for p in &ported {
            let committed_path = oracle_dir.join(format!("script{}.asm.ts", p.out_id));
            match std::fs::read_to_string(&committed_path) {
                Ok(committed) => {
                    checked += 1;
                    if committed != p.text {
                        mismatches.push(p.out_id);
                    }
                }
                Err(_) => mismatches.push(p.out_id),
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "event": "cs2_port",
                "from": from,
                "to": to,
                "interface": closure_of_interface,
                "listings": ported.len(),
                "out_dir": out_dir.map(|p| p.display().to_string()),
                "oracle_checked": checked,
                "oracle_mismatches": mismatches,
                "byte_exact": mismatches.is_empty(),
            }))?
        );
    } else {
        println!(
            "cs2 port — interface {closure_of_interface} ({from}→{to}): {} listing(s)",
            ported.len()
        );
        if let Some(dir) = out_dir {
            println!("  wrote to {}", dir.display());
        }
        if check_oracle {
            if mismatches.is_empty() {
                println!(
                    "  oracle: BYTE-EXACT ({checked} listing(s) match the committed artifacts)"
                );
            } else {
                println!(
                    "  oracle: {} of {} listing(s) DIFFER: {:?}",
                    mismatches.len(),
                    checked,
                    mismatches
                );
            }
        }
    }

    if check_oracle && !mismatches.is_empty() {
        bail!(
            "cs2 port is not byte-exact against the committed oracle ({} mismatch(es))",
            mismatches.len()
        );
    }
    Ok(())
}

/// `port <sub>` dispatch (currently `plan`).
pub fn run_port_command(ctx: &CommandContext, command: &PortCommand) -> Result<()> {
    let cache = ctx.cache();
    let data_dir = ctx.data_dir();
    match *command {
        PortCommand::Plan {
            interface,
            from,
            to,
            json,
        } => {
            let donor_pack_root = PathBuf::from(crate::pack_root::DONOR_PACK_ROOT);
            let base_pack_root = crate::explain::default_base_pack_root();
            Ok(crate::port::plan::run(&crate::port::plan::PlanOptions {
                interface,
                from,
                to,
                donor_cache: cache,
                donor_pack_root: &donor_pack_root,
                base_pack_root: &base_pack_root,
                data_dir,
                json,
            })?)
        }
    }
}
