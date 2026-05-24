use crate::dep_tree::ResolverContext;
use crate::interface::ComponentDeps;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct InterfaceIndexDeps {
    pub scripts: Vec<u32>,
    pub sprites: Vec<u32>,
    pub models: Vec<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InterfaceIndexEntry {
    pub id: u32,
    pub components: u32,
    pub named_components: Vec<String>,
    pub onload_scripts: Vec<u32>,
    pub deps: InterfaceIndexDeps,
}

fn merge_component_deps(target: &mut InterfaceIndexDeps, deps: &ComponentDeps) {
    for script in &deps.scripts {
        if target.scripts.binary_search(script).is_err() {
            target.scripts.push(*script);
        }
    }
    for graphic in &deps.graphics {
        if target.sprites.binary_search(graphic).is_err() {
            target.sprites.push(*graphic);
        }
    }
    for model in &deps.models {
        if target.models.binary_search(model).is_err() {
            target.models.push(*model);
        }
    }
}

fn finalize_deps(deps: &mut InterfaceIndexDeps) {
    deps.scripts.sort_unstable();
    deps.sprites.sort_unstable();
    deps.models.sort_unstable();
}

pub fn build_interface_index(ctx: &ResolverContext) -> Vec<InterfaceIndexEntry> {
    let mut entries = Vec::new();

    for (&interface_id, components) in &ctx.parsed_components {
        let mut named_components = BTreeSet::new();
        let mut onload_scripts = BTreeSet::new();
        let mut deps = InterfaceIndexDeps {
            scripts: Vec::new(),
            sprites: Vec::new(),
            models: Vec::new(),
        };

        for deps_comp in components.values() {
            if let Some(name) = deps_comp.name.as_deref() {
                if !name.is_empty() {
                    named_components.insert(name.to_string());
                }
            }
            onload_scripts.extend(deps_comp.onload_scripts.iter().copied());
            merge_component_deps(&mut deps, deps_comp);
        }

        finalize_deps(&mut deps);

        entries.push(InterfaceIndexEntry {
            id: interface_id,
            components: components.len() as u32,
            named_components: named_components.into_iter().collect(),
            onload_scripts: onload_scripts.into_iter().collect(),
            deps,
        });
    }

    entries.sort_by_key(|entry| entry.id);
    entries
}

pub fn write_interface_index(ctx: &ResolverContext, out_file: &Path) -> Result<()> {
    let entries = build_interface_index(ctx);
    let json = serde_json::to_string_pretty(&entries)?;
    if let Some(parent) = out_file.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(out_file, json)?;
    eprintln!(
        "interface index written to {} ({} interfaces)",
        out_file.display(),
        entries.len()
    );
    Ok(())
}
