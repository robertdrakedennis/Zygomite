use anyhow::Result;
use clap::Parser;
use rs3_cache_rs::cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    rs3_cache_rs::cli::run(cli)
}
