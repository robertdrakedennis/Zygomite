use super::ast::ScriptId;
use crate::vars::VarDomain;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Variable { domain: VarDomain, id: u16 },
    VarBit { id: u16 },
    Enum { id: u32 },
    Param { id: u32 },
    Script { id: ScriptId },
    Local { index: usize, type_: LocalType },
    Argument { index: usize, type_: LocalType },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LocalType {
    Int,
    Long,
    Object,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub resolved_name: Option<String>,
}

impl Symbol {
    pub fn new(name: impl Into<String>, kind: SymbolKind) -> Self {
        Self {
            name: name.into(),
            kind,
            resolved_name: None,
        }
    }

    pub fn with_resolved(mut self, resolved_name: impl Into<String>) -> Self {
        self.resolved_name = Some(resolved_name.into());
        self
    }
}

#[derive(Debug, Default, Clone)]
pub struct Scope {
    symbols: HashMap<String, Symbol>,
}

impl Scope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn define(&mut self, symbol: Symbol) {
        self.symbols.insert(symbol.name.clone(), symbol);
    }

    pub fn get(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.symbols.contains_key(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.values()
    }
}

#[derive(Debug, Default)]
pub struct Scopes {
    scopes: Vec<Scope>,
}

impl Scopes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn current_scope(&self) -> Option<&Scope> {
        self.scopes.last()
    }

    pub fn current_scope_mut(&mut self) -> Option<&mut Scope> {
        self.scopes.last_mut()
    }

    pub fn define(&mut self, symbol: Symbol) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.define(symbol);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.get(name) {
                return Some(sym);
            }
        }
        None
    }

    pub fn lookup_or(&self, name: &str, or: impl FnOnce() -> Symbol) -> Symbol {
        self.lookup(name).cloned().unwrap_or_else(or)
    }
}

#[derive(Debug, Default, Clone)]
pub struct SymbolTable {
    pub global_scope: Scope,
    pub var_map: HashMap<(VarDomain, u16), String>,
    pub varbit_map: HashMap<u16, String>,
    pub enum_map: HashMap<u32, String>,
    pub param_map: HashMap<u32, String>,
    pub script_names: HashMap<ScriptId, String>,
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
        for (&id, varbit) in varbits {
            self.varbit_map
                .insert(id as u16, varbit.varbit_name.clone());
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

    pub fn with_script_names(mut self, names: HashMap<ScriptId, String>) -> Self {
        self.script_names = names;
        self
    }

    pub fn var_name(&self, domain: VarDomain, id: u16) -> Option<&String> {
        self.var_map.get(&(domain, id))
    }

    pub fn varbit_name(&self, id: u16) -> Option<&String> {
        self.varbit_map.get(&id)
    }

    pub fn script_name(&self, id: ScriptId) -> Option<&String> {
        self.script_names.get(&id)
    }

    pub fn resolve_var_ref(&self, domain: VarDomain, id: u16) -> String {
        self.var_name(domain, id)
            .cloned()
            .unwrap_or_else(|| format!("VARS.get({} * 1000000 + {id})!", u64::from(domain)))
    }

    pub fn resolve_varbit_ref(&self, id: u16) -> String {
        self.varbit_name(id)
            .cloned()
            .unwrap_or_else(|| format!("VARBITS.get({id})!"))
    }
}
