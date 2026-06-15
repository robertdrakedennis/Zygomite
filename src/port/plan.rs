//! `port plan` (plan §10): the representability dry-run over an interface's whole
//! CS2 closure — the "enumerate the 948→910 delta up front" that the old flow
//! lacked. Subsumes the manual `explain-interface --transitive` closure + the
//! proc-collision detection + the component-type / cc-model gaps into one report.
//!
//! It reuses the existing closure machinery ([`crate::explain::compute_transitive`])
//! for the proc collisions + missing-from-910 set, then runs
//! [`crate::port::represent`] over each decoded donor script for the opcode-level
//! deltas (missing ops, arity drift, op renames, db-field packing), and appends the
//! target-capability gaps (cc_list / cc_radiogroup / stylesheet / modern fonts)
//! from the 910 descriptor.
//!
//! Opcode deltas are tagged `in_port` (rendered `►`) when they occur in a script the
//! port actually emits — the `missing_from_910` splice set — vs merely elsewhere in
//! the closure (`·`). Only the in-port count is a real blocker; the closure-wide
//! total is context, since any large 948 closure contains long-arith / `db_filter_*`
//! / `ui_*` opcodes that no ported script touches. The `verdict` line reports the
//! scoped blocker count. (`missing_from_910` is a slight *superset* of a built
//! driver's emit-set — a driver may stub/prune some splice scripts — so for a
//! dry-run before a driver exists it errs toward surfacing more, which is the safe
//! direction.)

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::cache::FlatCache;
use crate::constants::ARCHIVE_CLIENTSCRIPTS;
use crate::error::{Context, Result};
use crate::explain::{self, ExplainInterfaceOptions, InterfaceSource, TransitiveOptions};
use crate::port::book::BuildDescriptor;
use crate::port::ir::cs2::{Cs2Ir, DbField};
use crate::port::represent::{self, Bridge, Finding, FindingKind};
use crate::script::{OpcodeBook, ScriptArgSignature, decode_script};

/// Options for [`run`].
pub struct PlanOptions<'a> {
    /// The interface whose CS2 closure to analyse.
    pub interface: u32,
    /// Donor build (948).
    pub from: u32,
    /// Target build (910).
    pub to: u32,
    /// Donor flat cache (decodes the closure scripts).
    pub donor_cache: &'a FlatCache,
    /// Donor pack root (for the interface's depth-1 component-bound scripts).
    pub donor_pack_root: &'a Path,
    /// 910-base scripts pack root (the roster for the collision/missing analysis).
    pub base_pack_root: &'a Path,
    /// Crate data dir (opcode books, stack effects, descriptors).
    pub data_dir: &'a Path,
    /// Emit JSON instead of the human report.
    pub json: bool,
}

/// One proc collision in the report (an id present in both builds as different
/// procs). Mirrors [`crate::explain_transitive::ScriptCollision`] for serialization.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ProcCollisionReport {
    pub group: u32,
    pub donor: ScriptArgSignature,
    pub base: ScriptArgSignature,
}

/// An opcode-level [`Finding`] tagged with whether it occurs in a script the port
/// actually emits. `in_port = true` means the construct appears in a
/// `missing_from_910` script (donor-new, or a proc collision folded into the
/// splice set) — a real port-work delta. `in_port = false` means it was seen only
/// elsewhere in the transitive closure, in a script the 910 base already provides
/// and the port never re-emits — closure-wide context, not a blocker. Scoping the
/// delta this way is what keeps the plan's UNHANDLED count honest: a large 948
/// closure always contains long-arith / `db_filter_*` / `ui_*` opcodes *somewhere*,
/// but they only matter if a ported script uses them.
#[derive(Clone, Debug, Serialize)]
pub struct ScopedFinding {
    #[serde(flatten)]
    pub finding: Finding,
    pub in_port: bool,
}

/// The full `port plan` report.
#[derive(Clone, Debug, Serialize)]
pub struct PlanReport {
    pub interface: u32,
    pub from: u32,
    pub to: u32,
    /// Size of the full transitive script closure.
    pub closure_len: usize,
    /// Donor-new + colliding scripts the 910 base must be spliced with.
    pub missing_from_910: usize,
    /// Proc-id collisions (id present, different proc) → must be remapped by
    /// `lower::proc_alloc`.
    pub proc_collisions: Vec<ProcCollisionReport>,
    /// Opcode-level representability findings (deduplicated by construct), each
    /// tagged `in_port` if it occurs in a script the port emits (the splice set).
    pub op_findings: Vec<ScopedFinding>,
    /// Per-component interface findings (composite widgets the 910 client lacks a
    /// body for → `lower::list_to_server_driven`; the cc-model gap).
    pub interface_findings: Vec<Finding>,
    /// Target-capability gaps (cc_list / cc_radiogroup / stylesheet / fonts).
    pub capability_findings: Vec<Finding>,
}

