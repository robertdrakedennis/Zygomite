use super::Diagnostics;
use super::structured::StructuredScript;
use crate::cache_bail as bail;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub const REVERSIBLE_FORMAT_VERSION: u32 = 1;
pub const META_PREFIX: &str = "// @rs3cache-meta ";
pub const ASM_BEGIN: &str = "// @rs3cache-asm-begin";
pub const ASM_END: &str = "// @rs3cache-asm-end";

const BLOCKING_DIAGNOSTIC_MESSAGES: &[(&str, &str)] = &[
    ("parity miss: residual goto in output", "residual_goto"),
    (
        "parity miss: commented branch remains in structured output",
        "commented_branch",
    ),
    ("bad stack state: residual pop() in output", "residual_pop"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReversibleMetadata {
    pub format_version: u32,
    pub build: u32,
    pub subbuild: u32,
    pub packed_id: i32,
    pub group_id: i32,
    pub file_id: u16,
    pub script_id: i32,
    pub export_name: String,
    pub raw_name: Option<String>,
    pub editable_structured: bool,
    pub structured_digest: String,
    pub blocking_diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedReversibleSource {
    pub metadata: ReversibleMetadata,
    pub structured_source: String,
    pub asm_trailer: String,
}

pub fn is_reversible_source(source: &str) -> bool {
    source
        .lines()
        .any(|line| line.trim_start().starts_with(META_PREFIX))
}

pub fn parse_reversible_source(source: &str) -> crate::error::Result<ParsedReversibleSource> {
    let mut metadata: Option<ReversibleMetadata> = None;
    let mut structured_lines = Vec::new();
    let mut asm_lines = Vec::new();
    let mut in_trailer = false;
    let mut saw_begin = false;
    let mut saw_end = false;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(META_PREFIX) {
            metadata = Some(serde_json::from_str(rest)?);
            continue;
        }
        if trimmed == ASM_BEGIN {
            saw_begin = true;
            in_trailer = true;
            continue;
        }
        if trimmed == ASM_END {
            saw_end = true;
            in_trailer = false;
            continue;
        }
        if in_trailer {
            asm_lines.push(line.to_string());
        } else {
            structured_lines.push(line.to_string());
        }
    }

    let Some(metadata) = metadata else {
        bail!("missing reversible metadata");
    };
    if !saw_begin || !saw_end {
        bail!("missing reversible ASM trailer markers");
    }

    Ok(ParsedReversibleSource {
        metadata,
        structured_source: structured_lines.join("\n"),
        asm_trailer: asm_lines.join("\n"),
    })
}

pub fn append_reversible_footer(
    source: &mut String,
    metadata: &ReversibleMetadata,
    asm_source: &str,
) -> crate::error::Result<()> {
    let wrapped = render_reversible_source(source, metadata, asm_source)?;
    *source = wrapped;
    Ok(())
}

pub fn render_reversible_source(
    structured_source: &str,
    metadata: &ReversibleMetadata,
    asm_source: &str,
) -> crate::error::Result<String> {
    let json = serde_json::to_string(metadata)?;
    let mut out = String::new();
    out.push_str(META_PREFIX);
    out.push_str(&json);
    out.push('\n');
    out.push('\n');
    out.push_str(structured_source.trim_end());
    out.push('\n');
    out.push('\n');
    out.push_str(ASM_BEGIN);
    out.push('\n');
    out.push_str(asm_source.trim_end());
    out.push('\n');
    out.push_str(ASM_END);
    out.push('\n');
    Ok(out)
}

pub fn structured_digest(script: &StructuredScript) -> String {
    let canonical = script.canonical_source();
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn blocking_diagnostics(diagnostics: &Diagnostics) -> Vec<String> {
    let mut blockers = Vec::new();
    for diagnostic in &diagnostics.diagnostics {
        if let Some(kind) = blocking_diagnostic_kind(&diagnostic.message)
            && !blockers.iter().any(|existing| existing == kind)
        {
            blockers.push(kind.to_string());
        }
    }
    blockers
}

pub fn blocking_diagnostic_kind(message: &str) -> Option<&'static str> {
    BLOCKING_DIAGNOSTIC_MESSAGES
        .iter()
        .find_map(|(known, kind)| (*known == message).then_some(*kind))
}

pub fn editable_structured(diagnostics: &Diagnostics) -> bool {
    blocking_diagnostics(diagnostics).is_empty()
}

#[cfg(test)]
mod tests {
    use super::{
        ASM_BEGIN, ASM_END, META_PREFIX, ReversibleMetadata, parse_reversible_source,
        render_reversible_source,
    };
    use crate::transpile::{parse_structured_typescript, structured_digest};

    fn simple_metadata(source: &str) -> ReversibleMetadata {
        let parsed = parse_structured_typescript(source).expect("parse structured source");
        ReversibleMetadata {
            format_version: super::REVERSIBLE_FORMAT_VERSION,
            build: 947,
            subbuild: 0,
            packed_id: 1234,
            group_id: 1234 >> 16,
            file_id: 0,
            script_id: 1234,
            export_name: "script1234".to_string(),
            raw_name: Some("[proc,script1234]".to_string()),
            editable_structured: true,
            structured_digest: structured_digest(&parsed),
            blocking_diagnostics: Vec::new(),
        }
    }

    #[test]
    fn reversible_source_roundtrip_preserves_sections() {
        let source = "export function script1234(): void {\n    return;\n}\n";
        let metadata = simple_metadata(source);
        let wrapped = render_reversible_source(
            source,
            &metadata,
            "// @cs2 name=[proc,script1234]\n// @cs2 return",
        )
        .expect("render reversible source");
        assert!(wrapped.starts_with(META_PREFIX));
        assert!(wrapped.contains(ASM_BEGIN));
        assert!(wrapped.contains(ASM_END));

        let parsed = parse_reversible_source(&wrapped).expect("parse reversible source");
        assert_eq!(parsed.metadata.export_name, "script1234");
        assert!(
            parsed
                .structured_source
                .contains("export function script1234()")
        );
        assert!(parsed.asm_trailer.contains("// @cs2 return"));
    }

    #[test]
    fn structured_digest_ignores_comments_and_whitespace() {
        let source_a = "export function script1234(): number {\n    return 1;\n}\n";
        let source_b =
            "// extra comment\n\nexport function script1234(): number {\n    return 1;\n}\n";
        let parsed_a = parse_structured_typescript(source_a).expect("parse a");
        let parsed_b = parse_structured_typescript(source_b).expect("parse b");
        assert_eq!(structured_digest(&parsed_a), structured_digest(&parsed_b));
    }
}
