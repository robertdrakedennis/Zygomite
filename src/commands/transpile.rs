//! `transpile-scripts` — decode every (or a filtered set of) clientscript and
//! emit editable TypeScript, with the recompile byte gate that proves each
//! generated module round-trips back to identical CS2 bytes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::cache::FlatCache;
use crate::cli::TranspileOutputStyle;
use crate::cli::context::{CommandContext, RuntimeVersion};
use crate::cli::shared::{
    build_reverse_compile_context_from_catalog, load_script_group_names_from_cache,
    sanitize_file_component, source_control_flow_fallback_reason, write_text,
};
use crate::commands::ts_export::{
    export_db_types, export_enum_types, export_index, export_interface_ids, export_inv_types,
    export_loc_types, export_named_config_ids, export_npc_types, export_obj_types,
    export_param_types, export_script_signatures, export_seq_types, export_spot_types,
    export_var_types, export_varbit_types,
};
use crate::constants::ARCHIVE_CLIENTSCRIPTS;
use crate::dep_tree::ResolverContext;
use crate::script::{CompiledScript, OpcodeBook, Operand, decode_script, encode_script, parse_cs2_asm};
use crate::transpile::{
    ReverseCompileContext, ScriptCatalog, Transpiler, is_reversible_source, lower_structured_script,
    parse_reversible_source, parse_structured_typescript, render_reversible_source,
};

/// Options for `transpile-scripts`.
#[derive(Clone, Debug)]
pub struct TranspileScriptsOpts {
    pub out_dir: PathBuf,
    pub filter_script: Option<String>,
    pub output_style: TranspileOutputStyle,
    pub max_scripts: usize,
    pub all_scripts: bool,
    pub limits: crate::transpile::TranspileLimits,
}

/// `transpile-scripts`.
pub fn run(ctx: &CommandContext, opts: TranspileScriptsOpts) -> Result<()> {
    let TranspileScriptsOpts {
        out_dir,
        filter_script,
        output_style,
        max_scripts,
        all_scripts,
        limits,
    } = opts;
    run_transpile_scripts(
        ctx.cache(),
        ctx.tar_path(),
        ctx.data_dir(),
        &out_dir,
        filter_script.as_deref(),
        output_style,
        max_scripts,
        all_scripts,
        limits,
        ctx.version,
    )
}

fn push_unique_diagnostic(diagnostics: &mut Vec<String>, diagnostic: String) {
    if !diagnostics.iter().any(|existing| existing == &diagnostic) {
        diagnostics.push(diagnostic);
    }
}

fn finalize_reversible_transpile_output(
    source: String,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
    diagnostics: &mut crate::transpile::Diagnostics,
    editable_structured: &mut bool,
    blocking_diagnostics: &mut Vec<String>,
) -> Result<String> {
    if !is_reversible_source(&source) {
        return Ok(source);
    }

    let mut parsed = parse_reversible_source(&source)?;
    // G3: annotate local declarations with inferred semantic types (opt-in). Applied
    // before the fidelity gate so the gate validates the annotated form — annotations
    // are byte-irrelevant (the reverse compiler recovers a local's slot/domain from its
    // name), so this never changes the recompile result.
    if std::env::var_os("RS3_INFER_LOCAL_TYPES").is_some() {
        annotate_parsed_local_types(&mut parsed, reverse_ctx);
    }
    let mut metadata = parsed.metadata.clone();
    if metadata.editable_structured
        && let Err(block) = recompile_fidelity_check(&parsed, &metadata, reverse_ctx, opcode_book)
    {
        metadata.editable_structured = false;
        push_unique_diagnostic(
            &mut metadata.blocking_diagnostics,
            block.blocker.to_string(),
        );
        // A low-cardinality sub-bucket so the coverage histogram ranks *why* a
        // blocker fired (recompile_mismatch_cause:push_constant_string->... or
        // reverse_unsupported_cause:ui_method) instead of collapsing them into
        // one opaque tag.
        if let Some(cause) = block.cause {
            push_unique_diagnostic(
                &mut metadata.blocking_diagnostics,
                format!("{}_cause:{cause}", block.blocker),
            );
        }
        diagnostics.warning(block.message);
    }

    // G1.4: the RuneScript-surface byte gate (opt-in, informational). Runs whenever the structured
    // TS surface is itself byte-exact (`editable_structured`); it gates whichever build is being
    // transpiled (driven by `--build`). It records a `runescript_gate` diagnostic on failure but
    // NEVER flips `editable_structured` — the TS editing surface is unaffected. This is the
    // authoritative proof that the RuneScript round-trip (`render_runescript` → `parse_runescript`
    // → encode) reproduces the original bytes.
    if std::env::var_os("RS3_RUNESCRIPT_GATE").is_some()
        && metadata.editable_structured
        && let Err(block) =
            recompile_fidelity_check_runescript(&parsed, &metadata, reverse_ctx, opcode_book)
    {
        push_unique_diagnostic(&mut metadata.blocking_diagnostics, "runescript_gate".to_string());
        if let Some(cause) = block.cause {
            push_unique_diagnostic(
                &mut metadata.blocking_diagnostics,
                format!("runescript_gate_cause:{cause}"),
            );
        }
        // Diagnostic visibility (opt-in): record the sanitized failure message so the opaque
        // `other`/`ui_method`/`array` buckets can be sub-classified by the actual `bail!` reason.
        push_unique_diagnostic(
            &mut metadata.blocking_diagnostics,
            format!("runescript_gate_msg:{}", gate_message_head(&block.message)),
        );
    }

    *editable_structured = metadata.editable_structured;
    blocking_diagnostics.clone_from(&metadata.blocking_diagnostics);
    Ok(render_reversible_source(
        &parsed.structured_source,
        &metadata,
        &parsed.asm_trailer,
    )?)
}

/// Rewrite local-declaration annotations in a parsed reversible source with G3's
/// inferred semantic types. Gosub callee arities come from the reverse context's
/// cross-script signatures, so gosub-calling scripts model too. Byte-irrelevant
/// (the semantic annotation maps to the same base as today's), so the fidelity gate
/// is unaffected — proven by the corpus run staying `blocked:0`.
fn annotate_parsed_local_types(
    parsed: &mut crate::transpile::ParsedReversibleSource,
    reverse_ctx: &ReverseCompileContext,
) {
    use crate::transpile::ScriptId;
    use crate::transpile::type_constraints::{
        CalleeSig, SignatureTable, annotate_local_declarations, infer_program,
    };
    let Ok(script) = crate::script::parse_cs2_asm(&parsed.asm_trailer) else {
        return;
    };
    let sigs = SignatureTable::embedded(parsed.metadata.build);
    // `script_signatures` is keyed by the *packed* script id (group << 16), while a
    // `gosub_with_params` operand is the bare group id — try the packed form first,
    // then the raw id in case a caller already holds a packed reference.
    let callee = |id: i32| {
        let sig = reverse_ctx
            .script_signatures
            .get(&ScriptId(id << 16))
            .or_else(|| reverse_ctx.script_signatures.get(&ScriptId(id)))?;
        Some(CalleeSig {
            arg_int: sig.arg_count_int,
            arg_obj: sig.arg_count_obj,
            arg_long: sig.arg_count_long,
            ret_int: sig.return_count_int,
            ret_obj: sig.return_count_obj,
            ret_long: sig.return_count_long,
        })
    };
    let inferred = infer_program(&[(0, &script)], sigs, &callee);
    if let Some(locals) = inferred.get(&0) {
        parsed.structured_source = annotate_local_declarations(&parsed.structured_source, locals);
    }
}

