use super::ast::{
    ArgumentVariable, Declaration, ImportStatement, InstructionNode, LocalVariable, OperandNode,
    Program, ScriptId, Statement, SwitchCase, TypeAnnotation, VarBitId, VarBitRefNode, VarId,
    VarRefNode,
};
use super::scope::SymbolTable;
use crate::script::{CompiledScript, Instruction, Operand, VarBitRef, VarRef};
use crate::vars::VarDomain;

pub struct CodeGen {
    symbol_table: SymbolTable,
}

impl CodeGen {
    pub fn new(symbol_table: SymbolTable) -> Self {
        Self { symbol_table }
    }

    pub fn generate(&self, script: &CompiledScript, script_id: ScriptId) -> Declaration {
        let arguments = self.build_arguments(script);
        let locals = self.build_locals(script);
        let instructions = self.build_instructions(script);
        Declaration {
            script_id,
            name: script.name.clone(),
            locals,
            arguments,
            instructions,
        }
    }

    fn build_arguments(&self, script: &CompiledScript) -> Vec<ArgumentVariable> {
        let mut args = Vec::new();
        for i in 0..script.argument_count_int as usize {
            args.push(ArgumentVariable {
                index: i,
                name: format!("arg_int_{i}"),
                type_annotation: TypeAnnotation::Number,
            });
        }
        for i in 0..script.argument_count_object as usize {
            args.push(ArgumentVariable {
                index: script.argument_count_int as usize + i,
                name: format!("arg_obj_{i}"),
                type_annotation: TypeAnnotation::String,
            });
        }
        for i in 0..script.argument_count_long as usize {
            let base = script.argument_count_int as usize + script.argument_count_object as usize;
            args.push(ArgumentVariable {
                index: base + i,
                name: format!("arg_long_{i}"),
                type_annotation: TypeAnnotation::BigInt,
            });
        }
        args
    }

    fn build_locals(&self, script: &CompiledScript) -> Vec<LocalVariable> {
        let mut locals = Vec::new();
        for i in 0..script.local_count_int as usize {
            locals.push(LocalVariable {
                index: i,
                name: format!("local_int_{i}"),
                type_annotation: TypeAnnotation::Number,
            });
        }
        for i in 0..script.local_count_object as usize {
            locals.push(LocalVariable {
                index: script.local_count_int as usize + i,
                name: format!("local_obj_{i}"),
                type_annotation: TypeAnnotation::String,
            });
        }
        for i in 0..script.local_count_long as usize {
            let base = script.local_count_int as usize + script.local_count_object as usize;
            locals.push(LocalVariable {
                index: base + i,
                name: format!("local_long_{i}"),
                type_annotation: TypeAnnotation::BigInt,
            });
        }
        locals
    }

    fn build_instructions(&self, script: &CompiledScript) -> Vec<InstructionNode> {
        script
            .code
            .iter()
            .enumerate()
            .map(|(i, instr)| self.convert_instruction(i, instr))
            .collect()
    }

    fn convert_instruction(&self, index: usize, instr: &Instruction) -> InstructionNode {
        InstructionNode {
            index,
            opcode: instr.opcode,
            command: instr.command.clone(),
            operand: self.convert_operand(&instr.operand),
        }
    }

    fn convert_operand(&self, operand: &Operand) -> OperandNode {
        match operand {
            Operand::Int(v) => OperandNode::Int(*v),
            Operand::Long(v) => OperandNode::Long(*v),
            Operand::Str(s) => OperandNode::String(s.clone()),
            Operand::Local(idx) => OperandNode::Local(*idx as usize),
            Operand::VarRef(vr) => OperandNode::VarRef(self.convert_var_ref(vr)),
            Operand::VarBitRef(vbr) => OperandNode::VarBitRef(self.convert_varbit_ref(vbr)),
            Operand::Branch(target) => OperandNode::Branch(*target as usize),
            Operand::Switch(cases) => OperandNode::Switch(
                cases
                    .iter()
                    .map(|c| SwitchCase {
                        value: c.value,
                        target: c.target as usize,
                    })
                    .collect(),
            ),
            Operand::Script(id) => OperandNode::Script(*id),
            Operand::Array(id) => OperandNode::Array(*id),
            Operand::Count(n) => OperandNode::Count(*n as usize),
            Operand::Byte(b) => OperandNode::Byte(*b),
        }
    }

    fn convert_var_ref(&self, vr: &VarRef) -> VarRefNode {
        VarRefNode {
            domain: vr.domain,
            id: VarId(vr.id),
            name: self.symbol_table.var_name(vr.domain, vr.id).cloned(),
            is_transmog: vr.transmog,
        }
    }

