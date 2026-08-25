//! The asteroid belt: Uranus' ring, up close. A population of rocks laid
//! out procedurally through the ring's volume (the same rock is always in
//! the same place — a hash of its cell), going round with the ring, each
//! with a small drift of its own so they meet. Near the ship a live set
//! of them is simulated: rocks knock each other about, and knock the ship.
//!
//! The rocks are the app's, not the sim's: a strike is an impulse on the
//! ship's state, applied after the step, like a jump.

use std::collections::{HashMap, HashSet};

use glam::DVec3;

use crate::warp::{RING_AXIS, RING_HALF_M, RING_INNER, RING_OUTER};
use farfall_sim::{Body, WorldParams};

/// How many rocks the live set holds at most (the shader's array too).
pub const LIVE: usize = 48;
/// Rocks are live within this distance of the ship, let go beyond a bit
/// more (hysteresis, so a rock on the edge does not flicker).
pub const LIVE_M: f64 = 5_000.0;
pub const DROP_M: f64 = 6_500.0;
/// The rock field: one cell this big in the ring's own coordinates (along
/// the ring, across it, through it), each holding a few rocks.
pub const CELL_M: f64 = 1_400.0;
/// The ship, as a sphere, for contact.
pub const SHIP_RADIUS_M: f64 = 5.0;
/// A rock's toughness: the energy (J) it takes to break one, per square
/// metre of its cross-section — cracking is a surface thing, so a rock
/// twice the size takes four times the punishment, not eight.
pub const TOUGH_J_PER_M2: f64 = 2.0e4;
/// Below this radius a broken rock is dust: no fragments.
pub const FRAG_MIN_M: f64 = 4.0;
/// Fragments' ids use this slot range, above the cell's own 0..3.
const FRAG_SLOT0: u8 = 8;
/// The share of a broken rock's volume that goes to dust and shards
/// rather than to pieces big enough to stay rocks.
pub const DUST_SHARE: f64 = 0.12;

/// A rock's identity: its cell and its slot in it.
pub type RockId = (i64, i64, i64, u8);

/// A rock in the live set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rock {
    /// Which cell and which of its rocks: its identity.
    pub id: (i64, i64, i64, u8),
    /// Centre, planet frame (m), and velocity (m/s).
    pub pos: DVec3,
    pub vel: DVec3,
    pub radius_m: f64,
    /// A seed for the shader's look, and a spin (rad/s about its own axis).
    pub seed: f32,
    pub spin: f32,
}

/// A bump on the hull this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Direction from the ship to the rock, world frame.
    pub from: DVec3,
    /// Closing speed, m/s, and the rock's radius.
    pub closing_mps: f64,
    pub radius_m: f64,
}

fn hash(x: i64, y: i64, z: i64, k: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f)
        ^ k.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

fn unit(h: u32) -> f64 {
    (h >> 8) as f64 / (1u64 << 24) as f64
}

/// The ring's frame about Uranus: `e1`, `e2` in its plane, `axis` through.
pub fn ring_frame() -> (DVec3, DVec3, DVec3) {
    let axis = RING_AXIS.normalize();
    let e1 = axis.cross(DVec3::Y).normalize();
    let e2 = axis.cross(e1).normalize();
    (e1, e2, axis)
}

/// The ring goes round rigidly at the rate of its middle.
pub fn ring_rate_radps(uranus: &Body) -> f64 {
    let r = uranus.radius_m * crate::warp::RING_MID;
    (uranus.mu / (r * r * r)).sqrt()
}

