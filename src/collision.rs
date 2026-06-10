//! Build-agnostic RS clip-flag collision grid, reconstructed from the NXT
//! `jag::oldscape::movement::CollisionMap` reverse-engineering
//! (`reclass-data/sigs-results/nxt216_collision_deepdive.md`).
//!
//! Collision is **not** stored in the cache — the client derives it at runtime
//! from decoded map tiles + loc placements + loc configs. This module is a
//! reference implementation of that derivation: feed it a [`crate::map::MapSquare`]
//! plus a per-loc [`LocClip`] lookup and it produces one clip-flag grid per level.
//!
//! Every bit value here is the canonical RS clip-flag set, **verified against the
//! NXT 216 binary** (the bit table, `AddWall` shape/rotation→bit mapping with
//! neighbour mirroring, the diagonal-wall DAT tables, and `proj = walk << 9`).
//! The values are build-agnostic; no rev-216 addresses are used.

use crate::map::MapSquare;

/// One map square is 64×64 tiles.
pub const SQUARE_SIZE: usize = 64;

// ---- clip-flag bits (NXT-verified; see nxt216_collision_deepdive.md §2) ------
// walk-lane cardinal wall bits
pub const WALL_N: i32 = 0x2;
pub const WALL_E: i32 = 0x8;
pub const WALL_S: i32 = 0x20;
pub const WALL_W: i32 = 0x80;
// walk-lane corner/diagonal wall bits (interleave the cardinals)
pub const WALL_NW: i32 = 0x1;
pub const WALL_NE: i32 = 0x4;
pub const WALL_SE: i32 = 0x10;
pub const WALL_SW: i32 = 0x40;
/// object footprint blocks walk
pub const BLOCK_LOC: i32 = 0x100;
/// projectile/LOS lane is the walk lane shifted up by 9 bits
pub const PROJ_SHIFT: u32 = 9;
/// object footprint blocks projectiles/LOS (`BLOCK_LOC << 9`)
pub const BLOCK_PROJ_LOC: i32 = BLOCK_LOC << PROJ_SHIFT; // 0x20000
// ground / meta lane (byte 2, bits 16-23) + edge bit 24
pub const BLOCK_GROUND_DECOR: i32 = 0x0004_0000;
pub const BLOCK_PROJ_GROUND: i32 = 0x0008_0000;
pub const ROOF: i32 = 0x0010_0000;
pub const BLOCK_GROUND: i32 = 0x0020_0000;
pub const MULTIWAY: i32 = 0x0040_0000;
pub const FREEMAP: i32 = 0x0080_0000;
pub const EDGE_INSIDE: i32 = 0x0100_0000;

// composite consult masks the runtime testers AND against (NXT §2.4)
pub const CONSULT_WALK_E: i32 = 0x0124_0108;
pub const CONSULT_WALK_W: i32 = 0x0124_0180;
pub const CONSULT_WALK_N: i32 = 0x0124_0102;
pub const CONSULT_WALK_S: i32 = 0x0124_0120;
pub const CONSULT_WALK_TILE: i32 = 0x0124_0100;
pub const CONSULT_PROJ_N: i32 = 0x010a_0400;
pub const CONSULT_PROJ_S: i32 = 0x010a_4000;
pub const CONSULT_PROJ_E: i32 = 0x010a_1000;
pub const CONSULT_PROJ_W: i32 = 0x010b_0000;

/// Per-loc collision-relevant config (from the loc `LocType`). Derived from the
/// loc config ops produced by [`crate::config::parse_loc`] via [`LocClip::from_loc_ops`].
#[derive(Clone, Copy, Debug)]
pub struct LocClip {
    pub width: u8,
    pub length: u8,
    /// whether the loc clips walk (`clipType != 0`)
    pub clips: bool,
    /// whether the loc also clips projectiles/LOS
    pub block_projectile: bool,
}

impl Default for LocClip {
    fn default() -> Self {
        // RS `blockwalk` default is 2 = "blocks walk only" (projectiles pass), so a
        // loc clips walk but NOT projectiles by default. `blockwalk=yes` (op27 → 1)
        // is the fully-solid case that also blocks projectiles.
        Self {
            width: 1,
            length: 1,
            clips: true,
            block_projectile: false,
        }
    }
}