    fn convert_varbit_ref(&self, vbr: &VarBitRef) -> VarBitRefNode {
        VarBitRefNode {
            id: VarBitId(vbr.id),
            name: self.symbol_table.varbit_name(vbr.id).cloned(),
            is_transmog: vbr.transmog,
        }
    }
}

pub fn generate_program(decl: &Declaration) -> Program {
    let comments: Vec<String> = vec![
        format!("script name: {}", decl.name.as_deref().unwrap_or("unknown")),
        format!(
            "script_{}: locals(int={}, obj={}, long={}) args(int={}, obj={}, long={})",
            decl.script_id,
            decl.locals
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::Number))
                .count(),
            decl.locals
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::String))
                .count(),
            decl.locals
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::BigInt))
                .count(),
            decl.arguments
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::Number))
                .count(),
            decl.arguments
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::String))
                .count(),
            decl.arguments
                .iter()
                .filter(|l| matches!(l.type_annotation, TypeAnnotation::BigInt))
                .count(),
        ),
    ];

    let instructions: Vec<Statement> = decl
        .instructions
        .iter()
        .map(|instr| {
            Statement::Comment(format!("{:05}: {}", instr.index, format_instruction(instr)))
        })
        .collect();

    Program {
        imports: vec![ImportStatement {
            module: "./index".to_string(),
            named_exports: vec![
                "VARS".to_string(),
                "VARBITS".to_string(),
                "ENUMS".to_string(),
                "PARAMS".to_string(),
            ],
            is_type_only: false,
        }],
        statements: instructions,
        comments,
    }
}

