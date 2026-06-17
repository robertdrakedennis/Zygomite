//! Interface back-end: typed [`InterfaceIr`] → 910 component bytes, validating
//! representability (plan §4.2 / §9 step 5).
//!
//! The encoder is *correct-by-construction*: a component whose [`ComponentKind`]
//! the target cannot represent (a composite widget that survived lowering on a
//! target without the `cc_list` family) is a hard encode-time error, not a runtime
//! crash. After the [`crate::port::lower::interface::list_to_server_driven`] pass
//! every component is a primitive kind with a [`Body::Raw`] body, so the encoder
//! emits each as `write_header(version 9, type) · body · tail` — the exact byte
//! sequence [`crate::interface::transcode::transcode_component`] produces, which is
//! how the byte-exact oracle (`1224-910.dat`) holds.
//!
//! Every emitted component is re-decoded through the faithful 910 mirror
//! ([`crate::interface::decode910`]) — the in-process replacement for "run the
//! client and see if it crashes": it must decode with no error and end exactly at
//! end-of-buffer.

use std::collections::BTreeMap;

use crate::cache_bail as bail;
use crate::error::{Context, Result};
use crate::packet::ByteWriter;
use crate::port::book::BuildDescriptor;
use crate::port::ir::interface::{Body, Component, InterfaceIr};

/// The encoded 910 group: per-component bytes (dense id order), the decompressed
/// group body, and the re-packed raw group `.dat`. The byte-stable artifact the
/// interface oracle asserts on is the `.dat` (the committed `1224-910.dat` was
/// produced by this same flate2 path, so it reproduces byte-for-byte).
pub struct EncodedGroup {
    /// Per-component 910 wire bytes in ascending component id.
    pub components: BTreeMap<u32, Vec<u8>>,
    /// The DECOMPRESSED group body (file chunks + footer).
    pub body: Vec<u8>,
    /// The re-packed raw group `.dat` (gzip JS5 container + version trailer).
    pub dat: Vec<u8>,
    /// The dense `0..n` component roster.
    pub roster: Vec<u32>,
}

/// Encode one IR component to its 910 wire bytes, validating representability.
/// `target` gates the composite-widget capability: a non-primitive kind on a
/// target without `cc_list` is an unrepresentable construct → a precise error.
pub fn encode_component(component: &Component, target: &BuildDescriptor) -> Result<Vec<u8>> {
    if !component.kind.is_primitive() && !target.capabilities.cc_list {
        bail!(
            "component kind {:?} (type {}) is not representable on build {}: it is a composite \
             widget with no 910 Component.decode body. Lower it (lower::list_to_server_driven) \
             or extend the client (descriptor.cc_list = true)",
            component.kind,
            component.kind.type_id(),
            target.build
        );
    }

    // After lowering, every component carries a Body::Raw (a primitive body). A
    // Body::Composite that reaches here means the lowering pass was not run.
    let body = match &component.body {
        Body::Raw(bytes) => bytes,
        Body::Composite { .. } => bail!(
            "component kind {:?} still carries a composite body at encode — run \
             lower::list_to_server_driven first",
            component.kind
        ),
    };

    let mut w = ByteWriter::with_capacity(
        component.header_tail.len() + body.len() + component.tail.len() + 2,
    );
    // Re-emit the header with the forced target version (9) + the component type,
    // preserving the name-present bit. `write_header` takes the ORIGINAL header
    // bytes (version + type + header-tail) to read the name bit; reconstruct that
    // slice from the typed fields.
    let mut header = Vec::with_capacity(2 + component.header_tail.len());
    header.push(version_byte(component.version));
    header.push(component.kind.type_id() | if component.name_bit { 0x80 } else { 0 });
    header.extend_from_slice(&component.header_tail);
    crate::interface::transcode::write_header(
        &mut w,
        &header,
        crate::port::lower::interface::TARGET_VERSION,
        component.kind.type_id(),
    );
    w.pdata(body);
    w.pdata(&component.tail);
    Ok(w.data)
}

/// The wire byte for a `version` field (`-1` → `255`).
fn version_byte(version: i16) -> u8 {
    if version == -1 { 0xFF } else { version as u8 }
}

/// Encode every component of the (already-lowered) IR group, validate each through
/// the 910 mirror, and re-pack into a raw group `.dat`. The roster must be the
/// dense `0..n` range the interface archive uses.
pub fn encode_group(
    ir: &InterfaceIr,
    target: &BuildDescriptor,
    version: u16,
) -> Result<EncodedGroup> {
    let mut components = BTreeMap::new();
    for (id, component) in ir.components.iter().enumerate() {
        let id = id as u32;
        let bytes = encode_component(component, target)
            .with_context(|| format!("encode component {id}"))?;
        // Validate through the faithful 910 mirror: no misalignment, exact size.
        let decoded = crate::interface::decode910::decode_component_910(&bytes).map_err(|e| {
            crate::error::CacheError::message(format!(
                "910 mirror rejected encoded component {id}: {e}"
            ))
        })?;
        if decoded.end_pos != bytes.len() {
            bail!(
                "encoded component {id} not exactly buffer-sized: end_pos {} != len {}",
                decoded.end_pos,
                bytes.len()
            );
        }
        components.insert(id, bytes);
    }

    let roster: Vec<u32> = components.keys().copied().collect();
    // Dense, contiguous from 0 — the 1-stripe packer needs it (matches the
    // interface archive's id space).
    for (expected, &id) in roster.iter().enumerate() {
        let expected = expected as u32;
        if id != expected {
            bail!("interface group roster is not dense from 0 (got {id} at slot {expected})");
        }
    }
    let ordered: Vec<Vec<u8>> = roster.iter().map(|id| components[id].clone()).collect();
    let body = crate::interface::transcode::pack_group_files_pub(&ordered);
    let dat = crate::interface::transcode::build_raw_group_pub(&body, version)?;
    Ok(EncodedGroup {
        components,
        body,
        dat,
        roster,
    })
}
