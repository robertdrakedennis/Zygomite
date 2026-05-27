use crate::dep_tree::{EntityType, ResolverContext, var_ref_to_entity_ref};
use crate::error::{Context, Result};
use crate::interface::{ComponentDeps, VarTransmitRef};
use crate::script::{CompiledScript, Operand, decode_script};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum DependencySourceKind {
    Operand,
    Hook,
    TransmitList,
    ComponentField,
    CommandStackSite,
}

#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum DependencyConfidence {
    Exact,
    Heuristic,
}

#[derive(Debug, Clone, Copy, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum RewriteStrategy {
    DirectOperand,
    ComponentMetadata,
    HeuristicStack,
}

#[derive(Debug, Clone, Serialize, Eq, PartialEq)]
pub struct DependencySite {
    pub entity_type: EntityType,
    pub id: u32,
    pub location: String,
    pub source_kind: DependencySourceKind,
    pub confidence: DependencyConfidence,
    pub rewrite_strategy: RewriteStrategy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentDependencyRecord {
    pub interface_id: u32,
    pub component_id: u32,
    pub component_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub dependency_sites: Vec<DependencySite>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptDependencyRecord {
    pub script_id: u32,
    pub packed_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_name: Option<String>,
    pub dependency_sites: Vec<DependencySite>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DependencyCoverage {
    pub script_count: usize,
    pub component_count: usize,
    pub total_sites: usize,
    pub exact_sites: usize,
    pub heuristic_sites: usize,
    pub by_entity_type: BTreeMap<String, usize>,
    pub by_source_kind: BTreeMap<String, usize>,
    pub by_rewrite_strategy: BTreeMap<String, usize>,
}

pub fn collect_component_dependency_records(
    components: &BTreeMap<u32, BTreeMap<u32, ComponentDeps>>,
) -> Vec<ComponentDependencyRecord> {
    let mut records = components
        .iter()
        .collect::<Vec<_>>()
        .into_par_iter()
        .flat_map_iter(|(&interface_id, group)| {
            group.iter().map(move |(&component_id, deps)| {
                let mut dependency_sites = collect_component_dependency_sites(deps);
                dependency_sites.sort_by_key(site_sort_key);
                ComponentDependencyRecord {
                    interface_id,
                    component_id,
                    component_type: deps.component_type.clone(),
                    name: deps.name.clone(),
                    dependency_sites,
                }
            })
        })
        .collect::<Vec<_>>();
    records.sort_by_key(|record| (record.interface_id, record.component_id));
    records
}

pub fn collect_component_dependency_sites(deps: &ComponentDeps) -> Vec<DependencySite> {
    let mut sites = Vec::new();

    let mut hook_scripts = deps.scripts.iter().copied().collect::<Vec<_>>();
    hook_scripts.sort_unstable();
    for script_id in hook_scripts {
        let location = if deps.onload_scripts.contains(&script_id) {
            "hook.onload".to_string()
        } else {
            "hook.any".to_string()
        };
        sites.push(site(
            EntityType::Script,
            script_id,
            location,
            DependencySourceKind::Hook,
            DependencyConfidence::Exact,
            RewriteStrategy::ComponentMetadata,
            None,
            None,
        ));
    }

    let mut varps = deps
        .varps
        .iter()
        .map(var_transmit_site)
        .collect::<Vec<DependencySite>>();
    sites.append(&mut varps);

    push_ids(
        &mut sites,
        EntityType::VarBit,
        &deps.varbits,
        "transmit_list.varbit",
        DependencySourceKind::TransmitList,
    );
    push_ids(
        &mut sites,
        EntityType::Inv,
        &deps.invs,
        "transmit_list.inv",
        DependencySourceKind::TransmitList,
    );
    push_ids(
        &mut sites,
        EntityType::Config,
        &deps.stats,
        "transmit_list.stat",
        DependencySourceKind::TransmitList,
    );
    push_ids(
        &mut sites,
        EntityType::Graphic,
        &deps.graphics,
        "field.graphic",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::Model,
        &deps.models,
        "field.model",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::Cursor,
        &deps.cursors,
        "field.cursor",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::Stylesheet,
        &deps.stylesheets,
        "field.stylesheet",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::Param,
        &deps.params,
        "field.param",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::Seq,
        &deps.seqs,
        "field.seq",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::FontMetrics,
        &deps.fontmetrics,
        "field.fontmetrics",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::Texture,
        &deps.textures,
        "field.texture",
        DependencySourceKind::ComponentField,
    );
    push_ids(
        &mut sites,
        EntityType::Enum,
        &deps.enums,
        "field.enum",
        DependencySourceKind::ComponentField,
    );

    sites
}

pub fn collect_script_dependency_record(
    packed_id: u32,
    script: &CompiledScript,
) -> ScriptDependencyRecord {
    let mut dependency_sites = collect_script_dependency_sites(script);
    dependency_sites.sort_by_key(site_sort_key);
    ScriptDependencyRecord {
        script_id: packed_id >> 16,
        packed_id,
        script_name: script.name.clone(),
        dependency_sites,
    }
}

pub fn collect_script_dependency_sites(script: &CompiledScript) -> Vec<DependencySite> {
    let mut sites = Vec::new();
    let mut stack = Vec::new();

    for (index, instruction) in script.code.iter().enumerate() {
        let location = format!("instruction[{index}]");
        match &instruction.operand {
            Operand::VarRef(var_ref) => {
                let entity_ref = var_ref_to_entity_ref(var_ref);
                sites.push(site(
                    entity_ref.entity_type,
                    entity_ref.id,
                    location,
                    DependencySourceKind::Operand,
                    DependencyConfidence::Exact,
                    RewriteStrategy::DirectOperand,
                    Some(instruction.command.clone()),
                    None,
                ));
                stack.clear();
            }
            Operand::VarBitRef(varbit_ref) => {
                sites.push(site(
                    EntityType::VarBit,
                    u32::from(varbit_ref.id),
                    location,
                    DependencySourceKind::Operand,
                    DependencyConfidence::Exact,
                    RewriteStrategy::DirectOperand,
                    Some(instruction.command.clone()),
                    None,
                ));
                stack.clear();
            }
            Operand::Script(script_id) => {
                if *script_id >= 0 {
                    sites.push(site(
                        EntityType::Script,
                        *script_id as u32,
                        location,
                        DependencySourceKind::Operand,
                        DependencyConfidence::Exact,
                        RewriteStrategy::DirectOperand,
                        Some(instruction.command.clone()),
                        None,
                    ));
                }
                stack.clear();
            }
            Operand::Int(value) if instruction.command == "push_constant_int" => {
                stack.push(StackValue::Int(*value));
            }
            Operand::Str(value) if instruction.command == "push_constant_string" => {
                stack.push(StackValue::Str(value.clone()));
            }
            _ => {
                if is_interface_hook_opcode(&instruction.command) {
                    extract_hook_dependency_site(
                        &mut sites,
                        &mut stack,
                        &instruction.command,
                        &location,
                    );
                    stack.clear();
                    continue;
                }
                if let Some((pops, asset_type)) = asset_command_info(&instruction.command) {
                    let mut popped = Vec::with_capacity(pops);
                    for _ in 0..pops {
                        if let Some(value) = stack.pop() {
                            popped.push(value);
                        }
                    }
                    if let Some(asset_id) = popped.iter().rev().find_map(StackValue::as_u32) {
                        sites.push(site(
                            asset_type,
                            asset_id,
                            location,
                            DependencySourceKind::CommandStackSite,
                            DependencyConfidence::Heuristic,
                            RewriteStrategy::HeuristicStack,
                            Some(instruction.command.clone()),
                            Some("inferred asset id from int stack".to_string()),
                        ));
                    }
                }
                stack.clear();
            }
        }
    }

    sites
}

pub fn write_dependency_files(ctx: &ResolverContext, out_dir: &Path) -> Result<DependencyCoverage> {
    fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    let component_records = collect_component_dependency_records(&ctx.parsed_components);
    let component_path = out_dir.join("components.json");
    fs::write(
        &component_path,
        serde_json::to_string_pretty(&component_records)?,
    )
    .with_context(|| format!("writing {}", component_path.display()))?;

    let scripts_path = out_dir.join("scripts.jsonl");
    let scripts_file = fs::File::create(&scripts_path)
        .with_context(|| format!("creating {}", scripts_path.display()))?;
    let mut writer = BufWriter::new(scripts_file);

    let mut coverage = DependencyCoverage {
        component_count: component_records.len(),
        ..DependencyCoverage::default()
    };
    for record in &component_records {
        coverage.record_sites(&record.dependency_sites);
    }

    let mut script_records = ctx
        .scripts
        .iter()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(&packed_id, bytes)| -> Result<ScriptDependencyRecord> {
            let record = if let Some(script) = ctx.decoded_scripts.get(&packed_id) {
                collect_script_dependency_record(packed_id, script)
            } else {
                let script = decode_script(bytes, &ctx.opcode_book, ctx.build)
                    .with_context(|| format!("decoding script {packed_id} for dependency dump"))?;
                collect_script_dependency_record(packed_id, &script)
            };
            Ok(record)
        })
        .collect::<Result<Vec<_>>>()?;
    script_records.sort_by_key(|record| record.packed_id);

    for record in script_records {
        serde_json::to_writer(&mut writer, &record)
            .with_context(|| format!("writing {}", scripts_path.display()))?;
        writer.write_all(b"\n")?;
        coverage.script_count += 1;
        coverage.record_sites(&record.dependency_sites);
    }
    writer.flush()?;

    let coverage_path = out_dir.join("coverage.json");
    fs::write(&coverage_path, serde_json::to_string_pretty(&coverage)?)
        .with_context(|| format!("writing {}", coverage_path.display()))?;

    eprintln!(
        "Wrote dependency artifacts to {} ({} components, {} scripts, {} sites)",
        out_dir.display(),
        coverage.component_count,
        coverage.script_count,
        coverage.total_sites
    );

    Ok(coverage)
}

impl DependencyCoverage {
    pub fn record_sites(&mut self, sites: &[DependencySite]) {
        self.total_sites += sites.len();
        for site in sites {
            match site.confidence {
                DependencyConfidence::Exact => self.exact_sites += 1,
                DependencyConfidence::Heuristic => self.heuristic_sites += 1,
            }
            *self
                .by_entity_type
                .entry(site.entity_type.as_label().to_string())
                .or_default() += 1;
            *self
                .by_source_kind
                .entry(
                    match site.source_kind {
                        DependencySourceKind::Operand => "operand",
                        DependencySourceKind::Hook => "hook",
                        DependencySourceKind::TransmitList => "transmit_list",
                        DependencySourceKind::ComponentField => "component_field",
                        DependencySourceKind::CommandStackSite => "command_stack_site",
                    }
                    .to_string(),
                )
                .or_default() += 1;
            *self
                .by_rewrite_strategy
                .entry(
                    match site.rewrite_strategy {
                        RewriteStrategy::DirectOperand => "direct_operand",
                        RewriteStrategy::ComponentMetadata => "component_metadata",
                        RewriteStrategy::HeuristicStack => "heuristic_stack",
                    }
                    .to_string(),
                )
                .or_default() += 1;
        }
    }
}

#[derive(Debug, Clone)]
enum StackValue {
    Int(i32),
    Str(String),
}

impl StackValue {
    fn as_u32(&self) -> Option<u32> {
        match self {
            Self::Int(value) if *value >= 0 => Some(*value as u32),
            Self::Str(_) => None,
            Self::Int(_) => None,
        }
    }
}

fn push_ids(
    sites: &mut Vec<DependencySite>,
    entity_type: EntityType,
    ids: &std::collections::HashSet<u32>,
    location_prefix: &str,
    source_kind: DependencySourceKind,
) {
    let mut ids = ids.iter().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    for id in ids {
        sites.push(site(
            entity_type,
            id,
            format!("{location_prefix}[{id}]"),
            source_kind,
            DependencyConfidence::Exact,
            RewriteStrategy::ComponentMetadata,
            None,
            None,
        ));
    }
}

fn var_transmit_site(var_ref: &VarTransmitRef) -> DependencySite {
    let (entity_type, id, domain_label) = match var_ref {
        VarTransmitRef::Player(id) => (EntityType::VarPlayer, *id, "player"),
        VarTransmitRef::Npc(id) => (EntityType::VarNpc, *id, "npc"),
        VarTransmitRef::Client(id) => (EntityType::VarClient, *id, "client"),
        VarTransmitRef::World(id) => (EntityType::VarWorld, *id, "world"),
        VarTransmitRef::Region(id) => (EntityType::VarRegion, *id, "region"),
        VarTransmitRef::Object(id) => (EntityType::VarObject, *id, "object"),
        VarTransmitRef::Clan(id) => (EntityType::VarClan, *id, "clan"),
        VarTransmitRef::ClanSetting(id) => (EntityType::VarClanSetting, *id, "clan_setting"),
        VarTransmitRef::Controller(id) => (EntityType::VarController, *id, "controller"),
        VarTransmitRef::Global(id) => (EntityType::VarGlobal, *id, "global"),
        VarTransmitRef::PlayerGroup(id) => (EntityType::VarPlayerGroup, *id, "player_group"),
        VarTransmitRef::VarClientString(id) => (EntityType::VarClient, *id, "client_string"),
    };
    site(
        entity_type,
        id,
        format!("transmit_list.varp.{domain_label}[{id}]"),
        DependencySourceKind::TransmitList,
        DependencyConfidence::Exact,
        RewriteStrategy::ComponentMetadata,
        None,
        None,
    )
}

fn extract_hook_dependency_site(
    sites: &mut Vec<DependencySite>,
    stack: &mut Vec<StackValue>,
    command: &str,
    location: &str,
) {
    let has_component = command.starts_with("if_");
    let component = has_component.then(|| stack.pop()).flatten();
    let descriptor = stack.pop();
    let Some(StackValue::Str(raw_descriptor)) = descriptor else {
        return;
    };

    let mut signature = raw_descriptor.as_str();
    if let Some(stripped) = signature.strip_suffix('Y') {
        signature = stripped;
        let watcher_count = stack
            .pop()
            .and_then(|value| match value {
                StackValue::Int(count) if count >= 0 => usize::try_from(count).ok(),
                _ => None,
            })
            .unwrap_or(0);
        for _ in 0..watcher_count {
            let _ = stack.pop();
        }
    }

    for _ in signature.chars() {
        let _ = stack.pop();
    }

    if let Some(script_id) = stack.pop().and_then(|value| value.as_u32()) {
        sites.push(site(
            EntityType::Script,
            script_id,
            location.to_string(),
            DependencySourceKind::CommandStackSite,
            DependencyConfidence::Heuristic,
            RewriteStrategy::HeuristicStack,
            Some(command.to_string()),
            Some("inferred callback script id from hook descriptor".to_string()),
        ));
    }

    if let Some(component_id) = component.and_then(|value| value.as_u32()) {
        sites.push(site(
            EntityType::Component,
            component_id,
            location.to_string(),
            DependencySourceKind::CommandStackSite,
            DependencyConfidence::Heuristic,
            RewriteStrategy::HeuristicStack,
            Some(command.to_string()),
            Some("inferred component id from hook stack argument".to_string()),
        ));
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal helper mirrors serialized DependencySite fields"
)]
fn site(
    entity_type: EntityType,
    id: u32,
    location: String,
    source_kind: DependencySourceKind,
    confidence: DependencyConfidence,
    rewrite_strategy: RewriteStrategy,
    command: Option<String>,
    note: Option<String>,
) -> DependencySite {
    DependencySite {
        entity_type,
        id,
        location,
        source_kind,
        confidence,
        rewrite_strategy,
        command,
        note,
    }
}

fn site_sort_key(site: &DependencySite) -> (String, &'static str, u32) {
    (site.location.clone(), site.entity_type.as_label(), site.id)
}

fn is_interface_hook_opcode(command: &str) -> bool {
    (command.starts_with("if_") || command.starts_with("cc_")) && command.contains("_seton")
}

fn asset_command_info(command: &str) -> Option<(usize, EntityType)> {
    if command.contains("model") {
        if command.contains("angle")
            || command.contains("zoom")
            || command.contains("xof")
            || command.contains("yof")
        {
            return None;
        }
        Some((2, EntityType::Model))
    } else if command.contains("graphic") || command.contains("sprite") {
        Some((2, EntityType::Graphic))
    } else if command.contains("cursor") {
        Some((2, EntityType::Cursor))
    } else if command.contains("font") {
        Some((2, EntityType::FontMetrics))
    } else if command.contains("texture") {
        Some((2, EntityType::Texture))
    } else if command.contains("stylesheet") || command.contains("style") {
        Some((2, EntityType::Stylesheet))
    } else if command.contains("seq") || command.contains("anim") {
        Some((2, EntityType::Seq))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DependencyConfidence, DependencySourceKind, RewriteStrategy,
        collect_component_dependency_sites, collect_script_dependency_sites,
        write_dependency_files,
    };
    use crate::config::ParamEntry;
    use crate::dep_tree::EntityType;
    use crate::dep_tree::ResolverContext;
    use crate::interface::{ComponentDeps, VarTransmitRef};
    use crate::script::{
        CompiledScript, Instruction, OpcodeBook, Operand, VarBitRef, VarRef, encode_script,
    };
    use crate::vars::VarDomain;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::PathBuf;

    fn test_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
    }

    fn build_ctx(
        build: u32,
        scripts: &[(u32, CompiledScript)],
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
            varps_by_domain: HashMap::new(),
            varbits: BTreeMap::new(),
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

    #[test]
    fn collect_script_dependency_sites_tracks_exact_and_heuristic_refs() {
        let script = CompiledScript {
            name: Some("[proc,test]".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_var".to_string(),
                    operand: Operand::VarRef(VarRef {
                        domain: VarDomain::Player,
                        id: 12,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varbit".to_string(),
                    operand: Operand::VarBitRef(VarBitRef {
                        id: 45,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "gosub_with_params".to_string(),
                    operand: Operand::Script(77),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1000),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(55),
                },
                Instruction {
                    opcode: 0,
                    command: "cc_setmodel".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(91),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".to_string(),
                    operand: Operand::Str(String::new()),
                },
                Instruction {
                    opcode: 0,
                    command: "cc_setonclick".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let sites = collect_script_dependency_sites(&script);
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::VarPlayer
                && site.id == 12
                && site.source_kind == DependencySourceKind::Operand
                && site.confidence == DependencyConfidence::Exact
                && site.rewrite_strategy == RewriteStrategy::DirectOperand
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::VarBit
                && site.id == 45
                && site.source_kind == DependencySourceKind::Operand
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::Script
                && site.id == 77
                && site.source_kind == DependencySourceKind::Operand
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::Model
                && site.id == 1000
                && site.source_kind == DependencySourceKind::CommandStackSite
                && site.confidence == DependencyConfidence::Heuristic
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::Script
                && site.id == 91
                && site.command.as_deref() == Some("cc_setonclick")
        }));
    }

    #[test]
    fn collect_component_dependency_sites_tracks_hooks_and_fields() {
        let mut deps = ComponentDeps {
            component_type: "layer".to_string(),
            name: Some("bank".to_string()),
            children: Vec::new(),
            scripts: HashSet::from([11, 12]),
            onload_scripts: HashSet::from([11]),
            varps: HashSet::from([VarTransmitRef::Player(10)]),
            varbits: HashSet::from([30]),
            invs: HashSet::from([93]),
            stats: HashSet::from([7]),
            graphics: HashSet::from([44]),
            models: HashSet::from([55]),
            cursors: HashSet::from([66]),
            stylesheets: HashSet::from([77]),
            params: HashSet::from([88]),
            seqs: HashSet::from([99]),
            fontmetrics: HashSet::from([101]),
            textures: HashSet::from([202]),
            enums: HashSet::from([303]),
        };
        deps.varps.insert(VarTransmitRef::VarClientString(12));

        let sites = collect_component_dependency_sites(&deps);
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::Script
                && site.id == 11
                && site.location == "hook.onload"
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::Script && site.id == 12 && site.location == "hook.any"
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::VarPlayer
                && site.id == 10
                && site.location == "transmit_list.varp.player[10]"
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::VarClient
                && site.id == 12
                && site.location == "transmit_list.varp.client_string[12]"
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::Graphic
                && site.id == 44
                && site.location == "field.graphic[44]"
        }));
        assert!(sites.iter().any(|site| {
            site.entity_type == EntityType::Enum
                && site.id == 303
                && site.location == "field.enum[303]"
        }));
    }

    #[test]
    fn write_dependency_files_writes_component_script_and_coverage_artifacts()
    -> crate::error::Result<()> {
        let script = CompiledScript {
            name: Some("[proc,test]".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![Instruction {
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: Operand::Script(77),
            }],
        };
        let deps = ComponentDeps {
            component_type: "graphic".to_string(),
            name: Some("widget".to_string()),
            children: Vec::new(),
            scripts: HashSet::from([77]),
            onload_scripts: HashSet::new(),
            varps: HashSet::from([VarTransmitRef::Player(10)]),
            varbits: HashSet::new(),
            invs: HashSet::new(),
            stats: HashSet::new(),
            graphics: HashSet::from([44]),
            models: HashSet::new(),
            cursors: HashSet::new(),
            stylesheets: HashSet::new(),
            params: HashSet::new(),
            seqs: HashSet::new(),
            fontmetrics: HashSet::new(),
            textures: HashSet::new(),
            enums: HashSet::new(),
        };
        let ctx = build_ctx(910, &[(77, script)], &[(105, 3, deps)])?;
        let temp_dir = tempfile::tempdir()?;
        let coverage = write_dependency_files(&ctx, temp_dir.path())?;

        assert!(temp_dir.path().join("components.json").is_file());
        assert!(temp_dir.path().join("scripts.jsonl").is_file());
        assert!(temp_dir.path().join("coverage.json").is_file());
        assert_eq!(coverage.component_count, 1);
        assert_eq!(coverage.script_count, 1);
        assert_eq!(coverage.total_sites, 4);
        Ok(())
    }

    #[test]
    fn write_dependency_files_orders_parallel_artifacts_stably() -> crate::error::Result<()> {
        let script_a = CompiledScript {
            name: Some("[proc,a]".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![Instruction {
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: Operand::Script(17),
            }],
        };
        let script_b = CompiledScript {
            name: Some("[proc,b]".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![Instruction {
                opcode: 0,
                command: "push_var".to_string(),
                operand: Operand::VarRef(VarRef {
                    domain: VarDomain::Player,
                    id: 5,
                    transmog: false,
                }),
            }],
        };
        let component_a = ComponentDeps {
            component_type: "layer".to_string(),
            name: Some("a".to_string()),
            children: Vec::new(),
            scripts: HashSet::from([17]),
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
        let component_b = ComponentDeps {
            component_type: "graphic".to_string(),
            name: Some("b".to_string()),
            children: Vec::new(),
            scripts: HashSet::from([99]),
            onload_scripts: HashSet::from([99]),
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

        let ctx = build_ctx(
            947,
            &[(200, script_b), (100, script_a)],
            &[(12, 5, component_b), (3, 9, component_a)],
        )?;
        let temp_dir = tempfile::tempdir()?;

        write_dependency_files(&ctx, temp_dir.path())?;

        let components = serde_json::from_slice::<serde_json::Value>(&std::fs::read(
            temp_dir.path().join("components.json"),
        )?)?;
        assert_eq!(
            components
                .as_array()
                .expect("component array")
                .iter()
                .map(|record| {
                    (
                        record["interface_id"].as_u64().expect("interface id"),
                        record["component_id"].as_u64().expect("component id"),
                    )
                })
                .collect::<Vec<_>>(),
            vec![(3, 9), (12, 5)]
        );

        let script_lines = std::fs::read_to_string(temp_dir.path().join("scripts.jsonl"))?;
        let packed_ids = script_lines
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|record| {
                u32::try_from(record["packed_id"].as_u64().expect("packed id"))
                    .expect("packed id fits")
            })
            .collect::<Vec<_>>();
        assert_eq!(packed_ids, vec![100 << 16, 200 << 16]);

        Ok(())
    }
}
