//! G1 — RuneScript emitter.
//!
//! A **second renderer** over [`StructuredScript`] (the same IR the TypeScript forward emitter
//! [`super::structured::StructuredScript::render`] consumes and the same IR the reverse path
//! `ts_parse` → `ts_lower::lower_structured_script` round-trips). It restyles the structured
//! surface from TypeScript into RuneScript in the zwyz `CodeFormatter` style — header
//! `name(type $arg)(ret)`, `$<type><idx>` locals, `def_` declarations, `calc(...)`, `~proc`,
//! `%var`, `switch_<type>`, comparison operators (`=`/`!`/`<`), `if`/`while`.
//!
//! **Presentation-only — zero coupling to the byte-exact path.** Nothing here is wired into the
//! recompile gate; it reads `StructuredScript` and produces text. The eventual RuneScript *parser*
//! (G1.3) produces the identical `StructuredScript` `ts_parse` yields, so the existing byte-fidelity
//! gate validates the RuneScript surface for free. See
//! `plans/tooling/cs2-runescript-decompiler.md` (G1).

use super::ast::{BinaryOp, Expression, UnaryOp};
use super::structured::{AssignmentTarget, StructuredScript, StructuredStmt, SwitchCaseStmt};
use std::collections::{HashMap, HashSet};

/// The CS2 local-variable stack domains. A local's domain is encoded in its decoder name
/// (`local_int_0`, `arg_obj_2`, `local_long_1`) and fixes its base RuneScript type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Domain {
    Int,
    Obj,
    Long,
}

impl Domain {
    fn from_token(token: &str) -> Option<Self> {
        match token {
            "int" => Some(Self::Int),
            "obj" | "string" => Some(Self::Obj),
            "long" => Some(Self::Long),
            _ => None,
        }
    }

    /// The base RuneScript type keyword for the domain (refined to a semantic type by G1.5).
    fn base_type(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Obj => "string",
            Self::Long => "long",
        }
    }
}

/// Context the emitter needs beyond the [`StructuredScript`] itself: the canonical command-name
/// registry (to undo the TS underscore-stripping + `UI.` namespacing) and the set of gosub-able
/// script names (to render `~proc` instead of a bare command).
#[derive(Debug, Clone)]
pub struct RuneScriptContext {
    /// `strip_underscores(canonical)` → canonical, for non-UI commands
    /// (`enumgetoutputcount` → `enum_getoutputcount`).
    generic: HashMap<String, String>,
    /// lower-cased, underscore-stripped UI suffix → the `cc_`/`if_` canonical pair
    /// (`setparamstring` → (`cc_setparam_string`, `if_setparam_string`)).
    ui: HashMap<String, UiPair>,
    /// Names that are scripts (gosub targets), rendered `~name`.
    scripts: HashSet<String>,
}

/// The `cc_`/`if_` forms of one UI opcode family (either may be absent for a given suffix).
#[derive(Debug, Clone, Default)]
struct UiPair {
    cc: Option<String>,
    if_: Option<String>,
}

/// The build-948 canonical opcode registry (`name,id[,gate]` per line, `//` comments).
const OPCODES_948: &str = include_str!("../../data/opcodes-948.txt");

impl RuneScriptContext {
    /// Build the context for the corpus build (948) with the given gosub-able script names.
    pub fn new(scripts: HashSet<String>) -> Self {
        Self::from_opcodes(OPCODES_948, scripts)
    }

