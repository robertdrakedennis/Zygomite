//! Per-build `BuildDescriptor` (plan §5): one typed source of truth for what a
//! build's CS2 can express and how it encodes it.
//!
//! Promotes the *loose* data already in the crate — `data/opcodes-{910,948}.txt`
//! (via [`crate::script::OpcodeBook`]), `data/cs2/registry-910.json`,
//! `data/stack-effects.txt`, and the book-free arg-signature reader
//! ([`crate::script::decode_script_arg_signature`]) — into a single
//! [`BuildDescriptor`] that the decode / represent / encode stages consume,
//! instead of each tool re-deriving it.
//!
//! Two facets matter for milestone 1:
//!  * the **opcode table** — `name ↔ id ↔ (pops,pushes) ↔ large-operand`, so the
//!    encoder can resolve a target id, reject an op the target lacks, and check
//!    stack balance;
//!  * the **db-field packing** — how a `{table,column,tuple}` triple packs into a
//!    field-id int, so the `>>4` repack is an encoding difference, not a rewrite.
//!
//! Capability flags (`cc_list`, `cc_radiogroup`, the modern-font gap, …) model the
//! 910 client-engine seam (plan §8) so `port plan` can report which donor
//! constructs the target cannot represent.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{Context, Result};
use crate::port::ir::cs2::DbField;
use crate::script::OpcodeBook;

/// How a build packs a db-field `{table, column, tuple}` into the int constant a
/// `push_constant_string int:N` carries. 948 packs `t<<12 | c<<4 | tuple`; 910
/// packs `t<<8 | c` (the tuple selector is dropped — 910's `db_getfield` returns
/// the whole tuple row). Modelled as data so the repack is `decode∘encode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbFieldPacking {
    /// Left-shift applied to the table id.
    pub table_shift: u32,
    /// Left-shift applied to the column id (and the width, in bits, of the tuple
    /// field below it). For 910 the tuple is absent, so `column_shift == 0`.
    pub column_shift: u32,
    /// Whether this build encodes a tuple selector in the low bits (width =
    /// `column_shift`). 948: true; 910: false.
    pub has_tuple: bool,
}

impl DbFieldPacking {
    /// The 948 donor packing: `table<<12 | column<<4 | tuple`.
    pub const DONOR_948: Self = Self {
        table_shift: 12,
        column_shift: 4,
        has_tuple: true,
    };
    /// The 910 base packing: `table<<8 | column`.
    pub const BASE_910: Self = Self {
        table_shift: 8,
        column_shift: 0,
        has_tuple: false,
    };

    /// Decode a packed field-id int into its `{table, column, tuple}` triple.
    #[must_use]
    pub fn decode(&self, packed: i32) -> DbField {
        let packed = packed as u32;
        let table = packed >> self.table_shift;
        if self.has_tuple {
            let column = (packed >> self.column_shift) & ((1 << (self.table_shift - self.column_shift)) - 1);
            let tuple = packed & ((1 << self.column_shift) - 1);
            DbField {
                table,
                column,
                tuple,
            }
        } else {
            let column = packed & ((1 << self.table_shift) - 1);
            DbField {
                table,
                column,
                tuple: 0,
            }
        }
    }

    /// Encode a `{table, column, tuple}` triple back into a packed field-id int.
    #[must_use]
    pub fn encode(&self, field: &DbField) -> i32 {
        let packed = if self.has_tuple {
            (field.table << self.table_shift)
                | (field.column << self.column_shift)
                | field.tuple
        } else {
            (field.table << self.table_shift) | field.column
        };
        packed as i32
    }
}

/// Per-opcode static stack effect (pops/pushes by type), read from
/// `data/stack-effects.txt` (the table the client ScriptRunner enforces). `None`
/// for variadic / callee-dependent opcodes, which the stack checker treats as
/// unresolvable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackEffect {
    pub int_pops: i64,
    pub obj_pops: i64,
    pub long_pops: i64,
    pub int_pushes: i64,
    pub obj_pushes: i64,
    pub long_pushes: i64,
}

/// Target-capability flags (plan §8). When the 910 client gains a capability the
/// flag flips and the corresponding lowering is no longer needed (the construct
/// becomes directly representable). These are the single authoritative answer to
/// "is the 910 client ready for X?".
// reason: each bool is an independent, named client-capability flag (not a state
// machine) — grouping them into an enum/bitflags would obscure the per-feature API.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// The `cc_list` / dropdown component family (component type 16 +
    /// `cc_setondropdownselect`). False on 910 → the list/dropdown must be lowered
    /// to a server-driven layer (or the construct fails loud).
    pub cc_list: bool,
    /// The `cc_radiogroup_*` opcode family. False on 910.
    pub cc_radiogroup: bool,
    /// Whether the cc-model addresses subcomponents by a sparse data id (true) or
    /// requires dense ids (false). Post the 2026-06-13 client change this is
    /// `sparse = true` on 910.
    pub cc_model_sparse: bool,
    /// Whether the build's stylesheet text-colour system (the `cc_setstylesheet`
    /// opcode family + the skin/text db tables) is present. False on 910 → the
    /// stylesheet chain is stubbed / lowered to constant colours.
    pub stylesheet: bool,
    /// Whether the build can decode modern (fontmetrics2 / ttf) fonts directly.
    /// False on 910 → modern fonts are pre-rasterized to bitmap (`font rasterize`)
    /// and `port plan` reports the gap.
    pub modern_fonts: bool,
}