/// Compute the plan report (no I/O side effects beyond reads).
pub fn compute(opts: &PlanOptions<'_>) -> Result<PlanReport> {
    if opts.from != 948 || opts.to != 910 {
        crate::cache_bail!(
            "port plan currently supports only --from 948 --to 910 (got {} → {})",
            opts.from,
            opts.to
        );
    }
    let d910 = BuildDescriptor::load(opts.data_dir, opts.to)?;
    let d948 = BuildDescriptor::load(opts.data_dir, opts.from)?;

    // 1) The interface's depth-1 component-bound scripts → the transitive closure
    //    + proc collisions (reusing the existing machinery).
    let explained = explain::explain(&ExplainInterfaceOptions {
        interface: opts.interface,
        build: opts.from,
        source: InterfaceSource::Pack(opts.donor_pack_root),
        json: false,
        transitive: None,
    })?;
    let transitive = explain::compute_transitive(
        &explained.requires.scripts,
        &TransitiveOptions {
            scripts_cache: opts.donor_cache.root(),
            scripts_build: opts.from,
            scripts_subbuild: u32::from(opts.from == 948),
            data_dir: opts.data_dir,
            base_pack_root: opts.base_pack_root,
        },
    )?;

    let proc_collisions: Vec<ProcCollisionReport> = transitive
        .collisions
        .values()
        .map(|c| ProcCollisionReport {
            group: c.group,
            donor: c.donor,
            base: c.base,
        })
        .collect();

    // 2) Decode each closure script and run `represent` for the opcode deltas. We
    //    deduplicate by (kind, construct) so the report lists each delta once with
    //    a representative script.
    let book_948 = OpcodeBook::load(opts.data_dir, opts.from, u32::from(opts.from == 948))?;
    let index = opts
        .donor_cache
        .archive_index(ARCHIVE_CLIENTSCRIPTS)
        .context("read donor clientscripts index")?;
    let mut seen: BTreeMap<(FindingKind, String), ScopedFinding> = BTreeMap::new();
    // db-field recognition: any constant that decodes to a plausible field is left
    // opaque here (the plan reports packing diffs only when the IR lift recognises
    // one); for the dry-run we recognise none (the encoder handles packing).
    let no_db = |_v: i32| -> Option<DbField> { None };
    for &group in &transitive.closure {
        // Does the port actually emit this script? Only the `missing_from_910`
        // splice set (donor-new scripts + proc collisions folded in) is re-emitted;
        // every other closure script the 910 base already provides, so its
        // donor-side opcodes are never ported. A construct seen in ANY emitted
        // script is a real delta (`in_port |= ...` upgrades a closure-only sighting).
        let in_port = transitive.missing_from_910.contains(&group);
        let Ok(files) = opts
            .donor_cache
            .group_files_with_index(&index, ARCHIVE_CLIENTSCRIPTS, group)
        else {
            continue;
        };
        let Some((_, bytes)) = files.into_iter().min_by_key(|(f, _)| *f) else {
            continue;
        };
        let Ok(script) = decode_script(&bytes, &book_948, opts.from) else {
            continue;
        };
        let ir = Cs2Ir::from_compiled(&script, &no_db);
        for f in represent::represent_script(&ir, Some(group as i32), &d910) {
            seen.entry((f.kind, f.construct.clone()))
                .and_modify(|sf| sf.in_port |= in_port)
                .or_insert(ScopedFinding { finding: f, in_port });
        }
    }
    // Keep the deduped findings in a stable order: by kind then construct.
    let op_findings: Vec<ScopedFinding> = seen.into_values().collect();

    // 3) Decode the donor interface group's components and run the interface
    //    representability dry-run (composite-widget bodies the 910 client lacks +
    //    the cc-model). Best-effort: a pack that does not hold the group simply
    //    yields no interface findings.
    let interface_findings = interface_component_findings(opts, &d910).unwrap_or_default();

    let capability_findings = represent::capability_findings(&d910);
    let _ = d948; // descriptor available for future config deltas.

    Ok(PlanReport {
        interface: opts.interface,
        from: opts.from,
        to: opts.to,
        closure_len: transitive.closure_len(),
        missing_from_910: transitive.missing_len(),
        proc_collisions,
        op_findings,
        interface_findings,
        capability_findings,
    })
}

