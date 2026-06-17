use super::ast::ScriptId;
use crate::vars::VarDomain;
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct SymbolTable {
    pub var_map: HashMap<(VarDomain, u16), String>,
    pub varbit_map: HashMap<u16, String>,
    pub enum_map: HashMap<u32, String>,
    pub param_map: HashMap<u32, String>,
    pub script_names: HashMap<ScriptId, String>,
    /// Maps interface component IDs to their RS3 names (e.g. 5 → "`chat_box`").
    pub component_names: HashMap<u32, String>,
    /// Maps enum key values to qualified names (e.g. 0 → "`Enum_1234.ATTACK`").
    pub enum_value_names: HashMap<i32, String>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_vars(
        mut self,
        varps: &HashMap<VarDomain, HashMap<u32, crate::vars::VarEntry>>,
    ) -> Self {
        for (domain, vars) in varps {
            for (&id, var) in vars {
                self.var_map
                    .insert((*domain, id as u16), var.var_name.clone());
            }
        }
        self
    }

    pub fn with_varbits(mut self, varbits: &HashMap<u32, crate::vars::VarBitEntry>) -> Self {
        let mut name_counts = HashMap::<String, usize>::new();
        for varbit in varbits.values() {
            *name_counts.entry(varbit.varbit_name.clone()).or_default() += 1;
        }
        for (&id, varbit) in varbits {
            if name_counts.get(&varbit.varbit_name) == Some(&1) {
                self.varbit_map
                    .insert(id as u16, varbit.varbit_name.clone());
            }
        }
        self
    }

    pub fn with_enums(mut self, enums: &HashMap<u32, crate::config::EnumEntry>) -> Self {
        for (&id, entry) in enums {
            self.enum_map.insert(id, format!("enum_{}", entry.id));
        }
        self
    }

    pub fn with_params(mut self, params: &HashMap<u32, crate::config::ParamEntry>) -> Self {
        for (&id, param) in params {
            self.param_map.insert(id, format!("param_{}", param.id));
        }
        self
    }

    pub fn var_name(&self, domain: VarDomain, id: u16) -> Option<&String> {
        self.var_map.get(&(domain, id))
    }

    pub fn varbit_name(&self, id: u16) -> Option<&String> {
        self.varbit_map.get(&id)
    }
}