struct FinalizedTranspileOutput {
    source: String,
    fallback_reason: Option<String>,
    primary_blocking_diagnostics: Vec<String>,
    primary_gate_messages: Vec<String>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "fallback finalization threads existing command context and mutable diagnostics"
)]
fn finalize_with_linear_fallback(
    source: String,
    transpiler: &Transpiler,
    script: &CompiledScript,
    script_id: crate::transpile::ScriptId,
    script_catalog: &ScriptCatalog,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
    diagnostics: &mut crate::transpile::Diagnostics,
    editable_structured: &mut bool,
    blocking_diagnostics: &mut Vec<String>,
) -> Result<FinalizedTranspileOutput> {
    let primary_control_fallback = source_control_flow_fallback_reason(&source);
    let primary_diagnostic_start = diagnostics.diagnostics.len();
    let finalized = finalize_reversible_transpile_output(
        source,
        reverse_ctx,
        opcode_book,
        diagnostics,
        editable_structured,
        blocking_diagnostics,
    )?;
    let primary_blocking_diagnostics = if *editable_structured {
        Vec::new()
    } else {
        blocking_diagnostics.clone()
    };
    let primary_gate_messages = diagnostics.diagnostics[primary_diagnostic_start..]
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();
    let should_try_linear_fallback = blocking_diagnostics.iter().any(|blocker| {
        matches!(
            blocker.as_str(),
            "recompile_mismatch" | "reverse_unsupported"
        )
    });
    if *editable_structured || !should_try_linear_fallback {
        return Ok(FinalizedTranspileOutput {
            source: finalized,
            fallback_reason: primary_control_fallback,
            primary_blocking_diagnostics,
            primary_gate_messages,
        });
    }

    let conservative = transpiler.transpile_structured_conservative(script, script_id)?;
    let crate::transpile::TranspiledScript {
        source: conservative_source,
        diagnostics: mut conservative_diagnostics,
        editable_structured: mut conservative_editable,
        blocking_diagnostics: mut conservative_blocking,
        ..
    } = conservative;
    add_ambiguous_export_warning(&mut conservative_diagnostics, script_catalog, script_id);
    let finalized_conservative = finalize_reversible_transpile_output(
        conservative_source,
        reverse_ctx,
        opcode_book,
        &mut conservative_diagnostics,
        &mut conservative_editable,
        &mut conservative_blocking,
    )?;
    if conservative_editable {
        let fallback_reason = source_control_flow_fallback_reason(&finalized_conservative);
        *diagnostics = conservative_diagnostics;
        *editable_structured = conservative_editable;
        *blocking_diagnostics = conservative_blocking;
        return Ok(FinalizedTranspileOutput {
            source: finalized_conservative,
            fallback_reason,
            primary_blocking_diagnostics,
            primary_gate_messages,
        });
    }

    let linear = transpiler.transpile_linear(script, script_id)?;
    let crate::transpile::TranspiledScript {
        source: linear_source,
        diagnostics: mut linear_diagnostics,
        editable_structured: mut linear_editable,
        blocking_diagnostics: mut linear_blocking,
        ..
    } = linear;
    add_ambiguous_export_warning(&mut linear_diagnostics, script_catalog, script_id);
    let finalized_linear = finalize_reversible_transpile_output(
        linear_source,
        reverse_ctx,
        opcode_book,
        &mut linear_diagnostics,
        &mut linear_editable,
        &mut linear_blocking,
    )?;
    if linear_editable {
        *diagnostics = linear_diagnostics;
        *editable_structured = linear_editable;
        *blocking_diagnostics = linear_blocking;
        Ok(FinalizedTranspileOutput {
            source: finalized_linear,
            fallback_reason: Some("gate_mismatch".to_string()),
            primary_blocking_diagnostics,
            primary_gate_messages,
        })
    } else {
        Ok(FinalizedTranspileOutput {
            source: finalized,
            fallback_reason: primary_control_fallback,
            primary_blocking_diagnostics,
            primary_gate_messages,
        })
    }
}


fn output_style_fallback_reason(
    output_style: TranspileOutputStyle,
    fallback_reason: Option<String>,
) -> Option<String> {
    match output_style {
        TranspileOutputStyle::HighTs => fallback_reason,
        TranspileOutputStyle::Reversible => Some("forced_reversible".to_string()),
    }
}

fn add_ambiguous_export_warning(
    diagnostics: &mut crate::transpile::Diagnostics,
    script_catalog: &ScriptCatalog,
    script_id: crate::transpile::ScriptId,
) {
    if let Some(metadata) = script_catalog.get(script_id) {
        let base_name = script_base_export_name(metadata);
        if metadata.export_name != base_name {
            diagnostics.warning(format!(
                "ambiguous export name '{}' resolved to '{}'",
                base_name, metadata.export_name
            ));
        }
    }
}

fn transpile_script_with_style(
    transpiler: &Transpiler,
    script: &CompiledScript,
    script_id: crate::transpile::ScriptId,
    output_style: TranspileOutputStyle,
) -> Result<crate::transpile::TranspiledScript> {
    match output_style {
        TranspileOutputStyle::HighTs => Ok(transpiler.transpile(script, script_id)?),
        TranspileOutputStyle::Reversible => Ok(transpiler.transpile_linear(script, script_id)?),
    }
}

/// A script is only truly `editable_structured` if its structured TS recompiles
/// to the **same bytes** as the original. The original is the embedded ASM
/// trailer (canonical); the candidate is the structured body lowered + encoded.
/// Comparing them gates out scripts that lower cleanly but to different bytes —
/// the false-editables that would silently corrupt the script if a user edited
/// the structured form. Returns `Err((blocker, message))` to mark non-editable.
/// Why a structured recompile was rejected by the fidelity gate. `blocker` is
/// the stable coverage tag; `cause` is a low-cardinality sub-bucket that turns
/// the opaque blocker into a ranked histogram (`<blocker>_cause:*`); `message`
/// is the human-readable detail.
struct RecompileBlock {
    blocker: &'static str,
    cause: Option<String>,
    message: String,
}

/// Bucket a `reverse_unsupported` failure into a low-cardinality cause so the
/// coverage histogram ranks *which* lowering gap blocked the script (parallel
/// to `recompile_mismatch_cause:*`). Keys are static substrings of the bail
/// sites, never interpolated names/ids.
fn classify_reverse_unsupported(message: &str) -> &'static str {
    // The non-lowering phases of recompile_fidelity_check each get their own
    // bucket; everything else is a structured-lowering bail, keyed by the bail
    // text regardless of anyhow context position.
    if message.starts_with("embedded ASM parse") {
        return "asm_parse";
    }
    if message.starts_with("encoding original") {
        return "encode_original";
    }
    if message.starts_with("structured parse") {
        return "structured_parse";
    }
    if message.starts_with("encoding structured") {
        return "encode_structured";
    }
    let patterns: &[(&str, &str)] = &[
        ("goto", "goto"),
        ("comment-only", "comment_control_flow"),
        ("UI hook", "ui_hook"),
        ("UI method", "ui_method"),
        ("UI.", "ui_arity"),
        ("callback watcher", "callback_watcher"),
        ("callback target", "callback_target"),
        ("callback literal", "callback_literal"),
        ("component constant", "unknown_component"),
        ("property access", "property_access"),
        ("negation", "negation"),
        ("logical not", "logical_not"),
        ("string arrays", "string_array"),
        ("array", "array"),
        ("void", "void_local"),
        ("identifier expression", "unknown_identifier"),
        ("assignment target", "assignment_target"),
        ("stack pseudo", "stack_pseudo"),
        ("branch label", "missing_branch_label"),
        ("switch label", "missing_switch_label"),
        ("numeric suffix", "numeric_suffix"),
        ("outside loop", "break_continue_outside_loop"),
    ];
    for (needle, bucket) in patterns {
        if message.contains(needle) {
            return bucket;
        }
    }
    "other"
}

