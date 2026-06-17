//! `generate-cs2-java` — generate the mechanical CS2 Java tables from the
//! registry produced by `extract-cs2-registry`.
//!
//! Three outputs:
//! - `com/jagex/game/script/ClientScriptCommand.java` (the opcode table — proven
//!   byte-identical to the hand-maintained file by gate B1);
//! - `rs2/client/clientscript/Cs2Dispatch.java` (the generated dispatch switch);
//! - `data/cs2/categories-910.json` (the category contract the splitter consumes).
//!
//! Read-only over the registry; writes only into the client tree and `data/cs2`
//! (or, with `--check`, writes nothing and reports drift). No timestamps; output
//! is byte-deterministic.
//!
//! Example invocation:
//!
//! ```bash
//! cd tools/rs3-cache-rs
//! cargo run --release -- generate-cs2-java
//! ```

use crate::cache_bail;
use crate::error::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolved options for the `generate-cs2-java` subcommand.
#[derive(Debug)]
pub struct Cs2JavaGenOpts<'a> {
    /// Registry JSON path (default: `<data-dir>/cs2/registry-910.json`).
    pub registry: Option<&'a Path>,
    /// Root of the client checkout.
    pub client_root: &'a Path,
    /// Java source root override (default: `<client-root>/client/src/main/java`).
    pub out_dir: Option<&'a Path>,
    /// Global data directory (holds the default registry path + `cs2/`).
    pub data_dir: &'a Path,
    /// Compare-only mode: write nothing, report drift, exit code 3 on any diff.
    pub check: bool,
}

const CATEGORIES_SCHEMA: &str = "cs2-categories/v1";

const CSC_HEADER: &str = include_str!("cs2_templates/csc_header.txt");
const CSC_TAIL_PRE: &str = include_str!("cs2_templates/csc_tail_pre.txt");
const CSC_TAIL_POST: &str = include_str!("cs2_templates/csc_tail_post.txt");

// ---------------------------------------------------------------------------
// Registry model (subset of the v3 schema needed for generation)
// ---------------------------------------------------------------------------

/// One command as read back from the registry JSON.
#[derive(Debug, Deserialize)]
struct RegistryCommand {
    name: String,
    id_910: u16,
    enum_field: Option<String>,
    enum_order: Option<u32>,
    obf: Option<String>,
    large_operand: bool,
    ctor_explicit_operand: bool,
    dispatch: RegistryDispatch,
}

/// Dispatch sub-object of a registry command.
#[derive(Debug, Deserialize)]
struct RegistryDispatch {
    kind: String,
    class: Option<String>,
    method: Option<String>,
    args: Vec<String>,
}

/// Top-level registry document (only the fields the generator consumes).
#[derive(Debug, Deserialize)]
struct RegistryDoc {
    commands: Vec<RegistryCommand>,
}

// ---------------------------------------------------------------------------
// Category assignment
// ---------------------------------------------------------------------------

/// Prefix → class table, applied to the registry `name`, first match wins.
/// Mirrors spec §3.3 exactly (order is significant).
const CATEGORY_RULES: &[(&[&str], &str)] = &[
    (&["if_"], "IfOps"),
    (&["cc_"], "CcOps"),
    (&["detail"], "DetailOps"),
    (&["cam", "viewport_"], "CamOps"),
    (&["worldmap_"], "WorldMapOps"),
    (&["chat", "activechatphrase_"], "ChatOps"),
    (
        &[
            "friend",
            "ignore",
            "clan",
            "player_group",
            "affinedclansettings",
        ],
        "SocialOps",
    ),
    (
        &["sound_", "jingle", "midi", "vorbis", "mixchannel"],
        "AudioOps",
    ),
    (
        &["oc_", "lc_", "nc_", "struct_", "enum", "_enum", "bas_"],
        "ConfigOps",
    ),
    (&["quest"], "QuestOps"),
    (&["stat"], "StatOps"),
    (&["inv_"], "InvOps"),
    (&["telemetry"], "TelemetryOps"),
    (&["db_"], "DbOps"),
    (
        &[
            "push_",
            "pop_",
            "branch",
            "gosub",
            "_switch",
            "_return",
            "define_array",
            "join_string",
        ],
        "CoreOps",
    ),
];

const FALLBACK_CLASS: &str = "MiscOps";

