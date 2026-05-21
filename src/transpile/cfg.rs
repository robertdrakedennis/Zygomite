use super::ast::{Expression, ReturnStatement, Statement, TypeAnnotation};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub index: usize,
    pub statements: Vec<Statement>,
    pub successors: Vec<usize>,
    pub is_loop_header: bool,
}

#[derive(Debug, Clone)]
pub struct ControlFlowGraph {
    pub blocks: Vec<BasicBlock>,
    pub entry: usize,
    pub exits: Vec<usize>,
}

pub struct CfgBuilder {
    instructions: Vec<super::ast::InstructionNode>,
}

impl CfgBuilder {
    pub fn new(instructions: Vec<super::ast::InstructionNode>) -> Self {
        Self { instructions }
    }

    pub fn build(self) -> ControlFlowGraph {
        if self.instructions.is_empty() {
            return ControlFlowGraph {
                blocks: vec![],
                entry: 0,
                exits: vec![],
            };
        }

        let leaders = self.compute_leaders();
        let blocks = self.create_blocks(&leaders);
        let edges = self.compute_edges(&blocks);

        let mut cfg_blocks = blocks;
        for (i, block) in cfg_blocks.iter_mut().enumerate() {
            block.successors = edges.get(&i).cloned().unwrap_or_default();
        }

        let exits: Vec<usize> = cfg_blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.successors.is_empty())
            .map(|(i, _)| i)
            .collect();

