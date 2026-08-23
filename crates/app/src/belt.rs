//! The asteroid belt: Uranus' ring, up close. A population of rocks laid
//! out procedurally through the ring's volume (the same rock is always in
//! the same place — a hash of its cell), going round with the ring, each
//! with a small drift of its own so they meet. Near the ship a live set
//! of them is simulated: rocks knock each other about, and knock the ship.
//!
//! The rocks are the app's, not the sim's: a strike is an impulse on the
//! ship's state, applied after the step, like a jump.

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

/// The belt's live state.
#[derive(Debug, Default)]
pub struct Belt {
    pub rocks: Vec<Rock>,
    /// The ship's bumps this step, for the shield and the sound.
    pub hits: Vec<Hit>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use farfall_sim::presets;

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
