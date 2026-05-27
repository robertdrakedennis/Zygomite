use rs3_cache_rs::script::{
    CompiledScript, Instruction, OpcodeBook, Operand, decode_script, encode_script, script_to_asm,
};
use rs3_cache_rs::transpile::{
    REVERSIBLE_FORMAT_VERSION, ReversibleMetadata, parse_structured_typescript,
    render_reversible_source, structured_digest,
};
use std::path::{Path, PathBuf};
use std::process::Command;

fn default_cache_dir(build: u32) -> PathBuf {
    PathBuf::from(format!(
        "/Users/robert/projects/alerion/cache/unpacked/{build}"
    ))
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/Users/robert/projects/alerion/tools/rs3-cache-rs/data")
}

fn cache_dir(build: u32) -> PathBuf {
    let scoped_key = format!("RS3_CACHE_DIR_{build}");
    std::env::var_os(&scoped_key)
        .or_else(|| std::env::var_os("RS3_CACHE_DIR"))
        .map_or_else(|| default_cache_dir(build), PathBuf::from)
}

fn data_dir() -> PathBuf {
    std::env::var_os("RS3_DATA_DIR").map_or_else(default_data_dir, PathBuf::from)
}

fn require_fixture(build: u32) -> Option<(PathBuf, PathBuf)> {
    let cache = cache_dir(build);
    let data = data_dir();
    if cache.is_dir() && data.is_dir() {
        Some((cache, data))
    } else {
        eprintln!(
            "skip: missing fixture build={build} (cache={}, data={})",
            cache.display(),
            data.display()
        );
        None
    }
}

fn run_assemble(
    build: u32,
    input: &Path,
    output: &Path,
    strict_structured: bool,
) -> std::process::Output {
    let (cache, data) = require_fixture(build).expect("fixture");
    let bin = env!("CARGO_BIN_EXE_rs3-cache-rs");
    let mut command = Command::new(bin);
    command.args([
        "--cache-dir",
        &cache.to_string_lossy(),
        "--data-dir",
        &data.to_string_lossy(),
        "--build",
        &build.to_string(),
        "--subbuild",
        "0",
        "assemble-script",
        "--input",
        &input.to_string_lossy(),
        "--output",
        &output.to_string_lossy(),
    ]);
    if strict_structured {
        command.arg("--strict-structured");
    }
    command.output().expect("run assemble-script")
}

fn opcode_book(build: u32) -> OpcodeBook {
    OpcodeBook::load(&data_dir(), build, 0).expect("load opcode book")
}

fn make_metadata(
    build: u32,
    packed_id: i32,
    export_name: &str,
    raw_name: &str,
    structured_source: &str,
    editable_structured: bool,
    blocking_diagnostics: Vec<String>,
) -> ReversibleMetadata {
    let parsed = parse_structured_typescript(structured_source).expect("parse structured source");
    ReversibleMetadata {
        format_version: REVERSIBLE_FORMAT_VERSION,
        build,
        subbuild: 0,
        packed_id,
        group_id: packed_id >> 16,
        file_id: u16::try_from(packed_id & 0xffff).expect("file id fits u16"),
        script_id: packed_id,
        export_name: export_name.to_string(),
        raw_name: Some(raw_name.to_string()),
        editable_structured,
        structured_digest: structured_digest(&parsed),
        blocking_diagnostics,
    }
}

fn clean_structured_source(export_name: &str, value: i32) -> String {
    format!(
        "export function {export_name}(): number {{\n    let int_0: number;\n\n    int_0 = {value};\n    return int_0;\n}}\n"
    )
}

fn clean_script(raw_name: &str, value: i32) -> CompiledScript {
    CompiledScript {
        name: Some(raw_name.to_string()),
        local_count_int: 1,
        local_count_object: 0,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(value),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    }
}

fn dirty_structured_source(export_name: &str) -> String {
    format!("export function {export_name}(): void {{\n    goto(5);\n    return;\n}}\n")
}

fn dirty_script(raw_name: &str) -> CompiledScript {
    CompiledScript {
        name: Some(raw_name.to_string()),
        local_count_int: 0,
        local_count_object: 0,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![Instruction {
            opcode: 0,
            command: "return".to_string(),
            operand: Operand::Byte(0),
        }],
    }
}

fn write_reversible_file(
    path: &Path,
    structured_source: &str,
    metadata: &ReversibleMetadata,
    script: &CompiledScript,
) {
    let asm = script_to_asm(script);
    let reversible =
        render_reversible_source(structured_source, metadata, &asm).expect("render reversible");
    std::fs::write(path, reversible).expect("write reversible file");
}

fn decode_output_script(build: u32, output: &Path) -> CompiledScript {
    let binary = std::fs::read(output).expect("read output binary");
    decode_script(&binary, &opcode_book(build), build).expect("decode output script")
}