/// Donor command classes that own their dispatch methods in their own packages.
/// A command qualified to one of these is never filed under a category class.
const DONOR_CLASSES: &[&str] = &["TwitchCommands", "QuestCommands", "QuickChatDynamicCommand"];

/// Assign a category class to a command `name` per the first-match prefix rules.
#[must_use]
fn category_for(name: &str) -> &'static str {
    for (prefixes, class) in CATEGORY_RULES {
        if prefixes.iter().any(|p| name.starts_with(p)) {
            return class;
        }
    }
    FALLBACK_CLASS
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// One generated output file: target path + content bytes.
pub struct Emission {
    /// Target path the content should be written to.
    pub path: PathBuf,
    /// Generated file content (UTF-8, byte-deterministic).
    pub content: String,
}

/// Build all three emissions from the registry document.
fn build_emissions(
    doc: &RegistryDoc,
    java_root: &Path,
    categories_path: &Path,
) -> Result<Vec<Emission>> {
    let client_script_command = emit_client_script_command(doc);
    let (dispatch, categories) = emit_dispatch_and_categories(doc)?;

    Ok(vec![
        Emission {
            path: java_root.join("com/jagex/game/script/ClientScriptCommand.java"),
            content: client_script_command,
        },
        Emission {
            path: java_root.join("rs2/client/clientscript/Cs2Dispatch.java"),
            content: dispatch,
        },
        Emission {
            path: categories_path.to_path_buf(),
            content: categories,
        },
    ])
}

/// Emit `ClientScriptCommand.java` from the registry.
fn emit_client_script_command(doc: &RegistryDoc) -> String {
    // Fields ordered by enum_order (declaration order).
    let mut by_order: Vec<&RegistryCommand> = doc
        .commands
        .iter()
        .filter(|c| c.enum_field.is_some())
        .collect();
    by_order.sort_by_key(|c| c.enum_order.unwrap_or(u32::MAX));

    let mut out = String::with_capacity(CSC_HEADER.len() + by_order.len() * 96 + 16_384);
    out.push_str(CSC_HEADER);

    for cmd in &by_order {
        let field = cmd
            .enum_field
            .as_deref()
            .expect("filtered to enum-bearing commands");
        if let Some(obf) = &cmd.obf {
            let _ = writeln!(out, "\t@ObfuscatedName(\"{obf}\")");
        }
        // Reproduce the exact constructor form. The second argument is written
        // only when the original source declared it explicitly; `large_operand`
        // is always emitted as `true` (large) — the only `false` literals in the
        // tree are explicit `ctor_explicit_operand` cases.
        let ctor = if cmd.ctor_explicit_operand {
            let flag = if cmd.large_operand { "true" } else { "false" };
            format!("new ClientScriptCommand({}, {flag})", cmd.id_910)
        } else {
            format!("new ClientScriptCommand({})", cmd.id_910)
        };
        let _ = writeln!(
            out,
            "\tpublic static final ClientScriptCommand {field} = {ctor};"
        );
        out.push('\n');
    }

    out.push_str(CSC_TAIL_PRE);

    // values() array — enum_field list sorted ascending by id_910.
    let mut by_id: Vec<&RegistryCommand> = by_order.clone();
    by_id.sort_by_key(|c| c.id_910);
    for (idx, cmd) in by_id.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(
            cmd.enum_field
                .as_deref()
                .expect("filtered to enum-bearing commands"),
        );
    }

    out.push_str(CSC_TAIL_POST);
    out
}

