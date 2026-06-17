//! G3.2b — constraint generation: a typed three-stack simulation over a decoded
//! script that feeds the [`super::type_infer`] engine.
//!
//! CS2 runs on three separate operand stacks (int / string / long). This module
//! replays a script's instructions over a model of those stacks where each slot is a
//! [`Node`] type-variable, emitting [`super::type_infer`] constraints from:
//! - command argument / result types (the typed `data/commands/*.txt` signatures —
//!   the data `expr_recovery::parse_command_signature_effect` parses but discards),
//! - local / var / varbit loads and stores,
//! - `gosub_with_params` argument → callee-parameter binding,
//! - branch comparisons.
//!
//! Commands without a typed signature fall back to the `data/stack-effects.txt`
//! pop/push *counts* (untyped, but keeps the stacks balanced). An unmodellable
//! instruction (unknown command, or a stack underflow) makes the whole script bail —
//! the caller then leaves that script's locals un-inferred rather than guess.
//!
//! Presentation-only: nothing here affects encoded bytes.

use super::type_infer::{LocalDomain, Node, TypeInfer};
use super::types::{BaseVarType, Type, lattice};
use crate::script::{CompiledScript, Operand};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Inferred semantic types for one script's locals, keyed by `(storage class, index)`.
pub type LocalTypes = HashMap<(LocalDomain, u32), Type>;
/// Inferred local types for a whole program, keyed by script id.
pub type ProgramTypes = HashMap<i32, LocalTypes>;
/// Conflict-causing type pairs and how often each first collapsed to `conflict`.
pub type ConflictHistogram = Vec<((Type, Type), usize)>;

/// A command's argument and result types (left-to-right), resolved against the
/// lattice. Empty `args`/`results` is a valid void/no-arg command.
#[derive(Debug, Clone)]
pub struct CommandSig {
    pub args: Vec<Type>,
    pub results: Vec<Type>,
}

#[derive(Debug, Clone, Copy)]
struct RawCounts {
    int_pops: u32,
    obj_pops: u32,
    long_pops: u32,
    int_pushes: u32,
    obj_pushes: u32,
    long_pushes: u32,
}

/// The argument/return arities of a callee, needed to balance the stacks across a
/// `gosub_with_params`. The real pipeline supplies these from its computed
/// signatures; tests supply them inline.
#[derive(Debug, Clone, Copy)]
pub struct CalleeSig {
    pub arg_int: u16,
    pub arg_obj: u16,
    pub arg_long: u16,
    pub ret_int: u16,
    pub ret_obj: u16,
    pub ret_long: u16,
}

/// Typed command signatures + untyped stack-effect counts, resolved for one build.
pub struct SignatureTable {
    typed: HashMap<String, CommandSig>,
    counts: HashMap<String, RawCounts>,
}

/// The command-signature files embedded for the typed table (a high-coverage subset;
/// anything missing falls back to `stack-effects.txt` counts).
const COMMAND_FILES: &[&str] = &[
    include_str!("../../data/commands/interface_components.txt"),
    include_str!("../../data/commands/interface_core.txt"),
    include_str!("../../data/commands/interface_misc.txt"),
    include_str!("../../data/commands/if_anim.txt"),
    include_str!("../../data/commands/achievement.txt"),
    include_str!("../../data/commands/camera.txt"),
    include_str!("../../data/commands/config_misc.txt"),
    include_str!("../../data/commands/config_quests.txt"),
    include_str!("../../data/commands/config_enums.txt"),
    include_str!("../../data/commands/config_db_table.txt"),
    include_str!("../../data/commands/config_objects.txt"),
    include_str!("../../data/commands/core.txt"),
    include_str!("../../data/commands/detail_options.txt"),
    include_str!("../../data/commands/entities.txt"),
    include_str!("../../data/commands/file_system.txt"),
    include_str!("../../data/commands/input.txt"),
    include_str!("../../data/commands/inventories.txt"),
    include_str!("../../data/commands/login.txt"),
    include_str!("../../data/commands/maths.txt"),
    include_str!("../../data/commands/mini_menu_ops.txt"),
    include_str!("../../data/commands/misc_ops.txt"),
    include_str!("../../data/commands/npc.txt"),
    include_str!("../../data/commands/stats.txt"),
    include_str!("../../data/commands/strings.txt"),
    include_str!("../../data/commands/store.txt"),
    include_str!("../../data/commands/streaming.txt"),
    include_str!("../../data/commands/wikisync.txt"),
];

const STACK_EFFECTS: &str = include_str!("../../data/stack-effects.txt");

impl SignatureTable {
    /// The embedded table for a build (cached for the two builds we target).
    pub fn embedded(build: u32) -> &'static Self {
        static B910: OnceLock<SignatureTable> = OnceLock::new();
        static B948: OnceLock<SignatureTable> = OnceLock::new();
        static OTHER: OnceLock<SignatureTable> = OnceLock::new();
        match build {
            910 => B910.get_or_init(|| Self::from_sources(COMMAND_FILES, STACK_EFFECTS, 910)),
            948 => B948.get_or_init(|| Self::from_sources(COMMAND_FILES, STACK_EFFECTS, 948)),
            other => OTHER.get_or_init(|| Self::from_sources(COMMAND_FILES, STACK_EFFECTS, other)),
        }
    }

    /// Build a table from raw signature/stack-effect text (the embedded path passes
    /// `include_str!` data; tests pass small inline corpora).
    pub fn from_sources(command_files: &[&str], stack_effects: &str, build: u32) -> Self {
        let lat = lattice();
        let mut typed: HashMap<String, CommandSig> = HashMap::new();
        // track the winning build gate per command so a higher applicable gate wins
        let mut gate: HashMap<String, u32> = HashMap::new();
        for file in command_files {
            for line in file.lines() {
                let Some((name, sig, min_build)) = parse_command_line(line, lat) else {
                    continue;
                };
                if min_build > build {
                    continue;
                }
                if gate.get(name).copied().unwrap_or(0) <= min_build || !typed.contains_key(name) {
                    gate.insert(name.to_string(), min_build);
                    typed.insert(name.to_string(), sig);
                }
            }
        }

        let mut counts: HashMap<String, RawCounts> = HashMap::new();
        for line in stack_effects.lines() {
            if let Some((name, c)) = parse_stack_effect_line(line) {
                counts.insert(name.to_string(), c);
            }
        }
        Self { typed, counts }
    }
}