    fn from_opcodes(opcodes: &str, scripts: HashSet<String>) -> Self {
        let mut generic = HashMap::new();
        let mut ui: HashMap<String, UiPair> = HashMap::new();
        for line in opcodes.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let Some(name) = line.split(',').next() else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if let Some(suffix) = name.strip_prefix("cc_").or_else(|| name.strip_prefix("if_")) {
                let key = strip_underscores(suffix).to_ascii_lowercase();
                let entry = ui.entry(key).or_default();
                if name.starts_with("cc_") {
                    entry.cc.get_or_insert_with(|| name.to_string());
                } else {
                    entry.if_.get_or_insert_with(|| name.to_string());
                }
            }
            // Every command (UI included) is reachable by its underscore-stripped name, because the
            // TS emitter strips underscores from *all* opcode identifiers.
            generic
                .entry(strip_underscores(name))
                .or_insert_with(|| name.to_string());
        }
        Self {
            generic,
            ui,
            scripts,
        }
    }

    /// Recover the canonical RuneScript name for a non-UI command rendered in the TS surface.
    /// The TS emitter strips underscores (`enum_getoutputcount` → `enumgetoutputcount`) and may add
    /// a trailing `_` to reserved words (`enum` → `enum_`), so we match on the underscore-stripped
    /// form. Returns `None` when no opcode matches (then the caller emits the name verbatim).
    fn canonical_command(&self, display: &str) -> Option<&str> {
        self.generic
            .get(&strip_underscores(display))
            .map(String::as_str)
    }

    /// Recover the canonical `cc_`/`if_` name for a `UI.<Method>` call. The method (curated or
    /// `sanitize_camel`) lower-cases to the underscore-stripped opcode suffix, so one key resolves
    /// both styles. With a leading component argument the `if_` form is chosen, else `cc_`.
    fn ui_command(&self, method: &str, has_component_arg: bool) -> Option<String> {
        let pair = self.ui.get(&method.to_ascii_lowercase())?;
        let chosen = if has_component_arg {
            pair.if_.as_ref().or(pair.cc.as_ref())
        } else {
            pair.cc.as_ref().or(pair.if_.as_ref())
        };
        chosen.cloned()
    }

    fn is_script(&self, name: &str) -> bool {
        self.scripts.contains(name)
    }

    /// Whether `name` is a canonical opcode (vs a synthetic/stack-pseudo name like `stackassign_2`).
    /// The RuneScript parser uses this to decide whether to undo the TS underscore-stripping: real
    /// opcodes are un-stripped, synthetics are passed through verbatim.
    #[must_use]
    pub fn is_canonical_command(&self, name: &str) -> bool {
        self.generic.get(&strip_underscores(name)).map(String::as_str) == Some(name)
    }
}

/// Render `script` as RuneScript source (header + body). Presentation-only.
pub fn render_runescript(script: &StructuredScript, ctx: &RuneScriptContext) -> String {
    let renamer = LocalRenamer::build(script);
    // A local that is never assigned gets no inline `def_`; declare it explicitly up front so the
    // RuneScript carries the full local count (the parser rebuilds `locals` from these + inline
    // def_s; `ts_lower` derives the bytecode header counts from that Vec).
    let mut assigned = HashSet::new();
    collect_assigned_locals(&script.body, &mut assigned);
    let imports: HashSet<String> = script
        .imports
        .iter()
        .flat_map(|import| import.named_exports.iter().cloned())
        .collect();
    let mut emitter = Emitter {
        ctx,
        renamer: &renamer,
        declared: renamer.argument_names(),
        imports,
        out: String::new(),
    };
    emitter.write_header(script);
    emitter.write_unassigned_local_decls(script, &assigned);
    emitter.write_block(&script.body, 0);
    emitter.out
}

/// Collect the decoder names of every local that appears as an assignment target (recursively).
fn collect_assigned_locals(body: &[StructuredStmt], out: &mut HashSet<String>) {
    for stmt in body {
        match stmt {
            StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier(name),
                ..
            } => {
                out.insert(name.clone());
            }
            StructuredStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_assigned_locals(then_body, out);
                if let Some(else_body) = else_body {
                    collect_assigned_locals(else_body, out);
                }
            }
            StructuredStmt::While { body } => collect_assigned_locals(body, out),
            StructuredStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    collect_assigned_locals(&case.body, out);
                }
                if let Some(default_body) = default_body {
                    collect_assigned_locals(default_body, out);
                }
            }
            _ => {}
        }
    }
}

/// Maps each decoder local/argument name (`local_int_0`, `arg_obj_1`) to its RuneScript name
/// (`$int2`, `$string0`). Arguments occupy the low per-domain indices; locals follow.
struct LocalRenamer {
    /// decoder name → RuneScript `$name`
    names: HashMap<String, String>,
    /// decoder name → (RuneScript type, RuneScript `$name`), arguments only, in header order
    arguments: Vec<(String, String)>,
    /// array decoder id (`array_<id>`) → RuneScript `$arr<n>` name
    arrays: HashMap<String, String>,
}

