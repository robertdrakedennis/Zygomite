//! Parse one material and print its ops. Usage: <cache-dir> <id>
use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::config::parse_material;

fn main() -> anyhow::Result<()> {
    let cache_dir = std::env::args().nth(1).expect("usage");
    let id: u32 = std::env::args().nth(2).expect("usage").parse()?;
    let cache = FlatCache::open(std::path::Path::new(&cache_dir))?;
    let files = cache.group_files(26, 0)?;
    let data = &files[&id];
    println!("len={} first-bytes={:02x?}", data.len(), &data[..data.len().min(8)]);
    match parse_material(id, data) {
        Ok(e) => println!("OK: {:?}", e.ops),
        Err(e) => println!("FAIL: {e}"),
    }
    Ok(())
}