/// Parse `[command,NAME](ARGS)(RETS) MINBUILD`. Returns `(name, sig, min_build)`.
/// `MINBUILD` and the return paren are optional; `hook`/`todo signature` lines skip.
fn parse_command_line<'a>(
    line: &'a str,
    lat: &super::types::TypeLattice,
) -> Option<(&'a str, CommandSig, u32)> {
    let line = line.trim();
    if line.contains("todo signature") || line.contains("hook ") || line.contains("(hook") {
        return None;
    }
    let rest = line.strip_prefix("[command,")?;
    let (name, rest) = rest.split_once(']')?;

    let args_paren = first_paren(rest);
    let args = args_paren
        .as_ref()
        .map(|p| parse_types(p.contents, lat))
        .unwrap_or_default();

    let after_args = args_paren.as_ref().map_or(rest, |p| &rest[p.end..]);
    let rets_paren = if after_args.trim_start().starts_with('(') {
        first_paren(after_args)
    } else {
        None
    };
    let results = rets_paren
        .as_ref()
        .map(|p| parse_types(p.contents, lat))
        .unwrap_or_default();

    let tail = rets_paren
        .as_ref()
        .map_or(after_args, |p| &after_args[p.end..]);
    let min_build = tail.split_whitespace().next().and_then(|t| t.parse().ok());

    Some((name, CommandSig { args, results }, min_build.unwrap_or(0)))
}

struct Paren<'a> {
    contents: &'a str,
    end: usize,
}

fn first_paren(value: &str) -> Option<Paren<'_>> {
    let start = value.find('(')?;
    let rel_end = value[start + 1..].find(')')?;
    let end = start + 1 + rel_end;
    Some(Paren {
        contents: &value[start + 1..end],
        end: end + 1,
    })
}

fn parse_types(types: &str, lat: &super::types::TypeLattice) -> Vec<Type> {
    types
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(|field| {
            let name = field.split_whitespace().next().unwrap_or(field);
            lat.by_name(name)
        })
        .collect()
}