impl LocalRenamer {
    fn build(script: &StructuredScript) -> Self {
        let mut arg_counts: HashMap<Domain, usize> = HashMap::new();
        for arg in &script.arguments {
            if let Some((_, domain, _)) = parse_local_name(&arg.name) {
                *arg_counts.entry(domain).or_default() += 1;
            }
        }

        let mut names = HashMap::new();
        let mut arguments = Vec::new();
        for arg in &script.arguments {
            if let Some((_, domain, index)) = parse_local_name(&arg.name) {
                let rs_name = format!("${}{index}", domain.base_type());
                names.insert(arg.name.clone(), rs_name.clone());
                arguments.push((domain.base_type().to_string(), rs_name));
            }
        }
        for local in &script.locals {
            if let Some((_, domain, index)) = parse_local_name(&local.name) {
                let offset = arg_counts.get(&domain).copied().unwrap_or(0);
                names.insert(
                    local.name.clone(),
                    format!("${}{}", domain.base_type(), offset + index),
                );
            }
        }

        // Array decls carry the index in the high bits of their operand (`define_array 65536` =
        // index 1), but array *accesses* use the bare index (`array_1`). Map both decoder spellings
        // — `array_<raw>` (decl/`define_array`) and `array_<index>` (access) — to one `$arr<index>`.
        let mut arrays = HashMap::new();
        for &raw in &script.arrays {
            let index = raw >> 16;
            let rs_name = format!("$arr{index}");
            arrays.insert(format!("array_{index}"), rs_name.clone());
            arrays.insert(format!("array_{raw}"), rs_name);
        }

        Self {
            names,
            arguments,
            arrays,
        }
    }

    fn argument_names(&self) -> HashSet<String> {
        self.arguments.iter().map(|(_, name)| name.clone()).collect()
    }

    fn local(&self, name: &str) -> Option<&str> {
        self.names.get(name).map(String::as_str)
    }

    fn array(&self, name: &str) -> Option<&str> {
        self.arrays.get(name).map(String::as_str)
    }
}

struct Emitter<'a> {
    ctx: &'a RuneScriptContext,
    renamer: &'a LocalRenamer,
    /// RuneScript local names already declared (`def_` only on first assignment); seeded with args.
    declared: HashSet<String>,
    /// This script's gosub callees (the IR's imports). A call renders `~name` iff its callee is
    /// imported here — NOT iff the name is some script globally, so a *command* call to a name that
    /// also happens to be a script (`openurl`, `error`) stays a command, not a gosub.
    imports: HashSet<String>,
    out: String,
}

