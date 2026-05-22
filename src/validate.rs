use crate::dep_tree::ResolverContext;
use crate::script::{CompiledScript, Operand};
use serde::{Deserialize, Serialize};

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
}

impl TypedStacks {
    fn new() -> Self {
        Self {
            ints: Vec::new(),
            objects: Vec::new(),
            longs: Vec::new(),
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
}

impl StackEffect {
    const fn int_push(n: usize) -> Self {
        Self {
            pushes_int: n,
            pushes_obj: 0,
            pushes_long: 0,
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
        let mut report = ValidationReport {
            script_id,
            script_name: self
                .ctx
                .decoded_scripts
                .get(&script_id)
                .and_then(|s| s.name.clone()),
            build: self.ctx.build,
            instruction_count: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
        };

        let Some(script) = self.ctx.decoded_scripts.get(&script_id) else {
            report.errors.push(ValidationError::ScriptNotFound {
                index: 0,
                called_id: script_id as i32,
            });
            return report;
        };
        let script = script.clone();
        report.instruction_count = script.code.len();

        self.pass_structural(&script, &mut report);
        self.pass_stack(&script, &mut report);
        self.pass_cross_ref(&script, &mut report);

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

    fn pass_stack(&self, script: &CompiledScript, report: &mut ValidationReport) {
        let mut stacks = TypedStacks::new();

        for (i, instr) in script.code.iter().enumerate() {
            let effect = self.stack_effect_for(instr);

            Self::apply_pop(&mut stacks.ints, effect.pops_int, i, "int", report);
            Self::apply_pop(&mut stacks.objects, effect.pops_obj, i, "obj", report);
            Self::apply_pop(&mut stacks.longs, effect.pops_long, i, "long", report);

            for _ in 0..effect.pushes_int {
                stacks.ints.push(());
            }
            for _ in 0..effect.pushes_obj {
                stacks.objects.push(());
            }
            for _ in 0..effect.pushes_long {
                stacks.longs.push(());
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

    fn apply_pop(
        stack: &mut Vec<()>,
        needed: usize,
        index: usize,
        label: &str,
        report: &mut ValidationReport,
    ) {
        let available = stack.len();
        if available < needed {
            report.errors.push(ValidationError::StackUnderflow {
                index,
                stack: label.to_string(),
                needed,
                available,
            });
            stack.clear();
        } else {
            stack.truncate(available - needed);
        }
    }

    fn pass_cross_ref(&self, script: &CompiledScript, report: &mut ValidationReport) {
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
                    if !self.ctx.scripts.contains_key(&(*called_id as u32)) {
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

    // Based on Ignis ClientScriptState runtime: three typed stacks
    // (intStack/isp, objectStack/osp, longStack/lsp).
    fn stack_effect_for(&self, instr: &crate::script::Instruction) -> StackEffect {
        let cmd = &instr.command;
        match cmd.as_str() {
            // ── Integer pushes ──
            "push_constant_int" => StackEffect::int_push(1),
            "push_int_local" => StackEffect::int_push(1),

            // ── Object (string) pushes ──
            "push_constant_string" => StackEffect::obj_push(1),
            "push_string_local" => StackEffect::obj_push(1),

            // ── Long pushes ──
            "push_long_constant" | "push_constant_long" => StackEffect::long_push(1),
            "push_long_local" => StackEffect::long_push(1),

            // ── push_var: resolve varp type to determine int/obj/long ──
            "push_var" => self.varp_stack_effect_for(&instr.operand, true),

            // ── Integer pops ──
            "pop_int_local" => StackEffect::int_pop(1),
            "pop_int_discard" => StackEffect::int_pop(1),

            // ── Object pops ──
            "pop_string_local" => StackEffect::obj_pop(1),
            "pop_string_discard" => StackEffect::obj_pop(1),

            // ── Long pops ──
            "pop_long_local" | "pop_long_discard" => StackEffect::long_pop(1),

            // ── pop_var: resolve varp type ──
            "pop_var" => self.varp_stack_effect_for(&instr.operand, false),

            // ── Integer arithmetic: pop 2, push 1 ──
            "add" | "sub" | "multiply" | "divide" | "mod" => StackEffect::int_op(2),

            // ── Logical: pop 2 ints, push 1 int ──
            "and" | "or" | "compare" => StackEffect::int_op(2),

            // ── String ops: pop 1 obj, push 1 ──
            "lowercase" | "uppercase" | "length" => StackEffect {
                pops_obj: 1,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "join_string" => StackEffect {
                pops_obj: 2,
                pushes_obj: 1,
                ..StackEffect::none()
            },

            // ── Unary int: pop 1, push 1 ──
            "neg" => StackEffect::int_op(1),

            // ── Control flow: no stack effect ──
            "branch" => StackEffect::none(),
            "branch_not" | "branch_if_true" | "branch_if_false" => StackEffect::int_pop(1),
            "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals"
            | "long_branch_equals"
            | "long_branch_less_than"
            | "long_branch_greater_than"
            | "long_branch_less_than_or_equals"
            | "long_branch_greater_than_or_equals" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "switch" => StackEffect::int_pop(1),
            "return" => StackEffect::none(),
            "gosub_with_params" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },

            // ── Array ops ──
            "define_array" => StackEffect::none(),
            "push_array_int" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "pop_array_int" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "push_array_string" => StackEffect {
                pops_obj: 1,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "pop_array_string" => StackEffect {
                pops_obj: 2,
                ..StackEffect::none()
            },

            // ── Varbit: pops int for bit index, pushes int value ──
            "push_varbit" => StackEffect::int_push(1),
            "pop_varbit" => StackEffect::int_pop(1),

            // ── Engine commands (cc_*, if_*): conservatively assume int args ──
            _ if cmd.starts_with("cc_") || cmd.starts_with("if_") => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },

            // ── Fallback patterns ──
            _ if cmd.starts_with("push_") => StackEffect::int_push(1),
            _ if cmd.starts_with("pop_") => StackEffect::int_pop(1),
            _ if cmd.starts_with("branch_") || cmd.starts_with("long_branch_") => {
                StackEffect::int_pop(1)
            }
            _ => StackEffect::none(),
        }
    }

    /// Resolve a varp reference to determine which stack `push_var`/`pop_var` affects.
    fn varp_stack_effect_for(&self, operand: &Operand, is_push: bool) -> StackEffect {
        if let Operand::VarRef(vr) = operand {
            let type_id = self
                .ctx
                .varps_by_domain
                .get(&vr.domain)
                .and_then(|vars| vars.get(&u32::from(vr.id)))
                .and_then(|v| v.type_id);
            match type_id {
                // type_id 1 = long, 2 = string, 0/None/N = int
                Some(1) if is_push => StackEffect::long_push(1),
                Some(1) => StackEffect {
                    pops_long: 1,
                    ..StackEffect::none()
                },
                Some(2) if is_push => StackEffect::obj_push(1),
                Some(2) => StackEffect {
                    pops_obj: 1,
                    ..StackEffect::none()
                },
                _ if is_push => StackEffect::int_push(1),
                _ => StackEffect::int_pop(1),
            }
        } else if is_push {
            StackEffect::int_push(1)
        } else {
            StackEffect::int_pop(1)
        }
    }
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