fn parse_stack_effect_line(line: &str) -> Option<(&str, RawCounts)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut fields = line.split_whitespace();
    let name = fields.next()?;
    let nums: Vec<u32> = fields.filter_map(|f| f.parse().ok()).collect();
    let [ip, op, lp, iu, ou, lu] = nums[..] else {
        return None;
    };
    Some((
        name,
        RawCounts {
            int_pops: ip,
            obj_pops: op,
            long_pops: lp,
            int_pushes: iu,
            obj_pushes: ou,
            long_pushes: lu,
        },
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stack {
    Int,
    Obj,
    Long,
}

fn stack_of(ty: Type) -> Stack {
    match ty.base() {
        Some(BaseVarType::Long) => Stack::Long,
        Some(BaseVarType::String | BaseVarType::CoordFine) => Stack::Obj,
        // integer types and the base-less set types live on the int stack
        Some(BaseVarType::Integer) | None => Stack::Int,
    }
}

/// One operand-stack slot: the type-variable node, plus the literal value when the
/// slot was produced by a constant push (needed to resolve data-dependent arities —
/// the hook descriptor string and watcher count).
struct Slot {
    node: Node,
    int_lit: Option<i32>,
    str_lit: Option<String>,
}

impl Slot {
    fn bare(node: Node) -> Self {
        Self {
            node,
            int_lit: None,
            str_lit: None,
        }
    }
}

#[derive(Default)]
struct Stacks {
    int: Vec<Slot>,
    obj: Vec<Slot>,
    long: Vec<Slot>,
}

impl Stacks {
    fn vec(&mut self, stack: Stack) -> &mut Vec<Slot> {
        match stack {
            Stack::Int => &mut self.int,
            Stack::Obj => &mut self.obj,
            Stack::Long => &mut self.long,
        }
    }

    fn push(&mut self, stack: Stack, node: Node) {
        self.vec(stack).push(Slot::bare(node));
    }

    fn push_const(
        &mut self,
        stack: Stack,
        node: Node,
        int_lit: Option<i32>,
        str_lit: Option<String>,
    ) {
        self.vec(stack).push(Slot {
            node,
            int_lit,
            str_lit,
        });
    }

    fn pop(&mut self, stack: Stack) -> Option<Node> {
        self.vec(stack).pop().map(|s| s.node)
    }

    fn pop_slot(&mut self, stack: Stack) -> Option<Slot> {
        self.vec(stack).pop()
    }

    fn clear(&mut self) {
        self.int.clear();
        self.obj.clear();
        self.long.clear();
    }

    fn all_empty(&self) -> bool {
        self.int.is_empty() && self.obj.is_empty() && self.long.is_empty()
    }
}

/// Instruction indices that are branch/switch targets — block boundaries where the
/// CS2 invariant requires an empty operand stack. Used to detect stack misalignment.
fn block_boundaries(script: &CompiledScript) -> std::collections::HashSet<usize> {
    let mut boundaries = std::collections::HashSet::new();
    for ins in &script.code {
        match &ins.operand {
            Operand::Branch(t) => {
                if let Ok(t) = usize::try_from(*t) {
                    boundaries.insert(t);
                }
            }
            Operand::Switch(cases) => {
                for case in cases {
                    if let Ok(t) = usize::try_from(case.target) {
                        boundaries.insert(t);
                    }
                }
            }
            _ => {}
        }
    }
    boundaries
}

/// The script could not be modelled — an unknown command (no signature, no
/// stack-effect entry) or a stack underflow. The caller leaves such scripts
/// un-inferred rather than guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmodellable;

/// Replay `script` over the typed three-stack model, emitting constraints into `inf`.
/// `callee` resolves a `gosub` target's arities. Returns [`Unmodellable`] if the
/// script can't be modelled (unknown command / stack underflow) — the caller skips it.
pub fn generate(
    script_id: i32,
    script: &CompiledScript,
    sigs: &SignatureTable,
    callee: &dyn Fn(i32) -> Option<CalleeSig>,
    inf: &mut TypeInfer,
) -> Result<(), Unmodellable> {
    let wk = *lattice().wk();
    let mut stacks = Stacks::default();
    let boundaries = block_boundaries(script);
    let mut prev_terminator = false;

    for (i, ins) in script.code.iter().enumerate() {
        let idx = i as u32;
        let cmd = ins.command.as_str();

        // At a branch/switch target the CS2 stack is empty. If our fall-through
        // simulation arrives non-empty, either the previous block terminated
        // (unreachable leftover — reset) or we mis-modelled an opcode (bail).
        if boundaries.contains(&i) && !stacks.all_empty() {
            if prev_terminator {
                stacks.clear();
            } else {
                return Err(Unmodellable);
            }
        }
        prev_terminator = matches!(cmd, "return" | "branch");

        match cmd {
            // ── constants (literal value retained for data-dependent arities) ──
            "push_constant_int" => {
                let n = inf.constant(wk.unknown_int);
                let lit = if let Operand::Int(v) = ins.operand {
                    Some(v)
                } else {
                    None
                };
                stacks.push_const(Stack::Int, n, lit, None);
            }
            "push_constant_string" => {
                // typed-constant trick: a string-tagged operand is a real string;
                // an int-tagged one is how the RT7 corpus encodes int constants.
                match &ins.operand {
                    Operand::Str(s) => {
                        let n = inf.constant(wk.string);
                        stacks.push_const(Stack::Obj, n, None, Some(s.clone()));
                    }
                    other => {
                        let n = inf.constant(wk.unknown_int);
                        let lit = if let Operand::Int(v) = other {
                            Some(*v)
                        } else {
                            None
                        };
                        stacks.push_const(Stack::Int, n, lit, None);
                    }
                }
            }
            "push_long_constant" => {
                let n = inf.constant(wk.unknown_long);
                stacks.push(Stack::Long, n);
            }

            // ── locals ──
            "push_int_local" => stacks.push(
                Stack::Int,
                local_node(script_id, LocalDomain::Integer, &ins.operand)?,
            ),
            "push_string_local" => stacks.push(
                Stack::Obj,
                local_node(script_id, LocalDomain::Object, &ins.operand)?,
            ),
            "push_long_local" => stacks.push(
                Stack::Long,
                local_node(script_id, LocalDomain::Long, &ins.operand)?,
            ),
            "pop_int_local" => {
                let v = stacks.pop(Stack::Int).ok_or(Unmodellable)?;
                inf.assign(
                    v,
                    local_node(script_id, LocalDomain::Integer, &ins.operand)?,
                );
            }
            "pop_string_local" => {
                let v = stacks.pop(Stack::Obj).ok_or(Unmodellable)?;
                inf.assign(v, local_node(script_id, LocalDomain::Object, &ins.operand)?);
            }
            "pop_long_local" => {
                let v = stacks.pop(Stack::Long).ok_or(Unmodellable)?;
                inf.assign(v, local_node(script_id, LocalDomain::Long, &ins.operand)?);
            }
            "pop_int_discard" => {
                stacks.pop(Stack::Int).ok_or(Unmodellable)?;
            }
            "pop_string_discard" => {
                stacks.pop(Stack::Obj).ok_or(Unmodellable)?;
            }
            "pop_long_discard" => {
                stacks.pop(Stack::Long).ok_or(Unmodellable)?;
            }

            // ── vars / varbits (player-int domain) ──
            "push_var" => stacks.push(Stack::Int, var_node(&ins.operand)?),
            "pop_var" => {
                let v = stacks.pop(Stack::Int).ok_or(Unmodellable)?;
                inf.assign(v, var_node(&ins.operand)?);
            }
            "push_varbit" => stacks.push(Stack::Int, varbit_node(&ins.operand)?),
            "pop_varbit" => {
                let v = stacks.pop(Stack::Int).ok_or(Unmodellable)?;
                inf.assign(v, varbit_node(&ins.operand)?);
            }

            // ── control flow ──
            "branch" => {}
            "branch_if_true" | "branch_if_false" => {
                stacks.pop(Stack::Int).ok_or(Unmodellable)?;
            }
            "branch_not"
            | "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals" => {
                let b = stacks.pop(Stack::Int).ok_or(Unmodellable)?;
                let a = stacks.pop(Stack::Int).ok_or(Unmodellable)?;
                inf.compare(a, b);
            }
            "long_branch_not"
            | "long_branch_equals"
            | "long_branch_less_than"
            | "long_branch_greater_than"
            | "long_branch_less_than_or_equals"
            | "long_branch_greater_than_or_equals" => {
                let b = stacks.pop(Stack::Long).ok_or(Unmodellable)?;
                let a = stacks.pop(Stack::Long).ok_or(Unmodellable)?;
                inf.compare(a, b);
            }
            "switch" | "text_switch" | "worldlist_switch" => {
                stacks.pop(Stack::Int).ok_or(Unmodellable)?;
            }
            "return" => stacks.clear(),

            // ── variadic string join: operand is the part count (pops N strings) ──
            "join_string" => {
                let count = match ins.operand {
                    Operand::Count(n) | Operand::Int(n) => {
                        usize::try_from(n).map_err(|_| Unmodellable)?
                    }
                    _ => return Err(Unmodellable),
                };
                for _ in 0..count {
                    stacks.pop(Stack::Obj).ok_or(Unmodellable)?;
                }
                let n = inf.constant(wk.string);
                stacks.push(Stack::Obj, n);
            }

            // ── calls ──
            "gosub_with_params" => {
                let Operand::Script(target) = ins.operand else {
                    return Err(Unmodellable);
                };
                let sig = callee(target).ok_or(Unmodellable)?;
                gosub(target, sig, &mut stacks, inf)?;
            }

            // ── variadic UI hooks (cc_seton* / if_seton*) ──
            // Arg count is data-dependent on the callback descriptor string; a fixed
            // stack-effect count here would misalign the rest of the script.
            cmd if is_hook_command(cmd) => hook(cmd, &mut stacks)?,

            // ── generic command (typed signature, else stack-effect counts) ──
            _ => generic(cmd, sigs, &mut stacks, inf, idx, wk)?,
        }
    }
    Ok(())
}

fn local_node(
    script_id: i32,
    domain: LocalDomain,
    operand: &Operand,
) -> Result<Node, Unmodellable> {
    match operand {
        Operand::Local(i) => Ok(Node::Local(script_id, domain, *i as u32)),
        _ => Err(Unmodellable),
    }
}

/// A variadic UI hook: `cc_seton*` / `if_seton*`. Its stack consumption depends on
/// the callback descriptor string, not a fixed arity.
fn is_hook_command(cmd: &str) -> bool {
    (cmd.starts_with("if_") || cmd.starts_with("cc_")) && cmd.contains("_seton")
}

/// Pop a hook command's data-dependent operands, mirroring the pop order in
/// `expr_recovery::recover_callback_literal`: component (`if_` only) → descriptor
/// string → optional watcher count + watchers (`Y` suffix) → one arg per descriptor
/// char → callback script id. Pushes nothing (hooks are void). Bails if the
/// descriptor or watcher count isn't a literal (the arity can't be resolved).
fn hook(cmd: &str, stacks: &mut Stacks) -> Result<(), Unmodellable> {
    if cmd.starts_with("if_") {
        stacks.pop(Stack::Int).ok_or(Unmodellable)?; // component
    }
    let descriptor = stacks.pop_slot(Stack::Obj).ok_or(Unmodellable)?;
    let desc = descriptor.str_lit.ok_or(Unmodellable)?;

    let mut signature = desc.as_str();
    if let Some(stripped) = signature.strip_suffix('Y') {
        signature = stripped;
        let count_slot = stacks.pop_slot(Stack::Int).ok_or(Unmodellable)?;
        let count =
            usize::try_from(count_slot.int_lit.ok_or(Unmodellable)?).map_err(|_| Unmodellable)?;
        for _ in 0..count {
            stacks.pop(Stack::Int).ok_or(Unmodellable)?; // watcher var id
        }
    }
    for ch in signature.chars() {
        stacks.pop(hook_arg_stack(ch)).ok_or(Unmodellable)?;
    }
    stacks.pop(Stack::Int).ok_or(Unmodellable)?; // callback script id
    Ok(())
}

/// Which operand stack a hook descriptor type-char draws from. Only `string` and
/// `coordfine` (object stack) and the handful of `long`-base type chars deviate from
/// the int stack; an unrecognised char defaults to int (the dominant case).
fn hook_arg_stack(ch: char) -> Stack {
    match ch {
        's' | 'Ž' => Stack::Obj,
        'r' | 'Œ' | '§' | 'û' | '¼' | '½' | 'Â' => Stack::Long,
        _ => Stack::Int,
    }
}

fn var_node(operand: &Operand) -> Result<Node, Unmodellable> {
    match operand {
        Operand::VarRef(v) => Ok(Node::Var(v.domain, u32::from(v.id))),
        _ => Err(Unmodellable),
    }
}

fn varbit_node(operand: &Operand) -> Result<Node, Unmodellable> {
    match operand {
        Operand::VarBitRef(v) => Ok(Node::VarBit(u32::from(v.id))),
        _ => Err(Unmodellable),
    }
}

fn gosub(
    target: i32,
    sig: CalleeSig,
    stacks: &mut Stacks,
    inf: &mut TypeInfer,
) -> Result<(), Unmodellable> {
    // Pop args off each stack and bind them to the callee's parameter locals
    // (parameters occupy the callee's first local slots, per storage class).
    bind_args(
        stacks,
        inf,
        Stack::Long,
        target,
        LocalDomain::Long,
        sig.arg_long,
    )?;
    bind_args(
        stacks,
        inf,
        Stack::Obj,
        target,
        LocalDomain::Object,
        sig.arg_obj,
    )?;
    bind_args(
        stacks,
        inf,
        Stack::Int,
        target,
        LocalDomain::Integer,
        sig.arg_int,
    )?;

    // Push the callee's results back (untyped: v1 keeps the stacks balanced; the
    // call site's consumer refines each value).
    let wk = *lattice().wk();
    for _ in 0..sig.ret_int {
        let n = inf.constant(wk.unknown_int);
        stacks.push(Stack::Int, n);
    }
    for _ in 0..sig.ret_obj {
        let n = inf.constant(wk.unknown_object);
        stacks.push(Stack::Obj, n);
    }
    for _ in 0..sig.ret_long {
        let n = inf.constant(wk.unknown_long);
        stacks.push(Stack::Long, n);
    }
    Ok(())
}

fn bind_args(
    stacks: &mut Stacks,
    inf: &mut TypeInfer,
    stack: Stack,
    target: i32,
    domain: LocalDomain,
    count: u16,
) -> Result<(), Unmodellable> {
    // Top of stack is the last argument → highest-index parameter local.
    let mut popped = Vec::with_capacity(count as usize);
    for _ in 0..count {
        popped.push(stacks.pop(stack).ok_or(Unmodellable)?);
    }
    for (k, node) in popped.into_iter().enumerate() {
        let local_index = u32::from(count) - 1 - k as u32;
        inf.assign(node, Node::Local(target, domain, local_index));
    }
    Ok(())
}

fn generic(
    cmd: &str,
    sigs: &SignatureTable,
    stacks: &mut Stacks,
    inf: &mut TypeInfer,
    idx: u32,
    wk: super::types::WellKnown,
) -> Result<(), Unmodellable> {
    if let Some(sig) = sigs.typed.get(cmd) {
        for arg_ty in sig.args.iter().rev() {
            let node = stacks.pop(stack_of(*arg_ty)).ok_or(Unmodellable)?;
            inf.assign_type(node, *arg_ty);
        }
        for (slot, res_ty) in sig.results.iter().enumerate() {
            let node = Node::Expr(idx, slot as u32);
            inf.assign_type(node, *res_ty);
            stacks.push(stack_of(*res_ty), node);
        }
        return Ok(());
    }
    if let Some(c) = sigs.counts.get(cmd) {
        for _ in 0..c.long_pops {
            stacks.pop(Stack::Long).ok_or(Unmodellable)?;
        }
        for _ in 0..c.obj_pops {
            stacks.pop(Stack::Obj).ok_or(Unmodellable)?;
        }
        for _ in 0..c.int_pops {
            stacks.pop(Stack::Int).ok_or(Unmodellable)?;
        }
        for _ in 0..c.int_pushes {
            let n = inf.constant(wk.unknown_int);
            stacks.push(Stack::Int, n);
        }
        for _ in 0..c.obj_pushes {
            let n = inf.constant(wk.unknown_object);
            stacks.push(Stack::Obj, n);
        }
        for _ in 0..c.long_pushes {
            let n = inf.constant(wk.unknown_long);
            stacks.push(Stack::Long, n);
        }
        return Ok(());
    }
    // Unknown command — can't model the stack effect.
    Err(Unmodellable)
}

/// Infer local types for a set of scripts as one interprocedural fixed point.
/// Scripts that can't be modelled contribute nothing (left un-inferred). `callee`
/// supplies gosub arities (the real pipeline derives these from its signatures).
pub fn infer_program(
    scripts: &[(i32, &CompiledScript)],
    sigs: &SignatureTable,
    callee: &dyn Fn(i32) -> Option<CalleeSig>,
) -> ProgramTypes {
    infer_program_diag(scripts, sigs, callee).0
}

/// Like [`infer_program`] but also returns the conflict-causing type-pair histogram
/// (descending), for diagnosing where `conflict`s come from.
pub fn infer_program_diag(
    scripts: &[(i32, &CompiledScript)],
    sigs: &SignatureTable,
    callee: &dyn Fn(i32) -> Option<CalleeSig>,
) -> (ProgramTypes, ConflictHistogram) {
    // Two-phase so a script that bails mid-way can't leave partial (misaligned)
    // constraints polluting the shared inference: probe each script in isolation,
    // then re-emit only the cleanly-modelled ones into the shared solver.
    let mut inf = TypeInfer::new();
    let mut modelled: Vec<i32> = Vec::new();
    for &(id, script) in scripts {
        let mut probe = TypeInfer::new();
        if generate(id, script, sigs, callee, &mut probe).is_ok() {
            let _ = generate(id, script, sigs, callee, &mut inf);
            modelled.push(id);
        }
    }
    inf.propagate();

    let mut out: ProgramTypes = HashMap::new();
    for &(id, script) in scripts {
        if !modelled.contains(&id) {
            continue;
        }
        let mut locals: LocalTypes = HashMap::new();
        read_locals(
            &inf,
            id,
            LocalDomain::Integer,
            script.local_count_int,
            &mut locals,
        );
        read_locals(
            &inf,
            id,
            LocalDomain::Object,
            script.local_count_object,
            &mut locals,
        );
        read_locals(
            &inf,
            id,
            LocalDomain::Long,
            script.local_count_long,
            &mut locals,
        );
        out.insert(id, locals);
    }

    let mut counts: HashMap<(Type, Type), usize> = HashMap::new();
    for &(a, b) in inf.conflict_log() {
        let key = if a <= b { (a, b) } else { (b, a) };
        *counts.entry(key).or_default() += 1;
    }
    let mut hist: Vec<((Type, Type), usize)> = counts.into_iter().collect();
    hist.sort_by(|x, y| y.1.cmp(&x.1));
    (out, hist)
}

/// Render an inferred local type for display (G3.3), applying the **non-regressive
/// policy**: a refined semantic type renders as its RuneScript keyword (`component`,
/// `loc`, `enum`, …); `conflict`, the `unknown_*` sets, and the generic `int`/`int_int`
/// all fall back to the slot's **base VM type** (`int` / `string` / `long`) — exactly
/// what is rendered today, so a typed local is only ever an improvement, never a
/// wrong guess. The base falls back to the local's *domain* (not the inferred type's
/// base, which `conflict` lacks).
pub fn render_local_type(inferred: Type, domain: LocalDomain) -> &'static str {
    let wk = lattice().wk();
    // `boolean` is excluded: it is int-base but its TS annotation (`Boolean`) is not
    // the `Number` the header-count + reference-domain machinery expects, so rendering
    // it would corrupt the recompile. Generic `int`/`int_int` fall back to base too.
    let displayable = inferred.base().is_some()
        && inferred != wk.int
        && inferred != wk.int_int
        && inferred != wk.boolean;
    if displayable {
        inferred.name()
    } else {
        match domain {
            LocalDomain::Object => "string",
            LocalDomain::Long => "long",
            LocalDomain::Integer | LocalDomain::Array => "int",
        }
    }
}

/// Rewrite the type annotation on each `let local_<d>_<n>: …;` declaration in a
/// rendered structured-TS source to its inferred semantic type (per [`render_local_type`]).
/// Byte-irrelevant: the reverse compiler recovers a local's slot/domain from its
/// *name*, never the annotation — so the recompile is byte-identical. Lines that don't
/// match a local declaration, and locals with no refined type, are left untouched.
pub fn annotate_local_declarations(source: &str, inferred: &LocalTypes) -> String {
    let trailing_newline = source.ends_with('\n');
    let body = source
        .lines()
        .map(|line| rewrite_local_decl(line, inferred).unwrap_or_else(|| line.to_string()))
        .collect::<Vec<_>>()
        .join("\n");
    if trailing_newline { body + "\n" } else { body }
}

fn rewrite_local_decl(line: &str, inferred: &LocalTypes) -> Option<String> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let rest = trimmed.strip_prefix("let ")?;
    let (name, after) = rest.split_once(": ")?;
    after.strip_suffix(';')?;
    let (domain, index) = parse_local_name(name)?;
    let inferred_ty = *inferred.get(&(domain, index))?;
    let rendered = render_local_type(inferred_ty, domain);
    // Only annotate genuinely-refined types; leave base locals on their TS annotation
    // (`number`/`string`/`bigint`) so unrefined locals are byte- and text-unchanged.
    if matches!(rendered, "int" | "string" | "long") {
        return None;
    }
    Some(format!("{indent}let {name}: {rendered};"))
}