impl Emitter<'_> {
    fn write_header(&mut self, script: &StructuredScript) {
        let params = self
            .renamer
            .arguments
            .iter()
            .map(|(ty, name)| format!("{ty} {name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let returns = return_types(&script.return_type).join(", ");

        self.out.push_str(&script.function_name);
        if !params.is_empty() || !returns.is_empty() {
            self.out.push('(');
            self.out.push_str(&params);
            self.out.push(')');
        }
        if !returns.is_empty() {
            self.out.push('(');
            self.out.push_str(&returns);
            self.out.push(')');
        }
        self.out.push('\n');
    }

    /// Emit a bare `def_<type> $name;` for each local never assigned in the body, so the parser can
    /// rebuild the full `locals` count even when a slot is only read (or unused).
    fn write_unassigned_local_decls(
        &mut self,
        script: &StructuredScript,
        assigned: &HashSet<String>,
    ) {
        for local in &script.locals {
            if assigned.contains(&local.name) {
                continue;
            }
            let Some(rs) = self.renamer.local(&local.name).map(str::to_string) else {
                continue;
            };
            if self.declared.insert(rs.clone()) {
                let ty = local_type_keyword(&rs).to_string();
                self.line(0, &format!("def_{ty} {rs};"));
            }
        }
    }

    fn write_block(&mut self, stmts: &[StructuredStmt], indent: usize) {
        for stmt in stmts {
            self.write_stmt(stmt, indent);
        }
    }

    fn write_stmt(&mut self, stmt: &StructuredStmt, indent: usize) {
        match stmt {
            StructuredStmt::While { body } => {
                self.line(indent, "while (true) {");
                self.write_block(body, indent + 1);
                self.line(indent, "}");
            }
            StructuredStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let head = format!("if ({}) {{", self.expr(condition, 0));
                self.line(indent, &head);
                self.write_block(then_body, indent + 1);
                if let Some(else_body) = else_body {
                    self.line(indent, "} else {");
                    self.write_block(else_body, indent + 1);
                }
                self.line(indent, "}");
            }
            StructuredStmt::Switch {
                expr,
                cases,
                default_body,
            } => self.write_switch(expr, cases, default_body.as_deref(), indent),
            StructuredStmt::Assignment { target, value } => {
                let rhs = self.expr(value, 0);
                let lhs = self.assignment_target(target);
                self.line(indent, &format!("{lhs} = {rhs};"));
            }
            StructuredStmt::Expr { expr } => {
                let text = self.expr(expr, 0);
                self.line(indent, &format!("{text};"));
            }
            StructuredStmt::Goto { target } => self.line(indent, &format!("goto({target});")),
            StructuredStmt::StackGoto { target, values } => {
                let parts = values
                    .iter()
                    .map(|v| self.expr(v, 0))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.line(indent, &format!("stackpush_then({parts}, goto({target}));"));
            }
            StructuredStmt::Label { target } => self.line(indent, &format!("label({target});")),
            StructuredStmt::Return { value } => match value {
                Some(value) => {
                    let text = self.expr(value, 0);
                    self.line(indent, &format!("return({text});"));
                }
                None => self.line(indent, "return;"),
            },
            StructuredStmt::Comment(text) => self.line(indent, &format!("// {text}")),
            StructuredStmt::Break => self.line(indent, "break;"),
            StructuredStmt::Continue => self.line(indent, "continue;"),
        }
    }

    fn write_switch(
        &mut self,
        expr: &Expression,
        cases: &[SwitchCaseStmt],
        default_body: Option<&[StructuredStmt]>,
        indent: usize,
    ) {
        // The discriminant type drives `switch_<type>`; the base int form covers the common case
        // until G1.5 threads the inferred type through.
        let head = format!("switch_int ({}) {{", self.expr(expr, 0));
        self.line(indent, &head);
        for case in cases {
            self.line(indent + 1, &format!("case {} :", case.value));
            self.write_block(&case.body, indent + 2);
        }
        if let Some(default_body) = default_body {
            self.line(indent + 1, "case default :");
            self.write_block(default_body, indent + 2);
        }
        self.line(indent, "}");
    }

    fn line(&mut self, indent: usize, text: &str) {
        for _ in 0..indent {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn assignment_target(&mut self, target: &AssignmentTarget) -> String {
        match target {
            AssignmentTarget::Identifier(name) => {
                if let Some(rs) = self.renamer.local(name) {
                    // First assignment to a local declares it; arguments are pre-declared.
                    if self.declared.insert(rs.to_string()) {
                        let ty = local_type_keyword(rs);
                        return format!("def_{ty} {rs}");
                    }
                    return rs.to_string();
                }
                reference_name(name)
            }
            AssignmentTarget::ArrayAccess { array, index } => {
                let arr = self
                    .renamer
                    .array(array)
                    .map_or_else(|| array_access_name(array), str::to_string);
                format!("{arr}({})", self.expr(index, 0))
            }
            AssignmentTarget::Opaque(name) => name.clone(),
        }
    }

    /// Render an expression at the given precedence (zwyz `CodeFormatter` model: arithmetic/bitwise
    /// ops wrap in `calc(...)` when the surrounding precedence is below 50).
    fn expr(&self, expr: &Expression, prec: i32) -> String {
        match expr {
            Expression::NumberLiteral(n) => n.value.to_string(),
            // The trailing `L` keeps the long/int distinction across the round-trip (a long lowers
            // to `push_long_constant`, an int to `push_constant_int`).
            Expression::BigIntLiteral(n) => format!("{}L", n.value),
            Expression::StringLiteral(s) => format!("\"{}\"", escape(&s.value)),
            Expression::BooleanLiteral(b) => b.value.to_string(),
            Expression::Identifier(id) => self.identifier(&id.name),
            Expression::PropertyAccess(access) => self.property_access(access),
            Expression::ArrayAccess(access) => {
                let arr = match access.array.as_ref() {
                    Expression::Identifier(id) => self
                        .renamer
                        .array(&id.name)
                        .map_or_else(|| array_access_name(&id.name), str::to_string),
                    other => self.expr(other, 0),
                };
                format!("{arr}({})", self.expr(&access.index, 0))
            }
            Expression::Call(call) => self.call(call),
            Expression::CallbackLiteral(cb) => self.callback(cb),
            Expression::BinaryOperation(bin) => self.binary(bin.op, &bin.left, &bin.right, prec),
            Expression::UnaryOperation(un) => self.unary(un.op, &un.operand),
            Expression::PushOperation(push) => self.expr(&push.value, prec),
            Expression::PopOperation(_) => "pop()".to_string(),
            Expression::GotoExpr(goto) => format!("goto({})", goto.target),
        }
    }

    fn identifier(&self, name: &str) -> String {
        if let Some(rs) = self.renamer.local(name) {
            return rs.to_string();
        }
        if let Some(rs) = self.renamer.array(name) {
            return rs.to_string();
        }
        reference_name(name)
    }

    fn property_access(&self, access: &super::ast::PropertyAccess) -> String {
        // `Enum_17613.KEY_0` and similar resolved constants keep their symbolic form for now (the
        // exact typed-constant rendering is a G1.6 polish item).
        if let Expression::Identifier(obj) = access.object.as_ref() {
            return format!("{}.{}", obj.name, access.property);
        }
        format!("{}.{}", self.expr(&access.object, 0), access.property)
    }

    fn call(&self, call: &super::ast::CallExpr) -> String {
        let Expression::Identifier(callee) = call.callee.as_ref() else {
            // A `UI.<Method>(...)` call: the callee is a property access.
            if let Expression::PropertyAccess(access) = call.callee.as_ref()
                && let Expression::Identifier(obj) = access.object.as_ref()
                && obj.name == "UI"
            {
                return self.ui_call(&access.property, &call.arguments);
            }
            let callee = self.expr(&call.callee, 0);
            return format!("{callee}({})", self.arguments(&call.arguments));
        };

        let name = callee.name.as_str();
        // `pop()` is a stack-shim placeholder, not a RuneScript command. `longconst(x)` stays a
        // normal command call (it lowers to a distinct typed-long-constant opcode, not a bare push),
        // so it round-trips through the generic command path below.
        if name == "pop" && call.arguments.is_empty() {
            return "pop()".to_string();
        }
        if let Some(rest) = name.strip_prefix("define_array_") {
            return self.define_array(rest, &call.arguments);
        }
        // A gosub renders `~name`. The IR's imports are the authoritative gosub set; fall back to the
        // global script catalog only for a name that is NOT also a command opcode (so a *command*
        // call to a name that happens to be a script — `openurl`, `error` — stays a command).
        if self.imports.contains(name)
            || (self.ctx.is_script(name) && !self.ctx.is_canonical_command(name))
        {
            if call.arguments.is_empty() {
                return format!("~{name}");
            }
            return format!("~{name}({})", self.arguments(&call.arguments));
        }
        let canonical = self
            .ctx
            .canonical_command(name)
            .map_or_else(|| name.to_string(), str::to_string);
        if call.arguments.is_empty() {
            format!("{canonical}()")
        } else {
            format!("{canonical}({})", self.arguments(&call.arguments))
        }
    }

    fn ui_call(&self, method: &str, arguments: &[Expression]) -> String {
        // The decompiler encodes cc_/if_ in the UI method's first-letter casing (mirroring
        // `ts_lower::resolve_ui_command`): capital-first → the `if_` form, lowercase-first → `cc_`.
        // Picking by casing makes the emitter render the exact opcode for the common generic methods,
        // so the round-trip is byte-exact (G1.5). The intricate tail — getters/hooks and the
        // `WithMode` mode argument (which the parser can't distinguish from a component) — is a
        // follow-on; those re-derive cc_/if_ by arg-count in `ts_lower` and currently round-trip
        // imperfectly but loudly (no silent corruption).
        // A few explicit curated methods always (or by arg-count) take the `if_` form, which the
        // first-letter casing rule gets wrong (`getText` is lowercase-first but is always `if_gettext`).
        // Targeted exact-match only — the broad getter/`WithMode` handling regressed (G1.5).
        let n = arguments.len();
        let explicit = match method {
            "getText" => Some("if_gettext"),
            "findInterface" => Some("if_find"),
            "sendToFront" => Some(if n == 1 { "if_sendtofront" } else { "cc_sendtofront" }),
            "sendToBack" => Some(if n == 1 { "if_sendtoback" } else { "cc_sendtoback" }),
            _ => None,
        };
        let canonical = if let Some(cmd) = explicit {
            cmd.to_string()
        } else {
            let if_form = method.starts_with(|c: char| c.is_ascii_uppercase());
            self.ctx.ui_command(method, if_form).unwrap_or_else(|| {
                // The fallback fires mainly for `WithMode` methods (the table has no `withmode`
                // keys). It must respect the cc_/if_ casing convention too: a capital-first method
                // is the `if_` opcode (`SetsizeWithMode` → `if_setsize`, NOT `cc_setsize`).
                // Hardcoding `cc_` flipped if_-form WithMode setters — invisible on 948 (its corpus
                // never exercised one) but caught on 910 (script5744).
                let prefix = if if_form { "if" } else { "cc" };
                format!("{prefix}_{}", to_snake(method))
            })
        };
        if arguments.is_empty() {
            format!("{canonical}()")
        } else {
            format!("{canonical}({})", self.arguments(arguments))
        }
    }

    fn define_array(&self, raw: &str, arguments: &[Expression]) -> String {
        let arr = self
            .renamer
            .array(&format!("array_{raw}"))
            .map_or_else(|| format!("$arr_{raw}"), str::to_string);
        // The access uses the bare index (`raw >> 16`), but the define_array operand is the full
        // `raw`; encode the low element-type bits so the parser recovers the exact operand.
        let type_low = raw.parse::<u32>().map_or(0, |r| r & 0xffff);
        let arr = if type_low != 0 {
            format!("{arr}_{type_low}")
        } else {
            arr
        };
        let size = arguments
            .first()
            .map_or_else(String::new, |a| self.expr(a, 0));
        format!("def_int {arr}({size})")
    }

    fn arguments(&self, arguments: &[Expression]) -> String {
        arguments
            .iter()
            .map(|a| self.expr(a, 0))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn callback(&self, cb: &super::ast::CallbackLiteral) -> String {
        // A UI hook. Rendered in the round-trippable `callback("script", [args], [watchers],
        // "descriptor")` form (mirroring the TS surface); the prettier zwyz `"script(args){…}"` hook
        // syntax is a G1.6 polish item. Args are renamed like any expression; watchers stay raw.
        let args = cb
            .arguments
            .iter()
            .map(|a| self.expr(a, 0))
            .collect::<Vec<_>>()
            .join(", ");
        let watchers = cb.watchers.join(", ");
        format!(
            "callback(\"{}\", [{args}], [{watchers}], \"{}\")",
            escape(&cb.script),
            escape(&cb.raw_descriptor)
        )
    }

    fn binary(&self, op: BinaryOp, left: &Expression, right: &Expression, prec: i32) -> String {
        let p = op_prec(op);
        let inner = format!(
            "{}{}{}",
            self.expr(left, p),
            binary_operator(op),
            self.expr(right, p + 1)
        );
        if is_calc_op(op) {
            if prec < CALC_THRESHOLD {
                format!("calc({inner})")
            } else if prec > p {
                format!("({inner})")
            } else {
                inner
            }
        } else if prec > p {
            format!("({inner})")
        } else {
            inner
        }
    }

    fn unary(&self, op: UnaryOp, operand: &Expression) -> String {
        let symbol = match op {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
        };
        format!("{symbol}{}", self.expr(operand, HIGH_PREC))
    }
}

/// Below this surrounding precedence an arithmetic/bitwise op must wrap in `calc(...)`.
const CALC_THRESHOLD: i32 = 50;
/// Precedence for a unary operand — high enough never to be parenthesised.
const HIGH_PREC: i32 = 100;

/// zwyz `CodeFormatter` operator precedences.
fn op_prec(op: BinaryOp) -> i32 {
    match op {
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            40
        }
        BinaryOp::Or => 50,
        BinaryOp::And => 60,
        BinaryOp::Add | BinaryOp::Sub => 70,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 80,
    }
}

/// Whether an operator is arithmetic/bitwise (`calc(...)`-wrapped) rather than a comparison.
fn is_calc_op(op: BinaryOp) -> bool {
    !matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
    )
}

/// The RuneScript spelling of a binary operator (note `=`/`!` for equality, RuneScript style).
fn binary_operator(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => " + ",
        BinaryOp::Sub => " - ",
        BinaryOp::Mul => " * ",
        BinaryOp::Div => " / ",
        BinaryOp::Mod => " % ",
        BinaryOp::Eq => " = ",
        BinaryOp::Ne => " ! ",
        BinaryOp::Lt => " < ",
        BinaryOp::Le => " <= ",
        BinaryOp::Gt => " > ",
        BinaryOp::Ge => " >= ",
        BinaryOp::And => " & ",
        BinaryOp::Or => " | ",
    }
}