impl RecompileBlock {
    fn reverse_unsupported(message: String) -> Self {
        let cause = classify_reverse_unsupported(&message).to_string();
        Self {
            blocker: "reverse_unsupported",
            cause: Some(cause),
            message,
        }
    }
}

fn recompile_fidelity_check(
    parsed: &crate::transpile::ParsedReversibleSource,
    metadata: &crate::transpile::ReversibleMetadata,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
) -> std::result::Result<(), RecompileBlock> {
    let build = metadata.build;
    let original = parse_cs2_asm(&parsed.asm_trailer).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("embedded ASM parse failed: {e}"))
    })?;
    let expected = encode_script(&original, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding original failed: {e}"))
    })?;

    let structured = parse_structured_typescript(&parsed.structured_source).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("structured parse failed: {e}"))
    })?;
    let compiled = lower_structured_script(&structured, metadata, reverse_ctx).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("structured lowering failed: {e}"))
    })?;
    let actual = encode_script(&compiled, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding structured failed: {e}"))
    })?;

    if actual != expected {
        let (cause, message) = recompile_divergence(&original, &compiled);
        return Err(RecompileBlock {
            blocker: "recompile_mismatch",
            cause: Some(cause),
            message,
        });
    }
    Ok(())
}

/// Reduce a gate failure message to a low-cardinality head for histogramming (diagnostic-only):
/// strip the `… failed: ` wrapper and collapse digit runs to `#`, so the opaque buckets group by
/// their actual `bail!` reason instead of per-script operand noise.
fn gate_message_head(message: &str) -> String {
    const PREFIXES: [&str; 6] = [
        "runescript lowering failed: ",
        "runescript parse failed: ",
        "structured parse failed: ",
        "embedded ASM parse failed: ",
        "encoding original failed: ",
        "encoding runescript failed: ",
    ];
    let mut body = message;
    for prefix in PREFIXES {
        if let Some(rest) = body.strip_prefix(prefix) {
            body = rest;
            break;
        }
    }
    let mut out = String::new();
    let mut last_digit = false;
    for c in body.chars() {
        if c.is_ascii_digit() {
            if !last_digit {
                out.push('#');
                last_digit = true;
            }
        } else {
            last_digit = false;
            out.push(c);
        }
        if out.len() >= 200 {
            break;
        }
    }
    out
}

/// The byte gate over the **RuneScript** surface (G1.4): render the structured form to RuneScript,
/// parse it back, lower the result, and compare bytes against the original — proving the RuneScript
/// editing surface round-trips byte-exactly. Reuses the same `reverse_ctx`/`opcode_book` and the same
/// `expected` bytes as the TS gate; the only inserted steps are `render_runescript` + `parse_runescript`.
/// Build-948-only (the gate context's command registry is 948).
fn recompile_fidelity_check_runescript(
    parsed: &crate::transpile::ParsedReversibleSource,
    metadata: &crate::transpile::ReversibleMetadata,
    reverse_ctx: &ReverseCompileContext,
    opcode_book: &OpcodeBook,
) -> std::result::Result<(), RecompileBlock> {
    let build = metadata.build;
    let original = parse_cs2_asm(&parsed.asm_trailer).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("embedded ASM parse failed: {e}"))
    })?;
    let expected = encode_script(&original, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding original failed: {e}"))
    })?;

    let structured = parse_structured_typescript(&parsed.structured_source).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("structured parse failed: {e}"))
    })?;
    let ctx = runescript_gate_context(reverse_ctx);
    let rendered = crate::transpile::render_runescript(&structured, ctx);
    let reparsed = crate::transpile::parse_runescript(&rendered, ctx).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("runescript parse failed: {e}"))
    })?;
    let compiled = lower_structured_script(&reparsed, metadata, reverse_ctx).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("runescript lowering failed: {e}"))
    })?;
    let actual = encode_script(&compiled, opcode_book, build).map_err(|e| {
        RecompileBlock::reverse_unsupported(format!("encoding runescript failed: {e}"))
    })?;

    // Deep-dive diagnostic: dump the original vs round-tripped instruction streams for a named script
    // (RS3_RS_DUMP=<substring of the structured source>) so ALL divergences are visible, not just the
    // first one `recompile_divergence` reports.
    if let Ok(target) = std::env::var("RS3_RS_DUMP")
        && !target.is_empty()
        && parsed.structured_source.contains(&target)
    {
        let dump = |s: &crate::script::CompiledScript| -> String {
            s.code
                .iter()
                .enumerate()
                .map(|(i, ins)| format!("{i}: {} {:?}", ins.command, ins.operand))
                .collect::<Vec<_>>()
                .join("\n")
        };
        std::fs::write("/tmp/rs-orig.asm", dump(&original)).ok();
        std::fs::write("/tmp/rs-roundtrip.asm", dump(&compiled)).ok();
        std::fs::write("/tmp/rs-rendered.rs", &rendered).ok();
    }

    if actual != expected {
        let (cause, message) = recompile_divergence(&original, &compiled);
        return Err(RecompileBlock {
            blocker: "runescript_mismatch",
            cause: Some(cause),
            message,
        });
    }
    Ok(())
}

/// The shared `RuneScriptContext` for the byte gate, built once from the script catalog. The gosub
/// name-set is **not** cosmetic for byte fidelity: a gosub whose script name collides with a command
/// name (`date_runeday`, `error`, …) or contains underscores must render with `~` so it parses back
/// as a gosub rather than re-lowering as that command. The catalog is the same for every script in a
/// build run, so the first call populates the set.
fn runescript_gate_context(
    reverse_ctx: &ReverseCompileContext,
) -> &'static crate::transpile::RuneScriptContext {
    use std::sync::OnceLock;
    static CTX: OnceLock<crate::transpile::RuneScriptContext> = OnceLock::new();
    CTX.get_or_init(|| {
        let scripts = reverse_ctx
            .script_catalog
            .export_name_map()
            .into_values()
            .collect();
        crate::transpile::RuneScriptContext::new(scripts)
    })
}

/// Describe the first instruction-level divergence between the original script
/// and the structured recompile, to make `recompile_mismatch` actionable.
/// Returns `(cause, message)`: `cause` is a low-cardinality bucket key (command
/// names only, never operand values) for the coverage histogram; `message` is
/// the human-readable detail for the diagnostics log.
fn recompile_divergence(
    original: &crate::script::CompiledScript,
    compiled: &crate::script::CompiledScript,
) -> (String, String) {
    for (i, (a, b)) in original.code.iter().zip(compiled.code.iter()).enumerate() {
        let command_differs = a.command != b.command;
        let operand_differs = format!("{:?}", a.operand) != format!("{:?}", b.operand);
        if command_differs || operand_differs {
            let cause = if command_differs {
                format!("{}->{}", a.command, b.command)
            } else {
                format!("{}:operand", a.command)
            };
            let message = format!(
                "recompile diverges at [{i}]: original `{} {:?}` vs structured `{} {:?}`",
                a.command, a.operand, b.command, b.operand
            );
            return (cause, message);
        }
    }
    if original.code.len() != compiled.code.len() {
        let cause = if original.code.len() < compiled.code.len() {
            "length:structured_longer"
        } else {
            "length:structured_shorter"
        };
        let message = format!(
            "recompile length mismatch: original {} instructions vs structured {}",
            original.code.len(),
            compiled.code.len()
        );
        return (cause.to_string(), message);
    }
    (
        "header_or_locals".to_string(),
        "recompile differs in header/locals/args only".to_string(),
    )
}


