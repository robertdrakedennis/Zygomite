//! `explain-interface --data-closure` (gap #2): the transitive DATA / state closure
//! of an interface — every config id (varbit, varp, enum, struct, dbtable, param,
//! obj, …) the interface's whole CS2 script closure reads or writes.
//!
//! `explain-interface --transitive` walks the script→script call graph and reports
//! the SCRIPT ids; it says nothing about the STATE those scripts touch. (Neither does
//! `dep-tree-script`, which follows only the inline var/varbit/script operands.) This
//! module fills that gap: over the same transitive closure it runs two passes.
//!
//! 1. **Inline pass — precise.** varbit / var ids carried *directly* in instruction
//!    operands (`Operand::VarBitRef` / `Operand::VarRef`). This is the headline:
//!    every `push_varbit` / `pop_var` id, exactly.
//! 2. **Typed-constant pass — best-effort.** A constant-tracking stack simulation
//!    ([`crate::transpile::type_constraints::simulate`]) attributes a push-constant id
//!    to the config-typed command argument that consumes it: the `4592` pushed before
//!    `struct_param` is a `param`; the id before `enum` is an `enum`; the `dbtable`
//!    arg of `db_listall` is a dbtable; and so on. An argument whose id is *not* a
//!    static constant (a local, a call result, a computed value) is counted as
//!    `dynamic` for its kind rather than guessed — the report stays honest about what
//!    is statically knowable.
//!
//! Return arities (needed to model `gosub` and keep the stacks balanced, since a
//! script header carries arg counts but not return counts) are derived by a small
//! local fixed point over the same simulation — no full transpile pipeline required.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::script::{CompiledScript, Operand};
use crate::transpile::type_constraints::{
    CalleeSig, ConstArg, SignatureTable, SimObserver, simulate,
};
use crate::transpile::types::Type;
use crate::vars::VarDomain;

/// Rounds of the return-arity fixed point. Arities propagate one call-edge per
/// round; convergence is typically 2–4 rounds, and the loop also early-exits on a
/// stable round, so this is a safety cap, not a tuning knob.
const MAX_ARITY_ROUNDS: usize = 8;

/// The transitive data / state closure of an interface's script closure.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DataClosure {
    /// How many closure scripts were scanned.
    pub scripts_scanned: usize,
    /// Config kind → the distinct, statically-resolved ids referenced. Keyed by a
    /// stable label (`varbit`, `varplayer`, `enum`, `struct`, `dbtable`, `param`, …).
    pub refs: BTreeMap<String, BTreeSet<i32>>,
    /// Config kind → count of references whose id was NOT a static constant
    /// (computed / from a local / a call result). Present in the closure but not
    /// statically resolvable to an id here.
    pub dynamic: BTreeMap<String, usize>,
}

impl DataClosure {
    fn add_ref(&mut self, kind: &str, id: i32) {
        self.refs.entry(kind.to_string()).or_default().insert(id);
    }

    fn add_dynamic(&mut self, kind: &str) {
        *self.dynamic.entry(kind.to_string()).or_default() += 1;
    }

    /// Total distinct statically-resolved references across all kinds.
    #[must_use]
    pub fn total_refs(&self) -> usize {
        self.refs.values().map(BTreeSet::len).sum()
    }
}