/// Decode the donor interface group's components (from the donor pack) and run
/// [`represent::represent_interface`] over them. Returns `Ok(vec![])` when the
/// donor pack does not hold the group (the interface delta is then simply empty).
fn interface_component_findings(opts: &PlanOptions<'_>, d910: &BuildDescriptor) -> Result<Vec<Finding>> {
    let pack_path = opts.donor_pack_root.join("client.interfaces.js5");
    let pack = crate::js5pack::PackArchive::open(&pack_path)
        .with_context(|| format!("open donor interfaces pack {}", pack_path.display()))?;
    let Some(files) = pack.group_files(opts.interface)? else {
        return Ok(Vec::new());
    };
    // Decode the donor components at the 947/948 layout.
    let ir = crate::port::ir::interface::InterfaceIr::from_donor_files(
        opts.interface,
        &files,
        crate::constants::BUILD,
    )?;
    Ok(represent::represent_interface(&ir, d910))
}

/// Run the plan and print the report.
pub fn run(opts: &PlanOptions<'_>) -> Result<()> {
    let report = compute(opts)?;
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("encode plan report JSON")?
        );
    } else {
        print!("{}", render_human(&report));
    }
    Ok(())
}

/// Render the human report.
fn render_human(report: &PlanReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "port plan — interface {} ({}→{})",
        report.interface, report.from, report.to
    );
    let _ = writeln!(
        out,
        "  CS2 closure: {} scripts ({} missing from {} base, incl. {} proc collisions)",
        report.closure_len,
        report.missing_from_910,
        report.to,
        report.proc_collisions.len()
    );

    let _ = writeln!(out, "\n  proc-id collisions (id present, DIFFERENT proc → remap via lower::proc_alloc):");
    if report.proc_collisions.is_empty() {
        let _ = writeln!(out, "    (none)");
    } else {
        for c in &report.proc_collisions {
            let _ = writeln!(
                out,
                "    script{:<6} donor {} vs {} base {}  → free-id remap",
                c.group,
                c.donor.display(),
                report.to,
                c.base.display()
            );
        }
    }

    let _ = writeln!(out, "\n  opcode-level deltas (►= in a ported script, a real delta · ·= elsewhere in closure, context; each with its named lowering or UNHANDLED):");
    if report.op_findings.is_empty() {
        let _ = writeln!(out, "    (none)");
    } else {
        // In-port deltas first (the real port-work), then closure-only context.
        let mut ordered: Vec<&ScopedFinding> = report.op_findings.iter().collect();
        ordered.sort_by_key(|sf| (!sf.in_port, sf.finding.kind, sf.finding.construct.clone()));
        for sf in ordered {
            let f = &sf.finding;
            let _ = writeln!(
                out,
                "  {} {:<16} {:<28} {:<24} {}",
                if sf.in_port { '►' } else { '·' },
                kind_label(f.kind),
                f.construct,
                bridge_label(f.bridge),
                f.detail
            );
        }
    }

    let _ = writeln!(out, "\n  interface component deltas (composite widgets the target lacks a body for):");
    if report.interface_findings.is_empty() {
        let _ = writeln!(out, "    (none)");
    } else {
        for f in &report.interface_findings {
            let _ = writeln!(
                out,
                "    {:<16} {:<28} {:<24} {}",
                kind_label(f.kind),
                f.construct,
                bridge_label(f.bridge),
                f.detail
            );
        }
    }

    let _ = writeln!(out, "\n  target-capability gaps (the client-engine seam):");
    for f in &report.capability_findings {
        let _ = writeln!(
            out,
            "    {:<16} {:<28} {:<24} {}",
            kind_label(f.kind),
            f.construct,
            bridge_label(f.bridge),
            f.detail
        );
    }

    // Classify each in-port delta — the op findings in emitted (splice-set) scripts
    // plus the interface-component findings (the interface itself IS ported) — by how
    // faithfully it survives:
    //   • FAITHFUL   — semantics preserved (sub→add, common-case arity/tuple drops,
    //                  enum rename, db-field repack, proc realloc).
    //   • LOSSY STUB — encodes, but behaviour is degraded/dropped (list & dropdown →
    //                  server-driven layer; stylesheet/button/check/enabled → neutralised).
    //   • UNHANDLED  — no lowering at all (must stub, prune, or extend the client).
    // This separates "the scripts encode" from "the feature is faithful": a UI can be
    // 100% "handled" yet visibly degraded if it leans on lossy stubs. Byte-exact
    // reproduction of a stubbed listing is still a stub.
    let (mut faithful, mut lossy, mut unhandled) = (0usize, 0usize, 0usize);
    let in_port_bridges = report
        .op_findings
        .iter()
        .filter(|sf| sf.in_port)
        .map(|sf| sf.finding.bridge)
        .chain(report.interface_findings.iter().map(|f| f.bridge));
    for b in in_port_bridges {
        match bridge_class(b) {
            BridgeClass::Faithful => faithful += 1,
            BridgeClass::LossyStub => lossy += 1,
            BridgeClass::Unhandled => unhandled += 1,
        }
    }
    let op_unhandled_total = report.op_findings.iter().filter(|sf| sf.finding.is_unhandled()).count();
    let cap_unhandled = report.capability_findings.iter().filter(|f| f.is_unhandled()).count();
    let _ = writeln!(
        out,
        "\nsummary: {} proc collisions · {} opcode deltas ({} closure-wide UNHANDLED) · {} interface deltas · {} capability gaps ({} UNHANDLED)",
        report.proc_collisions.len(),
        report.op_findings.len(),
        op_unhandled_total,
        report.interface_findings.len(),
        report.capability_findings.len(),
        cap_unhandled,
    );
    let _ = writeln!(
        out,
        "emitted-script faithfulness: {faithful} faithful · {lossy} lossy-stub (degraded) · {unhandled} UNHANDLED"
    );
    let verdict = if unhandled > 0 {
        let extra = if lossy > 0 {
            format!(" · also {lossy} lossy stub(s), degraded even where it encodes")
        } else {
            String::new()
        };
        format!("BLOCKED — {unhandled} in-port construct(s) with no lowering{extra}")
    } else if lossy > 0 {
        format!("DEGRADED — encodes, but {lossy} in-port construct(s) go through a lossy stub (behaviour reduced; NOT a faithful port)")
    } else {
        format!("FAITHFUL — all {faithful} in-port delta(s) are semantics-preserving lowerings")
    };
    let _ = writeln!(out, "verdict: {verdict}");
    out
}