/// Split a decoder local name (`local_int_0`, `arg_obj_2`) into (is_argument, domain, index).
fn parse_local_name(name: &str) -> Option<(bool, Domain, usize)> {
    let mut parts = name.splitn(3, '_');
    let kind = parts.next()?;
    let domain = Domain::from_token(parts.next()?)?;
    let index = parts.next()?.parse::<usize>().ok()?;
    let is_argument = match kind {
        "arg" => true,
        "local" => false,
        _ => return None,
    };
    Some((is_argument, domain, index))
}

/// The RuneScript base type keyword from a `$<type><index>` local name (`$int2` → `int`).
fn local_type_keyword(rs_name: &str) -> &str {
    let trimmed = rs_name.trim_start_matches('$');
    let end = trimmed
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    &trimmed[..end]
}

/// A bare reference (var/varbit/unresolved identifier). Decoder var identifiers carry a `var…`
/// prefix → render `%name`; everything else is passed through verbatim.
fn reference_name(name: &str) -> String {
    if name.starts_with("var") {
        format!("%{name}")
    } else {
        name.to_string()
    }
}

/// Render an `array_<id>` access name as `$arr<index>` even when the array isn't in the renamer
/// (accessed-but-not-defined arrays, e.g. one passed in by the caller). Access ids are the bare
/// index; a stray decl raw (`>= 1<<16`) folds to its index. The parser inverts `$arr<index>` →
/// `array_<index>`, so this round-trips.
fn array_access_name(name: &str) -> String {
    if let Some(id) = name.strip_prefix("array_")
        && let Ok(n) = id.parse::<u32>()
    {
        let index = if n >= (1 << 16) { n >> 16 } else { n };
        return format!("$arr{index}");
    }
    reference_name(name)
}