/// `local_int_5` → `(Integer, 5)`. Returns `None` for non-local names (args, arrays).
fn parse_local_name(name: &str) -> Option<(LocalDomain, u32)> {
    let body = name.strip_prefix("local_")?;
    let (kind, index) = body.rsplit_once('_')?;
    let domain = match kind {
        "int" => LocalDomain::Integer,
        "obj" => LocalDomain::Object,
        "long" => LocalDomain::Long,
        _ => return None,
    };
    Some((domain, index.parse().ok()?))
}

fn read_locals(
    inf: &TypeInfer,
    script_id: i32,
    domain: LocalDomain,
    count: u16,
    out: &mut LocalTypes,
) {
    for index in 0..u32::from(count) {
        let ty = inf.type_of(Node::Local(script_id, domain, index));
        out.insert((domain, index), ty);
    }
}

// ── data-closure simulation (gap #2) ─────────────────────────────────────────
//
// [`simulate`] is a leaner, observer-driven sibling of [`generate`]: it replays a
// script over the SAME three-stack model and reuses the SAME private helpers
// ([`Stacks`]/[`Slot`]/[`stack_of`]/[`is_hook_command`]/[`hook`]/[`block_boundaries`]),
// but tracks only per-slot literal *constants* instead of type-variable nodes, so it
// needs no [`TypeInfer`]. It powers `explain-interface --data-closure`
// ([`crate::data_closure`]): surfacing which config ids a script's commands consume.
//
// Unlike [`generate`] (which bails the whole script on the first unmodellable
// instruction), [`simulate`] is robust: an opaque instruction marks the model
// unreliable and clears the stacks, suppressing observer callbacks only until the
// next basic-block boundary restores the (CS2-guaranteed) empty stack. A single
// opaque op therefore costs at most the refs in its basic block, never the script.

