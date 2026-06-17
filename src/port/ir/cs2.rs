//! The typed CS2 intermediate representation (plan §4.1).
//!
//! This is the build-neutral, in-memory model the port passes operate on. It is
//! a *typed lift* of [`crate::script::CompiledScript`]: every construct addresses
//! its semantic identity (an opcode by NAME, a call by a [`ProcIdentity`], a
//! db-field by `{table, column, tuple}`) rather than a build-specific encoding.
//! The build's *encoding* of a value (db-field bit packing, the proc id) lives in
//! the [`crate::port::book::BuildDescriptor`], so a 948→910 "rewrite" becomes
//! "two builds' encodings of one ref", checked by construction at encode time.
//!
//! The reversible `@cs2` asm (see [`crate::script::script_to_asm`]) is the IR's
//! *serialization*; the typed IR here is the in-memory form. Lossless conversion
//! to/from [`CompiledScript`] ([`Cs2Ir::from_compiled`] / [`Cs2Ir::to_compiled`])
//! anchors byte-exactness: a port that round-trips through the IR and re-emits the
//! asm reproduces the committed artifacts exactly.

use crate::error::{Context, Result};
use crate::script::{
    CompiledScript, Instruction, Operand as RawOperand, SwitchCase, VarBitRef, VarRef,
};

/// A script's typed local/argument counts (the `(int, obj, long)` triples from
/// the header). Build-neutral.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub local_int: u16,
    pub local_obj: u16,
    pub local_long: u16,
    pub arg_int: u16,
    pub arg_obj: u16,
    pub arg_long: u16,
}

/// A stable identity for a callee proc, independent of its raw id in any build.
///
/// Two scripts at the *same* numeric id in 948 and 910 are the *same* proc iff
/// their identities match; when they differ it is a proc-id collision (plan §6)
/// and the encoder must allocate a fresh target id. Today the identity is the
/// callee's declared argument signature plus the raw id it was decoded under —
/// the same key the transitive-collision analysis compares on
/// ([`crate::explain_transitive::ScriptCollision`]). The raw id is retained so a
/// pass that has no descriptor-level remap can still serialize the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProcIdentity {
    /// The raw script id the call carried in the *source* build.
    pub source_id: i32,
}

impl ProcIdentity {
    #[must_use]
    pub fn from_source_id(source_id: i32) -> Self {
        Self { source_id }
    }
}

/// A db-field reference, semantically `{table, column, tuple}`. The build's *bit
/// packing* of this triple lives in the descriptor (948 packs `t<<12|c<<4|tuple`,
/// 910 packs `t<<8|c`), so the `>>4` repack is "two encodings of one ref" rather
/// than a text rewrite. Carried inside a `push_constant_string int:N` whose value
/// the source descriptor recognised as a packed field id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbField {
    pub table: u32,
    pub column: u32,
    pub tuple: u32,
}

/// A typed IR operand. Each kind is the *semantic* value; the encoding lives in
/// the descriptor. Mirrors [`crate::script::Operand`] but lifts calls to a
/// [`ProcIdentity`] and (optionally) db-field constants to a typed [`DbField`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    /// `push_constant_int` operand.
    IntConst(i32),
    /// `push_long_constant` / typed-long operand.
    LongConst(i64),
    /// `push_constant_string str:"…"` operand.
    StrConst(String),
    /// A typed `push_constant_string int:N` (the 948 typed-constant int form).
    /// Kept distinct from [`Operand::IntConst`] because the wire encoding differs
    /// (`push_constant_string` carries a type tag; `push_constant_int` does not).
    TypedIntConst(i32),
    /// A typed-constant int that the source descriptor decoded as a packed
    /// db-field id. Re-encoded through the *target* descriptor's packing.
    DbFieldConst(DbField),
    /// `push_*_local` / `pop_*_local` slot index.
    LocalRef(i32),
    /// A var reference (`push_var`/`pop_var` and the fixed-domain var commands).
    VarRef(VarRef),
    /// A varbit reference (`push_varbit`/`pop_varbit` and fixed varbit commands).
    VarBitRef(VarBitRef),
    /// An absolute jump target (instruction index), as the reversible asm encodes
    /// branch operands.
    Jump(i32),
    /// A `switch` table: `(value → absolute target index)` pairs.
    Switch(Vec<SwitchCase>),
    /// A `gosub_with_params` call, carrying the callee's identity (not a raw id).
    Call(ProcIdentity),
    /// A `define_array` / array-op id.
    ArrayRef(i32),
    /// A `join_string` operand count.
    Count(i32),
    /// A raw 1-byte operand for an opcode with no structured operand.
    Byte(u8),
    /// A raw 32-bit operand for a large-operand opcode with no structured form
    /// (the `raw32:` serialization).
    Raw32(i32),
    /// No operand.
    None,
}