impl LocClip {
    /// Build from the text ops emitted by [`crate::config::parse_loc`]. `blockwalk`
    /// is 0 (none) / 1 (walk + projectiles) / 2 (walk only, the default):
    /// - `blockwalk=no` (op17 → 0): no collision.
    /// - `blockwalk=yes` (op27 → 1): blocks walk and projectiles.
    /// - `blockrange=no` (op18): the `breakroutefinding` flag, which `LocType::postDecode`
    ///   resolves to `blockwalk = 0` — i.e. no collision.
    pub fn from_loc_ops(ops: &[String]) -> Self {
        let mut clip = Self::default();
        for op in ops {
            if let Some(v) = op.strip_prefix("width=") {
                if let Ok(n) = v.parse() {
                    clip.width = n;
                }
            } else if let Some(v) = op.strip_prefix("length=") {
                if let Ok(n) = v.parse() {
                    clip.length = n;
                }
            } else if op == "blockwalk=no" || op == "blockrange=no" {
                clip.clips = false;
                clip.block_projectile = false;
            } else if op == "blockwalk=yes" {
                clip.clips = true;
                clip.block_projectile = true;
            }
        }
        clip
    }
}

/// A single-level clip-flag grid for one map square (64×64 ints, `[x][z]`).
#[derive(Clone, Debug)]
pub struct CollisionMap {
    size: usize,
    flags: Vec<i32>,
}

impl Default for CollisionMap {
    fn default() -> Self {
        Self::new(SQUARE_SIZE)
    }
}

impl CollisionMap {
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            size,
            flags: vec![0; size * size],
        }
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    #[inline]
    fn idx(&self, x: i32, z: i32) -> Option<usize> {
        if x < 0 || z < 0 || x as usize >= self.size || z as usize >= self.size {
            return None;
        }
        Some(x as usize * self.size + z as usize)
    }

    /// Tile flag word, or `0` if out of bounds.
    #[must_use]
    pub fn get(&self, x: i32, z: i32) -> i32 {
        self.idx(x, z).map_or(0, |i| self.flags[i])
    }

    /// `grid[x][z] |= mask` (no-op if out of bounds) — the NXT `AddCMap` primitive.
    pub fn add(&mut self, x: i32, z: i32, mask: i32) {
        if let Some(i) = self.idx(x, z) {
            self.flags[i] |= mask;
        }
    }

    /// `grid[x][z] &= ~mask` — the NXT `RemCMap` primitive.
    pub fn remove(&mut self, x: i32, z: i32, mask: i32) {
        if let Some(i) = self.idx(x, z) {
            self.flags[i] &= !mask;
        }
    }

    /// Seed border tiles fully-blocked and interior with the EDGE/INSIDE bit,
    /// matching NXT `Reset` (`border = 0xFFFFFF`, `interior = 0x1000000`). Only
    /// meaningful for a fully-padded region; a bare square keeps the default
    /// clear init so squares can be merged.
    pub fn reset_nxt(&mut self) {
        let n = self.size;
        for x in 0..n {
            for z in 0..n {
                let interior = x != 0 && z != 0 && x < n - 5 && z < n - 5;
                self.flags[x * n + z] = if interior { EDGE_INSIDE } else { 0x00ff_ffff };
            }
        }
    }

    /// OR a walk-lane mask into a tile, plus its projectile-lane mirror when `block_proj`.
    fn mark(&mut self, x: i32, z: i32, walk_mask: i32, block_proj: bool) {
        self.add(x, z, walk_mask);
        if block_proj {
            self.add(x, z, walk_mask << PROJ_SHIFT);
        }
    }

    /// `AddWall` — set the wall's edge bit on its own tile and the mirror bit on
    /// the adjacent tile, in the walk lane (and projectile lane if `block_proj`).
    /// shape: 0 = straight, 1/3 = diagonal/pillar, 2 = corner/L (NXT §3.1).
    pub fn add_wall(&mut self, x: i32, z: i32, shape: u8, rotation: u8, block_proj: bool) {
        match shape {
            0 => match rotation & 3 {
                0 => { self.mark(x, z, WALL_W, block_proj); self.mark(x - 1, z, WALL_E, block_proj); }
                1 => { self.mark(x, z, WALL_N, block_proj); self.mark(x, z + 1, WALL_S, block_proj); }
                2 => { self.mark(x, z, WALL_E, block_proj); self.mark(x + 1, z, WALL_W, block_proj); }
                _ => { self.mark(x, z, WALL_S, block_proj); self.mark(x, z - 1, WALL_N, block_proj); }
            },
            // diagonal/pillar — matches the NXT DAT tables at 0x100d3fe80
            1 | 3 => match rotation & 3 {
                0 => { self.mark(x, z, WALL_NW, block_proj); self.mark(x - 1, z + 1, WALL_SE, block_proj); }
                1 => { self.mark(x, z, WALL_NE, block_proj); self.mark(x + 1, z + 1, WALL_SW, block_proj); }
                2 => { self.mark(x, z, WALL_SE, block_proj); self.mark(x + 1, z - 1, WALL_NW, block_proj); }
                _ => { self.mark(x, z, WALL_SW, block_proj); self.mark(x - 1, z - 1, WALL_NE, block_proj); }
            },
            2 => match rotation & 3 {
                0 => { self.mark(x, z, WALL_W | WALL_N, block_proj); self.mark(x - 1, z, WALL_E, block_proj); self.mark(x, z + 1, WALL_S, block_proj); }
                1 => { self.mark(x, z, WALL_N | WALL_E, block_proj); self.mark(x, z + 1, WALL_S, block_proj); self.mark(x + 1, z, WALL_W, block_proj); }
                2 => { self.mark(x, z, WALL_E | WALL_S, block_proj); self.mark(x + 1, z, WALL_W, block_proj); self.mark(x, z - 1, WALL_N, block_proj); }
                _ => { self.mark(x, z, WALL_S | WALL_W, block_proj); self.mark(x, z - 1, WALL_N, block_proj); self.mark(x - 1, z, WALL_E, block_proj); }
            },
            _ => {}
        }
    }

    /// `AddLoc` — OR `BLOCK_LOC` (+ `BLOCK_PROJ_LOC`) over a `width×length`
    /// footprint, transposing for rotations 1/3 (NXT §3.2).
    pub fn add_object(&mut self, x: i32, z: i32, width: u8, length: u8, rotation: u8, block_proj: bool) {
        let (sx, sz) = if rotation & 1 == 1 {
            (i32::from(length), i32::from(width))
        } else {
            (i32::from(width), i32::from(length))
        };
        let mut mask = BLOCK_LOC;
        if block_proj {
            mask |= BLOCK_PROJ_LOC;
        }
        for dx in 0..sx.max(1) {
            for dz in 0..sz.max(1) {
                self.add(x + dx, z + dz, mask);
            }
        }
    }

    /// `BlockGroundDecor` — `byte[2] |= 4` (NXT §3.3).
    pub fn add_ground_decor(&mut self, x: i32, z: i32) {
        self.add(x, z, BLOCK_GROUND_DECOR);
    }

    /// `BlockGround` — full-tile walk block, `byte[2] |= 0x20` (NXT §3.3).
    pub fn block_ground(&mut self, x: i32, z: i32) {
        self.add(x, z, BLOCK_GROUND);
    }

    /// Flag words as a `[x][z]` nested grid (for serialization/inspection).
    #[must_use]
    pub fn to_rows(&self) -> Vec<Vec<i32>> {
        (0..self.size)
            .map(|x| (0..self.size).map(|z| self.flags[x * self.size + z]).collect())
            .collect()
    }

    /// Number of tiles with any flag set.
    #[must_use]
    pub fn nonzero_count(&self) -> usize {
        self.flags.iter().filter(|&&f| f != 0).count()
    }
}