/// Parse the TS `return_type` string into RuneScript return type keywords.
/// `void` → none; `number`/`string`/`bigint`/`boolean` → one; `[a, b]` tuple → many.
fn return_types(return_type: &str) -> Vec<String> {
    let trimmed = return_type.trim();
    if trimmed.is_empty() || trimmed == "void" {
        return Vec::new();
    }
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ts_type_to_runescript)
        .collect()
}

fn ts_type_to_runescript(ts: &str) -> String {
    match ts {
        "number" => "int",
        "bigint" => "long",
        "string" => "string",
        "boolean" => "boolean",
        other => other,
    }
    .to_string()
}

/// Strip every `_` from a name (the TS emitter's irreversible transform; we index canonical names
/// by this form to recover them).
fn strip_underscores(name: &str) -> String {
    name.chars().filter(|&c| c != '_').collect()
}

/// `camelCase` → `snake_case` (fallback when a `UI.<Method>` has no registry match).
fn to_snake(method: &str) -> String {
    let mut out = String::new();
    for (i, c) in method.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::{RuneScriptContext, render_runescript};
    use crate::transpile::ast::{
        ArgumentVariable, BinaryOp, BinaryOperation, CallExpr, Expression, Identifier,
        LocalVariable, NumberLiteral, PropertyAccess, ScriptId, StringLiteral, TypeAnnotation,
    };
    use crate::transpile::structured::{AssignmentTarget, StructuredScript, StructuredStmt};
    use std::collections::HashSet;

    fn ctx() -> RuneScriptContext {
        let mut scripts = HashSet::new();
        scripts.insert("rs3_scrollbar_vertical_7791".to_string());
        RuneScriptContext::new(scripts)
    }

    fn ident(name: &str) -> Expression {
        Expression::Identifier(Identifier {
            name: name.to_string(),
        })
    }

    fn num(value: i32) -> Expression {
        Expression::NumberLiteral(NumberLiteral { value })
    }

    fn call(name: &str, arguments: Vec<Expression>) -> Expression {
        Expression::Call(CallExpr {
            callee: Box::new(ident(name)),
            arguments,
        })
    }

    fn script(
        arguments: Vec<ArgumentVariable>,
        locals: Vec<LocalVariable>,
        return_type: &str,
        body: Vec<StructuredStmt>,
    ) -> StructuredScript {
        StructuredScript {
            script_id: ScriptId(0),
            raw_name: None,
            header_comments: Vec::new(),
            imports: Vec::new(),
            function_name: "demo".to_string(),
            arguments,
            locals,
            arrays: Vec::new(),
            return_type: return_type.to_string(),
            body,
        }
    }

    fn arg(name: &str) -> ArgumentVariable {
        ArgumentVariable {
            index: 0,
            name: name.to_string(),
            type_annotation: TypeAnnotation::Number,
        }
    }

    fn local(name: &str) -> LocalVariable {
        LocalVariable {
            index: 0,
            name: name.to_string(),
            type_annotation: TypeAnnotation::Number,
        }
    }

    #[test]
    fn header_renders_typed_params_and_return() {
        let s = script(
            vec![arg("arg_int_0"), arg("arg_int_1")],
            Vec::new(),
            "number",
            vec![StructuredStmt::Return { value: Some(num(1)) }],
        );
        let out = render_runescript(&s, &ctx());
        assert!(
            out.starts_with("demo(int $int0, int $int1)(int)\n"),
            "got: {out}"
        );
        assert!(out.contains("return(1);"));
    }

    #[test]
    fn locals_follow_arguments_in_per_domain_index() {
        // one int arg → its local int slot 0 becomes $int1 (after the arg at $int0).
        let s = script(
            vec![arg("arg_int_0")],
            vec![local("local_int_0")],
            "void",
            vec![StructuredStmt::Assignment {
                target: AssignmentTarget::Identifier("local_int_0".to_string()),
                value: num(7),
            }],
        );
        let out = render_runescript(&s, &ctx());
        // first assignment declares the local with def_; arg is pre-declared.
        assert!(out.contains("def_int $int1 = 7;"), "got: {out}");
    }

    #[test]
    fn arithmetic_value_wraps_in_calc_and_comparison_uses_runescript_ops() {
        let cond = Expression::BinaryOperation(BinaryOperation {
            op: BinaryOp::Lt,
            left: Box::new(ident("local_int_0")),
            right: Box::new(Expression::BinaryOperation(BinaryOperation {
                op: BinaryOp::Add,
                left: Box::new(ident("local_int_1")),
                right: Box::new(num(1)),
            })),
        });
        let s = script(
            Vec::new(),
            vec![local("local_int_0"), local("local_int_1")],
            "void",
            vec![StructuredStmt::If {
                condition: cond,
                then_body: vec![StructuredStmt::Break],
                else_body: None,
            }],
        );
        let out = render_runescript(&s, &ctx());
        // comparison renders ` < `; the arithmetic operand of the comparison is calc-wrapped.
        assert!(out.contains("if ($int0 < calc($int1 + 1)) {"), "got: {out}");
    }

    #[test]
    fn command_name_recovers_underscores_and_gosub_renders_tilde() {
        let s = script(
            Vec::new(),
            vec![local("local_int_0")],
            "void",
            vec![
                // enumgetoutputcount → enum_getoutputcount (registry un-strip)
                StructuredStmt::Assignment {
                    target: AssignmentTarget::Identifier("local_int_0".to_string()),
                    value: call("enumgetoutputcount", vec![num(14058)]),
                },
                // a known script name → ~gosub
                StructuredStmt::Expr {
                    expr: call("rs3_scrollbar_vertical_7791", vec![ident("local_int_0")]),
                },
            ],
        );
        let out = render_runescript(&s, &ctx());
        assert!(out.contains("enum_getoutputcount(14058)"), "got: {out}");
        assert!(
            out.contains("~rs3_scrollbar_vertical_7791($int0)"),
            "got: {out}"
        );
    }

    #[test]
    fn ui_call_recovers_canonical_cc_name() {
        let s = script(
            Vec::new(),
            vec![local("local_int_0")],
            "void",
            vec![StructuredStmt::Expr {
                expr: Expression::Call(CallExpr {
                    callee: Box::new(Expression::PropertyAccess(PropertyAccess {
                        object: Box::new(ident("UI")),
                        property: "deleteAll".to_string(),
                    })),
                    arguments: vec![ident("local_int_0")],
                }),
            }],
        );
        let out = render_runescript(&s, &ctx());
        assert!(out.contains("cc_deleteall($int0);"), "got: {out}");
    }

    #[test]
    fn string_literal_and_var_reference() {
        let s = script(
            Vec::new(),
            Vec::new(),
            "void",
            vec![StructuredStmt::If {
                condition: Expression::BinaryOperation(BinaryOperation {
                    op: BinaryOp::Eq,
                    left: Box::new(ident("varplayerbit_3043")),
                    right: Box::new(num(1)),
                }),
                then_body: vec![StructuredStmt::Expr {
                    expr: Expression::StringLiteral(StringLiteral {
                        value: "hi".to_string(),
                    }),
                }],
                else_body: None,
            }],
        );
        let out = render_runescript(&s, &ctx());
        assert!(out.contains("if (%varplayerbit_3043 = 1) {"), "got: {out}");
        assert!(out.contains("\"hi\";"), "got: {out}");
    }
}
