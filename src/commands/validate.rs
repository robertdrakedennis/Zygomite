//! `validate-script` — validate a CS2 script's bytecode against the target build.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::context::CommandContext;
use crate::dep_tree::ResolverContext;

/// Options for `validate-script`.
#[derive(Clone, Debug)]
pub struct ValidateScriptOpts {
    pub script_id: u32,
    pub out_file: Option<PathBuf>,
    pub emit_json: bool,
}

/// `validate-script`.
pub fn run(ctx: &CommandContext, opts: ValidateScriptOpts) -> Result<()> {
    let ValidateScriptOpts {
        script_id,
        out_file,
        emit_json,
    } = opts;
    let out_file = out_file.as_deref();

    let resolver = ResolverContext::load(
        ctx.cache(),
        ctx.tar_path(),
        ctx.data_dir(),
        ctx.build(),
        ctx.subbuild(),
    )?;
    let validator = crate::validate::Cs2Validator::new(&resolver);
    let report = validator.validate(script_id);

    if emit_json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    if let Some(path) = out_file {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(path, &json)?;
        eprintln!("validation report written to {}", path.display());
    } else {
        let name = report.script_name.as_deref().unwrap_or("(unnamed)");
        eprintln!(
            "script_{id} \"{name}\" ({count} instructions, build {build})",
            id = report.script_id,
            count = report.instruction_count,
            build = report.build
        );
        if report.errors.is_empty() {
            eprintln!("  [PASS] 0 errors");
        } else {
            for err in &report.errors {
                match err {
                    crate::validate::ValidationError::UnknownOpcode { index, opcode } => {
                        eprintln!("  FAIL [{index}] unknown opcode {opcode}");
                    }
                    crate::validate::ValidationError::InvalidBranchTarget {
                        index,
                        target,
                        total_instructions,
                    } => {
                        eprintln!(
                            "  FAIL [{index}] branch target {target} out of range (0..{total_instructions})"
                        );
                    }
                    crate::validate::ValidationError::VarpNotFound { index, domain, id } => {
                        eprintln!("  FAIL [{index}] varp {domain}:{id} not found");
                    }
                    crate::validate::ValidationError::VarbitNotFound { index, id } => {
                        eprintln!("  FAIL [{index}] varbit {id} not found");
                    }
                    crate::validate::ValidationError::ScriptNotFound { index, called_id } => {
                        eprintln!("  FAIL [{index}] called script {called_id} not found");
                    }
                    crate::validate::ValidationError::StackUnderflow {
                        index,
                        stack,
                        needed,
                        available,
                    } => {
                        eprintln!(
                            "  FAIL [{index}] {stack} stack underflow: needs {needed}, has {available}"
                        );
                    }
                    crate::validate::ValidationError::UnbalancedReturn {
                        index,
                        int_stack,
                        obj_stack,
                        long_stack,
                    } => {
                        eprintln!(
                            "  FAIL [{index}] return with values on stacks: int={int_stack}, obj={obj_stack}, long={long_stack}"
                        );
                    }
                    crate::validate::ValidationError::MissingReturn => {
                        eprintln!("  FAIL missing return statement");
                    }
                }
            }
            eprintln!("  {} error(s)", report.errors.len());
        }
        for warn in &report.warnings {
            eprintln!("  WARN {warn}");
        }
    }
    Ok(())
}