#[expect(
    clippy::too_many_arguments,
    reason = "CLI dispatcher passes parsed command fields"
)]
fn run_transpile_scripts(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    filter_script: Option<&str>,
    output_style: TranspileOutputStyle,
    max_scripts: usize,
    all_scripts: bool,
    limits: crate::transpile::TranspileLimits,
    version: RuntimeVersion,
) -> Result<()> {
    if let Some(filter) = filter_script
        && !all_scripts
    {
        return run_filtered_transpile_scripts(
            cache,
            tar_path,
            data_dir,
            out_dir,
            filter,
            output_style,
            max_scripts,
            limits,
            version,
        );
    }

    let types_exist = out_dir.join("index.ts").exists();
    let mut ctx = if types_exist {
        ResolverContext::load_transpile(cache, tar_path, data_dir, version.build, version.subbuild)?
    } else {
        ResolverContext::load_lazy(cache, tar_path, data_dir, version.build, version.subbuild)?
    };
    let opcode_book = ctx.opcode_book.clone();
    let script_group_names = load_script_group_names_from_cache(cache, data_dir)?;
    let mut script_catalog_builder = crate::transpile::ScriptCatalogBuilder::new(
        &script_group_names,
        &opcode_book,
        version.build,
    )
    .without_return_types();
    for (&packed_id_raw, data) in &ctx.scripts {
        script_catalog_builder.add_script(packed_id_raw, data);
    }
    let script_catalog = script_catalog_builder.build();
    let mut transpiler = Transpiler::new()
        .with_version(version.build, version.subbuild)
        .with_enums(&ctx.enums)
        .with_enums_map(&ctx.enums)
        .with_vars(&ctx.varps_by_domain)
        .with_varbits(&ctx.varbits)
        .with_params(&ctx.params)
        .with_limits(limits)
        .with_script_catalog(script_catalog.clone())
        .with_components(&ctx.parsed_components)
        .with_script_signatures(&ctx.scripts, &opcode_book, version.build);

    let mut reverse_ctx = build_reverse_compile_context_from_catalog(&ctx, script_catalog.clone());
    reverse_ctx.script_signatures.extend(
        transpiler
            .script_signatures()
            .iter()
            .map(|(script_id, signature)| (*script_id, signature.clone())),
    );

    fs::create_dir_all(out_dir)?;

    // Generate type definitions so script imports resolve.
    // Skip if index.ts already exists (user may have run ts-export).
    if !out_dir.join("index.ts").exists() {
        export_var_types(&ctx, out_dir)?;
        export_varbit_types(&ctx, out_dir)?;
        export_enum_types(&ctx, out_dir)?;
        export_param_types(&ctx, out_dir)?;
        export_interface_ids(&ctx, out_dir)?;
        export_inv_types(&ctx, out_dir)?;
        export_obj_types(&ctx, out_dir)?;
        export_npc_types(&ctx, out_dir)?;
        export_loc_types(&ctx, out_dir)?;
        export_seq_types(&ctx, out_dir)?;
        export_spot_types(&ctx, out_dir)?;
        export_named_config_ids(&ctx, out_dir)?;
        export_db_types(&ctx, out_dir)?;
        export_script_signatures(out_dir, &script_catalog)?;
        export_index(out_dir)?;
    }

    trim_transpile_runtime_context(&mut ctx);

    let script_limit = if all_scripts { usize::MAX } else { max_scripts };
    let trace_transpile = std::env::var_os("RS3_TRANSPILE_TRACE").is_some();

    let mut signature_cache = transpiler.script_signatures().clone();
    let mut script_count = 0;
    let mut errors = 0;
    let mut barrel_exports: Vec<String> = Vec::new();
    let mut script_diagnostics = Vec::new();

    for (&script_id_raw, data) in &ctx.scripts {
        let script_id = crate::transpile::ScriptId(script_id_raw as i32);
        if trace_transpile {
            let script_name = transpiler
                .script_name_for(script_id)
                .unwrap_or_else(|| format!("script{script_id}"));
            eprintln!("trace: transpile script_{script_id_raw} {script_name}");
        }

        if let Some(filter) = filter_script {
            let name = transpiler.script_name_for(script_id);
            if name.map(|n| !n.contains(filter)).unwrap_or(true) {
                continue;
            }
        }

        let script = match decode_script(data, &opcode_book, version.build) {
            Ok(script) => script,
            Err(err) => {
                eprintln!("failed to decode script_{script_id}: {err}");
                errors += 1;
                continue;
            }
        };
        for referenced_script in collect_referenced_scripts(&script) {
            let Some(metadata) = script_catalog.resolve_call_target(referenced_script.0) else {
                continue;
            };
            let Some(target_data) = ctx.scripts.get(&(metadata.packed_id.0 as u32)) else {
                continue;
            };
            ensure_transpile_script_signature_from_bytes(
                &mut signature_cache,
                &mut transpiler,
                &script_catalog,
                metadata.packed_id,
                target_data,
                &opcode_book,
                version.build,
            );
            // Mirror the inferred return type into the lowering context so the
            // recompile-fidelity gate classifies this call (void vs value) the
            // same way the renderer did.
            if let Some(signature) = signature_cache.get(&metadata.packed_id) {
                reverse_ctx
                    .script_signatures
                    .insert(metadata.packed_id, signature.clone());
            }
        }
        ensure_transpile_script_signature(
            &mut signature_cache,
            &mut transpiler,
            &script_catalog,
            script_id,
            &script,
            version.build,
        );

        match transpile_script_with_style(&transpiler, &script, script_id, output_style) {
            Ok(ts) => {
                let crate::transpile::TranspiledScript {
                    source,
                    diagnostics,
                    editable_structured,
                    blocking_diagnostics,
                    control_flow_fallback_reason,
                    ..
                } = ts;
                let script_name = transpiler
                    .script_name_for(script_id)
                    .unwrap_or_else(|| format!("script{script_id}"));
                let function_name = script_name.clone();
                let filename = format!("{}.ts", sanitize_file_component(&script_name));
                let out_path = out_dir.join(&filename);
                barrel_exports.push(format!(
                    "export {{ {function_name} }} from './{filename_no_ext}';",
                    filename_no_ext = filename.trim_end_matches(".ts")
                ));
                let mut diagnostics = diagnostics;
                let mut editable_structured = editable_structured;
                let mut blocking_diagnostics = blocking_diagnostics;
                add_ambiguous_export_warning(&mut diagnostics, &script_catalog, script_id);
                let finalized = finalize_with_linear_fallback(
                    source,
                    &transpiler,
                    &script,
                    script_id,
                    &script_catalog,
                    &reverse_ctx,
                    &opcode_book,
                    &mut diagnostics,
                    &mut editable_structured,
                    &mut blocking_diagnostics,
                )?;
                let high_ts_style = HighTsScriptStyle::from_source(&finalized.source);
                let high_ts_fallback_reason = output_style_fallback_reason(
                    output_style,
                    finalized.fallback_reason.or(control_flow_fallback_reason),
                );
                let high_ts_gate_diagnostics = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_blocking_diagnostics,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                let high_ts_gate_messages = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_gate_messages,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                fs::write(&out_path, &finalized.source)?;
                if let Some(metadata) = script_catalog.get(script_id) {
                    script_diagnostics.push(ScriptDiagnosticsEntry {
                        packed_id: metadata.packed_id.0,
                        group_id: metadata.group_id.0,
                        export_name: metadata.export_name.clone(),
                        module_name: metadata.module_name.clone(),
                        editable_structured,
                        blocking_diagnostics,
                        high_ts_style,
                        high_ts_fallback_reason,
                        high_ts_gate_diagnostics,
                        high_ts_gate_messages,
                        diagnostics: diagnostics.diagnostics,
                    });
                }
                script_count += 1;
                if script_count >= script_limit {
                    break;
                }
            }
            Err(e) => {
                eprintln!("failed to transpile script_{script_id}: {e}");
                errors += 1;
            }
        }
    }

    // Write scripts barrel file so you can import { script_N } from './scripts'
    if !barrel_exports.is_empty() {
        barrel_exports.sort();
        let mut lines = vec![
            "// Auto-generated scripts barrel".to_string(),
            "// Source: RS3 cache transpile-scripts".to_string(),
            String::new(),
        ];
        lines.extend(barrel_exports);
        write_text(&out_dir.join("scripts.ts"), &lines.join("\n"))?;
    }

    write_transpile_diagnostics_report(out_dir, version.build, Vec::new(), script_diagnostics)?;

    eprintln!(
        "transpiled {script_count} scripts ({errors} errors) to {}",
        out_dir.display()
    );
    Ok(())
}