/// The curated set of cache-config REFERENCE types whose ids are "expected state"
/// for a port, keyed by the lattice type name. Primitive / int-enum value types
/// (`int`, `boolean`, `colour`, `key`, `windowmode`, …) and the `var_*`/varbit
/// lattice types (the inline pass owns those, precisely) are intentionally excluded.
fn reference_kind(ty: Type) -> Option<&'static str> {
    let name = ty.name();
    let is_ref = matches!(
        name,
        "enum"
            | "struct"
            | "dbrow"
            | "dbtable"
            | "dbcolumn"
            | "dbfilter"
            | "param"
            | "inv"
            | "loc"
            | "npc"
            | "obj"
            | "namedobj"
            | "seq"
            | "spotanim"
            | "model"
            | "idkit"
            | "mesh"
            | "animationclip"
            | "skeleton"
            | "stat"
            | "npc_stat"
            | "category"
            | "component"
            | "interface"
            | "toplevelinterface"
            | "overlayinterface"
            | "clientinterface"
            | "graphic"
            | "fontmetrics"
            | "texture"
            | "material"
            | "stylesheet"
            | "mapelement"
            | "cutscene"
            | "cutscene2d"
            | "vfx"
            | "mesanim"
            | "ui_anim"
            | "ui_anim_curve"
            | "anim_state_machine"
            | "midi"
            | "jingle"
            | "synth"
            | "vorbis"
            | "audiogroup"
            | "audiobuss"
            | "sound"
            | "bas"
            | "seqgroup"
            | "hitmark"
            | "headbar"
            | "hunt"
            | "quest"
            | "achievement"
            | "chatcat"
            | "chatphrase"
            | "label"
            | "walktrigger"
            | "clientscript"
    );
    is_ref.then_some(name)
}

/// The inline-operand label for a var domain.
fn var_kind(domain: VarDomain) -> &'static str {
    match domain {
        VarDomain::Player => "varplayer",
        VarDomain::Npc => "varnpc",
        VarDomain::Client => "varclient",
        VarDomain::World => "varworld",
        VarDomain::Region => "varregion",
        VarDomain::Object => "varobject",
        VarDomain::Clan => "varclan",
        VarDomain::ClanSetting => "varclansetting",
        VarDomain::Controller => "varcontroller",
        VarDomain::Global => "varglobal",
        VarDomain::PlayerGroup => "varplayergroup",
    }
}

/// Pass 1 — precise inline operand refs (varbit / var ids carried in the operand).
fn collect_inline(script: &CompiledScript, out: &mut DataClosure) {
    for ins in &script.code {
        match &ins.operand {
            Operand::VarBitRef(v) => out.add_ref("varbit", i32::from(v.id)),
            Operand::VarRef(v) => out.add_ref(var_kind(v.domain), i32::from(v.id)),
            _ => {}
        }
    }
}

/// Observer that records the maximum operand-stack depths seen at any `return`,
/// i.e. the script's return arity (used by the gosub-balancing fixed point).
#[derive(Default)]
struct ReturnArity {
    int: usize,
    obj: usize,
    long: usize,
}

impl SimObserver for ReturnArity {
    fn on_return(&mut self, int: usize, obj: usize, long: usize) {
        self.int = self.int.max(int);
        self.obj = self.obj.max(obj);
        self.long = self.long.max(long);
    }
}

/// Observer that records each config-typed command argument into the closure: a
/// constant id is a precise ref; a non-constant is a `dynamic` tally for its kind.
struct RefCollector<'a> {
    out: &'a mut DataClosure,
}

impl SimObserver for RefCollector<'_> {
    fn interested_in(&self, ty: Type) -> bool {
        reference_kind(ty).is_some()
    }

    fn on_typed_arg(&mut self, ty: Type, value: Option<ConstArg>) {
        let Some(kind) = reference_kind(ty) else {
            return;
        };
        match value {
            // A config id is always an integer; a string-typed slot or a computed
            // value can't be resolved to an id here, so it tallies as dynamic.
            Some(ConstArg::Int(id)) => self.out.add_ref(kind, id),
            _ => self.out.add_dynamic(kind),
        }
    }
}

/// `(arg_int, arg_obj, arg_long)` triple.
type Arity3 = (u16, u16, u16);

