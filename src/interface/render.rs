//! Human-readable rendering and id/name helpers for interface components.
//!
//! [`render_interface_group`] formats a whole interface group's components (via
//! [`super::parse_component`]); [`component_uid`] / [`component_fallback_name`] derive
//! the CS2 component uid and the fallback TypeScript property name.

use std::collections::BTreeMap;

use super::parse_component;

/// Full component UID used by CS2 opcodes: `(interface_id << 16) | component_id`.
pub fn component_uid(interface_id: u32, component_id: u32) -> u32 {
    (interface_id << 16) | (component_id & 0xFFFF)
}

/// Fallback TypeScript property name when the interface binary has no explicit name.
pub fn component_fallback_name(interface_id: u32, component_id: u32) -> String {
    format!("Interface_{interface_id}_Com_{component_id}")
}

pub fn render_interface_group(
    group: u32,
    files: &BTreeMap<u32, Vec<u8>>,
    build: u32,
) -> Vec<String> {
    let mut out = Vec::new();
    for (component, data) in files {
        match parse_component(*component, data, build) {
            Ok(lines) => {
                out.extend(lines);
                out.push(String::new());
            }
            Err(error) => {
                out.push(format!("[com{component}]"));
                out.push("type=unsupported".to_string());
                out.push(format!(
                    "parse_error={}",
                    error.to_string().replace('\n', " ")
                ));
                out.push(String::new());
            }
        }
    }
    if out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out.insert(0, format!("; interface_{group}"));
    out
}