/// One IR instruction: a semantically-named opcode plus a typed operand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insn {
    /// The opcode's semantic NAME, resolved from the *source* book (e.g.
    /// `cc_setondropdownselect`, `sub`, `_enum`). The target encoder resolves the
    /// numeric id from the *target* book; an op the target book lacks is an
    /// unrepresentable construct (plan §6).
    pub op: String,
    /// The typed operand.
    pub operand: Operand,
}

impl Insn {
    /// A no-operand instruction.
    #[must_use]
    pub fn bare(op: impl Into<String>) -> Self {
        Self {
            op: op.into(),
            operand: Operand::None,
        }
    }
}

/// A whole decoded CS2 script as typed IR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cs2Ir {
    /// The optional script export name (carried verbatim through the pipeline).
    pub name: Option<String>,
    pub header: Header,
    pub code: Vec<Insn>,
}

impl Cs2Ir {
    /// Lift a decoded [`CompiledScript`] into typed IR. Lossless: every operand
    /// variant maps to a typed IR operand, so [`Self::to_compiled`] inverts it.
    ///
    /// `db_field_decode` is the *source* descriptor's hook that recognises a typed
    /// int constant as a packed db-field id; when it returns `Some(DbField)` the
    /// operand is lifted to [`Operand::DbFieldConst`]. Pass a closure returning
    /// `None` to keep all int constants opaque.
    pub fn from_compiled(
        script: &CompiledScript,
        db_field_decode: &dyn Fn(i32) -> Option<DbField>,
    ) -> Self {
        let header = Header {
            local_int: script.local_count_int,
            local_obj: script.local_count_object,
            local_long: script.local_count_long,
            arg_int: script.argument_count_int,
            arg_obj: script.argument_count_object,
            arg_long: script.argument_count_long,
        };
        let code = script
            .code
            .iter()
            .map(|instr| Insn {
                op: instr.command.clone(),
                operand: lift_operand(&instr.command, &instr.operand, db_field_decode),
            })
            .collect();
        Self {
            name: script.name.clone(),
            header,
            code,
        }
    }

    /// Lower the IR back to a [`CompiledScript`] (the form [`crate::script::script_to_asm`]
    /// and [`crate::script::encode_script`] consume).
    ///
    /// `db_field_encode` is the *target* descriptor's packer for a [`DbField`]
    /// (the inverse of the decode hook); `call_id` resolves a [`ProcIdentity`] to
    /// the target raw id (the proc-allocation result). Both are pure data lookups.
    pub fn to_compiled(
        &self,
        db_field_encode: &dyn Fn(&DbField) -> i32,
        call_id: &dyn Fn(&ProcIdentity) -> Result<i32>,
    ) -> Result<CompiledScript> {
        let mut code = Vec::with_capacity(self.code.len());
        for insn in &self.code {
            code.push(Instruction {
                opcode: 0,
                command: insn.op.clone(),
                operand: lower_operand(&insn.op, &insn.operand, db_field_encode, call_id)?,
            });
        }
        Ok(CompiledScript {
            name: self.name.clone(),
            local_count_int: self.header.local_int,
            local_count_object: self.header.local_obj,
            local_count_long: self.header.local_long,
            argument_count_int: self.header.arg_int,
            argument_count_object: self.header.arg_obj,
            argument_count_long: self.header.arg_long,
            code,
        })
    }
}

/// Lift a raw decoded operand into a typed IR operand. The command name
/// disambiguates the `push_constant_string` family (int / long / str typed
/// constants) from `push_constant_int`.
fn lift_operand(
    command: &str,
    operand: &RawOperand,
    db_field_decode: &dyn Fn(i32) -> Option<DbField>,
) -> Operand {
    match operand {
        RawOperand::Int(v) => {
            if command == "push_constant_string" {
                // A typed int constant. Try to recognise it as a packed db-field.
                if let Some(field) = db_field_decode(*v) {
                    Operand::DbFieldConst(field)
                } else {
                    Operand::TypedIntConst(*v)
                }
            } else if command == "push_constant_int" {
                Operand::IntConst(*v)
            } else {
                // A large-operand opcode's raw32, or an Int used where the asm
                // formatter expects a structured slot. The asm round-trip treats
                // unknown-command Int operands as `raw32:`.
                Operand::Raw32(*v)
            }
        }
        RawOperand::Long(v) => Operand::LongConst(*v),
        RawOperand::Str(s) => Operand::StrConst(s.clone()),
        RawOperand::Local(v) => Operand::LocalRef(*v),
        RawOperand::VarRef(vr) => Operand::VarRef(vr.clone()),
        RawOperand::VarBitRef(vbr) => Operand::VarBitRef(vbr.clone()),
        RawOperand::Branch(t) => Operand::Jump(*t),
        RawOperand::Switch(cases) => Operand::Switch(cases.clone()),
        RawOperand::Script(id) => Operand::Call(ProcIdentity::from_source_id(*id)),
        RawOperand::Array(v) => Operand::ArrayRef(*v),
        RawOperand::Count(v) => Operand::Count(*v),
        RawOperand::Byte(b) => Operand::Byte(*b),
    }
}

