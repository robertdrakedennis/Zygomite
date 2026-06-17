//! Interface (`.if3`) component model: decoding, rendering, and dependency
//! extraction, split by concern across the submodules below.
//!
//! - [`decode`] — [`parse_component`] dispatch + every `decode_*` body decoder.
//! - [`render`] — human-readable group rendering + id/name helpers.
//! - [`deps`] — component dependency-graph extraction.
//!
//! A couple of small types are shared by [`decode`] and [`deps`] and live here:
//! [`format_if_type`] (component-type label) and [`TransmitListType`].
//!
//! The public API is re-exported flat (`interface::*`), so callers use
//! `crate::interface::parse_component`, `::ComponentDeps`, `::render_interface_group`, etc.

pub mod component;
pub mod decode;
pub mod decode910;
pub mod deps;
pub mod render;
pub mod transcode;

pub use decode::*;
pub use deps::*;
pub use render::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum TransmitListType {
    VarPlayer,
    Inv,
    Stat,
    VarClient,
    VarClientString,
}

pub(crate) fn format_if_type(if_type: i32) -> &'static str {
    match if_type {
        0 => "layer",
        3 => "rectangle",
        4 => "text",
        5 => "graphic",
        6 => "model",
        9 => "line",
        10 => "button",
        11 => "panel",
        12 => "check",
        13 => "input",
        14 => "slider",
        15 => "grid",
        16 => "list",
        17 => "combo",
        18 => "pagedlayer",
        19 => "pagedlayerheader",
        20 => "carousel",
        21 => "pagedcarousel",
        22 => "radiogroup",
        23 => "groupbox",
        24 => "radialprogressoverlay",
        26 => "crmview",
        27 => "cutscenelayer",
        28 => "modelgroup",
        _ => "unknown",
    }
}