/// Emit `Cs2Dispatch.java` and the categories JSON together (they share the
/// per-method class resolution).
fn emit_dispatch_and_categories(doc: &RegistryDoc) -> Result<(String, String)> {
    // method -> class, validated for the shared-method rule (§3.3).
    let mut method_class: BTreeMap<String, String> = BTreeMap::new();
    // class -> sorted set of methods filed under it (local dispatch only).
    let mut class_methods: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();

    // Commands by id, ascending, for case emission.
    let mut by_id: Vec<&RegistryCommand> = doc.commands.iter().collect();
    by_id.sort_by_key(|c| c.id_910);

    let mut cases = String::with_capacity(by_id.len() * 80);
    let mut unassigned: Vec<u16> = Vec::new();

    for cmd in &by_id {
        if cmd.dispatch.kind == "unassigned" {
            unassigned.push(cmd.id_910);
            continue;
        }
        if cmd.dispatch.kind != "call" {
            cache_bail!(
                "command `{}` (id {}) has unexpected dispatch kind `{}`",
                cmd.name,
                cmd.id_910,
                cmd.dispatch.kind
            );
        }
        let method = cmd.dispatch.method.as_deref().with_context(|| {
            format!(
                "command `{}` (id {}) is a call with no method",
                cmd.name, cmd.id_910
            )
        })?;

        // Resolve the destination class. A command is *donor-dispatched* when
        // its `dispatch.class` names one of the donor command classes (which own
        // their own methods in their own packages). Everything else is
        // *locally dispatched* and filed under its category class.
        //
        // The registry may be pre-split (`dispatch.class == null` for locals) or
        // post-split (`dispatch.class == <category>` for locals — set by the
        // extractor after the Stage 2 split). Generation must be identical in
        // both cases, so the category is always derived from `name`, and
        // donor-ness is decided by donor-class membership, not by null-ness.
        let donor_class = cmd
            .dispatch
            .class
            .as_deref()
            .filter(|c| DONOR_CLASSES.contains(c));

        let class = if let Some(donor) = donor_class {
            donor.to_owned()
        } else {
            let category = category_for(&cmd.name).to_owned();

            // Shared-method rule: a locally-dispatched method must map to one class.
            if let Some(existing) = method_class.get(method) {
                if existing != &category {
                    cache_bail!(
                        "method `{method}` maps to conflicting classes `{existing}` and `{category}` \
                         (commands sharing a dispatch method must map to one class)"
                    );
                }
            } else {
                method_class.insert(method.to_owned(), category.clone());
            }

            // Categories JSON covers only locally-dispatched methods.
            class_methods
                .entry(category.clone())
                .or_default()
                .insert(method.to_owned());

            category
        };

        // Render the call arguments, substituting the $state marker.
        let mut args = String::new();
        for (i, arg) in cmd.dispatch.args.iter().enumerate() {
            if i > 0 {
                args.push_str(", ");
            }
            if arg == "$state" {
                args.push_str("state");
            } else {
                args.push_str(arg);
            }
        }

        let _ = writeln!(cases, "\t\t\tcase {}:", cmd.id_910);
        let _ = writeln!(cases, "\t\t\t\t{class}.{method}({args});");
        cases.push_str("\t\t\t\treturn;\n");
    }

    // Unassigned ids -> grouped empty labels immediately before the terminal default.
    let mut tail_labels = String::new();
    for id in &unassigned {
        let _ = writeln!(tail_labels, "\t\t\tcase {id}:");
    }

    // Imports: the interpreter types plus the donor command classes the switch
    // dispatches to (their methods stay in their original packages).
    let dispatch = format!(
        "package rs2.client.clientscript;\n\
\n\
import com.jagex.game.client.QuestCommands;\n\
import com.jagex.game.client.TwitchCommands;\n\
import com.jagex.game.config.vartype.bit.VarBitOverflowException;\n\
import com.jagex.game.script.ClientScriptCommand;\n\
import com.jagex.game.script.ClientScriptState;\n\
import com.jagex.game.shared.framework.chat.QuickChatDynamicCommand;\n\
import com.jagex.graphics.camera.CameraException;\n\
\n\
public final class Cs2Dispatch {{\n\
\n\
\tpublic static void execute(ClientScriptCommand command, ClientScriptState state) throws CameraException, VarBitOverflowException {{\n\
\t\tswitch(command.index) {{\n\
{cases}{tail_labels}\t\t\tdefault:\n\
\t\t\t\tthrow new RuntimeException();\n\
\t\t}}\n\
\t}}\n\
}}\n"
    );

    // categories-910.json — deterministic, sorted by class then method.
    let categories = build_categories_json(&class_methods);

    Ok((dispatch, categories))
}