        ControlFlowGraph {
            blocks: cfg_blocks,
            entry: 0,
            exits,
        }
    }

    fn compute_leaders(&self) -> Vec<usize> {
        let mut leaders = HashSet::new();
        leaders.insert(0);

        for (i, instr) in self.instructions.iter().enumerate() {
            if let Some(targets) = self.extract_branch_targets(instr) {
                for &target in &targets {
                    if target < self.instructions.len() {
                        leaders.insert(target);
                    }
                }
            }

            let is_branch_end = matches!(
                instr.command.as_str(),
                "branch"
                    | "branch_not"
                    | "branch_if_true"
                    | "branch_if_false"
                    | "branch_equals"
                    | "gosub_with_params"
                    | "return"
            );
            if is_branch_end {
                let next = i + 1;
                if next < self.instructions.len() {
                    leaders.insert(next);
                }
            }
        }

        let mut leaders: Vec<usize> = leaders.into_iter().collect();
        leaders.sort_unstable();
        leaders
    }

    fn extract_branch_targets(&self, instr: &super::ast::InstructionNode) -> Option<Vec<usize>> {
        match instr.command.as_str() {
            "branch" | "branch_not" | "branch_if_true" | "branch_if_false" | "branch_equals" => {
                if let super::ast::OperandNode::Branch(target) = instr.operand {
                    Some(vec![target, instr.index + 1])
                } else {
                    None
                }
            }
            "switch" => {
                if let super::ast::OperandNode::Switch(cases) = &instr.operand {
                    let targets: Vec<usize> = cases.iter().map(|c| c.target).collect();
                    Some(targets)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn create_blocks(&self, leaders: &[usize]) -> Vec<BasicBlock> {
        let mut blocks = Vec::new();

        for (bi, &start) in leaders.iter().enumerate() {
            let end = if bi + 1 < leaders.len() {
                leaders[bi + 1]
            } else {
                self.instructions.len()
            };

            let statements: Vec<Statement> = (start..end)
                .filter_map(|i| self.instruction_to_statement(&self.instructions[i]))
                .collect();

            let is_loop_header = if start > 0 {
                self.has_back_edge(start, leaders)
            } else {
                false
            };

            blocks.push(BasicBlock {
                index: start,
                statements,
                successors: vec![],
                is_loop_header,
            });
        }

        blocks
    }

    fn has_back_edge(&self, target: usize, _leaders: &[usize]) -> bool {
        for i in 0..self.instructions.len() {
            if let Some(targets) = self.extract_branch_targets(&self.instructions[i])
                && targets.contains(&target)
                && i > target
            {
                return true;
            }
        }
        false
    }

    fn compute_edges(&self, blocks: &[BasicBlock]) -> HashMap<usize, Vec<usize>> {
        let mut edges: HashMap<usize, Vec<usize>> = HashMap::new();

        for (bi, block) in blocks.iter().enumerate() {
            if let Some(last_instr) = self.instructions.iter().find(|i| i.index == block.index)
                && let Some(targets) = self.extract_branch_targets(last_instr)
            {
                for &target in &targets {
                    if let Some(target_block) = blocks.iter().position(|b| b.index == target) {
                        edges.entry(bi).or_default().push(target_block);
                    }
                }
            }

            let next_block = bi + 1;
            if next_block < blocks.len() {
                edges.entry(bi).or_default().push(next_block);
            }
        }

        edges
    }

    fn instruction_to_statement(&self, instr: &super::ast::InstructionNode) -> Option<Statement> {
        use super::ast::{
            CallExpr, ExpressionStatement, GotoStatement, Identifier, NumberLiteral, StringLiteral,
            VariableDeclaration,
        };

        match instr.command.as_str() {
            "push_constant_int" => {
                if let super::ast::OperandNode::Int(v) = instr.operand {
                    return Some(Statement::ExpressionStatement(ExpressionStatement {
                        expr: Expression::NumberLiteral(NumberLiteral { value: v }),
                        semicolon: true,
                    }));
                }
            }
            "push_constant_string" => {
                if let super::ast::OperandNode::String(s) = &instr.operand {
                    return Some(Statement::ExpressionStatement(ExpressionStatement {
                        expr: Expression::StringLiteral(StringLiteral { value: s.clone() }),
                        semicolon: true,
                    }));
                }
            }
            "push_int_local" | "push_string_local" | "push_long_local" => {
                if let super::ast::OperandNode::Local(idx) = instr.operand {
                    let type_ = if instr.command.contains("long") {
                        "local_long"
                    } else if instr.command.contains("string") {
                        "local_obj"
                    } else {
                        "local_int"
                    };
                    return Some(Statement::ExpressionStatement(ExpressionStatement {
                        expr: Expression::Identifier(Identifier {
                            name: format!("{type_}_{idx}"),
                        }),
                        semicolon: true,
                    }));
                }
            }
            "pop_int_local" | "pop_string_local" | "pop_long_local" => {
                if let super::ast::OperandNode::Local(idx) = instr.operand {
                    let (type_, ts_type) = if instr.command.contains("long") {
                        ("local_long", TypeAnnotation::BigInt)
                    } else if instr.command.contains("string") {
                        ("local_obj", TypeAnnotation::String)
                    } else {
                        ("local_int", TypeAnnotation::Number)
                    };
                    return Some(Statement::VariableDeclaration(VariableDeclaration {
                        name: format!("{type_}_{idx}"),
                        type_hint: ts_type,
                        initializer: Some(Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "pop".to_string(),
                            })),
                            arguments: vec![],
                        })),
                    }));
                }
            }
            "branch" => {
                if let super::ast::OperandNode::Branch(target) = instr.operand {
                    return Some(Statement::GotoStatement(GotoStatement { target }));
                }
            }
            "return" => {
                return Some(Statement::ReturnStatement(ReturnStatement { value: None }));
            }
            _ => {}
        }

        None
    }
}

pub fn build_cfg(instructions: Vec<super::ast::InstructionNode>) -> ControlFlowGraph {
    CfgBuilder::new(instructions).build()
}

pub struct StructuredCodeGen {
    cfg: ControlFlowGraph,
}

impl StructuredCodeGen {
    pub fn new(cfg: ControlFlowGraph) -> Self {
        Self { cfg }
    }

    pub fn generate(self) -> Vec<Statement> {
        let mut statements = Vec::new();
        let blocks = self.cfg.blocks.clone();

        for block in &blocks {
            if block.is_loop_header {
                let loop_stmts = self.extract_loop_from_block(block);
                statements.extend(loop_stmts);
            } else {
                statements.extend(block.statements.clone());
            }
        }

        statements
    }

    fn extract_loop_from_block(&self, block: &BasicBlock) -> Vec<Statement> {
        let mut loop_stmts = Vec::new();
        loop_stmts.push(Statement::Comment("while {".to_string()));

        for stmt in &block.statements {
            loop_stmts.push(stmt.clone());
        }

        loop_stmts.push(Statement::Comment("}".to_string()));
        loop_stmts
    }
}

pub fn generate_structured(instructions: Vec<super::ast::InstructionNode>) -> Vec<Statement> {
    let cfg = build_cfg(instructions);
    let generator = StructuredCodeGen::new(cfg);
    generator.generate()
}