/// Lower a typed IR operand back to the raw operand the encoder/asm-writer want.
fn lower_operand(
    command: &str,
    operand: &Operand,
    db_field_encode: &dyn Fn(&DbField) -> i32,
    call_id: &dyn Fn(&ProcIdentity) -> Result<i32>,
) -> Result<RawOperand> {
    Ok(match operand {
        Operand::IntConst(v) => RawOperand::Int(*v),
        Operand::TypedIntConst(v) => RawOperand::Int(*v),
        Operand::DbFieldConst(field) => RawOperand::Int(db_field_encode(field)),
        Operand::LongConst(v) => RawOperand::Long(*v),
        Operand::StrConst(s) => RawOperand::Str(s.clone()),
        Operand::LocalRef(v) => RawOperand::Local(*v),
        Operand::VarRef(vr) => RawOperand::VarRef(vr.clone()),
        Operand::VarBitRef(vbr) => RawOperand::VarBitRef(vbr.clone()),
        Operand::Jump(t) => RawOperand::Branch(*t),
        Operand::Switch(cases) => RawOperand::Switch(cases.clone()),
        Operand::Call(identity) => {
            RawOperand::Script(call_id(identity).with_context(|| {
                format!("resolving call target for proc {}", identity.source_id)
            })?)
        }
        Operand::ArrayRef(v) => RawOperand::Array(*v),
        Operand::Count(v) => RawOperand::Count(*v),
        Operand::Byte(b) => RawOperand::Byte(*b),
        Operand::Raw32(v) => RawOperand::Int(*v),
        Operand::None => {
            // No-operand opcode. The asm writer/encoder ignore the operand for a
            // bare instruction, but a Byte(0) is the canonical placeholder that
            // round-trips identically (matching `decode_operand`'s default).
            let _ = command;
            RawOperand::Byte(0)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::script_to_asm;

    fn no_db(_: i32) -> Option<DbField> {
        None
    }

    #[test]
    fn round_trips_compiled_to_ir_and_back() -> Result<()> {
        let script = CompiledScript {
            name: Some("probe".to_string()),
            local_count_int: 2,
            local_count_object: 1,
            local_count_long: 0,
            argument_count_int: 1,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".into(),
                    operand: RawOperand::Int(60163),
                },
                Instruction {
                    opcode: 0,
                    command: "push_int_local".into(),
                    operand: RawOperand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "gosub_with_params".into(),
                    operand: RawOperand::Script(5360),
                },
                Instruction {
                    opcode: 0,
                    command: "branch".into(),
                    operand: RawOperand::Branch(5),
                },
                Instruction {
                    opcode: 0,
                    command: "return".into(),
                    operand: RawOperand::Byte(0),
                },
            ],
        };
        let ir = Cs2Ir::from_compiled(&script, &no_db);
        // Identity call resolution: source id passes through.
        let back = ir.to_compiled(&|f| (f.table << 8 | f.column) as i32, &|p| Ok(p.source_id))?;
        // The asm serialization of the round-trip must be byte-identical.
        assert_eq!(script_to_asm(&script), script_to_asm(&back));
        Ok(())
    }

    #[test]
    fn db_field_lift_uses_typed_field() -> Result<()> {
        let script = CompiledScript {
            name: None,
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![Instruction {
                opcode: 0,
                command: "push_constant_string".into(),
                // 962611 = table 235, col 3, tuple 3 in the 948 packing.
                operand: RawOperand::Int(962_611),
            }],
        };
        let decode = |v: i32| {
            if v == 962_611 {
                Some(DbField {
                    table: 235,
                    column: 3,
                    tuple: 3,
                })
            } else {
                None
            }
        };
        let ir = Cs2Ir::from_compiled(&script, &decode);
        assert!(matches!(ir.code[0].operand, Operand::DbFieldConst(_)));
        // Re-pack through the 910 layout (t<<8|c) → 60163.
        let back = ir.to_compiled(
            &|f| ((f.table << 8) | f.column) as i32,
            &|p| Ok(p.source_id),
        )?;
        assert!(script_to_asm(&back).contains("push_constant_string int:60163"));
        Ok(())
    }
}