impl Capabilities {
    /// The 910 base client's capabilities as of this session (plan §8): sparse
    /// cc-ids landed; list/dropdown/radiogroup queued; no stylesheet text-colour
    /// system; no modern-font decoder.
    pub const BASE_910: Self = Self {
        cc_list: false,
        cc_radiogroup: false,
        cc_model_sparse: true,
        stylesheet: false,
        modern_fonts: false,
    };
    /// The 948 donor build expresses all of these natively.
    pub const DONOR_948: Self = Self {
        cc_list: true,
        cc_radiogroup: true,
        cc_model_sparse: true,
        stylesheet: true,
        modern_fonts: true,
    };
}

/// The build's INTERFACE capabilities (plan §5 interface facet): the component
/// wire version its `Component.decode` handles, and the set of component types it
/// has a decode body for. The 910 decoder is version-led up to 9 and implements
/// bodies only for the primitive types; the 948 donor wire version is 11 with the
/// composite-widget bodies. Modelled so `represent_interface` answers "can the
/// target encode this component type?" from data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceCaps {
    /// The highest component wire version the build's `Component.decode` fully
    /// handles (910: 9; 948: 11). Transcoded components are written at the target's
    /// version.
    pub wire_version: u8,
    /// The numeric component `type` ids the build has a decode body for. The 910
    /// decoder implements only the primitives {0,3,4,5,6,9}; the 948 donor also has
    /// the composite-widget bodies.
    pub body_types: &'static [u8],
}

impl InterfaceCaps {
    /// The 910 base: wire version 9, primitive component bodies only (the set
    /// `Component.decode` has a case for — mirrors
    /// [`crate::interface::transcode::TYPES_910_HAS_BODY`]).
    pub const BASE_910: Self = Self {
        wire_version: 9,
        body_types: &[0, 3, 4, 5, 6, 9],
    };
    /// The 948 donor: wire version 11, every component type (primitives + the
    /// composite widgets button/panel/check/input/grid/list/crmview).
    pub const DONOR_948: Self = Self {
        wire_version: 11,
        body_types: &[0, 3, 4, 5, 6, 9, 10, 11, 12, 13, 15, 16, 26],
    };

    /// Whether the build's `Component.decode` has a body for component `type_id`.
    #[must_use]
    pub fn has_body_for(&self, type_id: u8) -> bool {
        self.body_types.contains(&type_id)
    }
}

/// A build's complete typed descriptor: opcode table, stack effects, db-field
/// packing, and capability flags.
pub struct BuildDescriptor {
    /// The build number (910 or 948).
    pub build: u32,
    /// The opcode table (`name ↔ id`, large-operand flags, alias map).
    pub opcodes: OpcodeBook,
    /// Static per-opcode stack effects, keyed by canonical name. Empty entries
    /// (variadic ops) are simply absent.
    stack: HashMap<String, StackEffect>,
    /// How this build packs db-field ids.
    pub db_packing: DbFieldPacking,
    /// What this build's client can represent.
    pub capabilities: Capabilities,
    /// The build's interface component wire version + supported component bodies.
    pub interface_caps: InterfaceCaps,
}

impl BuildDescriptor {
    /// Build the descriptor for build 910 or 948 from the crate's `data/` dir.
    pub fn load(data_dir: &Path, build: u32) -> Result<Self> {
        let opcodes = OpcodeBook::load(data_dir, build, u32::from(build == 948))
            .with_context(|| format!("load opcode book for build {build}"))?;
        let stack = load_stack_effects(data_dir)?;
        let (db_packing, capabilities, interface_caps) = match build {
            910 => (
                DbFieldPacking::BASE_910,
                Capabilities::BASE_910,
                InterfaceCaps::BASE_910,
            ),
            948 => (
                DbFieldPacking::DONOR_948,
                Capabilities::DONOR_948,
                InterfaceCaps::DONOR_948,
            ),
            other => {
                crate::cache_bail!("BuildDescriptor only supports builds 910 and 948, got {other}")
            }
        };
        Ok(Self {
            build,
            opcodes,
            stack,
            db_packing,
            capabilities,
            interface_caps,
        })
    }