/// Build the `categories-910.json` text deterministically (manual JSON to keep
/// formatting fully under control and byte-stable).
fn build_categories_json(
    class_methods: &BTreeMap<String, std::collections::BTreeSet<String>>,
) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"schema\": \"{CATEGORIES_SCHEMA}\",");
    out.push_str("  \"classes\": {\n");
    let mut class_iter = class_methods.iter().peekable();
    while let Some((class, methods)) = class_iter.next() {
        let _ = write!(out, "    \"{class}\": [");
        let mut method_iter = methods.iter().peekable();
        if method_iter.peek().is_some() {
            out.push('\n');
            while let Some(method) = method_iter.next() {
                let comma = if method_iter.peek().is_some() {
                    ","
                } else {
                    ""
                };
                let _ = writeln!(out, "      \"{method}\"{comma}");
            }
            out.push_str("    ]");
        } else {
            out.push(']');
        }
        if class_iter.peek().is_some() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  }\n");
    out.push_str("}\n");
    out
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Build the generated emissions for the given options without writing anything.
///
/// Used by the drift-gate test and by [`run`].
pub fn generate(opts: &Cs2JavaGenOpts<'_>) -> Result<Vec<Emission>> {
    let registry_path: PathBuf = opts.registry.map_or_else(
        || opts.data_dir.join("cs2").join("registry-910.json"),
        Path::to_path_buf,
    );
    let java_root: PathBuf = opts.out_dir.map_or_else(
        || opts.client_root.join("client/src/main/java"),
        Path::to_path_buf,
    );
    let categories_path = opts.data_dir.join("cs2").join("categories-910.json");

    let registry_text = fs::read_to_string(&registry_path)
        .with_context(|| format!("failed to read registry {}", registry_path.display()))?;
    let doc: RegistryDoc = serde_json::from_str(&registry_text)
        .with_context(|| format!("failed to parse registry {}", registry_path.display()))?;

    build_emissions(&doc, &java_root, &categories_path)
}

/// Run the `generate-cs2-java` subcommand.
pub fn run(opts: &Cs2JavaGenOpts<'_>) -> Result<()> {
    let emissions = generate(opts)?;

    if opts.check {
        let mut diffs: Vec<&Emission> = Vec::new();
        for emission in &emissions {
            let on_disk = fs::read_to_string(&emission.path).unwrap_or_default();
            if on_disk != emission.content {
                diffs.push(emission);
            }
        }
        if diffs.is_empty() {
            println!(
                "generate-cs2-java --check: all {} file(s) up to date",
                emissions.len()
            );
            return Ok(());
        }
        println!("generate-cs2-java --check: {} file(s) differ:", diffs.len());
        for emission in &diffs {
            let on_disk = fs::read_to_string(&emission.path).unwrap_or_default();
            println!(
                "  - {} (on-disk {} bytes, generated {} bytes, first diff at byte {})",
                emission.path.display(),
                on_disk.len(),
                emission.content.len(),
                first_diff(&on_disk, &emission.content)
            );
        }
        std::process::exit(3);
    }

    for emission in &emissions {
        if let Some(dir) = emission.path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create dir {}", dir.display()))?;
        }
        fs::write(&emission.path, emission.content.as_bytes())
            .with_context(|| format!("failed to write {}", emission.path.display()))?;
        println!("wrote {}", emission.path.display());
    }

    Ok(())
}

/// Index of the first byte that differs between two strings (or the shorter
/// length when one is a prefix of the other).
fn first_diff(a: &str, b: &str) -> usize {
    a.bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()))
}

