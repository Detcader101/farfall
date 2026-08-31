//! The ship's arms: what it shoots, and everything round the shooting —
//! hardpoints, heat, magazines, the reactor's share. Slugs fly in the
//! world in f64 and are tested against the belt's rocks each fixed step;
//! a hit wounds the rock (crates/app/src/belt.rs) and leaves a burst for
//! the tracer pass to draw. Like the belt, none of this touches the sim:
//! recoil is an impulse on the ship after the step, like a strike.

use glam::{DQuat, DVec3};

use crate::bay::{Hardpoint, Mount, STOCK};
use crate::belt::Belt;

/// The ship's mass for recoil, kg.
pub const SHIP_MASS_KG: f64 = 12_000.0;
/// How long a slug lives, s, and how many can be in the air.
pub const SLUG_LIFE_S: f64 = 4.0;
pub const MAX_SLUGS: usize = 32;
/// Bursts kept for the pass (the newest win).
pub const MAX_BURSTS: usize = 16;
/// Shards in the air at once (the debris pass's array).
pub const MAX_SHARDS: usize = 64;
/// Scars kept on the rocks (the scar pass's array).
pub const MAX_SCARS: usize = 32;
/// The railgun's time to charge at full power, s.
pub const RAIL_CHARGE_S: f64 = 1.1;
/// Past this heat a weapon jams; it clears below half.
pub const JAM_HEAT: f32 = 1.0;
pub const UNJAM_HEAT: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weapon {
    /// Twin wing cannon: fast, hot, greedy for shells.
    Cannon,
    /// The nose rail: charge, then one slug at a silly speed.
    Rail,
}

impl Weapon {
    pub const ALL: [Weapon; 2] = [Weapon::Cannon, Weapon::Rail];

    pub fn name(self) -> &'static str {
        match self {
            Weapon::Cannon => "CANNON",
            Weapon::Rail => "RAIL",
        }
    }
    /// The settings/world-file key.
    pub fn key(self) -> &'static str {
        match self {
            Weapon::Cannon => "cannon",
            Weapon::Rail => "rail",
        }
    }
    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|w| w.key() == k)
    }
    /// Rounds a second at full power (the rail's is its charge, see
    /// [`RAIL_CHARGE_S`]).
    pub fn rate_hz(self) -> f64 {
        match self {
            Weapon::Cannon => 9.0,
            Weapon::Rail => 1.0 / RAIL_CHARGE_S,
        }
    }
    pub fn muzzle_mps(self) -> f64 {
        match self {
            Weapon::Cannon => 1_400.0,
            Weapon::Rail => 6_000.0,
        }
    }
    pub fn slug_kg(self) -> f64 {
        match self {
            Weapon::Cannon => 0.6,
            Weapon::Rail => 14.0,
        }
    }
    /// Heat per shot (1 is the jam).
    pub fn heat_per_shot(self) -> f32 {
        match self {
            Weapon::Cannon => 0.06,
            Weapon::Rail => 0.55,
        }
    }
    /// Heat shed a second, idle.
    pub fn cool_per_s(self) -> f32 {
        match self {
            Weapon::Cannon => 0.22,
            Weapon::Rail => 0.35,
        }
    }
    pub fn magazine(self) -> u32 {
        match self {
            Weapon::Cannon => 600,
            Weapon::Rail => 24,
        }
    }
    /// The pass's kind index.
    pub fn kind(self) -> u8 {
        match self {
            Weapon::Cannon => 0,
            Weapon::Rail => 1,
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

/// Where the stock guns are, ship frame (x right, y up, -z the nose),
/// metres — the hardpoints' places, see [`crate::bay::Hardpoint`].
pub const WING_L: DVec3 = DVec3::new(-2.6, -0.35, -0.6);
pub const WING_R: DVec3 = DVec3::new(2.6, -0.35, -0.6);
pub const NOSE: DVec3 = DVec3::new(0.0, -0.45, -4.2);

/// A slug in the air, world frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slug {
    pub pos: DVec3,
    pub vel: DVec3,
    pub born_s: f64,
    pub weapon: Weapon,
}

/// Something that flashed: at a muzzle, on a rock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Burst {
    /// World frame; `vel` carries it with whatever it is on.
    pub pos: DVec3,
    pub vel: DVec3,
    pub at_s: f64,
    /// 0 muzzle flash, 1 a hit on a rock, 2 a rock breaking, 3 a rail hit.
    pub kind: u8,
    /// A scale: rock radius over 20 m for hits, 1 for flashes.
    pub size: f32,
    pub seed: f32,
}

