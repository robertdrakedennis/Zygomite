use rs3_cache_rs::font::bitmap::FontMetrics;
use rs3_cache_rs::font::raster::{SizeMode, rasterize};
use rs3_cache_rs::js5pack::PackArchive;
use std::path::Path;
fn om(id: u32) -> FontMetrics {
    FontMetrics::decode(&std::fs::read(format!("/Users/robert/projects/alerion/server/cache-patches/relic-system-948/fonts/metrics/{id}.bin")).unwrap()).unwrap()
}
fn main() {
    let pr = Path::new("/Users/robert/projects/alerion/server/data/pack-910-base-948-overlay");
    // fmt1 fonts: 56->face3 target_asc17, 57->face4 target_asc16
    for (font, face, tgt) in [(56u32, 3u32, 17f32), (57, 4, 16.)] {
        let pack = PackArchive::open(&pr.join("client.ttf.js5")).unwrap();
        let bytes = pack
            .group_files(face)
            .unwrap()
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .1;
        let o = om(font);
        let r = rasterize(&bytes, SizeMode::Ascent(tgt)).unwrap();
        println!(
            "font {font} face {face} tgt_asc={tgt}: ORACLE {}/{}/{} {}x{}  MINE {}/{}/{} {}x{} @{:.1}px",
            o.ascent,
            o.descent,
            o.line_height,
            o.atlas_w,
            o.atlas_h,
            r.ascent,
            r.descent,
            r.line_height,
            r.atlas_w,
            r.atlas_h,
            r.size_px
        );
    }
}