/// The rock population in one cell, at time `t`: the cell is indexed in
/// the ring's co-rotating coordinates (arc length along the ring at the
/// middle radius, radial offset, height).
pub fn cell_rocks(uranus: &Body, cell: (i64, i64, i64), t_s: f64) -> Vec<Rock> {
    let (e1, e2, axis) = ring_frame();
    let r_in = uranus.radius_m * RING_INNER;
    let r_out = uranus.radius_m * RING_OUTER;
    let r_mid = uranus.radius_m * crate::warp::RING_MID;
    let theta0 = ring_rate_radps(uranus) * t_s;
    let n = (hash(cell.0, cell.1, cell.2, 7) % 4) as u8; // 0..3 rocks a cell
    let mut out = Vec::with_capacity(n as usize);
    for k in 0..n {
        let h = |salt: u32| unit(hash(cell.0, cell.1, cell.2, salt + 16 * k as u32));
        let along = (cell.0 as f64 + h(1)) * CELL_M;
        let r = r_in + (cell.1 as f64 + h(2)) * CELL_M;
        if r < r_in || r > r_out {
            continue;
        }
        let height = (cell.2 as f64 + h(3)) * CELL_M;
        if height.abs() > RING_HALF_M {
            continue;
        }
        let theta = along / r_mid + theta0;
        let radial = e1 * theta.cos() + e2 * theta.sin();
        let tangent = axis.cross(radial);
        let pos = uranus.centre + radial * r + axis * height;
        // The ring's own motion at this radius, plus a drift of its own
        // of a few metres a second any way, so rocks meet.
        let orbital = tangent * (uranus.mu / r).sqrt();
        let drift = (radial * (h(4) - 0.5) + tangent * (h(5) - 0.5) + axis * (h(6) - 0.5)) * 6.0;
        // Sizes: mostly small, a few big — log-distributed 5..300 m.
        let radius_m = 5.0 * 60f64.powf(h(7).powf(1.4));
        out.push(Rock {
            id: (cell.0, cell.1, cell.2, k),
            pos,
            vel: orbital + drift,
            radius_m,
            seed: h(8) as f32,
            spin: ((h(9) - 0.5) * 0.6) as f32,
        });
    }
    out
}

/// The cell a point falls in, in the ring's coordinates at time `t`.
pub fn cell_of(uranus: &Body, p: DVec3, t_s: f64) -> (i64, i64, i64) {
    let (e1, e2, axis) = ring_frame();
    let r_in = uranus.radius_m * RING_INNER;
    let r_mid = uranus.radius_m * crate::warp::RING_MID;
    let rel = p - uranus.centre;
    let height = rel.dot(axis);
    let flat = rel - axis * height;
    let r = flat.length().max(1.0);
    let theta = flat.dot(e2).atan2(flat.dot(e1)) - ring_rate_radps(uranus) * t_s;
    let along = theta.rem_euclid(std::f64::consts::TAU) * r_mid;
    (
        (along / CELL_M).floor() as i64,
        ((r - r_in) / CELL_M).floor() as i64,
        (height / CELL_M).floor() as i64,
    )
}

/// What a hit did to a rock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub destroyed: bool,
    /// Fragments spawned (0 when the rock was dust-sized or survived).
    pub fragments: usize,
}

/// The belt's live state.
#[derive(Debug, Default)]
pub struct Belt {
    pub rocks: Vec<Rock>,
    /// The ship's bumps this step, for the shield and the sound.
    pub hits: Vec<Hit>,
    /// Damage taken so far by rocks that have been shot, J, by id: the
    /// rocks themselves are regenerated from their hash, so the wounds
    /// are kept aside.
    pub wounds: HashMap<RockId, f64>,
    /// Rocks that are gone: never brought back in from the hash.
    pub dead: HashSet<RockId>,
    frag_seq: u32,
}

impl Belt {
    /// Is the ship anywhere near the ring at all? Everything else is skipped
    /// when it is not — the belt costs nothing at Earth.
    pub fn near(uranus: &Body, p: DVec3) -> bool {
        let rel = p - uranus.centre;
        let axis = RING_AXIS.normalize();
        let height = rel.dot(axis).abs();
        let r = (rel - axis * rel.dot(axis)).length();
        let margin = LIVE_M * 3.0;
        r > uranus.radius_m * RING_INNER - margin
            && r < uranus.radius_m * RING_OUTER + margin
            && height < RING_HALF_M + margin
    }