fn kind_label(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::MissingOp => "missing_op",
        FindingKind::ArityDrift => "arity_drift",
        FindingKind::OpRename => "op_rename",
        FindingKind::IdPackingDiff => "id_packing",
        FindingKind::ProcCollision => "proc_collision",
        FindingKind::MissingProc => "missing_proc",
        FindingKind::CapabilityGap => "capability_gap",
        FindingKind::MissingComponentKind => "missing_component",
        FindingKind::CcModelMismatch => "cc_model_mismatch",
    }
}

/// How faithfully a [`Bridge`] preserves the donor construct's behaviour — the axis
/// that distinguishes a real port from a byte-exact stub.
enum BridgeClass {
    /// Semantics preserved (sub→add, common-case arity/tuple drops, enum rename,
    /// db-field repack, proc realloc).
    Faithful,
    /// Encodes, but behaviour is degraded/dropped (list & dropdown → server-driven
    /// layer; the neutralised stylesheet/button/check/enabled opcodes; long localiser).
    LossyStub,
    /// No registered lowering at all.
    Unhandled,
}

fn bridge_class(bridge: Bridge) -> BridgeClass {
    match bridge {
        Bridge::SubToAdd
        | Bridge::TostringDropRadix
        | Bridge::DbfindDropTuple
        | Bridge::EnumRename
        | Bridge::DbfieldRepack
        | Bridge::ProcAlloc => BridgeClass::Faithful,
        Bridge::TostringLocalisedLong
        | Bridge::UnmappedPop1Neutralise
        | Bridge::ListToServerDriven => BridgeClass::LossyStub,
        Bridge::None => BridgeClass::Unhandled,
    }
}

fn bridge_label(bridge: Bridge) -> &'static str {
    match bridge {
        Bridge::SubToAdd => "lower::sub_to_add",
        Bridge::TostringDropRadix => "lower::tostring_drop_radix",
        Bridge::TostringLocalisedLong => "lower::tostring_loc_long",
        Bridge::DbfindDropTuple => "lower::dbfind_drop_tuple",
        Bridge::EnumRename => "lower::enum_rename",
        Bridge::DbfieldRepack => "lower::dbfield_repack",
        Bridge::UnmappedPop1Neutralise => "lower::unmapped_pop1",
        Bridge::ProcAlloc => "lower::proc_alloc",
        Bridge::ListToServerDriven => "lower::list_to_server_driven",
        Bridge::None => "UNHANDLED",
    }
}