/// A shard of rock: a chip off a hit, a piece of a break. Tumbles, cools.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shard {
    pub pos: DVec3,
    pub vel: DVec3,
    /// The tumble: an axis and a rate (rad/s).
    pub axis: DVec3,
    pub spin: f64,
    /// Half its longest side, metres.
    pub size: f32,
    pub born_s: f64,
    pub life_s: f32,
    pub seed: f32,
}

/// A crater on a rock that held: where, how big, how hot it started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scar {
    pub rock: crate::belt::RockId,
    /// From the rock's centre, a unit direction, planet frame.
    pub dir: DVec3,
    pub born_s: f64,
    pub size_m: f32,
    pub seed: f32,
}

impl Burst {
    pub fn life_s(kind: u8) -> f64 {
        match kind {
            0 => 0.09,
            1 => 0.55,
            2 => 1.6,
            _ => 0.8,
        }
    }
}

/// A slug that landed on a rock this step, for whoever wants to know
/// (the haul chips ore off it; a mimic in that rock shows itself).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landed {
    pub rock: crate::belt::Rock,
    pub energy_j: f64,
    pub destroyed: bool,
}

/// The arms this fixed step: what the ship is told.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ship {
    pub pos: DVec3,
    pub vel: DVec3,
    pub orient: DQuat,
    /// Where the guns point, world frame (the nose, or the gaze).
    pub aim: DVec3,
}

#[derive(Debug, Clone)]
pub struct Arms {
    pub selected: Weapon,
    pub heat: [f32; 2],
    pub ammo: [u32; 2],
    pub jammed: [bool; 2],
    /// The reactor's share for the guns, 0..1.
    pub power: f32,
    /// The ship's fit: what each hardpoint carries (the SHIP bay's).
    pub mounts: [Mount; 4],
    /// The rail's charge 0..1 while the trigger is held.
    pub charge: f32,
    pub slugs: Vec<Slug>,
    pub bursts: Vec<Burst>,
    /// What landed on a rock this step.
    pub landed: Vec<Landed>,
    /// Counts for the sound: shots and the last one's kind; rock hits
    /// and the last one's size; breaks.
    pub shots: u32,
    pub shot_kind: u8,
    pub bangs: u32,
    pub bang_size: f32,
    pub breaks: u32,
    /// The debris: shards in the air, how many a break throws, how long
    /// they last (the settings').
    pub shards: Vec<Shard>,
    pub shards_per_break: u32,
    pub shard_life_s: f32,
    /// The scars on the rocks, their size (a multiplier, 0 none) and how
    /// long one takes to cool, seconds (the settings').
    pub scars: Vec<Scar>,
    pub scar_size: f32,
    pub scar_cool_s: f32,
    /// Which side the last shot left from, -1..1 (x of its mount over
    /// the wing's reach): for the camera's jolt.
    pub last_side: f32,
    next_shot_s: f64,
    /// Which of a weapon's mounts fires next: they take turns.
    round: usize,
    trigger_was: bool,
    seq: u32,
}

impl Default for Arms {
    fn default() -> Self {
        Self {
            selected: Weapon::Cannon,
            heat: [0.0; 2],
            ammo: [Weapon::Cannon.magazine(), Weapon::Rail.magazine()],
            jammed: [false; 2],
            power: 0.5,
            charge: 0.0,
            slugs: Vec::new(),
            bursts: Vec::new(),
            landed: Vec::new(),
            shots: 0,
            shot_kind: 0,
            bangs: 0,
            bang_size: 0.0,
            breaks: 0,
            shards: Vec::new(),
            shards_per_break: 24,
            shard_life_s: 5.0,
            scars: Vec::new(),
            scar_size: 1.0,
            scar_cool_s: 12.0,
            last_side: 0.0,
            next_shot_s: 0.0,
            mounts: STOCK,
            round: 0,
            trigger_was: false,
            seq: 1,
        }
    }
}

