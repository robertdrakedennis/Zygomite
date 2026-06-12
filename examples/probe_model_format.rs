//! Probe RT7 model decode for specific ids; print header fields and failure detail.
//! Usage: <cache-dir> <build> <id> [<id>...]
use rs3_cache_rs::cache::FlatCache;
use rs3_cache_rs::model::Model;

fn main() -> anyhow::Result<()> {
    let cache_dir = std::env::args().nth(1).expect("usage: <cache-dir> <build> <id>...");
    let build: u32 = std::env::args().nth(2).expect("usage").parse()?;
    let ids: Vec<u32> = std::env::args().skip(3).map(|s| s.parse().unwrap()).collect();
    let cache = FlatCache::open(std::path::Path::new(&cache_dir))?;
    for id in ids {
        let files = cache.group_files(47, id)?;
        let Some(data) = files.get(&0) else {
            println!("model {id}: no file 0");
            continue;
        };
        let head: Vec<String> = data[..data.len().min(24)].iter().map(|b| format!("{b:02x}")).collect();
        println!(
            "model {id}: len={} format={} version={} always_0f={} mesh_count={} counts={:?} head={}",
            data.len(), data[0], data[1], data[2], data[3], &data[4..9], head.join(" ")
        );
        match Model::decode(data, build) {
            Ok(m) => {
                let md = m.meshdata.as_ref();
                println!(
                    "  OK verts={:?} faces={:?} skin={:?} renders={:?}",
                    md.map(|d| d.vertex_count),
                    md.map(|d| d.face_count),
                    md.map(|d| d.skin.is_some()),
                    md.map(|d| d.renders.len())
                );
            }
            Err(e) => println!("  FAIL: {e:#}"),
        }
    }
    Ok(())
}