#[test]
fn assemble_reversible_structured_edits_910() {
    if require_fixture(910).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("script910.ts");
    let output = dir.path().join("script910.cs2");
    let export_name = "script910001";
    let raw_name = "[proc,script910001]";
    let original_source = clean_structured_source(export_name, 1);
    let metadata = make_metadata(
        910,
        910_001,
        export_name,
        raw_name,
        &original_source,
        true,
        vec![],
    );
    write_reversible_file(
        &input,
        &original_source,
        &metadata,
        &clean_script(raw_name, 1),
    );

    let edited = std::fs::read_to_string(&input)
        .expect("read reversible source")
        .replace("int_0 = 1;", "int_0 = 2;");
    std::fs::write(&input, edited).expect("write edited source");

    let result = run_assemble(910, &input, &output, true);
    assert!(
        result.status.success(),
        "assemble failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let decoded = decode_output_script(910, &output);
    assert!(matches!(
        decoded.code.first().map(|instruction| &instruction.operand),
        Some(Operand::Int(2))
    ));
}

#[test]
fn assemble_reversible_structured_edits_947() {
    if require_fixture(947).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("script947.ts");
    let output = dir.path().join("script947.cs2");
    let export_name = "script947001";
    let raw_name = "[proc,script947001]";
    let original_source = clean_structured_source(export_name, 1);
    let metadata = make_metadata(
        947,
        947_001,
        export_name,
        raw_name,
        &original_source,
        true,
        vec![],
    );
    write_reversible_file(
        &input,
        &original_source,
        &metadata,
        &clean_script(raw_name, 1),
    );

    let edited = std::fs::read_to_string(&input)
        .expect("read reversible source")
        .replace("int_0 = 1;", "int_0 = 2;");
    std::fs::write(&input, edited).expect("write edited source");

    let result = run_assemble(947, &input, &output, true);
    assert!(
        result.status.success(),
        "assemble failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let decoded = decode_output_script(947, &output);
    assert!(matches!(
        decoded.code.first().map(|instruction| &instruction.operand),
        Some(Operand::Int(2))
    ));
}

#[test]
fn assemble_reversible_dirty_script_falls_back_to_embedded_asm() {
    if require_fixture(947).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("dirty.ts");
    let output = dir.path().join("dirty.cs2");
    let export_name = "script947_dirty";
    let raw_name = "[proc,script947_dirty]";
    let structured_source = dirty_structured_source(export_name);
    let metadata = make_metadata(
        947,
        947_777,
        export_name,
        raw_name,
        &structured_source,
        false,
        vec!["residual_goto".to_string()],
    );
    let original_script = dirty_script(raw_name);
    write_reversible_file(&input, &structured_source, &metadata, &original_script);

    let drifted = std::fs::read_to_string(&input)
        .expect("read reversible source")
        .replace(
            "export function",
            "// formatting drift should not break digest\nexport function",
        );
    std::fs::write(&input, drifted).expect("write drifted source");

    let result = run_assemble(947, &input, &output, false);
    assert!(
        result.status.success(),
        "assemble failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );

    let expected = encode_script(&original_script, &opcode_book(947), 947).expect("encode script");
    let actual = std::fs::read(&output).expect("read fallback output");
    assert_eq!(actual, expected);
}

#[test]
fn assemble_reversible_dirty_script_strict_mode_fails_without_fallback() {
    if require_fixture(947).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("dirty-strict.ts");
    let output = dir.path().join("dirty-strict.cs2");
    let export_name = "script947_dirty";
    let raw_name = "[proc,script947_dirty]";
    let structured_source = dirty_structured_source(export_name);
    let metadata = make_metadata(
        947,
        947_778,
        export_name,
        raw_name,
        &structured_source,
        false,
        vec!["residual_goto".to_string()],
    );
    let original_script = dirty_script(raw_name);
    write_reversible_file(&input, &structured_source, &metadata, &original_script);

    let result = run_assemble(947, &input, &output, true);
    assert!(!result.status.success(), "strict mode should fail");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("structured edits blocked"));
    assert!(stderr.contains("residual_goto"));
}

#[test]
fn assemble_reversible_dirty_script_edit_fails_with_blocker() {
    if require_fixture(910).is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("dirty-edit.ts");
    let output = dir.path().join("dirty-edit.cs2");
    let export_name = "script910_dirty";
    let raw_name = "[proc,script910_dirty]";
    let structured_source = dirty_structured_source(export_name);
    let metadata = make_metadata(
        910,
        910_777,
        export_name,
        raw_name,
        &structured_source,
        false,
        vec!["residual_goto".to_string()],
    );
    write_reversible_file(
        &input,
        &structured_source,
        &metadata,
        &dirty_script(raw_name),
    );

    let edited = std::fs::read_to_string(&input)
        .expect("read reversible source")
        .replace("goto(5);", "goto(6);");
    std::fs::write(&input, edited).expect("write edited dirty source");

    let result = run_assemble(910, &input, &output, false);
    assert!(
        !result.status.success(),
        "dirty structured edit should fail"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("structured edits blocked"));
    assert!(stderr.contains("residual_goto"));
}