fn format_instruction(instr: &InstructionNode) -> String {
    match instr.command.as_str() {
        "push_constant_int" => {
            if let OperandNode::Int(v) = instr.operand {
                format!("push({v});")
            } else {
                format!("push({});", format_operand_raw(&instr.operand))
            }
        }
        "push_long_constant" => {
            if let OperandNode::Long(v) = instr.operand {
                format!("push({v}n);")
            } else {
                format!("push({});", format_operand_raw(&instr.operand))
            }
        }
        "push_constant_string" => {
            if let OperandNode::String(s) = &instr.operand {
                format!("push(\"{}\");", escape_ts_string(s))
            } else {
                format!("push({});", format_operand_raw(&instr.operand))
            }
        }
        "push_var" => {
            if let OperandNode::VarRef(vr) = &instr.operand {
                if let Some(ref name) = vr.name {
                    format!("push({name});")
                } else {
                    format!(
                        "push(VARS.get({} * 1000000 + {})!);",
                        u64::from(vr.domain),
                        vr.id
                    )
                }
            } else {
                format!("VAR({});", format_operand_raw(&instr.operand))
            }
        }
        "pop_var" => {
            if let OperandNode::VarRef(vr) = &instr.operand {
                if let Some(ref name) = vr.name {
                    format!("{name} = pop();")
                } else {
                    format!(
                        "VARS.get({} * 1000000 + {}) = pop();",
                        u64::from(vr.domain),
                        vr.id
                    )
                }
            } else {
                format!("pop({});", format_operand_raw(&instr.operand))
            }
        }
        "push_varbit" | "pop_varbit" => {
            if let OperandNode::VarBitRef(vbr) = &instr.operand {
                match &vbr.name {
                    Some(name) => format!("push({name});"),
                    None => format!("push(VARBITS.get({})!);", vbr.id),
                }
            } else {
                format!("VARBIT({});", format_operand_raw(&instr.operand))
            }
        }
        "push_varc_int" | "pop_varc_int" | "push_varc_string" | "pop_varc_string" => {
            if let OperandNode::Int(v) = instr.operand {
                format!(
                    "push(VARS.get({} * 1000000 + {v})!);",
                    VarDomain::Client as u64
                )
            } else {
                format!("push({});", format_operand_raw(&instr.operand))
            }
        }
        "push_int_local" => {
            if let OperandNode::Local(idx) = instr.operand {
                format!("push(local_int_{idx});")
            } else {
                format!("push({});", format_operand_raw(&instr.operand))
            }
        }
        "pop_int_local" => {
            if let OperandNode::Local(idx) = instr.operand {
                format!("local_int_{idx} = pop();")
            } else {
                format!("pop({});", format_operand_raw(&instr.operand))
            }
        }
        "push_string_local" => {
            if let OperandNode::Local(idx) = instr.operand {
                format!("push(local_obj_{idx});")
            } else {
                format!("push({});", format_operand_raw(&instr.operand))
            }
        }
        "pop_string_local" => {
            if let OperandNode::Local(idx) = instr.operand {
                format!("local_obj_{idx} = pop();")
            } else {
                format!("pop({});", format_operand_raw(&instr.operand))
            }
        }
        "push_long_local" => {
            if let OperandNode::Local(idx) = instr.operand {
                format!("push(local_long_{idx});")
            } else {
                format!("push({});", format_operand_raw(&instr.operand))
            }
        }
        "pop_long_local" => {
            if let OperandNode::Local(idx) = instr.operand {
                format!("local_long_{idx} = pop();")
            } else {
                format!("pop({});", format_operand_raw(&instr.operand))
            }
        }
        "branch" => {
            if let OperandNode::Branch(target) = instr.operand {
                format!("goto({target});")
            } else {
                format!("goto({});", format_operand_raw(&instr.operand))
            }
        }
        "branch_not" => format!("if (!pop()) goto({});", format_operand_raw(&instr.operand)),
        "branch_equals" => format!(
            "if (pop() == pop()) goto({});",
            format_operand_raw(&instr.operand)
        ),
        "branch_if_true" => format!("if (pop()) goto({});", format_operand_raw(&instr.operand)),
        "branch_if_false" => format!("if (!pop()) goto({});", format_operand_raw(&instr.operand)),
        "gosub_with_params" => {
            if let OperandNode::Script(id) = instr.operand {
                format!("script_{id}(pop());")
            } else {
                format!("call({});", format_operand_raw(&instr.operand))
            }
        }
        "switch" => {
            if let OperandNode::Switch(cases) = &instr.operand {
                let arms: Vec<String> = cases
                    .iter()
                    .map(|c| format!("case {}: goto({});", c.value, c.target))
                    .collect();
                format!(
                    "switch(pop()) {{\n        {}\n    }}",
                    arms.join("\n        ")
                )
            } else {
                format!("switch({});", format_operand_raw(&instr.operand))
            }
        }
        "join_string" => {
            if let OperandNode::Count(n) = instr.operand {
                format!("push(pop().concat(...pop_multi({n})));")
            } else {
                format!("concat({});", format_operand_raw(&instr.operand))
            }
        }
        "define_array" => {
            if let OperandNode::Array(id) = instr.operand {
                format!("array_{id} = [];")
            } else {
                format!("define_array({});", format_operand_raw(&instr.operand))
            }
        }
        "cc_create" => {
            if let OperandNode::Int(id) = instr.operand {
                format!("UI.create({id});")
            } else {
                format!("UI.create({});", format_operand_raw(&instr.operand))
            }
        }
        "cc_delete" => "UI.delete(pop() as number);".to_string(),
        "cc_settext" => "UI.setText(pop() as number, pop() as string);".to_string(),
        "cc_setgraphic" => "UI.setGraphic(pop() as number, pop() as number);".to_string(),
        "cc_sethide" => "UI.setHide(pop() as number, pop() as boolean);".to_string(),
        _ => format!(
            "{}({});",
            format_command_name(&instr.command),
            format_operand_raw(&instr.operand)
        ),
    }
}

fn format_operand_raw(operand: &OperandNode) -> String {
    match operand {
        OperandNode::Int(v) => v.to_string(),
        OperandNode::Long(v) => format!("{v}n"),
        OperandNode::String(s) => format!("\"{}\"", escape_ts_string(s)),
        OperandNode::Local(idx) => format!("local_{idx}"),
        OperandNode::VarRef(v) => format!("{}:{}", v.domain.as_label(), v.id),
        OperandNode::VarBitRef(v) => format!("varbit:{}", v.id),
        OperandNode::Branch(target) => format!("->{target}"),
        OperandNode::Switch(cases) => {
            let arms: Vec<String> = cases
                .iter()
                .map(|c| format!("{}->{}", c.value, c.target))
                .collect();
            format!("{{{}}}", arms.join(", "))
        }
        OperandNode::Script(id) => format!("script_{id}"),
        OperandNode::Array(id) => format!("array_{id}"),
        OperandNode::Count(n) => format!("count_{n}"),
        OperandNode::Byte(b) => b.to_string(),
    }
}

fn escape_ts_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn format_command_name(cmd: &str) -> String {
    let cmd = cmd.replace('_', "");
    sanitize_ts_ident(&cmd)
}

fn sanitize_ts_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if i == 0 && c.is_ascii_digit() {
            out.push('_');
        }
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unnamed".to_string()
    } else {
        out
    }
}
