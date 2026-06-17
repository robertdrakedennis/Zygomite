    use super::{MigrationAnalyzer, RemapTable, VarpRemapTarget};
    use crate::config::ParamEntry;
    use crate::dep_tree::ResolverContext;
    use crate::interface::{ComponentDeps, VarTransmitRef};
    use crate::script::{CompiledScript, Instruction, OpcodeBook, Operand, encode_script};
    use crate::vars::{VarBitEntry, VarDomain, VarEntry};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::PathBuf;

    fn test_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
    }

    fn simple_script(name: &str, instructions: Vec<Instruction>, locals: u16) -> CompiledScript {
        CompiledScript {
            name: Some(name.to_string()),
            local_count_int: locals,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: instructions,
        }
    }

    fn build_ctx(
        build: u32,
        scripts: &[(u32, CompiledScript)],
        varps: &[(VarDomain, u32, VarEntry)],
        varbits: &[(u32, VarBitEntry)],
        components: &[(u32, u32, ComponentDeps)],
    ) -> crate::error::Result<ResolverContext> {
        let opcode_book = OpcodeBook::load(&test_data_dir(), build, 0)?;
        let mut raw_scripts = BTreeMap::new();
        let mut decoded_scripts = BTreeMap::new();
        for (script_group_id, script) in scripts {
            let packed_id = script_group_id << 16;
            raw_scripts.insert(packed_id, encode_script(script, &opcode_book, build)?);
            decoded_scripts.insert(packed_id, script.clone());
        }

        let mut varps_by_domain: HashMap<VarDomain, BTreeMap<u32, VarEntry>> = HashMap::new();
        for (domain, id, entry) in varps {
            varps_by_domain
                .entry(*domain)
                .or_default()
                .insert(*id, entry.clone());
        }

        let mut varbit_map = BTreeMap::new();
        for (id, entry) in varbits {
            varbit_map.insert(*id, entry.clone());
        }

        let mut parsed_components = BTreeMap::new();
        for (interface_id, component_id, deps) in components {
            parsed_components
                .entry(*interface_id)
                .or_insert_with(BTreeMap::new)
                .insert(*component_id, deps.clone());
        }

        Ok(ResolverContext {
            build,
            opcode_book,
            interfaces: BTreeMap::new(),
            scripts: raw_scripts,
            varps_by_domain,
            varbits: varbit_map,
            params: BTreeMap::<u32, ParamEntry>::new(),
            enums: BTreeMap::new(),
            structs: BTreeMap::new(),
            decoded_scripts,
            parsed_components,
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

    fn player_var(id: u32) -> VarEntry {
        VarEntry {
            id,
            domain: VarDomain::Player,
            var_name: format!("varplayer_{id}"),
            type_id: Some(0),
            lifetime: None,
            transmit_level: None,
            client_code: None,
            domain_default: true,
            wiki_sync: false,
        }
    }

    fn player_varbit(id: u32, base_var: u32) -> VarBitEntry {
        VarBitEntry {
            id,
            varbit_name: format!("varbit_{id}"),
            domain: Some(VarDomain::Player),
            base_var: Some(base_var),
            start_bit: Some(0),
            end_bit: Some(1),
            wiki_sync: false,
        }
    }

    #[test]
    fn validate_script_target_rewrites_and_encodes_dependency_bundle() -> crate::error::Result<()> {
        let source_root = simple_script(
            "[proc,source_root]",
            vec![
                Instruction {
                    opcode: 0,
                    command: "push_var".to_string(),
                    operand: Operand::VarRef(crate::script::VarRef {
                        domain: VarDomain::Player,
                        id: 1,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varbit".to_string(),
                    operand: Operand::VarBitRef(crate::script::VarBitRef {
                        id: 5,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "gosub_with_params".to_string(),
                    operand: Operand::Script(200),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
            1,
        );
        let source_leaf = simple_script(
            "[proc,source_leaf]",
            vec![Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            }],
            0,
        );
        let source_ctx = build_ctx(
            947,
            &[(100, source_root), (200, source_leaf)],
            &[(VarDomain::Player, 1, player_var(1))],
            &[(5, player_varbit(5, 1))],
            &[],
        )?;
        let target_ctx = build_ctx(910, &[], &[], &[], &[])?;

        let analyzer = MigrationAnalyzer::new(source_ctx, target_ctx);
        let script_report = analyzer.analyze_script(100);
        let mut remap = RemapTable::default();
        remap.scripts.insert(200, 700);
        remap.varps.insert(
            "player:1".to_string(),
            VarpRemapTarget {
                domain: "player".to_string(),
                id: 500,
            },
        );
        remap.varbits.insert(5, 600);

        let target_validation =
            analyzer.validate_script_target(&script_report.entities, Some(&remap), false);
        assert_eq!(target_validation.summary.scripts_checked, 2);
        assert_eq!(target_validation.summary.scripts_encoded, 2);
        assert_eq!(target_validation.summary.scripts_valid, 2);
        assert_eq!(target_validation.summary.scripts_with_errors, 0);
        assert_eq!(target_validation.summary.scripts_blocked, 0);

        let root = target_validation
            .scripts
            .iter()
            .find(|script| script.source_script_id == 100)
            .expect("root script validation");
        assert!(
            root.reference_updates
                .iter()
                .any(|update| update.to == "varp player:500")
        );
        assert!(
            root.reference_updates
                .iter()
                .any(|update| update.to == "varbit 600")
        );
        assert!(
            root.reference_updates
                .iter()
                .any(|update| update.to == "script 700")
        );
        Ok(())
    }

    #[test]
    fn validate_interface_target_checks_component_refs_against_overlay_bundle()
    -> crate::error::Result<()> {
        let source_root = simple_script(
            "[proc,source_root]",
            vec![Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            }],
            0,
        );
        let mut component = ComponentDeps {
            component_type: "layer".to_string(),
            name: Some("stockmarket".to_string()),
            children: Vec::new(),
            scripts: HashSet::new(),
            onload_scripts: HashSet::new(),
            varps: HashSet::new(),
            varbits: HashSet::new(),
            invs: HashSet::new(),
            stats: HashSet::new(),
            graphics: HashSet::new(),
            models: HashSet::new(),
            cursors: HashSet::new(),
            stylesheets: HashSet::new(),
            params: HashSet::new(),
            seqs: HashSet::new(),
            fontmetrics: HashSet::new(),
            textures: HashSet::new(),
            enums: HashSet::new(),
        };
        component.scripts.insert(100);
        component.varps.insert(VarTransmitRef::Player(1));
        component.varbits.insert(5);

        let source_ctx = build_ctx(
            947,
            &[(100, source_root)],
            &[(VarDomain::Player, 1, player_var(1))],
            &[(5, player_varbit(5, 1))],
            &[(77, 1, component)],
        )?;
        let target_ctx = build_ctx(910, &[], &[], &[], &[])?;

        let analyzer = MigrationAnalyzer::new(source_ctx, target_ctx);
        let interface_report = analyzer.analyze_interface(77);
        let target_validation =
            analyzer.validate_interface_target(77, &interface_report.entities, None, false);

        assert_eq!(target_validation.summary.components_checked, 1);
        assert_eq!(target_validation.summary.components_blocked, 0);
        assert_eq!(target_validation.summary.scripts_checked, 1);
        assert_eq!(target_validation.summary.scripts_valid, 1);
        assert!(
            target_validation
                .components
                .iter()
                .all(|component| component.blocking_issues.is_empty())
        );
        Ok(())
    }
