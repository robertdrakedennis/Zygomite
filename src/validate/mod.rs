use crate::dep_tree::ResolverContext;
use crate::script::{CompiledScript, OpcodeBook, Operand};
use crate::transpile::{
    ScriptCatalog, ScriptCatalogBuilder, ScriptId, ScriptSignature, build_script_catalog,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod stack_effect;

#[cfg(test)]
mod tests;

// ── Error types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationError {
    UnknownOpcode {
        index: usize,
        opcode: u16,
    },
    InvalidBranchTarget {
        index: usize,
        target: i32,
        total_instructions: usize,
    },
    VarpNotFound {
        index: usize,
        domain: String,
        id: u32,
    },
    VarbitNotFound {
        index: usize,
        id: u32,
    },
    ScriptNotFound {
        index: usize,
        called_id: i32,
    },
    /// Popping from a typed stack that has insufficient values.
    StackUnderflow {
        index: usize,
        stack: String,
        needed: usize,
        available: usize,
    },
    UnbalancedReturn {
        index: usize,
        int_stack: usize,
        obj_stack: usize,
        long_stack: usize,
    },
    MissingReturn,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub script_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_name: Option<String>,
    pub build: u32,
    pub instruction_count: usize,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

// ── Three typed stacks (matching the Ignis runtime) ──

struct TypedStacks {
    ints: Vec<()>,
    objects: Vec<()>,
    longs: Vec<()>,
    unknowns: Vec<()>,
}

impl TypedStacks {
    fn new() -> Self {
        Self {
            ints: Vec::new(),
            objects: Vec::new(),
            longs: Vec::new(),
            unknowns: Vec::new(),
        }
    }
}

/// Per-stack pop/push counts resulting from an instruction.
struct StackEffect {
    pops_int: usize,
    pops_obj: usize,
    pops_long: usize,
    pushes_int: usize,
    pushes_obj: usize,
    pushes_long: usize,
    pushes_unknown: usize,
}

impl StackEffect {
    const fn int_push(n: usize) -> Self {
        Self {
            pushes_int: n,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
        }
    }
    const fn obj_push(n: usize) -> Self {
        Self {
            pushes_obj: n,
            pushes_int: 0,
            pushes_long: 0,
            pushes_unknown: 0,
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
        }
    }
    const fn long_push(n: usize) -> Self {
        Self {
            pushes_long: n,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_unknown: 0,
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
        }
    }
    const fn int_pop(n: usize) -> Self {
        Self {
            pops_int: n,
            pops_obj: 0,
            pops_long: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn int_op(n: usize) -> Self {
        Self {
            pops_int: n,
            pushes_int: 1,
            pops_obj: 0,
            pops_long: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn obj_pop(n: usize) -> Self {
        Self {
            pops_obj: n,
            pops_int: 0,
            pops_long: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn long_pop(n: usize) -> Self {
        Self {
            pops_long: n,
            pops_int: 0,
            pops_obj: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn none() -> Self {
        Self {
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
}

// ── Validator ──

pub struct Cs2Validator<'a> {
    ctx: &'a ResolverContext,
}

impl<'a> Cs2Validator<'a> {
    pub fn new(ctx: &'a ResolverContext) -> Self {
        Self { ctx }
    }

    pub fn validate(&self, script_id: u32) -> ValidationReport {
        let Some(script) = self.ctx.decoded_scripts.get(&script_id) else {
            return missing_script_report(script_id, self.ctx.build);
        };
        let script_catalog = build_validation_catalog(self.ctx, &[]);
        let script_signatures = script_catalog.signature_map();

        self.validate_compiled(
            script_id,
            script,
            &script_catalog,
            &script_signatures,
            script.name.clone(),
        )
    }

    pub fn validate_compiled(
        &self,
        script_id: u32,
        script: &CompiledScript,
        script_catalog: &ScriptCatalog,
        script_signatures: &HashMap<ScriptId, ScriptSignature>,
        script_name: Option<String>,
    ) -> ValidationReport {
        let mut report = ValidationReport {
            script_id,
            script_name,
            build: self.ctx.build,
            instruction_count: script.code.len(),
            errors: Vec::new(),
            warnings: Vec::new(),
        };

        self.pass_structural(script, &mut report);
        self.pass_stack(script, script_catalog, script_signatures, &mut report);
        self.pass_cross_ref(script, script_catalog, &mut report);
        report
    }

    fn pass_structural(&self, script: &CompiledScript, report: &mut ValidationReport) {
        let total = script.code.len();
        for (i, instr) in script.code.iter().enumerate() {
            if self.ctx.opcode_book.name(instr.opcode).is_err()
                && !instr.command.starts_with("cmd_")
            {
                report.errors.push(ValidationError::UnknownOpcode {
                    index: i,
                    opcode: instr.opcode,
                });
            }

            match &instr.operand {
                Operand::Branch(offset) if *offset < 0 || *offset as usize >= total => {
                    report.errors.push(ValidationError::InvalidBranchTarget {
                        index: i,
                        target: *offset,
                        total_instructions: total,
                    });
                }
                Operand::Switch(cases) => {
                    for case in cases {
                        if case.target < 0 || case.target as usize >= total {
                            report.errors.push(ValidationError::InvalidBranchTarget {
                                index: i,
                                target: case.target,
                                total_instructions: total,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn pass_stack(
        &self,
        script: &CompiledScript,
        script_catalog: &crate::transpile::ScriptCatalog,
        script_signatures: &std::collections::HashMap<
            crate::transpile::ScriptId,
            crate::transpile::ScriptSignature,
        >,
        report: &mut ValidationReport,
    ) {
        let mut stacks = TypedStacks::new();

        for (i, instr) in script.code.iter().enumerate() {
            let effect = self.stack_effect_for(instr, script_catalog, script_signatures);

            Self::apply_int_pop(&mut stacks, effect.pops_int, i, report);
            Self::apply_obj_pop(&mut stacks, effect.pops_obj, i, report);
            Self::apply_long_pop(&mut stacks, effect.pops_long, i, report);

            for _ in 0..effect.pushes_int {
                stacks.ints.push(());
            }
            for _ in 0..effect.pushes_obj {
                stacks.objects.push(());
            }
            for _ in 0..effect.pushes_long {
                stacks.longs.push(());
            }
            for _ in 0..effect.pushes_unknown {
                stacks.unknowns.push(());
            }
        }

        match script.code.last().map(|i| i.command.as_str()) {
            Some("return" | "branch") => {}
            Some(cmd) => report
                .warnings
                .push(format!("ends with '{cmd}' (expected 'return' or 'branch')")),
            None => report.errors.push(ValidationError::MissingReturn),
        }
    }

    fn apply_int_pop(
        stacks: &mut TypedStacks,
        needed: usize,
        index: usize,
        report: &mut ValidationReport,
    ) {
        let available = stacks.ints.len() + stacks.unknowns.len();
        if available < needed {
            report.errors.push(ValidationError::StackUnderflow {
                index,
                stack: "int".to_string(),
                needed,
                available,
            });
            stacks.ints.clear();
            stacks.unknowns.clear();
        } else {
            let typed_used = needed.min(stacks.ints.len());
            stacks.ints.truncate(stacks.ints.len() - typed_used);
            let unknown_used = needed - typed_used;
            if unknown_used > 0 {
                stacks
                    .unknowns
                    .truncate(stacks.unknowns.len() - unknown_used);
            }
        }
    }

    fn apply_obj_pop(
        stacks: &mut TypedStacks,
        needed: usize,
        index: usize,
        report: &mut ValidationReport,
    ) {
        let available = stacks.objects.len() + stacks.unknowns.len();
        if available < needed {
            report.errors.push(ValidationError::StackUnderflow {
                index,
                stack: "obj".to_string(),
                needed,
                available,
            });
            stacks.objects.clear();
            stacks.unknowns.clear();
        } else {
            let typed_used = needed.min(stacks.objects.len());
            stacks.objects.truncate(stacks.objects.len() - typed_used);
            let unknown_used = needed - typed_used;
            if unknown_used > 0 {
                stacks
                    .unknowns
                    .truncate(stacks.unknowns.len() - unknown_used);
            }
        }
    }

    fn apply_long_pop(
        stacks: &mut TypedStacks,
        needed: usize,
        index: usize,
        report: &mut ValidationReport,
    ) {
        let available = stacks.longs.len() + stacks.unknowns.len();
        if available < needed {
            report.errors.push(ValidationError::StackUnderflow {
                index,
                stack: "long".to_string(),
                needed,
                available,
            });
            stacks.longs.clear();
            stacks.unknowns.clear();
        } else {
            let typed_used = needed.min(stacks.longs.len());
            stacks.longs.truncate(stacks.longs.len() - typed_used);
            let unknown_used = needed - typed_used;
            if unknown_used > 0 {
                stacks
                    .unknowns
                    .truncate(stacks.unknowns.len() - unknown_used);
            }
        }
    }

    fn pass_cross_ref(
        &self,
        script: &CompiledScript,
        script_catalog: &crate::transpile::ScriptCatalog,
        report: &mut ValidationReport,
    ) {
        for (i, instr) in script.code.iter().enumerate() {
            match &instr.operand {
                Operand::VarRef(vr) => {
                    let exists = self
                        .ctx
                        .varps_by_domain
                        .get(&vr.domain)
                        .and_then(|vars| vars.get(&u32::from(vr.id)))
                        .is_some();
                    if !exists {
                        report.errors.push(ValidationError::VarpNotFound {
                            index: i,
                            domain: vr.domain.as_label().to_string(),
                            id: u32::from(vr.id),
                        });
                    }
                }
                Operand::VarBitRef(vbr) => {
                    if !self.ctx.varbits.contains_key(&u32::from(vbr.id)) {
                        report.errors.push(ValidationError::VarbitNotFound {
                            index: i,
                            id: u32::from(vbr.id),
                        });
                    }
                }
                Operand::Script(called_id) => {
                    if script_catalog.resolve_call_target(*called_id).is_none() {
                        report.warnings.push(format!(
                            "[{i}] gosub_with_params to script {called_id}: not found in build {}",
                            self.ctx.build
                        ));
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn build_validation_catalog(
    ctx: &ResolverContext,
    extra_scripts: &[(u32, Vec<u8>)],
) -> ScriptCatalog {
    let empty_group_names = HashMap::<u32, String>::new();
    if extra_scripts.is_empty() {
        return build_script_catalog(
            &ctx.scripts,
            &empty_group_names,
            &ctx.opcode_book,
            ctx.build,
        );
    }
    let mut merged_scripts = ctx.scripts.clone();
    for (packed_id, bytes) in extra_scripts {
        merged_scripts.insert(*packed_id, bytes.clone());
    }
    build_script_catalog(
        &merged_scripts,
        &empty_group_names,
        &ctx.opcode_book,
        ctx.build,
    )
}

pub fn extend_validation_catalog(
    base_catalog: &ScriptCatalog,
    opcode_book: &OpcodeBook,
    build: u32,
    extra_scripts: &[(u32, &[u8])],
) -> ScriptCatalog {
    if extra_scripts.is_empty() {
        return base_catalog.clone();
    }

    let empty_group_names = HashMap::<u32, String>::new();
    let mut builder = ScriptCatalogBuilder::new(&empty_group_names, opcode_book, build);
    for (packed_id, bytes) in extra_scripts {
        builder.add_script(*packed_id, bytes);
    }

    let mut catalog = base_catalog.clone();
    let overlay_catalog = builder.build();
    for metadata in overlay_catalog.iter() {
        catalog.insert(metadata.clone());
    }
    catalog
}

fn missing_script_report(script_id: u32, build: u32) -> ValidationReport {
    let mut report = ValidationReport {
        script_id,
        script_name: None,
        build,
        instruction_count: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    report.errors.push(ValidationError::ScriptNotFound {
        index: 0,
        called_id: script_id as i32,
    });
    report
}

// ── Batch validation ──

#[derive(Debug, Clone, Serialize)]
pub struct BatchReport {
    pub build: u32,
    pub scripts_validated: usize,
    pub scripts_with_errors: usize,
    pub total_errors: usize,
    pub results: Vec<ValidationReport>,
}

pub fn validate_scripts(ctx: &ResolverContext, script_ids: &[u32]) -> BatchReport {
    let validator = Cs2Validator::new(ctx);
    let mut results = Vec::new();
    let mut scripts_with_errors = 0;
    let mut total_errors = 0;

    for &id in script_ids {
        let report = validator.validate(id);
        if !report.is_valid() {
            scripts_with_errors += 1;
        }
        total_errors += report.errors.len();
        results.push(report);
    }

    BatchReport {
        build: ctx.build,
        scripts_validated: results.len(),
        scripts_with_errors,
        total_errors,
        results,
    }
}