/// A slug travelling `a` to `b` against a sphere: the fraction along the
/// segment of the first touch, if any.
pub fn segment_hits_sphere(a: DVec3, b: DVec3, centre: DVec3, radius: f64) -> Option<f64> {
    let d = b - a;
    let f = a - centre;
    let aa = d.dot(d);
    if aa < 1e-12 {
        return (f.length() <= radius).then_some(0.0);
    }
    let bb = 2.0 * f.dot(d);
    let cc = f.dot(f) - radius * radius;
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let t0 = (-bb - s) / (2.0 * aa);
    let t1 = (-bb + s) / (2.0 * aa);
    if (0.0..=1.0).contains(&t0) {
        Some(t0)
    } else if t0 < 0.0 && t1 >= 0.0 {
        Some(0.0)
    } else {
        None
    }
}

impl Arms {
    pub fn select(&mut self, w: Weapon) {
        self.selected = w;
        self.charge = 0.0;
    }

    pub fn next_weapon(&mut self) {
        let i = Weapon::ALL
            .iter()
            .position(|&w| w == self.selected)
            .unwrap_or(0);
        self.select(Weapon::ALL[(i + 1) % Weapon::ALL.len()]);
    }

    pub fn heat_of(&self, w: Weapon) -> f32 {
        self.heat[w.index()]
    }
    pub fn ammo_of(&self, w: Weapon) -> u32 {
        self.ammo[w.index()]
    }
    pub fn jammed_of(&self, w: Weapon) -> bool {
        self.jammed[w.index()]
    }

