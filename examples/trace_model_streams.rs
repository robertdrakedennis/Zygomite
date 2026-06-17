//! Trace v5 meshdata stream offsets for RT7 models to locate the 0x17-flag format delta.
//! Usage: <cache-dir> <id> [<id>...]
use rs3_cache_rs::cache::FlatCache;

struct R<'a> {
    d: &'a [u8],
    p: usize,
}
impl R<'_> {
    fn g1(&mut self) -> u32 {
        let v = self.d[self.p];
        self.p += 1;
        u32::from(v)
    }
    fn g2(&mut self) -> u32 {
        let v = u16::from_le_bytes([self.d[self.p], self.d[self.p + 1]]);
        self.p += 2;
        u32::from(v)
    }
    fn g4(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.d[self.p..self.p + 4].try_into().unwrap());
        self.p += 4;
        v
    }
    const fn skip(&mut self, n: usize) {
        self.p += n;
    }
    const fn left(&self) -> isize {
        self.d.len() as isize - self.p as isize
    }
}

fn main() -> anyhow::Result<()> {
    let cache_dir = std::env::args().nth(1).expect("usage: <cache-dir> <id>...");
    let ids: Vec<u32> = std::env::args()
        .skip(2)
        .map(|s| s.parse().unwrap())
        .collect();
    let cache = FlatCache::open(std::path::Path::new(&cache_dir))?;
    for id in ids {
        let files = cache.group_files(47, id)?;
        let data: &[u8] = &files[&0];
        let mut r = R { d: data, p: 0 };
        let format = r.g1();
        let version = r.g1();
        let flags = r.g1();
        let mesh_count = r.g1();
        let c0 = r.g1();
        let c1 = r.g1();
        let c2 = r.g1();
        let c3 = r.g1();
        let c4 = if version >= 5 { r.g1() } else { 0 };
        println!(
            "== model {id}: len={} format={format} version={version} flags=0x{flags:02x} meshes={mesh_count} counts={c0},{c1},{c2},{c3},{c4}",
            data.len()
        );
        let gf = r.g1();
        let unkint = r.g1();
        let face_count = r.g2();
        let vc = r.g4() as usize;
        println!(
            "   meshdata: group_flags=0x{gf:02x} unkint={unkint} face_count={face_count} verts={vc} pos_after_header={}",
            r.p
        );
        let has_vertices = gf & 1 != 0;
        let has_facebones = gf & 4 != 0;
        let has_boneids = gf & 8 != 0;
        let has_skin = gf & 32 != 0;
        if has_vertices {
            r.skip(vc * 6);
            println!("   after positions(6B): pos={} left={}", r.p, r.left());
            r.skip(vc * 3);
            println!("   after normals(3B): pos={} left={}", r.p, r.left());
            r.skip(vc * 4);
            println!("   after tangents(4B): pos={} left={}", r.p, r.left());
            r.skip(vc * 4);
            println!("   after uv(4B): pos={} left={}", r.p, r.left());
        }
        if has_boneids {
            r.skip(vc * 2);
            println!("   after boneids(2B): pos={} left={}", r.p, r.left());
        }
        if has_skin {
            // per-vertex: u16 idcount, ids, u16 wcount, weights — sanity-check counts
            let mut max_ids = 0u32;
            let mut bad = false;
            for i in 0..vc {
                if r.left() < 2 {
                    println!("   SKIN ran out at vertex {i} pos={}", r.p);
                    bad = true;
                    break;
                }
                let idc = r.g2();
                if idc > 16 {
                    println!(
                        "   SKIN vertex {i}: id_count={idc} (implausible) pos={}",
                        r.p - 2
                    );
                    bad = true;
                    break;
                }
                r.skip(idc as usize * 2);
                let wc = r.g2();
                if wc > 16 {
                    println!(
                        "   SKIN vertex {i}: weight_count={wc} (implausible) pos={}",
                        r.p - 2
                    );
                    bad = true;
                    break;
                }
                r.skip(wc as usize);
                max_ids = max_ids.max(idc);
            }
            if bad {
                continue;
            }
            println!(
                "   after skin: pos={} left={} max_ids_per_vert={max_ids}",
                r.p,
                r.left()
            );
        }
        if has_vertices {
            r.skip(vc * 2);
            println!("   after colours(2B): pos={} left={}", r.p, r.left());
            r.skip(vc);
            println!("   after alpha(1B): pos={} left={}", r.p, r.left());
        }
        if has_facebones {
            r.skip(vc * 2);
            println!("   after facebones(2B): pos={} left={}", r.p, r.left());
        }
        for m in 0..mesh_count {
            if r.left() < 12 {
                println!("   RENDER {m}: insufficient header bytes left={}", r.left());
                break;
            }
            let rgf = r.g1();
            let unkint = u32::from_be_bytes(r.d[r.p..r.p + 4].try_into().unwrap());
            r.skip(4);
            let mat = r.g2();
            let unkb2 = r.g1();
            let len = r.g2() as usize;
            let isz = if vc <= 0xffff { 2 } else { 4 };
            println!(
                "   render {m}: gf=0x{rgf:02x} unkint={unkint} mat={mat} b2={unkb2} indices={len} ({isz}B each) pos={} left_after={}",
                r.p,
                r.left() - (len * isz) as isize
            );
            r.skip(len * isz);
        }
        println!(
            "   END pos={} len={} leftover={}",
            r.p,
            data.len(),
            r.left()
        );
    }
    Ok(())
}
