//! `extract-protocol` / `generate-protocol`.
//!
//! Extract the canonical game-protocol schema from the Java client (the source
//! of truth), cross-diff the server's TypeScript mirrors, and emit the
//! parity-gate artifacts both ends consume.
//!
//! The client's `ServerProt.java` / `ClientProt.java` / `LoginProt.java` define
//! every packet's name, opcode, and size. `extract-protocol` parses those into
//! schema JSON, diffs the three server TS tables against them (checks P1–P6),
//! and writes a findings report plus a checked-in divergence baseline.
//! `generate-protocol` then turns the schema + baseline into the server's
//! `protocol910.ts` tables and the client's `protocol-910.tsv` resource.
//!
//! Both commands are read-only over the client/server source trees; field
//! layouts and encode generation are out of scope for this stage.
//!
//! Example invocations:
//!
//! ```bash
//! cd tools/rs3-cache-rs
//! cargo run --release -- --data-dir data extract-protocol
//! cargo run --release -- --data-dir data generate-protocol --check
//! ```
//!
//! `survey-payloads` is a third command (ported from the retired
//! `scripts/protocol-payload-survey.py`): it parses the server's hand-written
//! `ServerProt.ts` encode bodies and the client's `Client.java` decode branches,
//! classifies each end simple/complex per the DSL rules, and emits
//! `payload-classification.json` + `payloads.json` (the simple+v2 tranche). It is
//! read-only over both source trees.
//!
//! The implementation is split by concern: [`types`] (the schema data model),
//! [`parse`] (Java/TS source parsing), [`extract`] (the `extract-protocol`
//! command + cross-diff), [`expr`] (the payload size-expression mini-language),
//! [`generate`] (the `generate-protocol` command + payload/encoder codegen), and
//! [`survey`] (the `survey-payloads` classification + schema audit).

mod expr;
mod extract;
mod generate;
mod parse;
mod survey;
mod types;

pub use extract::{ExtractOutput, ExtractProtocolOpts, extract, run_extract};
pub use generate::{
    GenerateOutput, GenerateProtocolOpts, PayloadField, PayloadPacket, PayloadParam, Payloads,
    generate, run_generate,
};
pub use parse::{parse_java, parse_ts};
pub use survey::{SurveyOutput, SurveyPayloadsOpts, run_survey, survey};
pub use types::{
    Divergence, DivergenceBaseline, Finding, JavaPacket, JavaParse, Prot, Schema, SchemaPacket,
    TsPacket,
};

// Re-exports consumed only by the relocated unit tests (which keep their original
// `use super::{…}` imports verbatim). Gated to test builds so they are not flagged
// as unused in the library build.
#[cfg(test)]
pub(crate) use expr::{emit_expr, parse_expr};
#[cfg(test)]
pub(crate) use generate::{
    array_default_len, codec_fixed_width, encoder_fn_name, is_v1_codec, mirror_reads, param_models,
    render_client_tsv, render_encoders_ts, render_server_ts, render_size_expr, validate_payload,
};

#[cfg(test)]
mod tests;