    fn unit(&mut self) -> f32 {
        self.seq = self.seq.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seq >> 8) as f32 / (1u32 << 24) as f32
    }

    /// One fixed step. `trigger`: the button this frame. Returns the recoil
    /// to give the ship, m/s.
    pub fn step(
        &mut self,
        t_s: f64,
        dt: f64,
        ship: &Ship,
        trigger: bool,
        belt: &mut Belt,
    ) -> DVec3 {
        self.landed.clear();
        let power = self.power.clamp(0.0, 1.0);
        // Cool, and clear jams.
        for (i, w) in Weapon::ALL.iter().enumerate() {
            self.heat[i] =
                (self.heat[i] - w.cool_per_s() * (0.6 + 0.8 * power) * dt as f32).max(0.0);
            if self.jammed[i] && self.heat[i] < UNJAM_HEAT {
                self.jammed[i] = false;
            }
        }
        let mut recoil = DVec3::ZERO;
        let w = self.selected;
        let i = w.index();
        let can_fire =
            !self.jammed[i] && self.ammo[i] > 0 && power > 0.02 && self.mounted(w).is_some();
        match w {
            Weapon::Cannon => {
                if trigger && can_fire && t_s >= self.next_shot_s {
                    let rate = w.rate_hz() * (0.45 + 0.55 * power as f64);
                    self.next_shot_s = t_s + 1.0 / rate;
                    let mount = self.next_mount(w);
                    recoil += self.fire(t_s, ship, mount, 1.0);
                }
            }
            Weapon::Rail => {
                if trigger && can_fire {
                    self.charge =
                        (self.charge + (dt / RAIL_CHARGE_S) as f32 * (0.3 + 0.7 * power)).min(1.0);
                } else if self.trigger_was && self.charge > 0.25 && can_fire {
                    // Let go: it fires with what it has.
                    let level = self.charge;
                    self.charge = 0.0;
                    let mount = self.next_mount(w);
                    recoil += self.fire(t_s, ship, mount, level as f64);
                } else {
                    self.charge = (self.charge - dt as f32 * 2.0).max(0.0);
                }
                if self.charge >= 1.0 && can_fire {
                    self.charge = 0.0;
                    let mount = self.next_mount(w);
                    recoil += self.fire(t_s, ship, mount, 1.0);
                }
            }
        }
        self.trigger_was = trigger;

        // Slugs fly, and meet rocks.
        let mut k = 0;
        while k < self.slugs.len() {
            let s = self.slugs[k];
            if t_s - s.born_s > SLUG_LIFE_S {
                self.slugs.swap_remove(k);
                continue;
            }
            let a = s.pos;
            let b = s.pos + s.vel * dt;
            let mut best: Option<(f64, usize)> = None;
            for (ri, r) in belt.rocks.iter().enumerate() {
                if let Some(f) = segment_hits_sphere(a, b, r.pos, r.radius_m * 1.05) {
                    if best.is_none_or(|(bf, _)| f < bf) {
                        best = Some((f, ri));
                    }
                }
            }
            if let Some((f, ri)) = best {
                let at = a + (b - a) * f;
                let rock = belt.rocks[ri];
                let rel = s.vel - rock.vel;
                let energy = 0.5 * s.weapon.slug_kg() * rel.length_squared();
                let momentum = s.weapon.slug_kg() * rel.length();
                let size = (rock.radius_m / 20.0) as f32;
                let d = belt.strike(ri, energy, momentum, at, rel.normalize_or_zero());
                self.landed.push(Landed {
                    rock,
                    energy_j: energy,
                    destroyed: d.destroyed,
                });
                // A rock near breaking throws more off with every hit.
                let cracked = if d.destroyed {
                    0.0
                } else {
                    belt.wound(ri) as f32
                };
                self.bangs = self.bangs.wrapping_add(1);
                self.bang_size = size.clamp(0.1, 3.0);
                let seed = self.unit();
                let kind = if d.destroyed {
                    self.breaks = self.breaks.wrapping_add(1);
                    2
                } else if s.weapon == Weapon::Rail {
                    3
                } else {
                    1
                };
                // Chips off a hit; a break throws its dust and chunks out.
                let count = if d.destroyed {
                    self.shards_per_break
                } else {
                    self.shards_per_break / 6
                };
                self.throw_shards(t_s, at, rock, rel.normalize_or_zero(), count, d.destroyed);
                if !d.destroyed && self.scar_size > 0.0 {
                    // A crater the size of the blow, within reason.
                    let size_m = ((0.6 + energy.sqrt() / 900.0) * self.scar_size as f64)
                        .min(rock.radius_m * 0.45) as f32;
                    let seed = self.unit();
                    if self.scars.len() >= MAX_SCARS {
                        self.scars.remove(0);
                    }
                    self.scars.push(Scar {
                        rock: rock.id,
                        dir: (at - rock.pos).normalize_or_zero(),
                        born_s: t_s,
                        size_m,
                        seed,
                    });
                }
                self.push_burst(Burst {
                    pos: at,
                    vel: rock.vel,
                    at_s: t_s,
                    kind,
                    size: if d.destroyed {
                        size.max(0.4)
                    } else {
                        (size * (1.0 + cracked)).clamp(0.25, 2.5)
                    },
                    seed,
                });
                self.slugs.swap_remove(k);
                continue;
            }
            self.slugs[k].pos = b;
            k += 1;
        }
        // Old bursts go; shards fly, then go.
        self.bursts.retain(|b| t_s - b.at_s < Burst::life_s(b.kind));
        for sh in self.shards.iter_mut() {
            sh.pos += sh.vel * dt;
        }
        self.shards.retain(|sh| t_s - sh.born_s < sh.life_s as f64);
        // Scars cool away, and go with their rock.
        let cool = self.scar_cool_s as f64;
        self.scars
            .retain(|sc| t_s - sc.born_s < cool && belt.rocks.iter().any(|r| r.id == sc.rock));
        recoil
    }

    /// Shards leave the strike: chips spray back off the face; a break's
    /// pieces go every way, the bigger the rock the bigger and faster.
    pub fn throw_shards(
        &mut self,
        t_s: f64,
        at: DVec3,
        rock: crate::belt::Rock,
        along: DVec3,
        count: u32,
        broke: bool,
    ) {
        let life = self.shard_life_s;
        for _ in 0..count {
            let dir = DVec3::new(
                self.unit() as f64 - 0.5,
                self.unit() as f64 - 0.5,
                self.unit() as f64 - 0.5,
            )
            .normalize_or_zero();
            // A hit sprays back toward the shooter; a break goes outward
            // from the rock's heart.
            let fling = if broke {
                (dir + (at - rock.pos).normalize_or_zero() * 0.4).normalize_or_zero()
            } else {
                (dir - along * 1.2).normalize_or_zero()
            };
            let speed =
                if broke { 3.0 } else { 6.0 } + (rock.radius_m.sqrt() * 4.0) * self.unit() as f64;
            let size =
                (rock.radius_m * (if broke { 0.08 } else { 0.03 }) * (0.5 + self.unit() as f64))
                    .clamp(0.2, 8.0) as f32;
            let axis = DVec3::new(
                self.unit() as f64 - 0.5,
                self.unit() as f64 - 0.5,
                self.unit() as f64 - 0.5,
            )
            .normalize_or_zero();
            let spin =
                (0.5 + 3.0 * self.unit() as f64) * if self.unit() > 0.5 { 1.0 } else { -1.0 };
            let life_s = life * (0.6 + 0.8 * self.unit());
            let seed = self.unit();
            if self.shards.len() >= MAX_SHARDS {
                self.shards.remove(0);
            }
            self.shards.push(Shard {
                pos: at + fling * size as f64,
                vel: rock.vel + fling * speed,
                axis,
                spin,
                size,
                born_s: t_s,
                life_s,
                seed,
            });
        }
    }

    pub fn push_burst(&mut self, b: Burst) {
        self.bursts.insert(0, b);
        self.bursts.truncate(MAX_BURSTS);
    }

    /// One slug leaves `mount` at `level` of the muzzle speed. Returns the
    /// recoil on the ship.
    /// The hardpoints carrying this weapon, in order; None if it is not
    /// mounted at all.
    pub fn mounted(&self, w: Weapon) -> Option<Vec<Hardpoint>> {
        let v: Vec<Hardpoint> = Hardpoint::ALL
            .iter()
            .zip(self.mounts.iter())
            .filter(|(_, m)| m.weapon() == Some(w))
            .map(|(h, _)| *h)
            .collect();
        (!v.is_empty()).then_some(v)
    }

    /// The weapon's mounts take turns: the next one's place.
    fn next_mount(&mut self, w: Weapon) -> DVec3 {
        let hs = self.mounted(w).unwrap_or_else(|| vec![Hardpoint::Nose]);
        let h = hs[self.round % hs.len()];
        self.round += 1;
        h.pos()
    }

    fn fire(&mut self, t_s: f64, ship: &Ship, mount: DVec3, level: f64) -> DVec3 {
        let w = self.selected;
        let i = w.index();
        self.ammo[i] -= 1;
        self.last_side = (mount.x / 2.6).clamp(-1.0, 1.0) as f32;
        self.heat[i] += w.heat_per_shot() * level as f32;
        if self.heat[i] >= JAM_HEAT {
            self.jammed[i] = true;
        }
        let dir = ship.aim.normalize_or_zero();
        // A little scatter for the cannon; the rail is true.
        let dir = if w == Weapon::Cannon {
            let (u, v) = (self.unit() - 0.5, self.unit() - 0.5);
            let side = dir.cross(DVec3::Y).normalize_or_zero();
            let up = side.cross(dir);
            (dir + (side * u as f64 + up * v as f64) * 0.012).normalize_or_zero()
        } else {
            dir
        };
        let speed = w.muzzle_mps() * (0.4 + 0.6 * level);
        let pos = ship.pos + ship.orient * mount;
        if self.slugs.len() >= MAX_SLUGS {
            self.slugs.remove(0);
        }
        self.slugs.push(Slug {
            pos,
            vel: ship.vel + dir * speed,
            born_s: t_s,
            weapon: w,
        });
        self.shots = self.shots.wrapping_add(1);
        self.shot_kind = w.kind();
        let seed = self.unit();
        self.push_burst(Burst {
            pos,
            vel: ship.vel,
            at_s: t_s,
            kind: 0,
            size: if w == Weapon::Rail {
                2.2 * level as f32
            } else {
                1.0
            },
            seed,
        });
        -dir * (w.slug_kg() * speed / SHIP_MASS_KG)
    }

    /// The line for the readout.
    pub fn text(&self) -> String {
        let w = self.selected;
        let heat = (self.heat_of(w) * 100.0).round();
        let state = if self.mounted(w).is_none() {
            " NO MOUNT".to_string()
        } else if self.jammed_of(w) {
            " JAM".to_string()
        } else if self.ammo_of(w) == 0 {
            " EMPTY".to_string()
        } else if w == Weapon::Rail && self.charge > 0.0 {
            format!(" CHG {:.0}%", self.charge * 100.0)
        } else {
            String::new()
        };
        format!("{} {}  HEAT {heat:.0}%{state}", w.name(), self.ammo_of(w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::belt::Rock;

    fn ship() -> Ship {
        Ship {
            pos: DVec3::ZERO,
            vel: DVec3::ZERO,
            orient: DQuat::IDENTITY,
            aim: DVec3::NEG_Z,
        }
    }

    fn belt_with_rock(z: f64, r: f64) -> Belt {
        let mut b = Belt::default();
        b.rocks.push(Rock {
            id: (0, 0, 0, 0),
            pos: DVec3::new(0.0, 0.0, z),
            vel: DVec3::ZERO,
            radius_m: r,
            seed: 0.5,
            spin: 0.0,
        });
        b
    }

    #[test]
    fn a_segment_finds_the_first_touch_of_a_sphere() {
        let c = DVec3::new(0.0, 0.0, -100.0);
        let f = segment_hits_sphere(DVec3::ZERO, DVec3::new(0.0, 0.0, -200.0), c, 10.0).unwrap();
        assert!((f - 0.45).abs() < 1e-9, "{f}");
        assert!(segment_hits_sphere(DVec3::ZERO, DVec3::new(0.0, 0.0, -50.0), c, 10.0).is_none());
        assert!(segment_hits_sphere(DVec3::ZERO, DVec3::new(0.0, 30.0, -200.0), c, 10.0).is_none());
        assert_eq!(
            segment_hits_sphere(c, DVec3::new(0.0, 0.0, -200.0), c, 10.0),
            Some(0.0)
        );
    }

    #[test]
    fn the_cannon_alternates_wings_at_its_rate_heats_and_kicks() {
        let mut arms = Arms {
            power: 1.0,
            ..Default::default()
        };
        let mut belt = Belt::default();
        let dt = 1.0 / 120.0;
        let mut t = 0.0;
        let mut recoil = DVec3::ZERO;
        for _ in 0..120 {
            recoil += arms.step(t, dt, &ship(), true, &mut belt);
            t += dt;
        }
        assert!((8..=10).contains(&arms.shots), "{} a second", arms.shots);
        assert_eq!(arms.ammo_of(Weapon::Cannon), 600 - arms.shots);
        assert!(arms.heat_of(Weapon::Cannon) > 0.1 && !arms.jammed_of(Weapon::Cannon));
        assert!(recoil.z > 0.0, "kicked back: {recoil:?}");
        assert!(recoil.length() < 1.0, "but not much: {recoil:?}");
        let xs: Vec<f64> = arms.slugs.iter().map(|s| s.pos.x).collect();
        assert!(
            xs.iter().any(|&x| x < -1.0) && xs.iter().any(|&x| x > 1.0),
            "both wings: {xs:?}"
        );
        assert!(arms.slugs.iter().all(|s| s.vel.z < -1_000.0));
        // Held down it jams, then cools back.
        let mut jammed = false;
        for _ in 0..(120 * 6) {
            arms.step(t, dt, &ship(), true, &mut belt);
            t += dt;
            jammed |= arms.jammed_of(Weapon::Cannon);
        }
        assert!(jammed, "heat {}", arms.heat_of(Weapon::Cannon));
        let shots = arms.shots;
        for _ in 0..(120 * 4) {
            arms.step(t, dt, &ship(), true, &mut belt);
            t += dt;
        }
        assert!(arms.shots > shots, "it clears and fires again");
        assert!(arms.text().starts_with("CANNON "));
    }

    #[test]
    fn the_rail_charges_and_fires_on_release_or_when_full() {
        let mut arms = Arms {
            power: 1.0,
            ..Default::default()
        };
        arms.select(Weapon::Rail);
        let mut belt = Belt::default();
        let dt = 1.0 / 120.0;
        let mut t = 0.0;
        for _ in 0..45 {
            arms.step(t, dt, &ship(), true, &mut belt);
            t += dt;
        }
        assert!(
            arms.charge > 0.25 && arms.shots == 0,
            "charging: {}",
            arms.charge
        );
        assert!(arms.text().contains("CHG"));
        let r = arms.step(t, dt, &ship(), false, &mut belt);
        assert_eq!(arms.shots, 1, "let go: it fires with what it has");
        assert!(r.z > 0.5, "the rail shoves the ship: {r:?}");
        assert!(arms.slugs[0].vel.length() < Weapon::Rail.muzzle_mps());
        // Held all the way: fires itself.
        for _ in 0..(120 * 2) {
            arms.step(t, dt, &ship(), true, &mut belt);
            t += dt;
        }
        assert!(arms.shots >= 2);
        assert!(arms
            .slugs
            .iter()
            .any(|s| s.vel.length() >= Weapon::Rail.muzzle_mps() - 1.0));
        // No power, no shot.
        let mut cold = Arms {
            power: 0.0,
            ..Default::default()
        };
        for _ in 0..240 {
            cold.step(t, dt, &ship(), true, &mut belt);
            t += dt;
        }
        assert_eq!(cold.shots, 0);
    }

    #[test]
    fn a_slug_hits_a_rock_wounds_it_and_a_rail_slug_breaks_a_small_one() {
        let mut arms = Arms {
            power: 1.0,
            ..Default::default()
        };
        let mut belt = belt_with_rock(-300.0, 30.0);
        let dt = 1.0 / 120.0;
        let mut t = 0.0;
        // One cannon shot, then wait for it to land.
        arms.step(t, dt, &ship(), true, &mut belt);
        for _ in 0..60 {
            t += dt;
            arms.step(t, dt, &ship(), false, &mut belt);
        }
        assert_eq!(arms.bangs, 1, "it landed");
        assert!(arms.slugs.is_empty());
        assert!(
            belt.wound(0) > 0.0 && belt.wound(0) < 0.2,
            "{}",
            belt.wound(0)
        );
        assert!(belt.rocks[0].vel.z < 0.0, "nudged away");
        assert!(arms
            .bursts
            .iter()
            .any(|b| b.kind == 1 && b.pos.z < -260.0 && b.pos.z > -300.0));
        // The rail on a small rock: gone.
        let mut belt = belt_with_rock(-500.0, 5.0);
        arms.select(Weapon::Rail);
        arms.charge = 1.0;
        arms.step(t, dt, &ship(), true, &mut belt);
        assert_eq!(arms.shots, 2);
        for _ in 0..60 {
            t += dt;
            arms.step(t, dt, &ship(), false, &mut belt);
        }
        assert_eq!(arms.breaks, 1);
        assert!(belt.rocks.is_empty(), "dust");
        assert!(arms.bursts.iter().any(|b| b.kind == 2));
        // Bursts age out.
        for _ in 0..(120 * 3) {
            t += dt;
            arms.step(t, dt, &ship(), false, &mut belt);
        }
        assert!(arms.bursts.is_empty());
    }

    #[test]
    fn shards_fly_off_a_hit_carry_the_rock_along_and_die_in_their_time() {
        let mut arms = Arms {
            shards_per_break: 24,
            shard_life_s: 2.0,
            ..Default::default()
        };
        let mut belt = Belt::default();
        belt.rocks.push(crate::belt::Rock {
            id: (0, 0, 0, 0),
            pos: DVec3::new(0.0, 0.0, -300.0),
            vel: DVec3::new(0.0, 0.0, 40.0),
            radius_m: 30.0,
            seed: 0.2,
            spin: 0.0,
        });
        let ship = Ship {
            pos: DVec3::ZERO,
            vel: DVec3::ZERO,
            orient: DQuat::IDENTITY,
            aim: DVec3::NEG_Z,
        };
        arms.select(Weapon::Cannon);
        let mut t = 0.0;
        // One shot, then wait for it to land.
        arms.step(t, 0.01, &ship, true, &mut belt);
        for _ in 0..60 {
            t += 0.01;
            arms.step(t, 0.01, &ship, false, &mut belt);
        }
        assert_eq!(
            arms.shards.len(),
            4,
            "a hit chips a sixth of a break's worth"
        );
        // They carry the rock's own velocity, spray back toward the gun,
        // and tumble.
        assert!(
            arms.shards.iter().all(|s| s.vel.z > 0.0),
            "{:?}",
            arms.shards[0]
        );
        assert!(arms.shards.iter().any(|s| s.spin.abs() > 0.4));
        assert!(arms.shards.iter().all(|s| s.size >= 0.2 && s.size <= 8.0));
        let p0 = arms.shards[0].pos;
        t += 0.01;
        arms.step(t, 0.01, &ship, false, &mut belt);
        assert!((arms.shards[0].pos - p0).length() > 0.0, "they move");
        for _ in 0..400 {
            t += 0.01;
            arms.step(t, 0.01, &ship, false, &mut belt);
        }
        assert!(
            arms.shards.is_empty(),
            "gone after their life: {}",
            arms.shards.len()
        );
        // None thrown when the setting is off.
        arms.shards_per_break = 0;
        arms.step(t, 0.01, &ship, true, &mut belt);
        for _ in 0..60 {
            t += 0.01;
            arms.step(t, 0.01, &ship, false, &mut belt);
        }
        assert!(arms.shards.is_empty());
    }

    #[test]
    fn a_hit_leaves_a_scar_that_cools_away_and_goes_with_its_rock() {
        let mut arms = Arms {
            scar_cool_s: 3.0,
            ..Default::default()
        };
        let mut belt = Belt::default();
        belt.rocks.push(crate::belt::Rock {
            id: (0, 0, 0, 0),
            pos: DVec3::new(0.0, 0.0, -300.0),
            vel: DVec3::ZERO,
            radius_m: 30.0,
            seed: 0.2,
            spin: 0.0,
        });
        let ship = Ship {
            pos: DVec3::ZERO,
            vel: DVec3::ZERO,
            orient: DQuat::IDENTITY,
            aim: DVec3::NEG_Z,
        };
        let mut t = 0.0;
        arms.step(t, 0.01, &ship, true, &mut belt);
        for _ in 0..60 {
            t += 0.01;
            arms.step(t, 0.01, &ship, false, &mut belt);
        }
        assert_eq!(arms.scars.len(), 1);
        let sc = arms.scars[0];
        assert_eq!(sc.rock, (0, 0, 0, 0));
        assert!(sc.dir.z > 0.9, "on the face toward the gun: {:?}", sc.dir);
        assert!(sc.size_m > 0.6 && sc.size_m < 13.5, "{}", sc.size_m);
        // Gone once cool.
        for _ in 0..310 {
            t += 0.01;
            arms.step(t, 0.01, &ship, false, &mut belt);
        }
        assert!(arms.scars.is_empty());
        // And gone with the rock.
        arms.step(t, 0.01, &ship, true, &mut belt);
        for _ in 0..60 {
            t += 0.01;
            arms.step(t, 0.01, &ship, false, &mut belt);
        }
        assert_eq!(arms.scars.len(), 1);
        belt.rocks.clear();
        arms.step(t, 0.01, &ship, false, &mut belt);
        assert!(arms.scars.is_empty());
        // None with the size off.
        arms.scar_size = 0.0;
        belt.rocks.push(crate::belt::Rock {
            id: (0, 0, 0, 1),
            pos: DVec3::new(0.0, 0.0, -300.0),
            vel: DVec3::ZERO,
            radius_m: 30.0,
            seed: 0.2,
            spin: 0.0,
        });
        arms.step(t, 0.01, &ship, true, &mut belt);
        for _ in 0..60 {
            t += 0.01;
            arms.step(t, 0.01, &ship, false, &mut belt);
        }
        assert!(arms.scars.is_empty());
    }

    #[test]
    fn the_guns_fire_from_whatever_the_bay_mounted_and_not_at_all_when_nothing_is() {
        let mut arms = Arms::default();
        let mut belt = Belt::default();
        let ship = Ship {
            pos: DVec3::ZERO,
            vel: DVec3::ZERO,
            orient: DQuat::IDENTITY,
            aim: DVec3::NEG_Z,
        };
        // Both wings and the belly carry cannon: three mounts take turns.
        arms.mounts = [Mount::Empty, Mount::Cannon, Mount::Cannon, Mount::Cannon];
        let mut t = 0.0;
        for _ in 0..40 {
            arms.step(t, 0.01, &ship, true, &mut belt);
            t += 0.01;
        }
        let xs: Vec<f64> = arms.slugs.iter().map(|s| s.pos.x.signum()).collect();
        assert!(arms.slugs.len() >= 3, "{}", arms.slugs.len());
        assert!(xs.contains(&-1.0) && xs.contains(&1.0), "{xs:?}");
        assert!(
            arms.slugs.iter().any(|s| s.pos.y < -1.5),
            "one from the belly: {:?}",
            arms.slugs.iter().map(|s| s.pos).collect::<Vec<_>>()
        );
        // Nothing mounted for the rail: it will not charge or fire.
        arms.select(Weapon::Rail);
        for _ in 0..200 {
            arms.step(t, 0.01, &ship, true, &mut belt);
            t += 0.01;
        }
        assert_eq!(arms.charge, 0.0);
        assert_eq!(arms.ammo_of(Weapon::Rail), Weapon::Rail.magazine());
        assert!(arms.text().contains("NO MOUNT"), "{}", arms.text());
    }
}