/// Compute the data/state closure for a set of decoded closure scripts (keyed by
/// canonical group id, decoded at `build`). Self-contained: reuses the embedded
/// command-signature table and the constant-stack simulation, deriving gosub return
/// arities by a local fixed point.
#[must_use]
pub fn compute(scripts: &[(i32, &CompiledScript)], build: u32) -> DataClosure {
    let sigs = SignatureTable::embedded(build);

    // Argument counts come straight from each script's header.
    let args: BTreeMap<i32, Arity3> = scripts
        .iter()
        .map(|(id, s)| {
            (
                *id,
                (
                    s.argument_count_int,
                    s.argument_count_object,
                    s.argument_count_long,
                ),
            )
        })
        .collect();

    // Re-key a raw gosub target to a known script: the bare id (single-file groups)
    // or its packed `(group<<16)|file` high half, mirroring the closure walk.
    let resolve = |raw: i32| -> Option<i32> {
        if args.contains_key(&raw) {
            Some(raw)
        } else {
            let group = raw >> 16;
            args.contains_key(&group).then_some(group)
        }
    };

    // Return arities, found by a local fixed point: each round simulates every script
    // using the previous round's return-arity estimates for its callees.
    let mut ret: BTreeMap<i32, Arity3> = scripts.iter().map(|(id, _)| (*id, (0, 0, 0))).collect();
    for _ in 0..MAX_ARITY_ROUNDS {
        let prev = ret.clone();
        let callee = make_callee(&args, &prev, &resolve);
        let mut changed = false;
        for (id, script) in scripts {
            let mut obs = ReturnArity::default();
            simulate(script, sigs, &callee, &mut obs);
            let new = (
                clamp_arity(obs.int),
                clamp_arity(obs.obj),
                clamp_arity(obs.long),
            );
            if prev.get(id) != Some(&new) {
                changed = true;
            }
            ret.insert(*id, new);
        }
        if !changed {
            break;
        }
    }

    // Collection pass: inline refs (precise) + typed-constant refs (best-effort).
    let mut out = DataClosure::default();
    let callee = make_callee(&args, &ret, &resolve);
    for (_, script) in scripts {
        out.scripts_scanned += 1;
        collect_inline(script, &mut out);
        let mut collector = RefCollector { out: &mut out };
        simulate(script, sigs, &callee, &mut collector);
    }
    out
}

/// Saturate a simulated stack depth into the `u16` arity field.
fn clamp_arity(depth: usize) -> u16 {
    u16::try_from(depth).unwrap_or(u16::MAX)
}

/// Build a gosub-arity resolver closure over the arg-count and return-count maps.
fn make_callee<'a>(
    args: &'a BTreeMap<i32, Arity3>,
    ret: &'a BTreeMap<i32, Arity3>,
    resolve: &'a impl Fn(i32) -> Option<i32>,
) -> impl Fn(i32) -> Option<CalleeSig> + 'a {
    move |raw: i32| {
        let key = resolve(raw)?;
        let a = args.get(&key)?;
        let r = ret.get(&key).copied().unwrap_or((0, 0, 0));
        Some(CalleeSig {
            arg_int: a.0,
            arg_obj: a.1,
            arg_long: a.2,
            ret_int: r.0,
            ret_obj: r.1,
            ret_long: r.2,
        })
    }
}