#[cfg(test)]
mod tests {
    use super::{
        RegistryCommand, RegistryDispatch, RegistryDoc, build_categories_json, category_for,
        emit_client_script_command, emit_dispatch_and_categories,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn cmd(
        name: &str,
        id: u16,
        enum_field: Option<&str>,
        enum_order: Option<u32>,
        obf: Option<&str>,
        large: bool,
        dispatch: RegistryDispatch,
    ) -> RegistryCommand {
        RegistryCommand {
            name: name.to_owned(),
            id_910: id,
            enum_field: enum_field.map(str::to_owned),
            enum_order,
            obf: obf.map(str::to_owned),
            large_operand: large,
            // 1-arg form unless explicitly large (matches the common shape).
            ctor_explicit_operand: large,
            dispatch,
        }
    }

    fn call(class: Option<&str>, method: &str, args: &[&str]) -> RegistryDispatch {
        RegistryDispatch {
            kind: "call".to_owned(),
            class: class.map(str::to_owned),
            method: Some(method.to_owned()),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn unassigned() -> RegistryDispatch {
        RegistryDispatch {
            kind: "unassigned".to_owned(),
            class: None,
            method: None,
            args: Vec::new(),
        }
    }

    #[test]
    fn category_mapping_first_match_wins() {
        assert_eq!(category_for("if_sendtofront"), "IfOps");
        assert_eq!(category_for("cc_getparentlayer"), "CcOps");
        assert_eq!(category_for("cam_moveto"), "CamOps");
        assert_eq!(category_for("viewport_setfov"), "CamOps");
        assert_eq!(category_for("enum"), "ConfigOps");
        assert_eq!(category_for("_enum"), "ConfigOps");
        assert_eq!(category_for("push_int"), "CoreOps");
        assert_eq!(category_for("something_unmatched"), "MiscOps");
        // `chat` matches ChatOps before `clan` would be considered (different rule).
        assert_eq!(category_for("chatsetmode"), "ChatOps");
        assert_eq!(category_for("clan_kick"), "SocialOps");
    }

    #[test]
    fn field_emission_with_and_without_annotation() {
        let doc = RegistryDoc {
            commands: vec![
                cmd(
                    "add",
                    842,
                    Some("add"),
                    Some(0),
                    None,
                    false,
                    call(None, "add", &["$state"]),
                ),
                cmd(
                    "push_constant_int",
                    204,
                    Some("PUSH_CONSTANT_INT"),
                    Some(1),
                    Some("ss.e"),
                    true,
                    call(None, "push_constant_int", &["$state"]),
                ),
            ],
        };
        let out = emit_client_script_command(&doc);
        // Unannotated 1-arg field present.
        assert!(out.contains(
            "\tpublic static final ClientScriptCommand add = new ClientScriptCommand(842);\n"
        ));
        // Annotated 2-arg field present with its annotation line directly above.
        assert!(out.contains(
            "\t@ObfuscatedName(\"ss.e\")\n\tpublic static final ClientScriptCommand PUSH_CONSTANT_INT = new ClientScriptCommand(204, true);\n"
        ));
    }

    #[test]
    fn values_array_sorted_by_id() {
        let doc = RegistryDoc {
            commands: vec![
                // declaration order B(order0,id9), A(order1,id2)
                cmd(
                    "b",
                    9,
                    Some("FIELD_B"),
                    Some(0),
                    None,
                    false,
                    call(None, "b", &["$state"]),
                ),
                cmd(
                    "a",
                    2,
                    Some("FIELD_A"),
                    Some(1),
                    None,
                    false,
                    call(None, "a", &["$state"]),
                ),
            ],
        };
        let out = emit_client_script_command(&doc);
        // Declaration order: FIELD_B first, then FIELD_A.
        let b_pos = out
            .find("ClientScriptCommand FIELD_B")
            .expect("FIELD_B decl");
        let a_pos = out
            .find("ClientScriptCommand FIELD_A")
            .expect("FIELD_A decl");
        assert!(b_pos < a_pos, "declarations in enum_order");
        // values() array is sorted by id ascending: FIELD_A (id2) before FIELD_B (id9).
        let arr = out
            .split("new ClientScriptCommand[] { ")
            .nth(1)
            .expect("values array");
        let arr_a = arr.find("FIELD_A").expect("FIELD_A in array");
        let arr_b = arr.find("FIELD_B").expect("FIELD_B in array");
        assert!(arr_a < arr_b, "values() sorted by id");
    }

    #[test]
    fn dispatch_cases_state_substitution_and_unassigned_grouping() {
        let doc = RegistryDoc {
            commands: vec![
                cmd(
                    "foo",
                    0,
                    None,
                    None,
                    None,
                    false,
                    call(None, "foo", &["$state"]),
                ),
                cmd(
                    "if_bar",
                    1,
                    None,
                    None,
                    None,
                    false,
                    call(None, "if_bar", &["true", "$state"]),
                ),
                cmd("u1", 2, None, None, None, false, unassigned()),
                cmd("u2", 3, None, None, None, false, unassigned()),
                cmd(
                    "tw",
                    4,
                    None,
                    None,
                    None,
                    false,
                    call(Some("TwitchCommands"), "ttv", &["$state"]),
                ),
            ],
        };
        let (dispatch, _cats) = emit_dispatch_and_categories(&doc).expect("emit");
        // $state substituted to `state`.
        assert!(dispatch.contains("\t\t\tcase 0:\n\t\t\t\tMiscOps.foo(state);\n\t\t\t\treturn;\n"));
        // boolean-variant keeps order.
        assert!(
            dispatch
                .contains("\t\t\tcase 1:\n\t\t\t\tIfOps.if_bar(true, state);\n\t\t\t\treturn;\n")
        );
        // explicit donor class preserved.
        assert!(
            dispatch
                .contains("\t\t\tcase 4:\n\t\t\t\tTwitchCommands.ttv(state);\n\t\t\t\treturn;\n")
        );
        // unassigned grouped before default.
        assert!(dispatch.contains(
            "\t\t\tcase 2:\n\t\t\tcase 3:\n\t\t\tdefault:\n\t\t\t\tthrow new RuntimeException();\n"
        ));
    }

    #[test]
    fn categories_excludes_donor_classes() {
        let doc = RegistryDoc {
            commands: vec![
                cmd(
                    "foo",
                    0,
                    None,
                    None,
                    None,
                    false,
                    call(None, "foo", &["$state"]),
                ),
                cmd(
                    "if_a",
                    1,
                    None,
                    None,
                    None,
                    false,
                    call(None, "if_a", &["$state"]),
                ),
                cmd(
                    "tw",
                    2,
                    None,
                    None,
                    None,
                    false,
                    call(Some("TwitchCommands"), "ttv", &["$state"]),
                ),
            ],
        };
        let (_dispatch, cats) = emit_dispatch_and_categories(&doc).expect("emit");
        assert!(cats.contains("\"MiscOps\""));
        assert!(cats.contains("\"foo\""));
        assert!(cats.contains("\"IfOps\""));
        // Donor-class method excluded from categories.
        assert!(!cats.contains("TwitchCommands"));
        assert!(!cats.contains("\"ttv\""));
    }

    #[test]
    fn shared_method_one_class() {
        // if_sendtofront / if_sendtoback both -> IfOps via name, shared method if_sendto.
        let doc = RegistryDoc {
            commands: vec![
                cmd(
                    "if_sendtofront",
                    0,
                    None,
                    None,
                    None,
                    false,
                    call(None, "if_sendto", &["true", "$state"]),
                ),
                cmd(
                    "if_sendtoback",
                    1,
                    None,
                    None,
                    None,
                    false,
                    call(None, "if_sendto", &["false", "$state"]),
                ),
            ],
        };
        let (_dispatch, cats) = emit_dispatch_and_categories(&doc).expect("emit");
        // Method filed once under IfOps.
        let occurrences = cats.matches("\"if_sendto\"").count();
        assert_eq!(occurrences, 1, "if_sendto filed once");
    }

    #[test]
    fn shared_method_conflicting_class_errors() {
        // Two names mapping the same method to different classes must fail.
        let doc = RegistryDoc {
            commands: vec![
                cmd(
                    "if_x",
                    0,
                    None,
                    None,
                    None,
                    false,
                    call(None, "shared", &["$state"]),
                ),
                cmd(
                    "cc_x",
                    1,
                    None,
                    None,
                    None,
                    false,
                    call(None, "shared", &["$state"]),
                ),
            ],
        };
        let result = emit_dispatch_and_categories(&doc);
        match result {
            Ok(_) => panic!("expected a conflicting-class error"),
            Err(err) => assert!(format!("{err}").contains("conflicting classes")),
        }
    }

    #[test]
    fn categories_json_is_deterministic_and_sorted() {
        let mut class_methods: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        class_methods
            .entry("IfOps".to_owned())
            .or_default()
            .extend(["if_b".to_owned(), "if_a".to_owned()]);
        class_methods
            .entry("CcOps".to_owned())
            .or_default()
            .insert("cc_a".to_owned());
        let json = build_categories_json(&class_methods);
        // CcOps before IfOps (class sorted); if_a before if_b (method sorted).
        let cc = json.find("\"CcOps\"").expect("CcOps");
        let iff = json.find("\"IfOps\"").expect("IfOps");
        assert!(cc < iff);
        let a = json.find("\"if_a\"").expect("if_a");
        let b = json.find("\"if_b\"").expect("if_b");
        assert!(a < b);
        // Stable across calls.
        assert_eq!(json, build_categories_json(&class_methods));
    }
}
