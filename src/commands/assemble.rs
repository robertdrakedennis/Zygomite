//! `assemble-script` / `assemble-script-batch` — assemble reversible or
//! pragma-annotated CS2 TypeScript back into CS2 binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::cache::FlatCache;
use crate::cli::context::{CommandContext, RuntimeVersion};
use crate::cli::shared::build_reverse_compile_context;
use crate::dep_tree::ResolverContext;
use crate::script::{CompiledScript, OpcodeBook, decode_script, encode_script, parse_cs2_asm};
use crate::transpile::{
    REVERSIBLE_FORMAT_VERSION, is_reversible_source, lower_structured_script,
    parse_reversible_source, parse_structured_typescript, structured_digest,
};

/// Options for `assemble-script`.
#[derive(Clone, Debug)]
pub struct AssembleScriptOpts {
    pub input: PathBuf,
    pub output: PathBuf,
    pub build: Option<u32>,
    pub subbuild: Option<u32>,
    pub strict_structured: bool,
    pub no_verify: bool,
    pub emit_json: bool,
}

/// Options for `assemble-script-batch`.
#[derive(Clone, Debug)]
pub struct AssembleBatchOpts {
    pub manifest: PathBuf,
    pub out_dir: PathBuf,
}

#[derive(Deserialize)]
struct BatchAssembleManifest {
    scripts: Vec<BatchAssembleScript>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchAssembleScript {
    script_id: u32,
    input: PathBuf,
    build: Option<u32>,
    subbuild: Option<u32>,
}

/// `assemble-script`.
pub fn run_script(ctx: &CommandContext, opts: AssembleScriptOpts) -> Result<()> {
    let AssembleScriptOpts {
        input,
        output,
        build,
        subbuild,
        strict_structured,
        no_verify,
        emit_json,
    } = opts;
    let cache = ctx.cache();
    let tar_path = ctx.tar_path();
    let data_dir = ctx.data_dir();
    let input = input.as_path();
    let output = output.as_path();

    let started = Instant::now();
    let source =
        std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;
    let reversible = if is_reversible_source(&source) {
        Some(
            parse_reversible_source(&source)
                .with_context(|| format!("parsing reversible TS from {}", input.display()))?,
        )
    } else {
        None
    };

    if let Some(parsed) = &reversible {
        if parsed.metadata.format_version != REVERSIBLE_FORMAT_VERSION {
            bail!(
                "unsupported reversible TS format version {} in {}",
                parsed.metadata.format_version,
                input.display()
            );
        }
        if let Some(requested_build) = build
            && requested_build != parsed.metadata.build
        {
            bail!(
                "assemble build {} mismatches source metadata build {}",
                requested_build,
                parsed.metadata.build
            );
        }
        if let Some(requested_subbuild) = subbuild
            && requested_subbuild != parsed.metadata.subbuild
        {
            bail!(
                "assemble subbuild {} mismatches source metadata subbuild {}",
                requested_subbuild,
                parsed.metadata.subbuild
            );
        }
    }

    let default_build = ctx.build();
    let default_subbuild = ctx.subbuild();
    let effective_build = reversible.as_ref().map_or_else(
        || build.unwrap_or(default_build),
        |parsed| parsed.metadata.build,
    );
    let effective_subbuild = reversible.as_ref().map_or_else(
        || subbuild.unwrap_or(default_subbuild),
        |parsed| parsed.metadata.subbuild,
    );
    let resolver = ResolverContext::load_transpile(
        cache,
        tar_path,
        data_dir,
        effective_build,
        effective_subbuild,
    )?;
    let opcode_book = &resolver.opcode_book;

    let (script, assemble_mode) = if let Some(parsed) = &reversible {
        let structured = parse_structured_typescript(&parsed.structured_source)
            .with_context(|| format!("parsing structured TS from {}", input.display()))?;
        let current_digest = structured_digest(&structured);
        if !strict_structured && current_digest == parsed.metadata.structured_digest {
            (
                parse_cs2_asm(&parsed.asm_trailer).with_context(|| {
                    format!("parsing embedded ASM trailer from {}", input.display())
                })?,
                "embedded-asm",
            )
        } else {
            if !parsed.metadata.editable_structured {
                let blockers = if parsed.metadata.blocking_diagnostics.is_empty() {
                    "unknown blocker".to_string()
                } else {
                    parsed.metadata.blocking_diagnostics.join(", ")
                };
                bail!(
                    "structured edits blocked for {}: {}. edit embedded ASM trailer or remove blocker",
                    parsed.metadata.export_name,
                    blockers
                );
            }
            let reverse_ctx = build_reverse_compile_context(&resolver, cache, data_dir)?;
            let compiled = lower_structured_script(&structured, &parsed.metadata, &reverse_ctx)
                .with_context(|| format!("lowering structured TS from {}", input.display()))?;
            (compiled, "structured")
        }
    } else {
        (
            parse_cs2_asm(&source)
                .with_context(|| format!("parsing ASM pragmas from {}", input.display()))?,
            "pragma-asm",
        )
    };
    let binary =
        encode_script(&script, opcode_book, effective_build).context("encoding CS2 binary")?;

    if !no_verify {
        verify_assembled_script(cache, data_dir, &resolver, &script, &binary, output)?;
    }

    std::fs::write(output, &binary).with_context(|| format!("writing {}", output.display()))?;

    if emit_json {
        let event = serde_json::json!({
            "event": "assemble_script",
            "outcome": "ok",
            "build": effective_build,
            "subbuild": effective_subbuild,
            "mode": assemble_mode,
            "instruction_count": script.code.len(),
            "bytes": binary.len(),
            "verified": !no_verify,
            "output": output.display().to_string(),
            "duration_ms": started.elapsed().as_millis() as u64,
        });
        println!("{}", serde_json::to_string(&event)?);
    } else {
        eprintln!(
            "Assembled {} instructions → {} ({} bytes, build {}, mode {})",
            script.code.len(),
            output.display(),
            binary.len(),
            effective_build,
            assemble_mode,
        );
    }
    Ok(())
}

/// `assemble-script-batch`. Operates purely on the manifest + pragma ASM inputs;
/// needs no flat cache, so it takes the data dir + default version directly.
pub fn run_batch(data_dir: &Path, version: RuntimeVersion, opts: AssembleBatchOpts) -> Result<()> {
    let AssembleBatchOpts { manifest, out_dir } = opts;
    let manifest = manifest.as_path();
    let out_dir = out_dir.as_path();

    let started = Instant::now();
    let batch: BatchAssembleManifest = serde_json::from_slice(
        &fs::read(manifest).with_context(|| format!("reading {}", manifest.display()))?,
    )?;
    fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let build = batch
        .scripts
        .first()
        .and_then(|script| script.build)
        .unwrap_or(version.build);
    let subbuild = batch
        .scripts
        .first()
        .and_then(|script| script.subbuild)
        .unwrap_or(version.subbuild);
    let opcode_book = OpcodeBook::load(data_dir, build, subbuild)?;
    for script in &batch.scripts {
        let effective_build = script.build.unwrap_or(build);
        let effective_subbuild = script.subbuild.unwrap_or(subbuild);
        ensure!(
            effective_build == build && effective_subbuild == subbuild,
            "assemble-script-batch requires one build/subbuild per invocation"
        );
        let source = fs::read_to_string(&script.input)
            .with_context(|| format!("reading {}", script.input.display()))?;
        ensure!(
            !is_reversible_source(&source),
            "assemble-script-batch only supports pragma ASM inputs: {}",
            script.input.display()
        );
        let compiled = parse_cs2_asm(&source)
            .with_context(|| format!("parsing ASM pragmas from {}", script.input.display()))?;
        let binary = encode_script(&compiled, &opcode_book, build)
            .with_context(|| format!("encoding script {}", script.script_id))?;
        fs::write(
            out_dir.join(format!("script-{}.cs2", script.script_id)),
            binary,
        )
        .with_context(|| format!("writing script {}", script.script_id))?;
    }
    eprintln!(
        "assemble-script-batch: assembled {} script(s) in {}ms",
        batch.scripts.len(),
        started.elapsed().as_millis()
    );
    Ok(())
}

/// Verify a freshly assembled script before writing it: (1) the emitted bytes
/// must decode back to an identical script (encoder/operand fidelity), and (2)
/// structural + stack-effect validation against the target build's catalog must
/// pass (no stack underflow, dangling branch targets, or unknown references).
/// This stops `assemble-script` from silently producing CS2 the client cannot
/// run. Bypass with `--no-verify`.
fn verify_assembled_script(
    cache: &FlatCache,
    data_dir: &Path,
    ctx: &ResolverContext,
    script: &CompiledScript,
    binary: &[u8],
    output: &Path,
) -> Result<()> {
    let decoded = decode_script(binary, &ctx.opcode_book, ctx.build)
        .with_context(|| format!("verifying {}: re-decoding emitted CS2", output.display()))?;
    // Compare command + operand + header, ignoring the numeric `opcode` field:
    // lowering/assembly leaves it as a `0` placeholder while decode fills in the
    // real id, but the byte fidelity we care about lives in command names and
    // operands. A self-consistent encoder bug (e.g. a zeroed placeholder operand)
    // still surfaces here, which a bytes-only `encode(decode(b)) == b` check misses.
    let normalize = |source: &CompiledScript| -> Result<serde_json::Value> {
        let mut clone = source.clone();
        for instruction in &mut clone.code {
            instruction.opcode = 0;
        }
        Ok(serde_json::to_value(&clone)?)
    };
    if normalize(&decoded)? != normalize(script)? {
        bail!(
            "post-compile verification failed for {}: re-decoded CS2 does not match the compiled \
             script (encoder fidelity bug); pass --no-verify to override",
            output.display()
        );
    }

    let reverse_ctx = build_reverse_compile_context(ctx, cache, data_dir)?;
    let script_id = script
        .name
        .as_deref()
        .and_then(|name| reverse_ctx.script_catalog.resolve_export_name(name))
        .map_or(0, |meta| meta.packed_id.0.unsigned_abs());
    let validator = crate::validate::Cs2Validator::new(ctx);
    let report = validator.validate_compiled(
        script_id,
        script,
        &reverse_ctx.script_catalog,
        &reverse_ctx.script_signatures,
        script.name.clone(),
    );
    if !report.errors.is_empty() {
        let detail = report
            .errors
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "post-compile validation failed for {} ({} error(s)): {detail}; pass --no-verify to \
             override",
            output.display(),
            report.errors.len(),
        );
    }
    Ok(())
}
