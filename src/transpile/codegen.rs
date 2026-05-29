use super::ast::{
    ArgumentVariable, Declaration, InstructionNode, LocalVariable, OperandNode, ScriptId,
    SwitchCase, TypeAnnotation, VarBitId, VarBitRefNode, VarId, VarRefNode,
};
use super::scope::SymbolTable;
use crate::script::{CompiledScript, Instruction, Operand, VarBitRef, VarRef};

pub struct CodeGen<'a> {
    symbol_table: &'a SymbolTable,
}

impl<'a> CodeGen<'a> {
    pub fn new(symbol_table: &'a SymbolTable) -> Self {
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
