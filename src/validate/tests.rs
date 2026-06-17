use super::{Cs2Validator, ResolverContext, ValidationError};
use crate::script::{CompiledScript, Instruction, OpcodeBook, Operand, encode_script};
use crate::vars::{VarDomain, VarEntry};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

fn test_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

fn build_ctx(
    build: u32,
    scripts: &[(u32, CompiledScript)],
) -> crate::error::Result<ResolverContext> {
    let opcode_book = OpcodeBook::load(&test_data_dir(), build, 0)?;
    let mut raw_scripts = BTreeMap::new();
    let mut decoded_scripts = BTreeMap::new();
    for (script_id, script) in scripts {
        raw_scripts.insert(*script_id, encode_script(script, &opcode_book, build)?);
        decoded_scripts.insert(*script_id, script.clone());
    }

    Ok(ResolverContext {
        build,
        opcode_book,
        interfaces: BTreeMap::new(),
        scripts: raw_scripts,
        varps_by_domain: HashMap::new(),
        varbits: BTreeMap::new(),
        params: BTreeMap::new(),
        enums: BTreeMap::new(),
        structs: BTreeMap::new(),
        decoded_scripts,
        parsed_components: BTreeMap::new(),
        npcs: BTreeMap::new(),
        objs: BTreeMap::new(),
        locs: BTreeMap::new(),
        seqs: BTreeMap::new(),
        spots: BTreeMap::new(),
        invs: BTreeMap::new(),
        dbtables: BTreeMap::new(),
        dbrows: BTreeMap::new(),
    })
}

#[test]
fn gosub_stack_effect_resolves_group_id_void_callee() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let void_callee_id = 100_u32 << 16;
    let void_entry_id = 200_u32 << 16;

    let void_callee = CompiledScript {
        name: Some("[proc,callee_void]".to_string()),
        local_count_int: 0,
        local_count_object: 0,
        local_count_long: 0,
        argument_count_int: 2,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![Instruction {
            opcode: 0,
            command: "return".to_string(),
            operand: Operand::Byte(0),
        }],
    };

    let void_entry = CompiledScript {
        name: Some("[proc,caller_void]".to_string()),
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
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(2),
            },
            Instruction {
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: Operand::Script(100),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(
        BUILD,
        &[(void_callee_id, void_callee), (void_entry_id, void_entry)],
    )?;
    let report = Cs2Validator::new(&ctx).validate(void_entry_id);

    assert!(
        report
            .warnings
            .iter()
            .all(|warning| !warning.contains("not found")),
        "unexpected warnings: {:?}",
        report.warnings
    );
    assert!(report.errors.iter().any(|error| matches!(
        error,
        ValidationError::StackUnderflow { index: 3, stack, .. } if stack == "int"
    )));
    Ok(())
}

