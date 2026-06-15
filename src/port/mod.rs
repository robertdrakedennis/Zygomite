//! The semantic 948→910 port layer (plan: `plans/tooling/semantic-port-layer.md`).
//!
//! A correct-by-construction cross-version compiler that replaces the ad-hoc
//! Python script porters and the after-the-fact gates:
//!
//! ```text
//! decode(srcBook) → typed IR → represent(dstBook) → lower(named passes) → encode(dstBook, validating)
//! ```
//!
//! * [`ir`] — the typed, build-neutral IR (CS2 first; §4.1).
//! * [`book`] — per-build [`book::BuildDescriptor`]s (§5), built from the existing
//!   `data/opcodes-{910,948}.txt` + `data/cs2/registry-910.json` + the
//!   arg-signature reader in `src/script.rs`.
//! * [`represent`] — the representability dry-run (§6): what 948 uses that 910
//!   cannot encode, classified, each with its named lowering or `Unhandled`.
//! * [`lower`] — the named, opt-in IR→IR bridges (§6): `sub_to_add`,
//!   `tostring_drop_radix`, `dbfind_drop_tuple`, `enum_rename`, the
//!   db-field repack, plus the structural proc allocation.
//! * [`encode`] — the validating back-end (§4.1): IR→bytecode with intrinsic
//!   stack-balance, arg-arity, proc-id allocation, and id-packing checks.
//! * [`ritual`] — the milestone-1 driver: re-ports the City of Um ritual selection
//!   (interface 1224) through the layer, the byte-exact oracle against the
//!   committed `.asm.ts`.

pub mod book;
pub mod config;
pub mod encode;
pub mod interface;
pub mod ir;
pub mod lodestone;
pub mod lower;
pub mod material_storage;
pub mod plan;
pub mod relic;
pub mod represent;
pub mod ritual;
