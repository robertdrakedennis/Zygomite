//! Integration test for `extract-cs2-registry` against the real client tree.
//!
//! Skips silently when the client `ScriptRunner.java` is absent so CI without
//! the client checkout still passes.

use rs3_cache_rs::cs2_registry::{parse_enum, parse_switch};
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn client_root() -> PathBuf {
    crate_dir().join("../../client")
}

fn script_runner_path(client_root: &Path) -> PathBuf {
    client_root.join("client/src/main/java/rs2/client/clientscript/ScriptRunner.java")
}

fn dispatch_path(client_root: &Path) -> PathBuf {
    client_root.join("client/src/main/java/rs2/client/clientscript/Cs2Dispatch.java")
}

fn command_enum_path(client_root: &Path) -> PathBuf {
    client_root.join("client/src/main/java/com/jagex/game/script/ClientScriptCommand.java")
}

#[test]
fn extract_real_registry_invariants() {
    let client_root = client_root();
    let sr_path = script_runner_path(&client_root);
    if !sr_path.is_file() {
        eprintln!(
            "skip: client ScriptRunner.java absent at {}",
            sr_path.display()
        );
        return;
    }

    // Dispatch source: prefer the post-split Cs2Dispatch.java, else fall back to
    // ScriptRunner.java's executeCommand (pre-split layout).
    let disp_path = dispatch_path(&client_root);
    let switch_path = if disp_path.is_file() {
        disp_path
    } else {
        sr_path
    };
    let switch_src = std::fs::read_to_string(&switch_path).expect("read dispatch source");
    let switch = parse_switch(&switch_src, &switch_path).expect("parse switch");

    // 1,432 contiguous case labels 0..=1431.
    assert_eq!(switch.case_count, 1432, "case_count");
    let ids: Vec<u16> = switch.dispatches.keys().copied().collect();
    assert_eq!(*ids.first().unwrap(), 0);
    assert_eq!(*ids.last().unwrap(), 1431);
    for (expected, actual) in (0_u16..=1431).zip(ids.iter().copied()) {
        assert_eq!(expected, actual, "ids must be contiguous 0..=1431");
    }

    // Unassigned fall-through set.
    let unassigned: Vec<u16> = switch.unassigned.iter().copied().collect();
    assert_eq!(unassigned, vec![79, 103, 247, 250, 393], "unassigned set");

    // The 8 extra-arg dispatches match the spec §1.1 shape 4 exactly, now
    // expressed via the full `args` list with the `$state` marker. The
    // qualifying class is `None` pre-split and the resolved category (or donor)
    // class post-split; assert the method/args here and the class via the
    // donor∪category membership check below.
    let expected_extra: [(u16, &str, &[&str]); 8] = [
        (69, "push_array", &["$state", "true", "true"]),
        (186, "push_array", &["$state", "false", "false"]),
        (529, "pop_array", &["$state", "true"]),
        (531, "pop_array", &["$state", "false"]),
        (593, "db_find", &["$state", "true"]),
        (847, "db_find", &["$state", "false"]),
        (869, "add", &["$state", "(short) -32146"]),
        (945, "push_array", &["$state", "true", "false"]),
    ];
    for (id, method, args) in expected_extra {
        let dispatch = &switch.dispatches[&id];
        assert_eq!(dispatch.method.as_deref(), Some(method), "id {id} method");
        assert_eq!(
            dispatch.args,
            args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            "id {id} args"
        );
    }

    // Every qualified class is one of the three donor command classes or one of
    // the post-split category classes. (Pre-split, only the donor classes
    // appear; post-split every local dispatch is qualified by its category.)
    let donors = ["TwitchCommands", "QuestCommands", "QuickChatDynamicCommand"];
    let categories = [
        "IfOps",
        "CcOps",
        "DetailOps",
        "CamOps",
        "WorldMapOps",
        "ChatOps",
        "SocialOps",
        "AudioOps",
        "ConfigOps",
        "QuestOps",
        "StatOps",
        "InvOps",
        "TelemetryOps",
        "DbOps",
        "CoreOps",
        "MiscOps",
    ];
    for (id, dispatch) in &switch.dispatches {
        if let Some(class) = &dispatch.class {
            assert!(
                donors.contains(&class.as_str()) || categories.contains(&class.as_str()),
                "id {id} unexpected qualifying class `{class}`"
            );
        }
    }

    // Enum constants parsed.
    let enum_path = command_enum_path(&client_root);
    let enum_src = std::fs::read_to_string(&enum_path).expect("read ClientScriptCommand.java");
    let enums = parse_enum(&enum_src, &enum_path).expect("parse enum");
    assert!(
        enums.fields.len() >= 1400,
        "expected >= 1400 enum constants, got {}",
        enums.fields.len()
    );

    // No duplicate ids/names within sources (C8 == 0 for these sources).
    assert!(switch.duplicate_ids.is_empty(), "switch duplicate ids");
    assert!(enums.duplicate_ids.is_empty(), "enum duplicate ids");
    assert!(enums.duplicate_names.is_empty(), "enum duplicate names");

    // C1/C2 require the registry name authority. Build minimal id-set check against
    // the real opcodes-910.txt to assert no C1 (every switch id has a name) and no
    // C2 (every named id has a switch case).
    let data_dir = crate_dir().join("data");
    let names_910 =
        std::fs::read_to_string(data_dir.join("opcodes-910.txt")).expect("read opcodes-910.txt");
    let mut named_ids = std::collections::BTreeSet::new();
    for line in names_910.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let (_, id_text) = line.split_once(',').expect("name,id");
        named_ids.insert(id_text.trim().parse::<u16>().expect("id"));
    }
    let switch_ids: std::collections::BTreeSet<u16> = switch.dispatches.keys().copied().collect();
    assert!(
        switch_ids.difference(&named_ids).next().is_none(),
        "C1: switch id with no opcodes-910 name"
    );
    assert!(
        named_ids.difference(&switch_ids).next().is_none(),
        "C2: opcodes-910 name with no switch case"
    );
}