/// Wall shapes routed through `AddWall` (NXT `AddWall` switch handles 0/1/2/3).
#[must_use]
pub fn is_wall_shape(shape: u8) -> bool {
    shape <= 3
}

/// Solid object shapes routed through `AddLoc` footprint.
///
/// Shape 9 is a full diagonal wall that blocks the whole tile; 10/11 are
/// centrepiece/general scenery. Wall-decor (4-8) and roofs (12-21) add no
/// ground collision.
#[must_use]
pub fn is_object_shape(shape: u8) -> bool {
    shape == 9 || shape == 10 || shape == 11
}

/// Ground decoration shape.
#[must_use]
pub fn is_ground_decor_shape(shape: u8) -> bool {
    shape == 22
}

/// Build per-level collision grids for a decoded map square.
///
/// `clip_lookup(loc_id)` returns the [`LocClip`] for a loc id; callers that lack
/// loc configs can pass `|_| LocClip::default()` to treat every loc as a solid
/// 1×1 blocker. Tiles flagged blocked in the landscape `scene_flags` (bit `0x1`)
/// add `BLOCK_GROUND`. Derived (render-split) loc placements are skipped — the
/// source placement carries collision.
pub fn build_collision<F>(map: &MapSquare, mut clip_lookup: F) -> Vec<CollisionMap>
where
    F: FnMut(i32) -> LocClip,
{
    let levels = 4usize;
    let mut grids: Vec<CollisionMap> = (0..levels).map(|_| CollisionMap::default()).collect();

    // 1. tile-driven ground blocks from landscape settings (bit 0x1 = blocked).
    if let Some(land) = &map.landscape {
        for (level, plane) in land.scene_flags.iter().enumerate() {
            if level >= levels {
                break;
            }
            for (x, col) in plane.iter().enumerate() {
                for (z, &flags) in col.iter().enumerate() {
                    if flags & 0x1 != 0 {
                        grids[level].block_ground(x as i32, z as i32);
                    }
                }
            }
        }
    }

    // 2. loc-driven collision.
    for loc in &map.locs {
        if loc.derived {
            continue; // collision uses the source placement, not render-split pieces
        }
        let level = usize::from(loc.level);
        if level >= levels {
            continue;
        }
        let clip = clip_lookup(loc.id);
        if !clip.clips {
            continue; // clipType 0 — does not block
        }
        let (x, z) = (i32::from(loc.x), i32::from(loc.z));
        let grid = &mut grids[level];
        if is_wall_shape(loc.shape) {
            grid.add_wall(x, z, loc.shape, loc.angle, clip.block_projectile);
        } else if is_object_shape(loc.shape) {
            grid.add_object(x, z, clip.width, clip.length, loc.angle, clip.block_projectile);
        } else if is_ground_decor_shape(loc.shape) && clip.block_projectile {
            // ground decor blocks walk only when fully solid (blockwalk == 1)
            grid.add_ground_decor(x, z);
        }
    }

    grids
}