struct LoadedScript {
    packed_id: u32,
    script: CompiledScript,
    data: Vec<u8>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "filtered transpile path needs CLI/runtime inputs and cache helpers"
)]
fn run_filtered_transpile_scripts(
    cache: &FlatCache,
    tar_path: &Path,
    data_dir: &Path,
    out_dir: &Path,
    filter_script: &str,
    output_style: TranspileOutputStyle,
    max_scripts: usize,
    limits: crate::transpile::TranspileLimits,
    version: RuntimeVersion,
) -> Result<()> {
    let types_exist = out_dir.join("index.ts").exists();
    let ctx = if types_exist {
        ResolverContext::load_transpile(cache, tar_path, data_dir, version.build, version.subbuild)?
    } else {
        ResolverContext::load_ts_export(cache, tar_path, data_dir, version.build, version.subbuild)?
    };
    let opcode_book = ctx.opcode_book.clone();
    let script_group_names = load_script_group_names_from_cache(cache, data_dir)?;
    let script_archive = FlatCache::open(cache.root())?;
    let script_index = script_archive.archive_index(ARCHIVE_CLIENTSCRIPTS)?;
    let selected_scripts = load_matching_scripts_from_cache(
        &script_archive,
        &script_index,
        &opcode_book,
        version.build,
        &script_group_names,
        filter_script,
        max_scripts,
    )?;

    fs::create_dir_all(out_dir)?;

    // Build the catalog over the selected scripts plus their call targets first,
    // with return-type inference. The filtered run only emits this small set, so
    // the catalog is sufficient for scripts.d.ts and lets it carry accurate
    // signatures (group-based names + real return types) instead of the bulk
    // `script<packed>(): unknown` declarations the cache scan produced.
    let mut script_data = BTreeMap::new();
    let mut script_catalog_builder = crate::transpile::ScriptCatalogBuilder::new(
        &script_group_names,
        &opcode_book,
        version.build,
    );
    for loaded in &selected_scripts {
        script_data.insert(loaded.packed_id, loaded.data.clone());
        script_catalog_builder.add_script(loaded.packed_id, &loaded.data);
    }
    for loaded in &selected_scripts {
        for referenced_script in collect_referenced_scripts(&loaded.script) {
            let raw_id = referenced_script.0;
            let Some((packed_id, data)) =
                load_script_call_target_from_cache(&script_archive, &script_index, raw_id)?
            else {
                continue;
            };
            if script_data.contains_key(&packed_id) {
                continue;
            }
            script_catalog_builder.add_script(packed_id, &data);
            script_data.insert(packed_id, data);
        }
    }
    let script_catalog = script_catalog_builder.build();
    let reverse_ctx = build_reverse_compile_context_from_catalog(&ctx, script_catalog.clone());

    if !types_exist {
        export_var_types(&ctx, out_dir)?;
        export_varbit_types(&ctx, out_dir)?;
        export_enum_types(&ctx, out_dir)?;
        export_param_types(&ctx, out_dir)?;
        export_interface_ids(&ctx, out_dir)?;
        export_inv_types(&ctx, out_dir)?;
        export_obj_types(&ctx, out_dir)?;
        export_npc_types(&ctx, out_dir)?;
        export_loc_types(&ctx, out_dir)?;
        export_seq_types(&ctx, out_dir)?;
        export_spot_types(&ctx, out_dir)?;
        export_named_config_ids(&ctx, out_dir)?;
        export_db_types(&ctx, out_dir)?;
        export_script_signatures(out_dir, &script_catalog)?;
        export_index(out_dir)?;
    }

    let mut transpiler = Transpiler::new()
        .with_version(version.build, version.subbuild)
        .with_enums(&ctx.enums)
        .with_enums_map(&ctx.enums)
        .with_vars(&ctx.varps_by_domain)
        .with_varbits(&ctx.varbits)
        .with_params(&ctx.params)
        .with_limits(limits)
        .with_script_catalog(script_catalog.clone())
        .with_components(&ctx.parsed_components);

    let mut signature_cache = HashMap::new();
    let mut script_count = 0;
    let mut errors = 0;
    let mut barrel_exports: Vec<String> = Vec::new();
    let mut script_diagnostics = Vec::new();

    for loaded in &selected_scripts {
        let script_id = crate::transpile::ScriptId(loaded.packed_id as i32);
        for referenced_script in collect_referenced_scripts(&loaded.script) {
            let Some(metadata) = script_catalog.resolve_call_target(referenced_script.0) else {
                continue;
            };
            let Some(target_data) = script_data.get(&(metadata.packed_id.0 as u32)) else {
                continue;
            };
            ensure_transpile_script_signature_from_bytes(
                &mut signature_cache,
                &mut transpiler,
                &script_catalog,
                metadata.packed_id,
                target_data,
                &opcode_book,
                version.build,
            );
        }
        ensure_transpile_script_signature(
            &mut signature_cache,
            &mut transpiler,
            &script_catalog,
            script_id,
            &loaded.script,
            version.build,
        );

        match transpile_script_with_style(&transpiler, &loaded.script, script_id, output_style) {
            Ok(ts) => {
                let crate::transpile::TranspiledScript {
                    source,
                    diagnostics,
                    editable_structured,
                    blocking_diagnostics,
                    control_flow_fallback_reason,
                    ..
                } = ts;
                let script_name = transpiler
                    .script_name_for(script_id)
                    .unwrap_or_else(|| format!("script{script_id}"));
                let function_name = script_name.clone();
                let filename = format!("{}.ts", sanitize_file_component(&script_name));
                let out_path = out_dir.join(&filename);
                barrel_exports.push(format!(
                    "export {{ {function_name} }} from './{filename_no_ext}';",
                    filename_no_ext = filename.trim_end_matches(".ts")
                ));
                let mut diagnostics = diagnostics;
                let mut editable_structured = editable_structured;
                let mut blocking_diagnostics = blocking_diagnostics;
                add_ambiguous_export_warning(&mut diagnostics, &script_catalog, script_id);
                let finalized = finalize_with_linear_fallback(
                    source,
                    &transpiler,
                    &loaded.script,
                    script_id,
                    &script_catalog,
                    &reverse_ctx,
                    &opcode_book,
                    &mut diagnostics,
                    &mut editable_structured,
                    &mut blocking_diagnostics,
                )?;
                let high_ts_style = HighTsScriptStyle::from_source(&finalized.source);
                let high_ts_fallback_reason = output_style_fallback_reason(
                    output_style,
                    finalized.fallback_reason.or(control_flow_fallback_reason),
                );
                let high_ts_gate_diagnostics = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_blocking_diagnostics,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                let high_ts_gate_messages = match output_style {
                    TranspileOutputStyle::HighTs => finalized.primary_gate_messages,
                    TranspileOutputStyle::Reversible => Vec::new(),
                };
                fs::write(&out_path, &finalized.source)?;
                if let Some(metadata) = script_catalog.get(script_id) {
                    script_diagnostics.push(ScriptDiagnosticsEntry {
                        packed_id: metadata.packed_id.0,
                        group_id: metadata.group_id.0,
                        export_name: metadata.export_name.clone(),
                        module_name: metadata.module_name.clone(),
                        editable_structured,
                        blocking_diagnostics,
                        high_ts_style,
                        high_ts_fallback_reason,
                        high_ts_gate_diagnostics,
                        high_ts_gate_messages,
                        diagnostics: diagnostics.diagnostics,
                    });
                }
                script_count += 1;
            }
            Err(err) => {
                eprintln!("failed to transpile script_{script_id}: {err}");
                errors += 1;
            }
        }
    }

    if !barrel_exports.is_empty() {
        barrel_exports.sort();
        let mut lines = vec![
            "// Auto-generated scripts barrel".to_string(),
            "// Source: RS3 cache transpile-scripts".to_string(),
            String::new(),
        ];
        lines.extend(barrel_exports);
        write_text(&out_dir.join("scripts.ts"), &lines.join("\n"))?;
    }

    write_transpile_diagnostics_report(out_dir, version.build, Vec::new(), script_diagnostics)?;

    eprintln!(
        "transpiled {script_count} scripts ({errors} errors) to {}",
        out_dir.display()
    );
    Ok(())
}