#[test]
fn gosub_stack_effect_resolves_group_id_value_callee() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let value_callee_id = 101_u32 << 16;
    let value_entry_id = 201_u32 << 16;

    let value_callee = CompiledScript {
        name: Some("[proc,callee_value]".to_string()),
        local_count_int: 0,
        local_count_object: 0,
        local_count_long: 0,
        argument_count_int: 2,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(7),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let value_entry = CompiledScript {
        name: Some("[proc,caller_value]".to_string()),
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
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(2),
            },
            Instruction {
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: Operand::Script(101),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(
        BUILD,
        &[
            (value_callee_id, value_callee),
            (value_entry_id, value_entry),
        ],
    )?;
    let report = Cs2Validator::new(&ctx).validate(value_entry_id);

    assert!(
        report
            .warnings
            .iter()
            .all(|warning| !warning.contains("not found")),
        "unexpected warnings: {:?}",
        report.warnings
    );
    assert!(
        !report
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
        "unexpected errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn oc_param_unknown_result_can_flow_to_object_stack() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 300_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,oc_param_string]".to_string()),
        local_count_int: 0,
        local_count_object: 1,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(2),
            },
            Instruction {
                opcode: 0,
                command: "oc_param".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        !report
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
        "unexpected errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn enum_unknown_result_can_flow_to_int_stack() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 301_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,enum_value]".to_string()),
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
                operand: Operand::Int(105),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(10),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(20),
            },
            Instruction {
                opcode: 0,
                command: "_enum".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        !report
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
        "unexpected errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn fixed_domain_var_refs_use_config_types_for_stack_validation() -> crate::error::Result<()> {
    const BUILD: u32 = 718;
    let script_id = 302_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,fixed_domain_var_refs]".to_string()),
        local_count_int: 0,
        local_count_object: 1,
        local_count_long: 1,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Str("x".to_string()),
            },
            Instruction {
                opcode: 0,
                command: "pop_varc_string".to_string(),
                operand: Operand::VarRef(crate::script::VarRef {
                    domain: VarDomain::Client,
                    id: 9,
                    transmog: false,
                }),
            },
            Instruction {
                opcode: 0,
                command: "push_varc_string".to_string(),
                operand: Operand::VarRef(crate::script::VarRef {
                    domain: VarDomain::Client,
                    id: 9,
                    transmog: false,
                }),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_varclansetting_long".to_string(),
                operand: Operand::VarRef(crate::script::VarRef {
                    domain: VarDomain::ClanSetting,
                    id: 77,
                    transmog: false,
                }),
            },
            Instruction {
                opcode: 0,
                command: "pop_long_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let mut ctx = build_ctx(BUILD, &[(script_id, script)])?;
    ctx.varps_by_domain.insert(
        VarDomain::Client,
        BTreeMap::from([(
            9_u32,
            VarEntry {
                id: 9,
                domain: VarDomain::Client,
                var_name: "varclient_9".to_string(),
                type_id: Some(2),
                lifetime: None,
                transmit_level: None,
                client_code: None,
                domain_default: true,
                wiki_sync: false,
            },
        )]),
    );
    ctx.varps_by_domain.insert(
        VarDomain::ClanSetting,
        BTreeMap::from([(
            77_u32,
            VarEntry {
                id: 77,
                domain: VarDomain::ClanSetting,
                var_name: "varclansetting_77".to_string(),
                type_id: Some(1),
                lifetime: None,
                transmit_level: None,
                client_code: None,
                domain_default: true,
                wiki_sync: false,
            },
        )]),
    );

    let report = Cs2Validator::new(&ctx).validate(script_id);
    assert!(
        !report
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
        "unexpected errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn typed_constant_push_resolves_stack_type_from_operand() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 304_u32 << 16;

    // push_constant_string is the typed-constant opcode: an int-tagged
    // constant feeds the int stack, so the following pop_int_local must not
    // underflow. Mirror the same for the long-tagged form.
    let script = CompiledScript {
        name: Some("[proc,typed_constant_push]".to_string()),
        local_count_int: 1,
        local_count_object: 0,
        local_count_long: 1,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Int(5),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Long(7),
            },
            Instruction {
                opcode: 0,
                command: "pop_long_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);
    assert!(
        !report
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
        "int/long-tagged typed constants should feed the matching stack: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn clan_and_player_group_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 303_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,clan_player_group_helpers]".to_string()),
        local_count_int: 0,
        local_count_object: 1,
        local_count_long: 1,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "activeclansettings_getclanname".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(2),
            },
            Instruction {
                opcode: 0,
                command: "activeclanchannel_getuserdisplayname".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(2),
            },
            Instruction {
                opcode: 0,
                command: "player_group_member_get_same_world_var".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_long_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        !report
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
        "unexpected errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn login_shop_notification_and_detail_helpers_use_exact_stack_effects() -> crate::error::Result<()>
{
    const BUILD: u32 = 910;
    let script_id = 304_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,login_shop_notification_detail_helpers]".to_string()),
        local_count_int: 3,
        local_count_object: 2,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "create_get_email".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Str("title".to_string()),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Str("body".to_string()),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(2),
            },
            Instruction {
                opcode: 0,
                command: "notifications_sendlocal".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(5),
            },
            Instruction {
                opcode: 0,
                command: "shop_getcategorydescription".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "autosetup_dosetupstatus".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "login_resetreply".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        !report
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
        "unexpected errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn db_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 305_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,db_helper_stack_effects]".to_string()),
        local_count_int: 2,
        local_count_object: 0,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(10),
            },
            Instruction {
                opcode: 0,
                command: "db_listall".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(20),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(30),
            },
            Instruction {
                opcode: 0,
                command: "db_find".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(20),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(30),
            },
            Instruction {
                opcode: 0,
                command: "db_find_with_count".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "db_findnext".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(2),
            },
            Instruction {
                opcode: 0,
                command: "db_getfieldcount".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(0),
            },
            Instruction {
                opcode: 0,
                command: "db_find_get".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "db_getrowtable".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(11),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(22),
            },
            Instruction {
                opcode: 0,
                command: "db_find_refine".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        report.errors.is_empty(),
        "unexpected validation errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn minimenu_and_messagebox_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 306_u32 << 16;

    let mut code = Vec::new();
    for value in 0..11 {
        code.push(Instruction {
            opcode: 0,
            command: "push_constant_int".to_string(),
            operand: Operand::Int(value),
        });
    }
    code.push(Instruction {
        opcode: 0,
        command: "setup_messagebox".to_string(),
        operand: Operand::Byte(0),
    });
    for value in 0..12 {
        code.push(Instruction {
            opcode: 0,
            command: "push_constant_int".to_string(),
            operand: Operand::Int(value),
        });
    }
    code.push(Instruction {
        opcode: 0,
        command: "formatminimenu".to_string(),
        operand: Operand::Byte(0),
    });
    code.extend([
        Instruction {
            opcode: 0,
            command: "get_active_minimenu_entry".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(1),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(2),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(0),
        },
        Instruction {
            opcode: 0,
            command: "get_second_minimenu_entry".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(1),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(2),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(1),
        },
        Instruction {
            opcode: 0,
            command: "get_minimenu_length".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(1),
        },
        Instruction {
            opcode: 0,
            command: "get_minimenu_target".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_string_local".to_string(),
            operand: Operand::Local(1),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(2),
        },
        Instruction {
            opcode: 0,
            command: "get_mousebuttons".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(1),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(2),
        },
        Instruction {
            opcode: 0,
            command: "push_constant_int".to_string(),
            operand: Operand::Int(10),
        },
        Instruction {
            opcode: 0,
            command: "push_constant_int".to_string(),
            operand: Operand::Int(20),
        },
        Instruction {
            opcode: 0,
            command: "minimenuopen".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "pop_int_local".to_string(),
            operand: Operand::Local(0),
        },
        Instruction {
            opcode: 0,
            command: "push_constant_int".to_string(),
            operand: Operand::Int(5),
        },
        Instruction {
            opcode: 0,
            command: "setsubmenuminlength".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "push_constant_string".to_string(),
            operand: Operand::Str("Attack".to_string()),
        },
        Instruction {
            opcode: 0,
            command: "push_constant_int".to_string(),
            operand: Operand::Int(1),
        },
        Instruction {
            opcode: 0,
            command: "opplayer".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "push_constant_string".to_string(),
            operand: Operand::Str("Use".to_string()),
        },
        Instruction {
            opcode: 0,
            command: "opplayert".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "defaultminimenu".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "minimenu_close".to_string(),
            operand: Operand::Byte(0),
        },
        Instruction {
            opcode: 0,
            command: "return".to_string(),
            operand: Operand::Byte(0),
        },
    ]);

    let script = CompiledScript {
        name: Some("[proc,minimenu_messagebox_helper_stack_effects]".to_string()),
        local_count_int: 3,
        local_count_object: 3,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code,
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        report.errors.is_empty(),
        "unexpected validation errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn interface_misc_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 307_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,interface_misc_helper_stack_effects]".to_string()),
        local_count_int: 4,
        local_count_object: 0,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "window_getinsets".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "targetmode_active".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(100),
            },
            Instruction {
                opcode: 0,
                command: "if_hassub".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(100),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(7),
            },
            Instruction {
                opcode: 0,
                command: "if_hassubmodal".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(100),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(8),
            },
            Instruction {
                opcode: 0,
                command: "if_hassuboverlay".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "if_get_gamescreen".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "if_set_gamescreen_enabled".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(25),
            },
            Instruction {
                opcode: 0,
                command: "interface_setpickingradius".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "interface_getpickingradius".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "if_close".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(200),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(300),
            },
            Instruction {
                opcode: 0,
                command: "if_opensubclient".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(200),
            },
            Instruction {
                opcode: 0,
                command: "if_closesubclient".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "force_interface_drag".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "cancel_interface_drag".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "targetmode_cancel".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        report.errors.is_empty(),
        "unexpected validation errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn display_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 308_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,display_helper_stack_effects]".to_string()),
        local_count_int: 5,
        local_count_object: 0,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "pushCanvasSize".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pushZeroInsets".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(7),
            },
            Instruction {
                opcode: 0,
                command: "pushFontMetrics".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(4),
            },
            Instruction {
                opcode: 0,
                command: "viewport_geteffectivesize".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "viewport_getzoom".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "viewport_getfov".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(800),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(600),
            },
            Instruction {
                opcode: 0,
                command: "fullscreen_enter".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "fullscreen_modecount".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(0),
            },
            Instruction {
                opcode: 0,
                command: "fullscreen_getmode".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(123),
            },
            Instruction {
                opcode: 0,
                command: "if_getnextsubid".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(4),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        report.errors.is_empty(),
        "unexpected validation errors: {:?}",
        report.errors
    );
    Ok(())
}

#[test]
fn worldlist_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
    const BUILD: u32 = 910;
    let script_id = 309_u32 << 16;

    let script = CompiledScript {
        name: Some("[proc,worldlist_helper_stack_effects]".to_string()),
        local_count_int: 9,
        local_count_object: 6,
        local_count_long: 0,
        argument_count_int: 0,
        argument_count_object: 0,
        argument_count_long: 0,
        code: vec![
            Instruction {
                opcode: 0,
                command: "worldlist_start".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(4),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_next".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(5),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(6),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(4),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(7),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(5),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(8),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(302),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_specific".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(3),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_fetch".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(4),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Str("world302.example".to_string()),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(302),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_switch".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(5),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(3),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(4),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(0),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_sort".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_autoworld".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_pingworlds".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "worldlist_specific_thisworld".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(6),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ],
    };

    let ctx = build_ctx(BUILD, &[(script_id, script)])?;
    let report = Cs2Validator::new(&ctx).validate(script_id);

    assert!(
        report.errors.is_empty(),
        "unexpected validation errors: {:?}",
        report.errors
    );
    Ok(())
}