/// `LineOfWalk` — can you walk a straight line `(x0,z0) → (x1,z1)` on one level?
/// (walk-lane consult masks, NXT §4.5).
#[must_use]
pub fn line_of_walk(grid: &CollisionMap, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
    trace_line(grid, x0, z0, x1, z1, false)
}

/// `LineOfSight` — projectile/LOS trace, consulting the projectile-lane masks
/// (NXT §4.4).
#[must_use]
pub fn line_of_sight(grid: &CollisionMap, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
    trace_line(grid, x0, z0, x1, z1, true)
}

/// Dominant-axis line trace (the RS LOS/LOW algorithm): step the major axis a
/// tile at a time, accumulating the minor axis in 16.16 fixed point; consult the
/// major-axis directional mask on entering each tile and the minor-axis mask when
/// the minor axis crosses a tile boundary. Returns false on the first blocked step.
fn trace_line(grid: &CollisionMap, x0: i32, z0: i32, x1: i32, z1: i32, proj: bool) -> bool {
    if (x0, z0) == (x1, z1) {
        return true;
    }
    let (m_e, m_w, m_n, m_s) = if proj {
        (CONSULT_PROJ_E, CONSULT_PROJ_W, CONSULT_PROJ_N, CONSULT_PROJ_S)
    } else {
        (CONSULT_WALK_E, CONSULT_WALK_W, CONSULT_WALK_N, CONSULT_WALK_S)
    };
    let dx = x1 - x0;
    let dz = z1 - z0;
    let x_mask = if dx > 0 { m_e } else { m_w };
    let z_mask = if dz > 0 { m_n } else { m_s };
    let xi = dx.signum();
    let zi = dz.signum();
    if dx.abs() >= dz.abs() {
        // x-major
        let mut z_fixed = (i64::from(z0) << 16) + 0x8000;
        let z_inc = (i64::from(dz) << 16) / i64::from(dx.abs());
        let (mut x, mut z) = (x0, z0);
        while x != x1 {
            x += xi;
            if grid.get(x, z) & x_mask != 0 {
                return false;
            }
            z_fixed += z_inc;
            let nz = (z_fixed >> 16) as i32;
            if nz != z {
                if grid.get(x, nz) & z_mask != 0 {
                    return false;
                }
                z = nz;
            }
        }
    } else {
        // z-major
        let mut x_fixed = (i64::from(x0) << 16) + 0x8000;
        let x_inc = (i64::from(dx) << 16) / i64::from(dz.abs());
        let (mut x, mut z) = (x0, z0);
        while z != z1 {
            z += zi;
            if grid.get(x, z) & z_mask != 0 {
                return false;
            }
            x_fixed += x_inc;
            let nx = (x_fixed >> 16) as i32;
            if nx != x {
                if grid.get(nx, z) & x_mask != 0 {
                    return false;
                }
                x = nx;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_wall_sets_own_and_neighbour() {
        let mut g = CollisionMap::default();
        // shape 0, rot 0 (west wall): this tile W, west neighbour E.
        g.add_wall(10, 10, 0, 0, false);
        assert_eq!(g.get(10, 10) & WALL_W, WALL_W);
        assert_eq!(g.get(9, 10) & WALL_E, WALL_E);
        // rot 1 (north): this tile N, north neighbour S.
        let mut g = CollisionMap::default();
        g.add_wall(10, 10, 0, 1, false);
        assert_eq!(g.get(10, 10) & WALL_N, WALL_N);
        assert_eq!(g.get(10, 11) & WALL_S, WALL_S);
    }

    #[test]
    fn wall_projectile_pass_is_walk_shifted_9() {
        let mut g = CollisionMap::default();
        g.add_wall(10, 10, 0, 0, true);
        assert_eq!(g.get(10, 10) & (WALL_W << 9), WALL_W << 9); // 0x10000
        assert_eq!(g.get(9, 10) & (WALL_E << 9), WALL_E << 9); // 0x1000
    }

    #[test]
    fn diagonal_wall_matches_dat_tables() {
        // rot0: self NW(0x1) at (x,z); neighbour SE(0x10) at (x-1,z+1).
        let mut g = CollisionMap::default();
        g.add_wall(20, 20, 1, 0, false);
        assert_eq!(g.get(20, 20) & WALL_NW, WALL_NW);
        assert_eq!(g.get(19, 21) & WALL_SE, WALL_SE);
        // rot2: self SE(0x10); neighbour NW(0x1) at (x+1,z-1).
        let mut g = CollisionMap::default();
        g.add_wall(20, 20, 3, 2, false);
        assert_eq!(g.get(20, 20) & WALL_SE, WALL_SE);
        assert_eq!(g.get(21, 19) & WALL_NW, WALL_NW);
    }

    #[test]
    fn corner_wall_rot0_is_n_plus_w() {
        let mut g = CollisionMap::default();
        g.add_wall(30, 30, 2, 0, false);
        assert_eq!(g.get(30, 30) & (WALL_W | WALL_N), WALL_W | WALL_N); // 0x82
        assert_eq!(g.get(29, 30) & WALL_E, WALL_E);
        assert_eq!(g.get(30, 31) & WALL_S, WALL_S);
    }

    #[test]
    fn object_footprint_blocks_loc_and_proj() {
        let mut g = CollisionMap::default();
        g.add_object(5, 5, 2, 3, 0, true);
        for dx in 0..2 {
            for dz in 0..3 {
                assert_eq!(g.get(5 + dx, 5 + dz) & BLOCK_LOC, BLOCK_LOC);
                assert_eq!(g.get(5 + dx, 5 + dz) & BLOCK_PROJ_LOC, BLOCK_PROJ_LOC);
            }
        }
        // tile outside footprint is clear
        assert_eq!(g.get(7, 5), 0);
    }

    #[test]
    fn object_rotation_transposes_footprint() {
        let mut g = CollisionMap::default();
        g.add_object(5, 5, 2, 4, 1, false); // rot1 → 4 wide × 2 deep
        assert_eq!(g.get(8, 6) & BLOCK_LOC, BLOCK_LOC); // x+3,z+1 inside transposed box
        assert_eq!(g.get(5, 7), 0); // z+2 outside transposed box
    }

    #[test]
    fn loc_clip_from_ops_defaults_and_overrides() {
        // default blockwalk 2: clips walk, projectiles pass.
        let d = LocClip::from_loc_ops(&[]);
        assert!(d.clips && !d.block_projectile && d.width == 1 && d.length == 1);
        // blockwalk=no (op17 → 0): no collision.
        let c = LocClip::from_loc_ops(&[
            "width=3".to_string(),
            "length=2".to_string(),
            "blockwalk=no".to_string(),
        ]);
        assert!(!c.clips && !c.block_projectile && c.width == 3 && c.length == 2);
        // blockwalk=yes (op27 → 1): fully solid, blocks projectiles too.
        let y = LocClip::from_loc_ops(&["blockwalk=yes".to_string()]);
        assert!(y.clips && y.block_projectile);
        // blockrange=no (op18 breakroutefinding → blockwalk 0): no collision.
        let p = LocClip::from_loc_ops(&["blockrange=no".to_string()]);
        assert!(!p.clips && !p.block_projectile);
    }

    #[test]
    fn line_of_walk_blocked_by_wall() {
        let mut g = CollisionMap::default();
        // open straight line passes
        assert!(line_of_walk(&g, 10, 10, 14, 10));
        // east wall on tile 12 blocks eastward walk crossing into 12
        g.add_wall(12, 10, 0, 2, false); // rot2 = E wall on (12,10)
        assert!(!line_of_walk(&g, 10, 10, 14, 10));
    }
}
