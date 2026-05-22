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
    StackUnderflow {
        index: usize,
        needed: usize,
        available: usize,
    },
    UnbalancedReturn {
        index: usize,
        stack_depth: usize,
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

// ── Stack types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackType {
    Int,
    Long,
    String,
}

impl std::fmt::Display for StackType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int => write!(f, "int"),
            Self::Long => write!(f, "long"),
            Self::String => write!(f, "string"),
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
        let mut stack: Vec<StackType> = Vec::new();

        for (i, instr) in script.code.iter().enumerate() {
            let effect = Self::stack_effect_for(&instr.command);

            if effect.pops > stack.len() {
                report.errors.push(ValidationError::StackUnderflow {
                    index: i,
                    needed: effect.pops,
                    available: stack.len(),
                });
                stack.clear();
            } else {
                for _ in 0..effect.pops {
                    stack.pop();
                }
            }

            let push_type = Self::push_type_for(&instr.command, &instr.operand);
            for _ in 0..effect.pushes {
                stack.push(push_type);
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

    fn stack_effect_for(cmd: &str) -> StackEffect {
        match cmd {
            "push_constant_int" | "push_long_constant" | "push_constant_string" => {
                StackEffect { pops: 0, pushes: 1 }
            }
            "push_var" | "push_varbit" | "pop_varbit" => StackEffect { pops: 0, pushes: 1 },
            "push_int_local" | "push_string_local" | "push_long_local" => {
                StackEffect { pops: 0, pushes: 1 }
            }
            "pop_int_local" | "pop_string_local" | "pop_long_local" | "pop_var" => {
                StackEffect { pops: 1, pushes: 0 }
            }
            "pop_int_discard" | "pop_string_discard" | "pop_long_discard" => {
                StackEffect { pops: 1, pushes: 0 }
            }
            "add" | "sub" | "multiply" | "divide" | "mod" | "and" | "or" | "compare" => {
                StackEffect { pops: 2, pushes: 1 }
            }
            "lowercase" | "uppercase" | "length" | "neg" => StackEffect { pops: 1, pushes: 1 },
            "join_string" => StackEffect { pops: 2, pushes: 1 },
            "branch" => StackEffect { pops: 0, pushes: 0 },
            "branch_not" | "branch_if_true" | "branch_if_false" => {
                StackEffect { pops: 1, pushes: 0 }
            }
            "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals"
            | "long_branch_equals"
            | "long_branch_less_than"
            | "long_branch_greater_than"
            | "long_branch_less_than_or_equals"
            | "long_branch_greater_than_or_equals" => StackEffect { pops: 2, pushes: 0 },
            "switch" => StackEffect { pops: 1, pushes: 0 },
            "return" => StackEffect { pops: 0, pushes: 0 },
            "gosub_with_params" => StackEffect { pops: 1, pushes: 1 },
            "define_array" => StackEffect { pops: 0, pushes: 0 },
            "push_array_int" | "push_array_string" => StackEffect { pops: 1, pushes: 1 },
            "pop_array_int" | "pop_array_string" => StackEffect { pops: 2, pushes: 0 },
            _ if cmd.starts_with("cc_") || cmd.starts_with("if_") => {
                StackEffect { pops: 2, pushes: 0 }
            }
            _ if cmd.starts_with("push_") => StackEffect { pops: 0, pushes: 1 },
            _ if cmd.starts_with("pop_")
                || cmd.starts_with("branch_")
                || cmd.starts_with("long_branch_") =>
            {
                StackEffect { pops: 1, pushes: 0 }
            }
            _ => StackEffect { pops: 0, pushes: 0 },
        }
    }

    fn push_type_for(cmd: &str, operand: &Operand) -> StackType {
        match cmd {
            "push_long_constant" | "push_long_local" => StackType::Long,
            "push_constant_string" | "push_string_local" => StackType::String,
            _ => match operand {
                Operand::Long(_) => StackType::Long,
                Operand::Str(_) => StackType::String,
                _ => StackType::Int,
            },
        }
    }
}

struct StackEffect {
    pops: usize,
    pushes: usize,
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