    /// One fixed step: bring in rocks near the ship, let far ones go, move
    /// them, knock them together and against the ship. Returns the impulse
    /// to give the ship (m/s), if it was hit.
    pub fn step(
        &mut self,
        params: &WorldParams,
        t_s: f64,
        dt: f64,
        ship_pos: DVec3,
        ship_vel: DVec3,
    ) -> DVec3 {
        self.hits.clear();
        let uranus = params.bodies(t_s)[3];
        if !Self::near(&uranus, ship_pos) {
            self.rocks.clear();
            return DVec3::ZERO;
        }
        // Let go of the far ones.
        self.rocks.retain(|r| (r.pos - ship_pos).length() < DROP_M);
        // Bring in the near ones: every rock of the cells around the
        // ship's that is within reach, nearest first, as many as the set
        // holds.
        let here = cell_of(&uranus, ship_pos, t_s);
        let reach = (LIVE_M / CELL_M).ceil() as i64;
        let mut near: Vec<Rock> = Vec::new();
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for dz in -reach..=reach {
                    let cell = (here.0 + dx, here.1 + dy, here.2 + dz);
                    for rock in cell_rocks(&uranus, cell, t_s) {
                        if (rock.pos - ship_pos).length() < LIVE_M
                            && !self.dead.contains(&rock.id)
                            && !self.rocks.iter().any(|r| r.id == rock.id)
                        {
                            near.push(rock);
                        }
                    }
                }
            }
        }
        near.sort_by(|a, b| {
            let da = (a.pos - ship_pos).length_squared();
            let db = (b.pos - ship_pos).length_squared();
            da.partial_cmp(&db).unwrap()
        });
        for rock in near {
            if self.rocks.len() >= LIVE {
                break;
            }
            self.rocks.push(rock);
        }
        // Move: the ring's gravity is the orbit they already have; only
        // their drift and their knocks change anything, so a straight
        // step is honest enough over a cell.
        for r in &mut self.rocks {
            r.pos += r.vel * dt;
        }
        // Rock against rock: elastic-ish spheres, mass by volume.
        let n = self.rocks.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let (a, b) = (self.rocks[i], self.rocks[j]);
                let d = b.pos - a.pos;
                let dist = d.length();
                let touch = a.radius_m + b.radius_m;
                if dist < touch && dist > 1e-6 {
                    let nrm = d / dist;
                    let closing = (a.vel - b.vel).dot(nrm);
                    if closing > 0.0 {
                        let ma = a.radius_m.powi(3);
                        let mb = b.radius_m.powi(3);
                        let jimp = (1.0 + 0.4) * closing / (1.0 / ma + 1.0 / mb);
                        self.rocks[i].vel -= nrm * (jimp / ma);
                        self.rocks[j].vel += nrm * (jimp / mb);
                    }
                    // Un-overlap.
                    let push = (touch - dist) * 0.5;
                    self.rocks[i].pos -= nrm * push;
                    self.rocks[j].pos += nrm * push;
                }
            }
        }
        // Rock against the ship.
        let mut impulse = DVec3::ZERO;
        let ship_mass = 12_000.0 / 2_000.0; // in rock units (2 t/m³ rock)
        for r in &mut self.rocks {
            let d = r.pos - ship_pos;
            let dist = d.length();
            let touch = r.radius_m + SHIP_RADIUS_M;
            if dist < touch && dist > 1e-6 {
                let nrm = d / dist;
                let closing = (ship_vel + impulse - r.vel).dot(nrm);
                if closing > 0.0 {
                    let mr = r.radius_m.powi(3);
                    let jimp = (1.0 + 0.3) * closing / (1.0 / ship_mass + 1.0 / mr);
                    impulse -= nrm * (jimp / ship_mass);
                    r.vel += nrm * (jimp / mr);
                    self.hits.push(Hit {
                        from: nrm,
                        closing_mps: closing,
                        radius_m: r.radius_m,
                    });
                }
                // The ship is not pushed out — it is the one that moves — but
                // the rock is, a hair, so they do not stick.
                r.pos += nrm * (touch - dist);
            }
        }
        impulse
    }
}

impl Belt {
    /// The energy it takes to break this rock.
    pub fn toughness_j(radius_m: f64) -> f64 {
        TOUGH_J_PER_M2 * std::f64::consts::PI * radius_m * radius_m
    }

    /// How hurt rock `i` is, 0 (whole) .. 1 (about to go).
    pub fn wound(&self, i: usize) -> f64 {
        let r = &self.rocks[i];
        (self.wounds.get(&r.id).copied().unwrap_or(0.0) / Self::toughness_j(r.radius_m))
            .clamp(0.0, 1.0)
    }

