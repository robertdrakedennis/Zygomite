//! `dep-tree-*` — resolve and write the dependency tree for an entity
//! (interface / script / varp / varbit / config).

use std::path::Path;

use anyhow::{Result, bail};

use crate::cli::context::CommandContext;
use crate::cli::shared::write_json;
use crate::cli::{ConfigKindArg, VarDomainArg};
use crate::dep_tree::{DependencyTree, EntityRef, EntityType, ResolverContext, build_tree};

/// Shared options for the dep-tree commands that key off a single id.
#[derive(Clone, Debug)]
pub struct DepTreeOpts {
    pub id: u32,
    pub max_depth: u32,
    pub out_file: std::path::PathBuf,
}

/// Options for `dep-tree-varp` (needs the var domain to pick the entity type).
#[derive(Clone, Debug)]
pub struct DepTreeVarpOpts {
    pub id: u32,
    pub domain: VarDomainArg,
    pub max_depth: u32,
    pub out_file: std::path::PathBuf,
}

/// Options for `dep-tree-config` (needs the config kind).
#[derive(Clone, Debug)]
pub struct DepTreeConfigOpts {
    pub kind: ConfigKindArg,
    pub id: u32,
    pub max_depth: u32,
    pub out_file: std::path::PathBuf,
}

fn report_tree(out_file: &Path, tree: &DependencyTree) -> Result<()> {
    write_json(out_file, tree)?;
    eprintln!(
        "dependency tree written to {} (nodes={}, cycles={}, depth_hits={})",
        out_file.display(),
        tree.total_nodes,
        tree.cycles_detected,
        tree.max_depth_hits
    );
    Ok(())
}

fn load_ctx(ctx: &CommandContext) -> Result<ResolverContext> {
    Ok(ResolverContext::load(
        ctx.cache(),
        ctx.tar_path(),
        ctx.data_dir(),
        ctx.build(),
        ctx.subbuild(),
    )?)
}

/// `dep-tree-interface`.
pub fn run_interface(ctx: &CommandContext, opts: DepTreeOpts) -> Result<()> {
    let DepTreeOpts {
        id,
        max_depth,
        out_file,
    } = opts;
    let resolver = load_ctx(ctx)?;
    let root = EntityRef::new(EntityType::Interface, id);
    let tree = build_tree(&resolver, &root, max_depth);
    report_tree(&out_file, &tree)
}

/// `dep-tree-script`.
pub fn run_script(ctx: &CommandContext, opts: DepTreeOpts) -> Result<()> {
    let DepTreeOpts {
        id,
        max_depth,
        out_file,
    } = opts;
    let resolver = load_ctx(ctx)?;
    let root = EntityRef::new(EntityType::Script, id);
    let tree = build_tree(&resolver, &root, max_depth);
    report_tree(&out_file, &tree)
}

/// `dep-tree-varp` — resolves the tree for a varp across its parameter sources.
pub fn run_varp(ctx: &CommandContext, opts: DepTreeVarpOpts) -> Result<()> {
    let DepTreeVarpOpts {
        id,
        domain,
        max_depth,
        out_file,
    } = opts;
    let resolver = load_ctx(ctx)?;
    let entity_type = match domain {
        VarDomainArg::Player => EntityType::VarPlayer,
        VarDomainArg::Npc => EntityType::VarNpc,
        VarDomainArg::Client => EntityType::VarClient,
        VarDomainArg::World => EntityType::VarWorld,
        VarDomainArg::Region => EntityType::VarRegion,
        VarDomainArg::Object => EntityType::VarObject,
        VarDomainArg::Clan => EntityType::VarClan,
        VarDomainArg::ClanSetting => EntityType::VarClanSetting,
        VarDomainArg::Controller => EntityType::VarController,
        VarDomainArg::Global => EntityType::VarGlobal,
        VarDomainArg::PlayerGroup => EntityType::VarPlayerGroup,
        VarDomainArg::All => bail!("dep-tree-varp requires a specific domain, not 'all'"),
    };
    let root = EntityRef::new(entity_type, id);
    let tree = build_tree(&resolver, &root, max_depth);
    report_tree(&out_file, &tree)
}

/// `dep-tree-varbit`.
pub fn run_varbit(ctx: &CommandContext, opts: DepTreeOpts) -> Result<()> {
    let DepTreeOpts {
        id,
        max_depth,
        out_file,
    } = opts;
    let resolver = load_ctx(ctx)?;
    let root = EntityRef::new(EntityType::VarBit, id);
    let tree = build_tree(&resolver, &root, max_depth);
    report_tree(&out_file, &tree)
}

/// `dep-tree-config` — resolves the tree for a config entry.
pub fn run_config(ctx: &CommandContext, opts: DepTreeConfigOpts) -> Result<()> {
    let DepTreeConfigOpts {
        kind,
        id,
        max_depth,
        out_file,
    } = opts;
    let resolver = load_ctx(ctx)?;
    let entity_type = kind.entity_type();
    let root = EntityRef::new(entity_type, id).labeled(kind.label());
    let tree = build_tree(&resolver, &root, max_depth);
    report_tree(&out_file, &tree)
}
