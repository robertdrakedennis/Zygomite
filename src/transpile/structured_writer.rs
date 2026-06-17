use super::ast::{Declaration, ImportStatement, TypeAnnotation};
use super::cfg::{build_cfg_for_build, detect_return_type_with_signatures, emit_linear_structured};
use super::structured::StructuredScript;
use super::structurer::StructureOptions;
use super::{ScriptCatalog, ScriptId, ScriptSignature, resolve_script_signature};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub struct StructuredWriter<'a> {
    var_names: &'a HashMap<(crate::vars::VarDomain, u16), String>,
    component_names: &'a HashMap<u32, String>,
    enum_value_names: &'a HashMap<i32, String>,
    script_catalog: &'a ScriptCatalog,
    script_signatures: &'a HashMap<ScriptId, ScriptSignature>,
    build: u32,
}

pub struct BuiltStructuredScript {
    pub script: StructuredScript,
    pub control_flow_fallback_reason: Option<String>,
}

impl<'a> StructuredWriter<'a> {
    pub fn new(
        var_names: &'a HashMap<(crate::vars::VarDomain, u16), String>,
        component_names: &'a HashMap<u32, String>,
        enum_value_names: &'a HashMap<i32, String>,
        script_catalog: &'a ScriptCatalog,
        script_signatures: &'a HashMap<ScriptId, ScriptSignature>,
        build: u32,
    ) -> Self {
        Self {
            var_names,
            component_names,
            enum_value_names,
            script_catalog,
            script_signatures,
            build,
        }
    }

    pub fn build_script(&self, decl: &Declaration) -> StructuredScript {
        self.build_script_with_report(decl).script
    }

    pub fn build_script_with_report(&self, decl: &Declaration) -> BuiltStructuredScript {
        self.build_script_with_options(decl, StructureOptions::AGGRESSIVE)
    }

    pub(crate) fn build_script_with_options(
        &self,
        decl: &Declaration,
        options: StructureOptions,
    ) -> BuiltStructuredScript {
        let blocks = build_cfg_for_build(
            &decl.instructions,
            self.var_names,
            self.component_names,
            self.enum_value_names,
            self.script_catalog,
            self.script_signatures,
            self.build,
        );
        let structured = super::structurer::structure_with_options(&blocks, options);
        BuiltStructuredScript {
            script: self.build_script_with_body(decl, structured.statements),
            control_flow_fallback_reason: structured.fallback_reason,
        }
    }

    pub fn build_linear_script(&self, decl: &Declaration) -> StructuredScript {
        let blocks = build_cfg_for_build(
            &decl.instructions,
            self.var_names,
            self.component_names,
            self.enum_value_names,
            self.script_catalog,
            self.script_signatures,
            self.build,
        );
        let structured = emit_linear_structured(&blocks);
        self.build_script_with_body(decl, structured)
    }

    fn build_script_with_body(
        &self,
        decl: &Declaration,
        structured: Vec<super::structured::StructuredStmt>,
    ) -> StructuredScript {
        let return_type =
            resolve_script_signature(self.script_catalog, self.script_signatures, decl.script_id)
                .and_then(|signature| {
                    (signature.return_type != "unknown").then_some(signature.return_type.as_str())
                })
                .unwrap_or_else(|| {
                    detect_return_type_with_signatures(
                        &structured,
                        self.script_catalog,
                        self.script_signatures,
                    )
                })
                .to_string();
        let function_name = self
            .script_catalog
            .export_name(decl.script_id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("script{}", decl.script_id));

        StructuredScript {
            script_id: decl.script_id,
            raw_name: decl.name.clone(),
            header_comments: build_header_comments(decl, self.script_catalog),
            imports: self.collect_imports(decl),
            function_name,
            arguments: decl.arguments.clone(),
            locals: decl.locals.clone(),
            arrays: collect_array_ids(&decl.instructions),
            return_type,
            body: structured,
        }
    }

    fn collect_imports(&self, decl: &Declaration) -> Vec<ImportStatement> {
        let mut index_imports = BTreeSet::new();
        let mut enum_imports = BTreeSet::new();
        let mut module_imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for instruction in &decl.instructions {
            match instruction.command.as_str() {
                "push_var"
                | "pop_var"
                | "push_varc_int"
                | "pop_varc_int"
                | "push_varc_string"
                | "pop_varc_string"
                | "push_varclan"
                | "push_varclan_long"
                | "push_varclan_string"
                | "push_varclansetting"
                | "push_varclansetting_long"
                | "push_varclansetting_string" => {
                    if let super::ast::OperandNode::VarRef(var_ref) = &instruction.operand
                        && var_ref.name.is_none()
                    {
                        index_imports.insert("VARS");
                    }
                }
                "push_varbit" | "pop_varbit" | "push_varclanbit" | "push_varclansettingbit" => {
                    if let super::ast::OperandNode::VarBitRef(varbit_ref) = &instruction.operand
                        && varbit_ref.name.is_none()
                    {
                        index_imports.insert("VARBITS");
                    }
                }
                "cc_create" => {
                    if let super::ast::OperandNode::Int(component_id) = instruction.operand
                        && self.component_names.contains_key(&(component_id as u32))
                    {
                        index_imports.insert("ComponentId");
                    }
                }
                "push_constant_string" => {
                    if let super::ast::OperandNode::Int(value) = instruction.operand
                        && let Some(qualified) = self.enum_value_names.get(&value)
                        && let Some((object, _)) = qualified.split_once('.')
                    {
                        enum_imports.insert(object.to_string());
                    }
                }
                "gosub_with_params" => {
                    if let super::ast::OperandNode::Script(group_id) = instruction.operand
                        && let Some(target) = self.script_catalog.resolve_call_target(group_id)
                        && target.packed_id != decl.script_id
                    {
                        module_imports
                            .entry(format!("./{}", target.module_name))
                            .or_default()
                            .insert(target.export_name.clone());
                    }
                }
                _ => {}
            }
        }

        let mut imports = Vec::new();
        if !index_imports.is_empty() {
            imports.push(ImportStatement {
                module: "./index".to_string(),
                named_exports: index_imports.into_iter().map(str::to_string).collect(),
                is_type_only: false,
            });
        }
        if !enum_imports.is_empty() {
            imports.push(ImportStatement {
                module: "./enums".to_string(),
                named_exports: enum_imports.into_iter().collect(),
                is_type_only: false,
            });
        }
        for (module, names) in module_imports {
            imports.push(ImportStatement {
                module,
                named_exports: names.into_iter().collect(),
                is_type_only: false,
            });
        }
        imports
    }
}

fn build_header_comments(decl: &Declaration, script_catalog: &ScriptCatalog) -> Vec<String> {
    let mut comments = vec!["Auto-generated CS2 to TypeScript".to_string()];
    if let Some(name) = &decl.name {
        comments.push(format!("Script name: {name}"));
    }
    if let Some(metadata) = script_catalog.get(decl.script_id) {
        comments.push(format!(
            "Meta: packed={} group={} file={} kind={} short_name={}",
            metadata.packed_id,
            metadata.group_id,
            metadata.file_id,
            metadata.kind.as_label(),
            metadata.short_name
        ));
    }
    comments.push(format!(
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
    ));
    comments
}

fn collect_array_ids(instructions: &[super::ast::InstructionNode]) -> Vec<u32> {
    use super::ast::OperandNode;
    let mut ids = Vec::new();
    for instr in instructions {
        if instr.command == "define_array"
            && let OperandNode::Array(id) = instr.operand
        {
            ids.push(id as u32);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}