    /// A slug of `energy_j` strikes rock `i` at `at`, going `dir` with
    /// momentum `momentum_kgmps`: the rock is shoved and wounded; broken
    /// past its toughness, it goes — into fragments, if it is big enough
    /// to leave any. Returns what happened.
    pub fn strike(
        &mut self,
        i: usize,
        energy_j: f64,
        momentum_kgmps: f64,
        at: DVec3,
        dir: DVec3,
    ) -> Damage {
        let rock = self.rocks[i];
        let mass = 2_000.0 * (4.0 / 3.0) * std::f64::consts::PI * rock.radius_m.powi(3);
        self.rocks[i].vel += dir.normalize_or_zero() * (momentum_kgmps / mass);
        let w = self.wounds.entry(rock.id).or_insert(0.0);
        *w += energy_j.max(0.0);
        if *w < Self::toughness_j(rock.radius_m) {
            return Damage {
                destroyed: false,
                fragments: 0,
            };
        }
        self.wounds.remove(&rock.id);
        self.dead.insert(rock.id);
        self.rocks.swap_remove(i);
        let rock_after = Rock {
            vel: rock.vel + dir.normalize_or_zero() * (momentum_kgmps / mass),
            ..rock
        };
        let n = self.fragment(rock_after, at);
        Damage {
            destroyed: true,
            fragments: n,
        }
    }