/// Render the human-readable data-closure block appended after the transitive
/// closure. `sample` ids per kind are shown, with a `+N more` / `~M dynamic` tail.
#[must_use]
pub fn render_human(dc: &DataClosure, sample: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "data closure (config/state read across {} closure script(s)):",
        dc.scripts_scanned
    );
    if dc.refs.is_empty() && dc.dynamic.is_empty() {
        let _ = writeln!(out, "  (none resolved)");
        return out;
    }
    // Stable, useful ordering: most-referenced kinds first, then by name.
    let mut kinds: Vec<&String> = dc.refs.keys().chain(dc.dynamic.keys()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    kinds.sort_by(|a, b| {
        let na = dc.refs.get(*b).map_or(0, BTreeSet::len);
        let nb = dc.refs.get(*a).map_or(0, BTreeSet::len);
        na.cmp(&nb).then_with(|| a.cmp(b))
    });
    for kind in kinds {
        let ids = dc.refs.get(kind);
        let count = ids.map_or(0, BTreeSet::len);
        let dynamic = dc.dynamic.get(kind).copied().unwrap_or(0);
        let mut shown: Vec<String> = ids
            .into_iter()
            .flatten()
            .take(sample)
            .map(ToString::to_string)
            .collect();
        if count > sample {
            shown.push(format!("(+{} more)", count - sample));
        }
        let dyn_note = if dynamic > 0 {
            format!("  ~{dynamic} dynamic")
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "  {:<16} [{:>4}] {}{}",
            kind,
            count,
            shown.join(", "),
            dyn_note
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{Instruction, VarBitRef, VarRef};

    fn ins(command: &str, operand: Operand) -> Instruction {
        Instruction {
            opcode: 0,
            command: command.to_string(),
            operand,
        }
    }

    fn script(code: Vec<Instruction>) -> CompiledScript {
        CompiledScript {
            name: None,
            local_count_int: 4,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code,
        }
    }

    #[test]
    fn inline_pass_collects_varbit_and_var_ids_precisely() {
        let s = script(vec![
            ins(
                "push_varbit",
                Operand::VarBitRef(VarBitRef {
                    id: 53898,
                    transmog: false,
                }),
            ),
            ins(
                "pop_var",
                Operand::VarRef(VarRef {
                    domain: VarDomain::Player,
                    id: 1234,
                    transmog: false,
                }),
            ),
            ins("return", Operand::Int(0)),
        ]);
        let dc = compute(&[(1, &s)], 948);
        assert_eq!(dc.refs["varbit"], BTreeSet::from([53898]));
        assert_eq!(dc.refs["varplayer"], BTreeSet::from([1234]));
        assert_eq!(dc.scripts_scanned, 1);
    }

    #[test]
    fn typed_constant_pass_attributes_enum_id_to_the_enum_arg() {
        // push inttype, outtype, ENUMID(constant), key(constant); enum(...) → the
        // 3rd arg (typed `enum`) is the constant 7421 → recorded as an enum ref.
        let s = script(vec![
            ins("push_constant_int", Operand::Int(0)),    // inputtype
            ins("push_constant_int", Operand::Int(0)),    // outputtype
            ins("push_constant_int", Operand::Int(7421)), // enum id
            ins("push_constant_int", Operand::Int(5)),    // key
            ins("enum", Operand::Byte(0)),
            ins("pop_int_discard", Operand::Int(0)),
            ins("return", Operand::Int(0)),
        ]);
        let dc = compute(&[(1, &s)], 948);
        assert_eq!(
            dc.refs.get("enum"),
            Some(&BTreeSet::from([7421])),
            "the enum-id constant must be attributed to the `enum` argument"
        );
    }

    #[test]
    fn non_constant_config_arg_is_counted_as_dynamic_not_guessed() {
        // The enum id comes from a LOCAL, not a constant → recorded as dynamic, with
        // no bogus id invented.
        let s = script(vec![
            ins("push_constant_int", Operand::Int(0)),
            ins("push_constant_int", Operand::Int(0)),
            ins("push_int_local", Operand::Local(0)), // enum id from a local
            ins("push_constant_int", Operand::Int(5)),
            ins("enum", Operand::Byte(0)),
            ins("pop_int_discard", Operand::Int(0)),
            ins("return", Operand::Int(0)),
        ]);
        let dc = compute(&[(1, &s)], 948);
        assert!(
            dc.refs.get("enum").is_none_or(BTreeSet::is_empty),
            "no enum id should be resolved from a non-constant arg"
        );
        assert_eq!(dc.dynamic.get("enum"), Some(&1));
    }

    #[test]
    fn primitive_arg_constants_are_not_recorded_as_refs() {
        // `enum`'s key arg is a plain `int`; its constant (5) must NOT be a ref.
        let s = script(vec![
            ins("push_constant_int", Operand::Int(0)),
            ins("push_constant_int", Operand::Int(0)),
            ins("push_constant_int", Operand::Int(7421)),
            ins("push_constant_int", Operand::Int(5)),
            ins("enum", Operand::Byte(0)),
            ins("pop_int_discard", Operand::Int(0)),
            ins("return", Operand::Int(0)),
        ]);
        let dc = compute(&[(1, &s)], 948);
        // Only the enum id is a ref; 5 (the int key) is not, and 0/0 (type args) are not.
        assert_eq!(dc.total_refs(), 1);
    }
}