    /// Whether the target opcode book can encode `op` (directly or via an alias).
    #[must_use]
    pub fn has_op(&self, op: &str) -> bool {
        self.opcodes.opcode_for(op).is_ok()
    }

    /// Whether the opcode book knows `op` as a CANONICAL command (not only via an
    /// alias). A non-canonical mnemonic (e.g. `enum` → canonical `_enum`) trips
    /// the assembler's fidelity gate and must be renamed before encode.
    #[must_use]
    pub fn has_canonical_op(&self, op: &str) -> bool {
        self.opcodes.by_name().contains_key(op)
    }

    /// The canonical name `op` resolves to (following aliases), or `None`.
    #[must_use]
    pub fn canonical_of(&self, op: &str) -> Option<String> {
        let id = self.opcodes.opcode_for(op).ok()?;
        self.opcodes.name(id).ok().map(ToString::to_string)
    }

    /// The static stack effect of `op`, or `None` if variadic / unknown.
    #[must_use]
    pub fn stack_effect(&self, op: &str) -> Option<StackEffect> {
        self.stack.get(op).copied()
    }

    /// Decode a packed db-field int through this build's packing.
    #[must_use]
    pub fn decode_db_field(&self, packed: i32) -> DbField {
        self.db_packing.decode(packed)
    }

    /// Encode a db-field triple through this build's packing.
    #[must_use]
    pub fn encode_db_field(&self, field: &DbField) -> i32 {
        self.db_packing.encode(field)
    }
}

/// Parse `data/stack-effects.txt` into a name→[`StackEffect`] map. Mirrors the
/// loader in `cs2/lint.rs` (lifted here as the descriptor's single owner).
fn load_stack_effects(data_dir: &Path) -> Result<HashMap<String, StackEffect>> {
    let path = data_dir.join("stack-effects.txt");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read stack effects {}", path.display()))?;
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() != 7 {
            continue;
        }
        let n = |i: usize| cols[i].parse::<i64>().ok();
        if let (Some(ip), Some(op), Some(lp), Some(ipu), Some(opu), Some(lpu)) =
            (n(1), n(2), n(3), n(4), n(5), n(6))
        {
            map.insert(
                cols[0].to_string(),
                StackEffect {
                    int_pops: ip,
                    obj_pops: op,
                    long_pops: lp,
                    int_pushes: ipu,
                    obj_pushes: opu,
                    long_pushes: lpu,
                },
            );
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
    }

    #[test]
    fn db_field_948_to_910_repack_matches_shift_right_four() {
        // 962611 (948) decodes to table 235, col 3, tuple 3; re-encoding through
        // the 910 packing gives 60163 — exactly the `>>4` the Python applied.
        let field = DbFieldPacking::DONOR_948.decode(962_611);
        assert_eq!(field.table, 235);
        assert_eq!(field.column, 3);
        assert_eq!(field.tuple, 3);
        assert_eq!(DbFieldPacking::BASE_910.encode(&field), 60163);
        assert_eq!(962_611 >> 4, 60163);
    }

    #[test]
    fn db_field_repack_equals_shift_for_every_ritual_field() {
        // Every db-field the ritual splice touches re-packs to exactly `v >> 4`.
        for v in [
            958_480, 958_545, 958_592, 962_560, 962_576, 962_592, 962_608, 962_609, 962_610, 962_611, 962_640,
            962_656, 962_672, 962_688, 962_704, 962_720, 962_736, 962_768, 962_784, 962_800, 962_832, 966_674,
            966_704, 966_736, 966_768, 966_784, 966_800, 966_816,
        ] {
            let field = DbFieldPacking::DONOR_948.decode(v);
            assert_eq!(
                DbFieldPacking::BASE_910.encode(&field),
                v >> 4,
                "field {v} did not repack to v>>4"
            );
        }
    }

    #[test]
    fn loads_910_descriptor_with_stack_effects() -> Result<()> {
        let d = BuildDescriptor::load(&data_dir(), 910)?;
        assert!(d.has_op("push_constant_string"));
        assert!(d.has_op("_enum"));
        // `enum` resolves only via alias on 910.
        assert!(!d.has_canonical_op("enum"));
        assert_eq!(d.canonical_of("enum").as_deref(), Some("_enum"));
        // `sub` is absent on 910.
        assert!(!d.has_op("sub"));
        let add = d.stack_effect("add").expect("add has a static effect");
        assert_eq!(add.int_pops, 2);
        assert_eq!(add.int_pushes, 1);
        Ok(())
    }

    #[test]
    fn loads_948_descriptor_with_sub_opcode() -> Result<()> {
        let d = BuildDescriptor::load(&data_dir(), 948)?;
        assert!(d.has_op("sub"));
        assert!(d.has_op("enum"));
        assert!(d.capabilities.cc_list);
        assert!(d.capabilities.stylesheet);
        Ok(())
    }
}