    /// Break a rock into pieces about the strike: a few chunks of a third
    /// to a half its size, flung apart from where it was hit, spinning.
    /// Dust-sized rocks leave nothing. Returns how many were made.
    fn fragment(&mut self, rock: Rock, at: DVec3) -> usize {
        if rock.radius_m < FRAG_MIN_M * 1.5 {
            return 0;
        }
        let seq = self.frag_seq;
        let h = |salt: u32| unit(hash(rock.id.0, rock.id.1, rock.id.2, salt ^ seq));
        let n = 2 + (hash(rock.id.0, rock.id.1, rock.id.2, 99 ^ seq) % 3) as usize;
        let n = n.min(LIVE - self.rocks.len());
        if n == 0 {
            return 0;
        }
        let away = (rock.pos - at).normalize_or_zero();
        // Mass is conserved: the pieces share the rock's volume (less the
        // dust) by random weights, and their flings sum to nothing so the
        // rock's momentum is theirs together.
        let weights: Vec<f64> = (0..n).map(|k| 0.5 + h(16 * k as u32 + 1)).collect();
        let wsum: f64 = weights.iter().sum();
        let volume = rock.radius_m.powi(3) * (1.0 - DUST_SHARE);
        let radii: Vec<f64> = weights.iter().map(|w| (volume * w / wsum).cbrt()).collect();
        let flings: Vec<DVec3> = (0..n)
            .map(|k| {
                let s = 16 * k as u32;
                let dir =
                    DVec3::new(h(s + 2) - 0.5, h(s + 3) - 0.5, h(s + 4) - 0.5).normalize_or_zero();
                // Pieces leave from the far side of the hit, mostly.
                (dir + away * 0.8).normalize_or_zero() * (2.0 + 6.0 * h(s + 5))
            })
            .collect();
        let masses: Vec<f64> = radii.iter().map(|r| r.powi(3)).collect();
        let msum: f64 = masses.iter().sum();
        let drift: DVec3 = flings
            .iter()
            .zip(masses.iter())
            .map(|(f, m)| *f * *m)
            .sum::<DVec3>()
            / msum;
        for k in 0..n {
            let s = 16 * k as u32;
            let radius_m = radii[k];
            let fling = flings[k] - drift;
            let slot = FRAG_SLOT0.wrapping_add((self.frag_seq % 240) as u8);
            self.frag_seq = self.frag_seq.wrapping_add(1);
            self.rocks.push(Rock {
                id: (rock.id.0, rock.id.1, rock.id.2, slot),
                pos: rock.pos + fling.normalize_or_zero() * (rock.radius_m - radius_m) * 0.9,
                vel: rock.vel + fling,
                radius_m: radius_m.max(1.0),
                seed: h(s + 6) as f32,
                spin: ((h(s + 7) - 0.5) * 2.0) as f32,
            });
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farfall_sim::presets;

    fn a_belt_with(radius_m: f64) -> Belt {
        let mut b = Belt::default();
        b.rocks.push(Rock {
            id: (1, 2, 3, 0),
            pos: DVec3::new(100.0, 0.0, 0.0),
            vel: DVec3::ZERO,
            radius_m,
            seed: 0.3,
            spin: 0.1,
        });
        b
    }

    #[test]
    fn a_rock_is_shoved_and_wounded_then_breaks_into_fragments_and_stays_dead() {
        let mut b = a_belt_with(20.0);
        let tough = Belt::toughness_j(20.0);
        let at = DVec3::new(80.0, 0.0, 0.0);
        let d = b.strike(0, tough * 0.4, 1.0e6, at, DVec3::X);
        assert!(!d.destroyed && d.fragments == 0);
        assert!(b.rocks[0].vel.x > 0.0, "shoved along the slug");
        assert!((b.wound(0) - 0.4).abs() < 1e-9, "{}", b.wound(0));
        let mass_kg = 2_000.0 * (4.0 / 3.0) * std::f64::consts::PI * 20.0f64.powi(3);
        let rock_vel_after = b.rocks[0].vel + DVec3::X * (1.0e6 / mass_kg);
        let d = b.strike(0, tough * 0.7, 1.0e6, at, DVec3::X);
        assert!(d.destroyed);
        assert!((2..=4).contains(&d.fragments), "{d:?}");
        assert_eq!(b.rocks.len(), d.fragments);
        // Mass and momentum are the rock's, less the dust.
        let volume: f64 = b.rocks.iter().map(|r| r.radius_m.powi(3)).sum();
        assert!(
            (volume / (20.0f64.powi(3) * (1.0 - DUST_SHARE)) - 1.0).abs() < 1e-9,
            "{volume}"
        );
        let mass: f64 = volume;
        let momentum: DVec3 = b
            .rocks
            .iter()
            .map(|r| r.vel * r.radius_m.powi(3))
            .sum::<DVec3>()
            / mass;
        assert!(
            (momentum - rock_vel_after).length() < 1e-6,
            "{momentum} vs {rock_vel_after}"
        );
        assert!(
            b.rocks
                .iter()
                .any(|r| (r.vel - rock_vel_after).length() > 1.0),
            "they fly apart"
        );
        assert!(b.dead.contains(&(1, 2, 3, 0)));
        assert!(b.wounds.is_empty());
        let mut ids = std::collections::HashSet::new();
        for r in &b.rocks {
            assert!(
                r.radius_m < 20.0 * 0.96 && r.radius_m > 20.0 * 0.3,
                "{}",
                r.radius_m
            );
            assert!(
                r.vel.length() > 1.0 && r.vel.length() < 20.0,
                "flung: {:?}",
                r.vel
            );
            assert!(ids.insert(r.id), "fragment ids are distinct");
            assert!(r.id.3 >= 8, "fragments never collide with the hash's slots");
        }
        // Dust: a small rock leaves nothing.
        let mut b = a_belt_with(5.0);
        let d = b.strike(0, Belt::toughness_j(5.0) * 2.0, 1.0, at, DVec3::X);
        assert!(d.destroyed && d.fragments == 0 && b.rocks.is_empty());
    }

    #[test]
    fn a_bigger_rock_is_tougher_by_its_face_not_its_bulk() {
        let a = Belt::toughness_j(10.0);
        let b = Belt::toughness_j(20.0);
        assert!((b / a - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_dead_rock_is_not_brought_back_in() {
        let p = presets::earth_compact();
        let uranus = p.bodies(0.0)[3];
        // Find any populated cell and stand in it.
        let mut found = None;
        'outer: for x in 0..200 {
            for y in 0..8 {
                for z in -2..3 {
                    let rocks = cell_rocks(&uranus, (x, y, z), 0.0);
                    if let Some(r) = rocks.first() {
                        found = Some(*r);
                        break 'outer;
                    }
                }
            }
        }
        let rock = found.expect("some rock in the ring");
        let mut b = Belt::default();
        let ship = rock.pos + DVec3::new(50.0, 0.0, 0.0);
        b.step(&p, 0.0, 1.0 / 120.0, ship, rock.vel);
        assert!(b.rocks.iter().any(|r| r.id == rock.id), "it comes in");
        let i = b.rocks.iter().position(|r| r.id == rock.id).unwrap();
        let d = b.strike(
            i,
            Belt::toughness_j(rock.radius_m) * 2.0,
            0.0,
            ship,
            -DVec3::X,
        );
        assert!(d.destroyed);
        b.step(&p, 1.0 / 120.0, 1.0 / 120.0, ship, rock.vel);
        assert!(!b.rocks.iter().any(|r| r.id == rock.id), "and stays gone");
    }

    #[test]
    fn the_ring_is_populated_the_same_way_every_time_and_only_in_the_ring() {
        let p = presets::earth_compact();
        let uranus = p.bodies(0.0)[3];
        let a = cell_rocks(&uranus, (3, 1, 0), 10.0);
        let b = cell_rocks(&uranus, (3, 1, 0), 10.0);
        assert_eq!(a, b);
        let mut total = 0;
        let (_, _, axis) = ring_frame();
        for x in 0..40 {
            for y in -1..8 {
                for z in -2..3 {
                    for r in cell_rocks(&uranus, (x, y, z), 0.0) {
                        total += 1;
                        let rel = r.pos - uranus.centre;
                        let h = rel.dot(axis).abs();
                        let rad = (rel - axis * rel.dot(axis)).length() / uranus.radius_m;
                        assert!(h <= RING_HALF_M, "{h}");
                        assert!((RING_INNER..=RING_OUTER).contains(&rad), "{rad}");
                        assert!(r.radius_m >= 5.0 && r.radius_m <= 300.0);
                        assert_eq!(cell_of(&uranus, r.pos, 0.0), (x, y, z));
                    }
                }
            }
        }
        assert!(total > 200, "{total}");
    }

    #[test]
    fn the_belt_is_empty_at_earth_and_alive_in_the_ring() {
        let p = presets::earth_compact();
        let mut belt = Belt::default();
        let home = presets::circular_orbit(&p, 12_000.0).ship;
        belt.step(&p, 0.0, 1.0 / 120.0, home.pos_m, home.vel_mps);
        assert!(belt.rocks.is_empty());
        // In the ring: rocks come live, no more than the set holds.
        let uranus = p.bodies(0.0)[3];
        let (e1, _, _) = ring_frame();
        let pos = uranus.centre + e1 * uranus.radius_m * crate::warp::RING_MID;
        for _ in 0..10 {
            belt.step(&p, 0.0, 1.0 / 120.0, pos, DVec3::ZERO);
        }
        assert!(!belt.rocks.is_empty());
        assert!(belt.rocks.len() <= LIVE);
        for r in &belt.rocks {
            assert!((r.pos - pos).length() < DROP_M);
        }
    }

    #[test]
    fn a_rock_knocks_the_ship_and_is_knocked_back() {
        let p = presets::earth_compact();
        let uranus = p.bodies(0.0)[3];
        let (e1, _, _) = ring_frame();
        let pos = uranus.centre + e1 * uranus.radius_m * crate::warp::RING_MID;
        let mut belt = Belt::default();
        belt.step(&p, 0.0, 1.0 / 120.0, pos, DVec3::ZERO);
        // Plant a rock coming straight at the ship.
        belt.rocks.clear();
        belt.rocks.push(Rock {
            id: (0, 0, 0, 9),
            pos: pos + DVec3::X * 20.0,
            vel: DVec3::X * -30.0,
            radius_m: 10.0,
            seed: 0.5,
            spin: 0.0,
        });
        let mut impulse = DVec3::ZERO;
        for _ in 0..120 {
            impulse = belt.step(&p, 0.0, 1.0 / 120.0, pos, DVec3::ZERO);
            if !belt.hits.is_empty() {
                break;
            }
        }
        assert_eq!(belt.hits.len(), 1, "it hits once");
        assert!(impulse.x < -1.0, "the ship is shoved away: {impulse:?}");
        assert!(belt.rocks[0].vel.x > -30.0, "and the rock slowed");
        assert!(belt.hits[0].closing_mps > 20.0);
    }
}