fn load_matching_scripts_from_cache<S: std::hash::BuildHasher + Clone>(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    opcode_book: &OpcodeBook,
    build: u32,
    group_names: &HashMap<u32, String, S>,
    filter_script: &str,
    max_scripts: usize,
) -> Result<Vec<LoadedScript>> {
    let mut selected = Vec::new();
    let mut seen_groups = HashSet::new();
    let mut preferred_groups: Vec<u32> = group_names
        .iter()
        .filter(|(_, name)| name.contains(filter_script))
        .map(|(&group, _)| group)
        .collect();
    preferred_groups.sort_unstable();
    preferred_groups.dedup();

    load_matching_scripts_from_groups(
        cache,
        index,
        opcode_book,
        build,
        group_names,
        filter_script,
        max_scripts,
        &preferred_groups,
        &mut seen_groups,
        &mut selected,
    )?;
    if selected.len() < max_scripts {
        load_matching_scripts_from_groups(
            cache,
            index,
            opcode_book,
            build,
            group_names,
            filter_script,
            max_scripts,
            &index.group_id,
            &mut seen_groups,
            &mut selected,
        )?;
    }
    Ok(selected)
}

#[expect(
    clippy::too_many_arguments,
    reason = "group scan needs filter and output accumulators"
)]
fn load_matching_scripts_from_groups<S: std::hash::BuildHasher + Clone>(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    opcode_book: &OpcodeBook,
    build: u32,
    group_names: &HashMap<u32, String, S>,
    filter_script: &str,
    max_scripts: usize,
    groups: &[u32],
    seen_groups: &mut HashSet<u32>,
    selected: &mut Vec<LoadedScript>,
) -> Result<()> {
    for &group in groups {
        if selected.len() >= max_scripts || !seen_groups.insert(group) {
            continue;
        }
        let files = cache.group_files_with_index(index, ARCHIVE_CLIENTSCRIPTS, group)?;
        for (file, data) in files {
            let packed_id = (group << 16) | file;
            let Ok(script) = decode_script(&data, opcode_book, build) else {
                continue;
            };
            let display_name = script
                .name
                .as_deref()
                .map(crate::transpile::extract_script_name_suffix)
                .filter(|name| !name.is_empty())
                .or_else(|| group_names.get(&group).cloned());
            // Match against the GROUP-based synthetic name (`script<group>`), the
            // same name the catalog and output files use. Naming by the packed id
            // here was the bug: group 621 became "script40697856" (unmatched)
            // while group 9476's packed id 621215744 spuriously contained the
            // "script621" filter substring.
            let function_name = crate::transpile::script_function_name(
                crate::transpile::ScriptId(group as i32),
                display_name.as_deref(),
            );
            if function_name.contains(filter_script)
                || display_name
                    .as_deref()
                    .is_some_and(|name| name.contains(filter_script))
            {
                selected.push(LoadedScript {
                    packed_id,
                    script,
                    data,
                });
                if selected.len() >= max_scripts {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn load_script_call_target_from_cache(
    cache: &FlatCache,
    index: &crate::js5::ArchiveIndex,
    raw_id: i32,
) -> Result<Option<(u32, Vec<u8>)>> {
    let Ok(raw_id_u32) = u32::try_from(raw_id) else {
        return Ok(None);
    };

    if index.group_id.contains(&raw_id_u32) {
        let files = cache.group_files_with_index(index, ARCHIVE_CLIENTSCRIPTS, raw_id_u32)?;
        if let Some((file, data)) = files.into_iter().min_by_key(|(file, _)| *file) {
            return Ok(Some(((raw_id_u32 << 16) | file, data)));
        }
    }

    let group = raw_id_u32 >> 16;
    let file = raw_id_u32 & 0xffff;
    if !index.group_id.contains(&group) {
        return Ok(None);
    }
    let files = cache.group_files_with_index(index, ARCHIVE_CLIENTSCRIPTS, group)?;
    Ok(files.get(&file).cloned().map(|data| (raw_id_u32, data)))
}

fn collect_referenced_scripts(script: &CompiledScript) -> Vec<crate::transpile::ScriptId> {
    script
        .code
        .iter()
        .filter_map(|instruction| match instruction.operand {
            Operand::Script(id) => Some(crate::transpile::ScriptId(id)),
            _ => None,
        })
        .collect()
}

fn ensure_transpile_script_signature(
    signature_cache: &mut HashMap<crate::transpile::ScriptId, crate::transpile::ScriptSignature>,
    transpiler: &mut Transpiler,
    script_catalog: &crate::transpile::ScriptCatalog,
    script_id: crate::transpile::ScriptId,
    script: &CompiledScript,
    build: u32,
) {
    if signature_cache.contains_key(&script_id) {
        return;
    }
    if std::env::var_os("RS3_TRANSPILE_TRACE").is_some() {
        let script_name = script_catalog.export_name(script_id).unwrap_or("script");
        eprintln!(
            "trace: signature script_{script_id} {script_name} instructions={} args={}/{}/{}",
            script.code.len(),
            script.argument_count_int,
            script.argument_count_object,
            script.argument_count_long,
        );
        if std::env::var_os("RS3_TRANSPILE_TRACE_OPS").is_some() {
            for (index, instruction) in script.code.iter().enumerate() {
                eprintln!(
                    "trace: op[{index}] {} {:?}",
                    instruction.command, instruction.operand
                );
            }
        }
    }

    let signature =
        infer_transpile_script_signature(signature_cache, script_catalog, script_id, script, build);
    signature_cache.insert(script_id, signature.clone());
    transpiler.set_script_signature(script_id, signature);
}

fn ensure_transpile_script_signature_from_bytes(
    signature_cache: &mut HashMap<crate::transpile::ScriptId, crate::transpile::ScriptSignature>,
    transpiler: &mut Transpiler,
    script_catalog: &crate::transpile::ScriptCatalog,
    script_id: crate::transpile::ScriptId,
    data: &[u8],
    opcode_book: &OpcodeBook,
    version: u32,
) {
    if signature_cache.contains_key(&script_id) {
        return;
    }

    let Ok(script) = decode_script(data, opcode_book, version) else {
        return;
    };
    ensure_transpile_script_signature(
        signature_cache,
        transpiler,
        script_catalog,
        script_id,
        &script,
        version,
    );
}

fn infer_transpile_script_signature(
    signature_cache: &HashMap<crate::transpile::ScriptId, crate::transpile::ScriptSignature>,
    script_catalog: &crate::transpile::ScriptCatalog,
    script_id: crate::transpile::ScriptId,
    script: &CompiledScript,
    build: u32,
) -> crate::transpile::ScriptSignature {
    let empty_components: HashMap<u32, String> = HashMap::new();
    let empty_enums: HashMap<i32, String> = HashMap::new();
    let inferred = crate::transpile::infer_return_signature_for_script(
        script,
        script_id,
        build,
        &empty_components,
        &empty_enums,
        script_catalog,
        signature_cache,
    );
    crate::transpile::ScriptSignature {
        arg_count_int: script.argument_count_int,
        arg_count_obj: script.argument_count_object,
        arg_count_long: script.argument_count_long,
        return_count_int: inferred.return_counts.int,
        return_count_obj: inferred.return_counts.obj,
        return_count_long: inferred.return_counts.long,
        return_type: inferred.return_type,
    }
}

fn trim_transpile_runtime_context(ctx: &mut ResolverContext) {
    ctx.interfaces.clear();
    ctx.decoded_scripts.clear();
    ctx.structs.clear();
    ctx.npcs.clear();
    ctx.objs.clear();
    ctx.locs.clear();
    ctx.seqs.clear();
    ctx.spots.clear();
    ctx.invs.clear();
    ctx.dbtables.clear();
    ctx.dbrows.clear();
}

#[derive(Serialize)]
struct TranspileDiagnosticsReport {
    build: u32,
    coverage: CoverageSummary,
    high_ts_coverage: HighTsCoverageSummary,
    catalog: Vec<crate::transpile::Diagnostic>,
    scripts: Vec<ScriptDiagnosticsEntry>,
}

/// Aggregate structured-decompilation coverage: how many scripts produced
/// editable structured TypeScript vs. fell back to the lossless ASM trailer,
/// plus a histogram of the blockers that forced the fallback. This is the
/// headline completeness metric (see docs/cs2-completeness-plan.md).
#[derive(Serialize)]
struct CoverageSummary {
    total: usize,
    editable: usize,
    blocked: usize,
    editable_pct: f64,
    blockers: BTreeMap<String, usize>,
}

impl CoverageSummary {
    fn from_scripts(scripts: &[ScriptDiagnosticsEntry]) -> Self {
        let total = scripts.len();
        let editable = scripts.iter().filter(|s| s.editable_structured).count();
        let mut blockers = BTreeMap::<String, usize>::new();
        for script in scripts {
            for blocker in &script.blocking_diagnostics {
                *blockers.entry(blocker.clone()).or_default() += 1;
            }
        }
        let editable_pct = if total == 0 {
            0.0
        } else {
            (editable as f64) * 100.0 / (total as f64)
        };
        Self {
            total,
            editable,
            blocked: total - editable,
            editable_pct,
            blockers,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "diagnostic report mirrors independent marker flags"
)]
struct HighTsScriptStyle {
    #[serde(rename = "controlFlowMarkers")]
    control_flow_markers: bool,
    #[serde(rename = "controlFlowMarkerOccurrences")]
    control_flow_marker_occurrences: usize,
    #[serde(rename = "gotoCalls")]
    goto_calls: usize,
    #[serde(rename = "labelCalls")]
    label_calls: usize,
    #[serde(rename = "stackPseudos")]
    stack_pseudos: bool,
    #[serde(rename = "stackPseudoOccurrences")]
    stack_pseudo_occurrences: usize,
    #[serde(rename = "withMode")]
    with_mode: bool,
    #[serde(rename = "withModeOccurrences")]
    with_mode_occurrences: usize,
    #[serde(rename = "unknownCommand")]
    unknown_command: bool,
    #[serde(rename = "unknownCommandOccurrences")]
    unknown_command_occurrences: usize,
    #[serde(rename = "noVisibleLowLevelMarkers")]
    no_visible_low_level_markers: bool,
}

impl HighTsScriptStyle {
    fn from_source(source: &str) -> Self {
        let goto_calls = source.matches("goto(").count();
        let label_calls = source.matches("label(").count();
        let control_flow_marker_occurrences = goto_calls + label_calls;
        let control_flow_markers = control_flow_marker_occurrences > 0;
        let stack_pseudo_occurrences = source.matches("stackpush_then(").count()
            + source.matches("stackassign_").count()
            + source.matches("pop(").count()
            + source.matches("push(").count()
            + source
                .matches("push_array_int_leave_index_on_stack")
                .count();
        let stack_pseudos = stack_pseudo_occurrences > 0;
        let with_mode_occurrences = source.matches("WithMode").count();
        let with_mode = with_mode_occurrences > 0;
        let unknown_command_occurrences = source.matches("unknowncommand").count();
        let unknown_command = unknown_command_occurrences > 0;
        let no_visible_low_level_markers =
            !(control_flow_markers || stack_pseudos || with_mode || unknown_command);
        Self {
            control_flow_markers,
            control_flow_marker_occurrences,
            goto_calls,
            label_calls,
            stack_pseudos,
            stack_pseudo_occurrences,
            with_mode,
            with_mode_occurrences,
            unknown_command,
            unknown_command_occurrences,
            no_visible_low_level_markers,
        }
    }
}

#[derive(Serialize)]
struct HighTsCoverageSummary {
    total: usize,
    #[serde(rename = "controlFlowMarkers")]
    control_flow_markers: usize,
    #[serde(rename = "controlFlowMarkerOccurrences")]
    control_flow_marker_occurrences: usize,
    #[serde(rename = "gotoCalls")]
    goto_calls: usize,
    #[serde(rename = "labelCalls")]
    label_calls: usize,
    #[serde(rename = "stackPseudos")]
    stack_pseudos: usize,
    #[serde(rename = "stackPseudoOccurrences")]
    stack_pseudo_occurrences: usize,
    #[serde(rename = "withMode")]
    with_mode: usize,
    #[serde(rename = "withModeOccurrences")]
    with_mode_occurrences: usize,
    #[serde(rename = "unknownCommand")]
    unknown_command: usize,
    #[serde(rename = "unknownCommandOccurrences")]
    unknown_command_occurrences: usize,
    #[serde(rename = "noVisibleLowLevelMarkers")]
    no_visible_low_level_markers: usize,
    #[serde(rename = "noVisibleLowLevelMarkersPct")]
    no_visible_low_level_markers_pct: f64,
    #[serde(rename = "fallbackReasons")]
    fallback_reasons: BTreeMap<String, usize>,
    #[serde(rename = "fallbackGateBlockers")]
    fallback_gate_blockers: BTreeMap<String, usize>,
}

impl HighTsCoverageSummary {
    fn from_scripts(scripts: &[ScriptDiagnosticsEntry]) -> Self {
        let total = scripts.len();
        let mut fallback_reasons = BTreeMap::<String, usize>::new();
        let mut fallback_gate_blockers = BTreeMap::<String, usize>::new();
        for script in scripts {
            if let Some(reason) = &script.high_ts_fallback_reason {
                *fallback_reasons.entry(reason.clone()).or_default() += 1;
            }
            for blocker in &script.high_ts_gate_diagnostics {
                *fallback_gate_blockers.entry(blocker.clone()).or_default() += 1;
            }
        }
        let no_visible_low_level_markers = scripts
            .iter()
            .filter(|script| script.high_ts_style.no_visible_low_level_markers)
            .count();
        let no_visible_low_level_markers_pct = if total == 0 {
            0.0
        } else {
            (no_visible_low_level_markers as f64) * 100.0 / (total as f64)
        };
        Self {
            total,
            control_flow_markers: scripts
                .iter()
                .filter(|script| script.high_ts_style.control_flow_markers)
                .count(),
            control_flow_marker_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.control_flow_marker_occurrences)
                .sum(),
            goto_calls: scripts
                .iter()
                .map(|script| script.high_ts_style.goto_calls)
                .sum(),
            label_calls: scripts
                .iter()
                .map(|script| script.high_ts_style.label_calls)
                .sum(),
            stack_pseudos: scripts
                .iter()
                .filter(|script| script.high_ts_style.stack_pseudos)
                .count(),
            stack_pseudo_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.stack_pseudo_occurrences)
                .sum(),
            with_mode: scripts
                .iter()
                .filter(|script| script.high_ts_style.with_mode)
                .count(),
            with_mode_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.with_mode_occurrences)
                .sum(),
            unknown_command: scripts
                .iter()
                .filter(|script| script.high_ts_style.unknown_command)
                .count(),
            unknown_command_occurrences: scripts
                .iter()
                .map(|script| script.high_ts_style.unknown_command_occurrences)
                .sum(),
            no_visible_low_level_markers,
            no_visible_low_level_markers_pct,
            fallback_reasons,
            fallback_gate_blockers,
        }
    }
}

#[derive(Serialize)]
struct ScriptDiagnosticsEntry {
    packed_id: i32,
    group_id: i32,
    export_name: String,
    module_name: String,
    #[serde(rename = "editableStructured")]
    editable_structured: bool,
    #[serde(rename = "blockingDiagnostics")]
    blocking_diagnostics: Vec<String>,
    #[serde(rename = "highTsStyle")]
    high_ts_style: HighTsScriptStyle,
    #[serde(rename = "highTsFallbackReason")]
    high_ts_fallback_reason: Option<String>,
    #[serde(rename = "highTsGateDiagnostics")]
    high_ts_gate_diagnostics: Vec<String>,
    #[serde(rename = "highTsGateMessages")]
    high_ts_gate_messages: Vec<String>,
    diagnostics: Vec<crate::transpile::Diagnostic>,
}

fn write_transpile_diagnostics_report(
    out_dir: &Path,
    build: u32,
    catalog: Vec<crate::transpile::Diagnostic>,
    mut scripts: Vec<ScriptDiagnosticsEntry>,
) -> Result<()> {
    scripts.sort_by(|a, b| a.packed_id.cmp(&b.packed_id));
    let coverage = CoverageSummary::from_scripts(&scripts);
    let high_ts_coverage = HighTsCoverageSummary::from_scripts(&scripts);
    // Canonical coverage event (queryable; the headline completeness metric).
    eprintln!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "event": "transpile_coverage",
            "build": build,
            "total": coverage.total,
            "editable": coverage.editable,
            "blocked": coverage.blocked,
            "editable_pct": coverage.editable_pct,
            "blockers": coverage.blockers,
        }))?
    );
    eprintln!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "event": "high_ts_coverage",
            "build": build,
            "total": high_ts_coverage.total,
            "control_flow_markers": high_ts_coverage.control_flow_markers,
            "control_flow_marker_occurrences": high_ts_coverage.control_flow_marker_occurrences,
            "goto_calls": high_ts_coverage.goto_calls,
            "label_calls": high_ts_coverage.label_calls,
            "stack_pseudos": high_ts_coverage.stack_pseudos,
            "stack_pseudo_occurrences": high_ts_coverage.stack_pseudo_occurrences,
            "with_mode": high_ts_coverage.with_mode,
            "with_mode_occurrences": high_ts_coverage.with_mode_occurrences,
            "unknown_command": high_ts_coverage.unknown_command,
            "unknown_command_occurrences": high_ts_coverage.unknown_command_occurrences,
            "no_visible_low_level_markers": high_ts_coverage.no_visible_low_level_markers,
            "no_visible_low_level_markers_pct": high_ts_coverage.no_visible_low_level_markers_pct,
            "fallback_reasons": high_ts_coverage.fallback_reasons,
            "fallback_gate_blockers": high_ts_coverage.fallback_gate_blockers,
        }))?
    );
    let report = TranspileDiagnosticsReport {
        build,
        coverage,
        high_ts_coverage,
        catalog,
        scripts,
    };
    let json = serde_json::to_string_pretty(&report)?;
    write_text(&out_dir.join("transpile-diagnostics.json"), &json)
}