/// A literal constant occupying an operand-stack slot, recovered by [`simulate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstArg {
    Int(i32),
    Str(String),
}

/// Observer for [`simulate`]. Callbacks fire only while the stack model is reliable
/// (see the module note above), so a recorded reference is always correctly aligned.
pub trait SimObserver {
    /// Whether `on_typed_arg` should fire for this argument type. Defaults to `false`
    /// so an observer pays nothing for argument types it does not care about.
    fn interested_in(&self, _ty: Type) -> bool {
        false
    }
    /// A typed command argument of semantic type `ty`, supplied by `value` (`Some`
    /// when it was a push-constant literal, `None` when computed / a local / a call
    /// result). Fired only for types where [`Self::interested_in`] returned `true`.
    fn on_typed_arg(&mut self, _ty: Type, _value: Option<ConstArg>) {}
    /// A `return` reached with `int`/`obj`/`long` values live on the three stacks —
    /// the script's return arity at this point.
    fn on_return(&mut self, _int: usize, _obj: usize, _long: usize) {}
}

/// A throwaway stack-slot node for [`simulate`], which never inspects `Slot::node`
/// (it reads only the literal). Any node value is correct.
fn dummy_node() -> Node {
    Node::Expr(0, 0)
}

/// Replay `script` over the constant-tracking three-stack model, driving `obs`.
/// `callee` supplies a gosub target's `(arg, return)` arities (the data-closure
/// driver derives these by a local fixed point). Never panics or aborts the walk;
/// see the module note for the reliability/degradation contract.
// The `degrade!` macro's `continue` is load-bearing where it expands mid-arm (inside
// `pop!`, to abort the rest of the instruction); clippy only sees it as redundant in
// the tail-position expansions. One uniform macro is clearer than splitting it.
#[allow(clippy::needless_continue)]
pub fn simulate(
    script: &CompiledScript,
    sigs: &SignatureTable,
    callee: &dyn Fn(i32) -> Option<CalleeSig>,
    obs: &mut dyn SimObserver,
) {
    let boundaries = block_boundaries(script);
    let mut stacks = Stacks::default();
    // `reliable` is false between an unmodellable instruction and the next block
    // boundary; callbacks are suppressed and pops can't underflow-bail in that window.
    let mut reliable = true;

    macro_rules! degrade {
        () => {{
            reliable = false;
            stacks.clear();
            continue;
        }};
    }
    macro_rules! pop {
        ($stk:expr) => {
            match stacks.pop($stk) {
                Some(node) => node,
                None => degrade!(),
            }
        };
    }

    for (i, ins) in script.code.iter().enumerate() {
        // A branch/switch target: the CS2 stack is empty here, so reset and trust.
        if boundaries.contains(&i) {
            stacks.clear();
            reliable = true;
        }
        if !reliable {
            continue;
        }
        match ins.command.as_str() {
            // ── constants (literal retained on the slot) ──
            "push_constant_int" => {
                let lit = if let Operand::Int(v) = ins.operand {
                    Some(v)
                } else {
                    None
                };
                stacks.push_const(Stack::Int, dummy_node(), lit, None);
            }
            "push_constant_string" => match &ins.operand {
                Operand::Str(s) => {
                    stacks.push_const(Stack::Obj, dummy_node(), None, Some(s.clone()));
                }
                other => {
                    let lit = if let Operand::Int(v) = other {
                        Some(*v)
                    } else {
                        None
                    };
                    stacks.push_const(Stack::Int, dummy_node(), lit, None);
                }
            },
            "push_long_constant" => stacks.push(Stack::Long, dummy_node()),

            // ── locals / vars / varbits (non-constant slots) ──
            "push_int_local" | "push_var" | "push_varbit" => stacks.push(Stack::Int, dummy_node()),
            "push_string_local" => stacks.push(Stack::Obj, dummy_node()),
            "push_long_local" => stacks.push(Stack::Long, dummy_node()),
            "pop_int_local" | "pop_int_discard" | "pop_var" | "pop_varbit" => {
                pop!(Stack::Int);
            }
            "pop_string_local" | "pop_string_discard" => {
                pop!(Stack::Obj);
            }
            "pop_long_local" | "pop_long_discard" => {
                pop!(Stack::Long);
            }

            // ── control flow ──
            "branch" => {}
            "branch_if_true" | "branch_if_false" => {
                pop!(Stack::Int);
            }
            "branch_not"
            | "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals" => {
                pop!(Stack::Int);
                pop!(Stack::Int);
            }
            "long_branch_not"
            | "long_branch_equals"
            | "long_branch_less_than"
            | "long_branch_greater_than"
            | "long_branch_less_than_or_equals"
            | "long_branch_greater_than_or_equals" => {
                pop!(Stack::Long);
                pop!(Stack::Long);
            }
            "switch" | "text_switch" | "worldlist_switch" => {
                pop!(Stack::Int);
            }
            "return" => {
                obs.on_return(stacks.int.len(), stacks.obj.len(), stacks.long.len());
                stacks.clear();
            }

            // ── variadic string join ──
            "join_string" => {
                let count = match ins.operand {
                    Operand::Count(n) | Operand::Int(n) => usize::try_from(n).unwrap_or(0),
                    _ => degrade!(),
                };
                for _ in 0..count {
                    pop!(Stack::Obj);
                }
                stacks.push(Stack::Obj, dummy_node());
            }

            // ── calls ──
            "gosub_with_params" => {
                let Operand::Script(target) = ins.operand else {
                    degrade!();
                };
                let Some(sig) = callee(target) else {
                    degrade!();
                };
                for _ in 0..sig.arg_long {
                    pop!(Stack::Long);
                }
                for _ in 0..sig.arg_obj {
                    pop!(Stack::Obj);
                }
                for _ in 0..sig.arg_int {
                    pop!(Stack::Int);
                }
                for _ in 0..sig.ret_int {
                    stacks.push(Stack::Int, dummy_node());
                }
                for _ in 0..sig.ret_obj {
                    stacks.push(Stack::Obj, dummy_node());
                }
                for _ in 0..sig.ret_long {
                    stacks.push(Stack::Long, dummy_node());
                }
            }

            // ── variadic UI hooks (reuse generate's descriptor-driven pop) ──
            cmd if is_hook_command(cmd) => {
                if hook(cmd, &mut stacks).is_err() {
                    degrade!();
                }
            }

            // ── generic command: typed signature (records refs), else counts ──
            cmd => {
                if let Some(sig) = sigs.typed.get(cmd) {
                    for arg_ty in sig.args.iter().rev() {
                        let Some(slot) = stacks.pop_slot(stack_of(*arg_ty)) else {
                            degrade!();
                        };
                        if obs.interested_in(*arg_ty) {
                            let value = slot
                                .int_lit
                                .map(ConstArg::Int)
                                .or_else(|| slot.str_lit.map(ConstArg::Str));
                            obs.on_typed_arg(*arg_ty, value);
                        }
                    }
                    for res_ty in &sig.results {
                        stacks.push(stack_of(*res_ty), dummy_node());
                    }
                } else if let Some(c) = sigs.counts.get(cmd) {
                    for _ in 0..c.long_pops {
                        pop!(Stack::Long);
                    }
                    for _ in 0..c.obj_pops {
                        pop!(Stack::Obj);
                    }
                    for _ in 0..c.int_pops {
                        pop!(Stack::Int);
                    }
                    for _ in 0..(c.int_pushes) {
                        stacks.push(Stack::Int, dummy_node());
                    }
                    for _ in 0..c.obj_pushes {
                        stacks.push(Stack::Obj, dummy_node());
                    }
                    for _ in 0..c.long_pushes {
                        stacks.push(Stack::Long, dummy_node());
                    }
                } else {
                    degrade!();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::Instruction;

    fn ins(command: &str, operand: Operand) -> Instruction {
        Instruction {
            opcode: 0,
            command: command.to_string(),
            operand,
        }
    }

    fn script(local_int: u16, code: Vec<Instruction>) -> CompiledScript {
        CompiledScript {
            name: None,
            local_count_int: local_int,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code,
        }
    }

    fn no_callee(_: i32) -> Option<CalleeSig> {
        None
    }

    #[test]
    fn parses_typed_and_build_gated_signatures() {
        let table = SignatureTable::from_sources(
            &[
                "[command,paint](component $x1)\n[command,db_find](dbcolumn $x1, int $x2, basevartype $x3) 919",
            ],
            "",
            948,
        );
        assert_eq!(table.typed["paint"].args[0].name(), "component");
        // build 948 ≥ 919 → the 3-arg gated variant applies
        assert_eq!(table.typed["db_find"].args.len(), 3);

        let pre919 = SignatureTable::from_sources(
            &["[command,db_find](dbcolumn $x1, int $x2, basevartype $x3) 919"],
            "",
            910,
        );
        // build 910 < 919 → the gated variant is filtered out entirely
        assert!(!pre919.typed.contains_key("db_find"));
    }

    #[test]
    fn local_typed_by_command_argument() {
        // push_int_local 0 ; paint(component) → local 0 is component.
        let sigs = SignatureTable::from_sources(&["[command,paint](component $x1)"], "", 948);
        let s = script(
            1,
            vec![
                ins("push_int_local", Operand::Local(0)),
                ins("paint", Operand::Int(0)),
            ],
        );
        let result = infer_program(&[(1, &s)], sigs_ref(&sigs), &no_callee);
        assert_eq!(result[&1][&(LocalDomain::Integer, 0)].name(), "component");
    }

    #[test]
    fn local_typed_by_command_result() {
        // r = cc_find() : component ; pop_int_local 0 → local 0 is component.
        let sigs = SignatureTable::from_sources(&["[command,cc_find]()(component)"], "", 948);
        let s = script(
            1,
            vec![
                ins("cc_find", Operand::Int(0)),
                ins("pop_int_local", Operand::Local(0)),
            ],
        );
        let result = infer_program(&[(1, &s)], sigs_ref(&sigs), &no_callee);
        assert_eq!(result[&1][&(LocalDomain::Integer, 0)].name(), "component");
    }

    #[test]
    fn type_flows_across_gosub_into_callee_param() {
        // caller: push_int_local 0 ; gosub 42(arg_int=1)
        // callee 42: push_int_local 0 (its param) ; paint(component)
        // → caller local 0 AND callee local 0 both infer component.
        let sigs = SignatureTable::from_sources(&["[command,paint](component $x1)"], "", 948);
        let caller = script(
            1,
            vec![
                ins("push_int_local", Operand::Local(0)),
                ins("gosub_with_params", Operand::Script(42)),
            ],
        );
        let proc_42 = script(
            1,
            vec![
                ins("push_int_local", Operand::Local(0)),
                ins("paint", Operand::Int(0)),
            ],
        );
        let arities = |id: i32| {
            (id == 42).then_some(CalleeSig {
                arg_int: 1,
                arg_obj: 0,
                arg_long: 0,
                ret_int: 0,
                ret_obj: 0,
                ret_long: 0,
            })
        };
        let result = infer_program(&[(1, &caller), (42, &proc_42)], sigs_ref(&sigs), &arities);
        assert_eq!(result[&1][&(LocalDomain::Integer, 0)].name(), "component");
        assert_eq!(result[&42][&(LocalDomain::Integer, 0)].name(), "component");
    }

    #[test]
    fn unknown_command_bails_gracefully() {
        // No signature, no stack-effect → script un-modellable → no inferred locals.
        let sigs = SignatureTable::from_sources(&[], "", 948);
        let s = script(1, vec![ins("totally_unknown_op", Operand::Int(0))]);
        let result = infer_program(&[(1, &s)], sigs_ref(&sigs), &no_callee);
        assert!(!result.contains_key(&1));
    }

    #[test]
    fn embedded_table_loads_real_signatures() {
        // Smoke test: the embedded 948 table parses without panicking and has a
        // recognisable typed signature.
        let table = SignatureTable::embedded(948);
        assert!(table.typed.contains_key("add"));
        assert_eq!(
            table.typed["add"].results.first().map(|t| t.name()),
            Some("int")
        );
    }

    // Helper: the test corpus owns the table; pass a borrow.
    fn sigs_ref(table: &SignatureTable) -> &SignatureTable {
        table
    }

    #[test]
    fn annotates_only_refined_local_declarations() {
        use crate::transpile::types::lattice;
        let l = lattice();
        let mut inferred: LocalTypes = HashMap::new();
        inferred.insert((LocalDomain::Integer, 0), l.by_name("component"));
        inferred.insert((LocalDomain::Integer, 1), l.wk().conflict); // unrefined → keep TS
        inferred.insert((LocalDomain::Object, 0), l.wk().string); // base → unchanged
        let source = "export function f(): void {\n    let local_int_0: number;\n    let local_int_1: number;\n    let local_obj_0: string;\n}\n";
        let out = annotate_local_declarations(source, &inferred);
        assert!(out.contains("let local_int_0: component;"), "{out}");
        assert!(
            out.contains("let local_int_1: number;"),
            "conflict keeps TS annotation"
        );
        assert!(out.contains("let local_obj_0: string;"), "base unchanged");
        assert!(
            out.contains("export function f(): void {"),
            "signature untouched"
        );
        assert!(out.ends_with("}\n"), "trailing newline preserved");
    }

    #[test]
    fn render_policy_is_non_regressive() {
        use crate::transpile::types::lattice;
        let l = lattice();
        let wk = l.wk();
        // refined semantic types render as their keyword
        assert_eq!(
            render_local_type(l.by_name("component"), LocalDomain::Integer),
            "component"
        );
        assert_eq!(
            render_local_type(l.by_name("loc"), LocalDomain::Integer),
            "loc"
        );
        assert_eq!(render_local_type(wk.string, LocalDomain::Object), "string");
        // conflict / unknown / generic-int fall back to the slot's base VM type
        assert_eq!(render_local_type(wk.conflict, LocalDomain::Integer), "int");
        assert_eq!(
            render_local_type(wk.unknown_int, LocalDomain::Integer),
            "int"
        );
        assert_eq!(render_local_type(wk.int_int, LocalDomain::Integer), "int");
        assert_eq!(render_local_type(wk.unknown, LocalDomain::Integer), "int");
        assert_eq!(
            render_local_type(wk.conflict, LocalDomain::Object),
            "string"
        );
        assert_eq!(render_local_type(wk.conflict, LocalDomain::Long), "long");
        assert_eq!(
            render_local_type(wk.unknown_object, LocalDomain::Object),
            "string"
        );
    }
}