fn script_base_export_name(metadata: &crate::transpile::ScriptMetadata) -> String {
    let base_name = crate::transpile::sanitize_export_name(&metadata.short_name);
    if base_name.is_empty() || base_name == "script" {
        format!("script{}", metadata.group_id.0)
    } else {
        base_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_ts_style_classifies_visible_low_level_markers() {
        let source = r#"
export function script0(): void {
    label(10);
    stackpush_then(1, goto(20));
    UI.setTextWithMode("x", 1);
    unknowncommand58(0);
}
"#;

        let style = HighTsScriptStyle::from_source(source);

        assert!(style.control_flow_markers);
        assert_eq!(1, style.goto_calls);
        assert_eq!(1, style.label_calls);
        assert_eq!(2, style.control_flow_marker_occurrences);
        assert!(style.stack_pseudos);
        assert_eq!(1, style.stack_pseudo_occurrences);
        assert!(style.with_mode);
        assert_eq!(1, style.with_mode_occurrences);
        assert!(style.unknown_command);
        assert_eq!(1, style.unknown_command_occurrences);
        assert!(!style.no_visible_low_level_markers);
    }

    #[test]
    fn high_ts_coverage_counts_markers_and_fallback_reasons() {
        let scripts = vec![
            ScriptDiagnosticsEntry {
                packed_id: 1,
                group_id: 1,
                export_name: "script1".to_string(),
                module_name: "script1".to_string(),
                editable_structured: true,
                blocking_diagnostics: Vec::new(),
                high_ts_style: HighTsScriptStyle::from_source("goto(1);"),
                high_ts_fallback_reason: Some("gate_mismatch".to_string()),
                high_ts_gate_diagnostics: vec![
                    "recompile_mismatch".to_string(),
                    "recompile_mismatch_cause:branch:operand".to_string(),
                ],
                high_ts_gate_messages: vec![
                    "recompile diverges at [0]: original `branch Branch(1)` vs structured `branch Branch(2)`"
                        .to_string(),
                ],
                diagnostics: Vec::new(),
            },
            ScriptDiagnosticsEntry {
                packed_id: 2,
                group_id: 2,
                export_name: "script2".to_string(),
                module_name: "script2".to_string(),
                editable_structured: true,
                blocking_diagnostics: Vec::new(),
                high_ts_style: HighTsScriptStyle::from_source("return;"),
                high_ts_fallback_reason: None,
                high_ts_gate_diagnostics: Vec::new(),
                high_ts_gate_messages: Vec::new(),
                diagnostics: Vec::new(),
            },
        ];

        let coverage = HighTsCoverageSummary::from_scripts(&scripts);

        assert_eq!(2, coverage.total);
        assert_eq!(1, coverage.control_flow_markers);
        assert_eq!(1, coverage.control_flow_marker_occurrences);
        assert_eq!(1, coverage.goto_calls);
        assert_eq!(0, coverage.label_calls);
        assert_eq!(1, coverage.no_visible_low_level_markers);
        assert_eq!(Some(&1), coverage.fallback_reasons.get("gate_mismatch"));
        assert_eq!(
            Some(&1),
            coverage.fallback_gate_blockers.get("recompile_mismatch")
        );
    }

}
