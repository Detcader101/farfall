//! Mimics: some of the ring's rocks are not rocks. A ship sits inside a
//! holographic shroud shaped like a stone until a slug lands on it — then
//! the projection winks off (the reveal), and what is left either hails
//! the miner or comes about and opens fire. And the rocks that ARE rocks
//! give up ore under the guns: every hit chips material, a break leaves a
//! lump, and a wreck is salvage.
//!
//! Like the belt and the arms, none of this touches `farfall-sim`: it is
//! keyed off the rock hash (so a mimic is where it is in every session,
//! with no list of the population), stepped after the belt each fixed
//! step, and it hands the ship impulses and the shield impacts back to
//! the app.

use std::collections::HashSet;

use glam::{DQuat, DVec3};

use crate::arms::{self, Burst, Slug, Weapon};
use crate::belt::{self, Rock, RockId, FRAG_SLOT0};

/// The most ships in the air at once (the pass has this many lanes).
pub const MAX_MIMICS: usize = 4;
/// How long the shroud takes to go, seconds: the ship glows through the
/// stone, the stone winks off, the hull hardens.
pub const REVEAL_S: f64 = 2.6;
/// The point in the reveal at which the rock projection is gone.
pub const SHROUD_OFF: f64 = 0.4;
/// The hull's hit sphere, metres.
pub const HULL_R_M: f64 = 8.0;
/// The player's own hit sphere, metres (the shield shell).
pub const OWN_R_M: f64 = 4.2;
/// Energy a mimic takes before it is a wreck, joules.
pub const MIMIC_TOUGH_J: f64 = 2.4e6;
/// A hostile mimic holds this range, metres, and fires inside `FIRE_M`.
pub const HOLD_M: f64 = 520.0;
pub const FIRE_M: f64 = 2_400.0;
/// A hail lasts this long on the readout, then the ship stands off.
pub const HAIL_S: f64 = 14.0;
/// A wreck drifts for this long before it is cleared.
pub const WRECK_S: f64 = 90.0;
/// What a slug does to the hull, 0..1 of it.
pub const HIT_HULL: f32 = 0.035;
/// Ore: joules to chip a kilogram of stone, and the lump a break leaves
/// as a share of the rock's mass.
pub const J_PER_KG: f64 = 3.0e4;
pub const BREAK_SHARE: f64 = 0.004;
/// Salvage off a wreck, tonnes.
pub const SALVAGE_T: f64 = 3.2;

/// What a mimic is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    /// Comes about, holds station and talks.
    Hail,
    /// Comes about and shoots.
    Hostile,
}

/// Where a mimic is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The shroud going, `REVEAL_S` long.
    Revealing,
    Hailing,
    Attacking,
    /// Running: full burn away, cleared past 8 km.
    Leaving,
    /// Dead: dark, tumbling, salvage taken.
    Wreck,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mimic {
    /// The rock it was: its identity, so it is never spawned twice.
    pub id: RockId,
    pub pos: DVec3,
    pub vel: DVec3,
    pub orient: DQuat,
    /// Tumble for a wreck, rad/s about a fixed axis.
    pub spin: DVec3,
    pub born_s: f64,
    pub phase: Phase,
    pub phase_s: f64,
    pub mood: Mood,
    /// Damage taken, joules.
    pub wound_j: f64,
    /// Engine effort 0..1, for the look.
    pub effort: f32,
    pub seed: f32,
    next_shot_s: f64,
    /// Fire in bursts: shots left in this one, and when the next begins.
    burst_left: u32,
}

impl Mimic {
    /// A ship put in the air by hand (the bench): at `pos`, `born_s`
    /// setting its reveal, in `phase` with `mood`.
    pub fn planted(
        pos: DVec3,
        vel: DVec3,
        orient: DQuat,
        born_s: f64,
        phase: Phase,
        mood: Mood,
        seed: f32,
    ) -> Self {
        Self {
            id: (0, 0, 0, 254),
            pos,
            vel,
            orient,
            spin: DVec3::new(0.3, 0.9, 0.2),
            born_s,
            phase,
            phase_s: born_s + REVEAL_S,
            mood,
            wound_j: 0.0,
            effort: 0.0,
            seed,
            next_shot_s: born_s,
            burst_left: 0,
        }
    }

    /// The reveal, 0 (a rock) .. 1 (a ship).
    pub fn reveal(&self, t_s: f64) -> f32 {
        ((t_s - self.born_s) / REVEAL_S).clamp(0.0, 1.0) as f32
    }
    /// Whether the rock projection is still up.
    pub fn shrouded(&self, t_s: f64) -> bool {
        (t_s - self.born_s) < REVEAL_S * SHROUD_OFF
    }
    /// How hurt, 0..1.
    pub fn wound(&self) -> f32 {
        (self.wound_j / MIMIC_TOUGH_J).clamp(0.0, 1.0) as f32
    }
    /// The look's kind lane: 0 hailing, 1 hostile, 2 wreck.
    pub fn kind(&self) -> u8 {
        match (self.phase, self.mood) {
            (Phase::Wreck, _) => 2,
            (_, Mood::Hostile) => 1,
            (_, Mood::Hail) => 0,
        }
    }
}

/// A slug from a mimic's guns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoeSlug {
    pub pos: DVec3,
    pub vel: DVec3,
    pub born_s: f64,
}

/// A slug on our own hull this step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OwnHit {
    /// Direction from the ship to where it came from, world frame.
    pub from: DVec3,
    pub size: f32,
}

/// The kinds of stone, by a rock's seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ore {
    Ice,
    Iron,
    Silicate,
}

impl Ore {
    pub fn of_seed(seed: f32) -> Ore {
        if seed < 0.45 {
            Ore::Ice
        } else if seed < 0.72 {
            Ore::Silicate
        } else {
            Ore::Iron
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Ore::Ice => "ICE",
            Ore::Iron => "IRON",
            Ore::Silicate => "SILICATE",
        }
    }
    fn index(self) -> usize {
        match self {
            Ore::Ice => 0,
            Ore::Iron => 1,
            Ore::Silicate => 2,
        }
    }
}

/// What the guns have brought in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Haul {
    /// Tonnes by kind: ice, iron, silicate, salvage.
    pub tonnes: [f64; 4],
    /// ORE YIELD, the setting (1 stock).
    pub yield_: f32,
    /// The last gain: what and when, for the readout.
    pub last: Option<(Ore, f64)>,
}

impl Default for Haul {
    fn default() -> Self {
        Self {
            tonnes: [0.0; 4],
            yield_: 1.0,
            last: None,
        }
    }
}

impl Haul {
    pub fn total_t(&self) -> f64 {
        self.tonnes.iter().sum()
    }

    /// A slug of `energy_j` on a rock: material chipped; a break leaves a
    /// lump of the rock as well.
    pub fn on_hit(&mut self, rock: &Rock, energy_j: f64, destroyed: bool, t_s: f64) {
        if self.yield_ <= 0.0 {
            return;
        }
        let ore = Ore::of_seed(rock.seed);
        let mass = 2_000.0 * (4.0 / 3.0) * std::f64::consts::PI * rock.radius_m.powi(3);
        let mut kg = (energy_j.max(0.0) / J_PER_KG).min(mass * 0.01);
        if destroyed {
            kg += mass * BREAK_SHARE;
        }
        // Lumps of any size stop being ore at the size of a house.
        let kg = kg.min(200_000.0) * self.yield_ as f64;
        self.tonnes[ore.index()] += kg / 1_000.0;
        self.last = Some((ore, t_s));
    }

    pub fn on_salvage(&mut self, t_s: f64) {
        self.tonnes[3] += SALVAGE_T * self.yield_ as f64;
        self.last = Some((Ore::Iron, t_s));
    }

    /// The readout's line, while the haul is fresh (`for_s` after a gain)
    /// or when asked for anyway.
    pub fn text(&self, t_s: f64, for_s: f64) -> Option<String> {
        let (ore, at) = self.last?;
        if t_s - at > for_s {
            return None;
        }
        let total = self.total_t();
        Some(if total >= 100.0 {
            format!("HAUL {total:.0} T  {}", ore.name())
        } else {
            format!("HAUL {total:.1} T  {}", ore.name())
        })
    }
}

/// The mimics: the ships in the air, their slugs, the dice, the counters
/// the sound plays on, and what they said.
#[derive(Debug, Clone, PartialEq)]
pub struct Mimics {
    pub ships: Vec<Mimic>,
    pub slugs: Vec<FoeSlug>,
    /// MIMICS: the share of rocks that are ships, 0..1.
    pub chance: f32,
    /// HOSTILITY: the share of mimics that shoot, 0..1.
    pub hostility: f32,
    /// Rocks that have shown themselves, so they never do twice.
    pub revealed: HashSet<RockId>,
    /// Counters for the sound: reveals, hails, their shots, wrecks.
    pub reveals: u32,
    pub hails: u32,
    pub foe_shots: u32,
    pub wrecks: u32,
    /// Our hull, 1 whole .. 0 (it does not go: crash is deferred).
    pub hull: f32,
    /// Slugs on our hull this step.
    pub own_hits: Vec<OwnHit>,
    /// The line on the readout and when it goes.
    pub line: Option<(String, f64)>,
    seq: u32,
}

impl Default for Mimics {
    fn default() -> Self {
        Self {
            ships: Vec::new(),
            slugs: Vec::new(),
            chance: 1.0,
            hostility: 0.5,
            revealed: HashSet::new(),
            reveals: 0,
            hails: 0,
            foe_shots: 0,
            wrecks: 0,
            hull: 1.0,
            own_hits: Vec::new(),
            line: None,
            seq: 7,
        }
    }
}

/// Is this rock a ship in a shroud? Keyed on the rock's identity, so the
/// answer never changes; fragments never are.
pub fn is_mimic(id: RockId, chance: f32) -> bool {
    if id.3 >= FRAG_SLOT0 {
        return false;
    }
    let h = belt::hash(id.0, id.1, id.2, 0xA11C_E000 ^ (id.3 as u32 * 977));
    belt::unit(h) < chance.clamp(0.0, 1.0) as f64
}

/// What a mimic does once seen, keyed the same way.
pub fn mood_of(id: RockId, hostility: f32) -> Mood {
    let h = belt::hash(id.0, id.1, id.2, 0xB0DE_0000 ^ (id.3 as u32 * 331));
    if belt::unit(h) < hostility.clamp(0.0, 1.0) as f64 {
        Mood::Hostile
    } else {
        Mood::Hail
    }
}

/// What a hailing mimic says, by its seed.
pub fn hail_text(seed: f32) -> &'static str {
    // The readout is 32 columns wide.
    const LINES: [&str; 6] = [
        "HAIL: HOLD FIRE MINER. PASSING",
        "HAIL: THIS ROCK IS TAKEN. GO ON",
        "HAIL: EASY. NO CLAIM HERE. LUCK",
        "HAIL: WE SAW NOTHING. NOR DID U",
        "HAIL: PROSPECTOR? THE ICE IS IN",
        "HAIL: FUEL LOW. CAN YOU SPARE",
    ];
    LINES[((seed.clamp(0.0, 0.999) * LINES.len() as f32) as usize).min(LINES.len() - 1)]
}

/// A rotation whose -Z (the nose) points along `fwd`, with `up` as a hint.
fn look_at(fwd: DVec3, up: DVec3) -> DQuat {
    let f = fwd.normalize_or_zero();
    if f == DVec3::ZERO {
        return DQuat::IDENTITY;
    }
    let mut r = f.cross(up).normalize_or_zero();
    if r == DVec3::ZERO {
        r = f.cross(DVec3::X).normalize_or_zero();
    }
    let u = r.cross(f);
    DQuat::from_mat3(&glam::DMat3::from_cols(r, u, -f)).normalize()
}

impl Mimics {
    fn unit(&mut self) -> f32 {
        self.seq = self.seq.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seq >> 8) as f32 / (1u32 << 24) as f32
    }

    /// A slug has landed on `rock`. If it was a mimic and not yet shown,
    /// the ship is born there and `true` comes back: the app keeps the
    /// rock in the belt until the shroud is off (see `shroud_off`).
    pub fn on_rock_struck(&mut self, rock: &Rock, t_s: f64, ship_pos: DVec3) -> bool {
        if !is_mimic(rock.id, self.chance) || self.revealed.contains(&rock.id) {
            return false;
        }
        if self.ships.len() >= MAX_MIMICS {
            return false;
        }
        self.revealed.insert(rock.id);
        let mood = mood_of(rock.id, self.hostility);
        let seed = belt::unit(belt::hash(rock.id.0, rock.id.1, rock.id.2, 0x5EED)) as f32;
        // It sat in the rock at some attitude: roughly across our line.
        let toward = (ship_pos - rock.pos).normalize_or_zero();
        let across = toward.cross(DVec3::Y).normalize_or_zero();
        let orient = look_at(
            (across * (seed as f64 - 0.5) * 2.0 + toward * 0.4).normalize_or_zero(),
            DVec3::Y,
        );
        self.ships.push(Mimic {
            id: rock.id,
            pos: rock.pos,
            vel: rock.vel,
            orient,
            spin: DVec3::ZERO,
            born_s: t_s,
            phase: Phase::Revealing,
            phase_s: t_s,
            mood,
            wound_j: 0.0,
            effort: 0.0,
            seed,
            next_shot_s: t_s + 1.0,
            burst_left: 0,
        });
        self.reveals = self.reveals.wrapping_add(1);
        self.line = Some(("CONTACT: THE ROCK IS A SHIP".to_string(), t_s + 3.0));
        true
    }

    /// The rocks whose projection has just gone: the app takes them out
    /// of the belt.
    pub fn shroud_off(&self, t_s: f64) -> Vec<RockId> {
        self.ships
            .iter()
            .filter(|m| !m.shrouded(t_s) && m.phase == Phase::Revealing)
            .map(|m| m.id)
            .collect()
    }

    /// Our slugs meet the ships: the ones that land are taken out of the
    /// list, wound the ship, and leave a burst. A ship past its toughness
    /// is a wreck, with salvage for the haul. Slugs are tested over the
    /// step they just flew (`dt`).
    pub fn take_fire(
        &mut self,
        arms: &mut arms::Arms,
        haul: &mut Haul,
        t_s: f64,
        dt: f64,
    ) -> Vec<(DVec3, DVec3, f32)> {
        let mut breaks = Vec::new();
        let mut k = 0;
        while k < arms.slugs.len() {
            let s: Slug = arms.slugs[k];
            let b = s.pos;
            let a = s.pos - s.vel * dt;
            let mut best: Option<(f64, usize)> = None;
            for (mi, m) in self.ships.iter().enumerate() {
                if m.shrouded(t_s) {
                    continue;
                }
                if let Some(f) = arms::segment_hits_sphere(a, b, m.pos, HULL_R_M) {
                    if best.is_none_or(|(bf, _)| f < bf) {
                        best = Some((f, mi));
                    }
                }
            }
            let Some((f, mi)) = best else {
                k += 1;
                continue;
            };
            let at = a + (b - a) * f;
            let m = self.ships[mi];
            let rel = s.vel - m.vel;
            let energy = 0.5 * s.weapon.slug_kg() * rel.length_squared();
            let momentum = s.weapon.slug_kg() * rel.length();
            let seed = self.unit();
            let seed2 = self.unit();
            let ship = &mut self.ships[mi];
            ship.vel += rel.normalize_or_zero() * (momentum / 14_000.0);
            let was_wreck = ship.phase == Phase::Wreck;
            ship.wound_j += energy.max(0.0);
            arms.bangs = arms.bangs.wrapping_add(1);
            arms.bang_size = 0.6;
            let mut kind = if s.weapon == Weapon::Rail { 3 } else { 1 };
            if !was_wreck && ship.wound_j >= MIMIC_TOUGH_J {
                ship.phase = Phase::Wreck;
                ship.phase_s = t_s;
                ship.effort = 0.0;
                ship.spin = DVec3::new(
                    (seed as f64 - 0.5) * 1.2,
                    0.4 + seed as f64,
                    (seed2 as f64 - 0.5) * 0.8,
                );
                self.wrecks = self.wrecks.wrapping_add(1);
                arms.breaks = arms.breaks.wrapping_add(1);
                haul.on_salvage(t_s);
                self.line = Some(("WRECK: SALVAGE TAKEN".to_string(), t_s + 4.0));
                kind = 2;
                breaks.push((at, ship.vel, 1.0));
            } else if !was_wreck && ship.mood == Mood::Hostile && ship.wound() > 0.55 {
                // Hurt enough, it runs.
                ship.phase = Phase::Leaving;
                ship.phase_s = t_s;
                self.line = Some(("THE SHIP BREAKS OFF".to_string(), t_s + 3.0));
            } else if !was_wreck && ship.mood == Mood::Hail && ship.phase != Phase::Leaving {
                // Shot at while talking: it goes hostile, or runs.
                if seed < 0.5 {
                    ship.mood = Mood::Hostile;
                    ship.phase = Phase::Attacking;
                    self.line = Some(("HAIL: SO BE IT".to_string(), t_s + 3.0));
                } else {
                    ship.phase = Phase::Leaving;
                    self.line = Some(("HAIL: WE ARE GONE".to_string(), t_s + 3.0));
                }
                ship.phase_s = t_s;
            }
            let vel = ship.vel;
            arms.push_burst(Burst {
                pos: at,
                vel,
                at_s: t_s,
                kind,
                size: if kind == 2 { 1.2 } else { 0.5 },
                seed,
            });
            arms.slugs.swap_remove(k);
        }
        breaks
    }

    /// One fixed step for every ship and their slugs. `own`: our ship.
    pub fn step(&mut self, t_s: f64, dt: f64, own: &arms::Ship) {
        self.own_hits.clear();
        if let Some((_, until)) = self.line {
            if t_s > until {
                self.line = None;
            }
        }
        let mut new_slugs: Vec<FoeSlug> = Vec::new();
        for i in 0..self.ships.len() {
            let mut m = self.ships[i];
            let to_us = own.pos - m.pos;
            let range = to_us.length();
            let dir_us = to_us / range.max(1.0);
            let rel_v = own.vel - m.vel;
            let age = t_s - m.phase_s;
            let mut thrust = DVec3::ZERO;
            let mut want_fwd: Option<DVec3> = None;
            match m.phase {
                Phase::Revealing => {
                    if t_s - m.born_s >= REVEAL_S {
                        m.phase = match m.mood {
                            Mood::Hail => Phase::Hailing,
                            Mood::Hostile => Phase::Attacking,
                        };
                        m.phase_s = t_s;
                        if m.mood == Mood::Hail {
                            self.hails = self.hails.wrapping_add(1);
                            self.line = Some((hail_text(m.seed).to_string(), t_s + HAIL_S));
                        } else {
                            self.line = Some(("THE SHIP COMES ABOUT".to_string(), t_s + 3.0));
                        }
                    }
                    // Turns to face us as it hardens.
                    want_fwd = Some(dir_us);
                }
                Phase::Hailing => {
                    // Face us, match our velocity, keep a courteous range.
                    want_fwd = Some(dir_us);
                    let hold = (range - HOLD_M * 1.5) * 0.02;
                    thrust = dir_us * hold.clamp(-8.0, 8.0) + rel_v * 0.15;
                    if age > HAIL_S + 4.0 {
                        m.phase = Phase::Leaving;
                        m.phase_s = t_s;
                    }
                }
                Phase::Attacking => {
                    // Keep the guns on us, hold the range, sidle so a
                    // still target is never given.
                    let side = dir_us.cross(DVec3::Y).normalize_or_zero();
                    let weave = (t_s * 0.6 + m.seed as f64 * 6.0).sin();
                    let hold = (range - HOLD_M) * 0.05;
                    thrust = dir_us * hold.clamp(-18.0, 18.0) + rel_v * 0.35 + side * weave * 6.0;
                    // The guns lead us a little: aim where we will be.
                    let flight = range / Weapon::Cannon.muzzle_mps();
                    let aim = (own.pos + rel_v * flight - m.pos).normalize_or_zero();
                    want_fwd = Some(aim);
                    let nose = m.orient * DVec3::NEG_Z;
                    let on = nose.dot(aim) > 0.9985;
                    if range < FIRE_M && on && t_s >= m.next_shot_s {
                        if m.burst_left == 0 {
                            m.burst_left = 4 + (self.unit() * 4.0) as u32;
                        }
                        m.burst_left -= 1;
                        let spread = DVec3::new(
                            (self.unit() as f64 - 0.5) * 0.012,
                            (self.unit() as f64 - 0.5) * 0.012,
                            0.0,
                        );
                        let dir = (aim + m.orient * spread).normalize_or_zero();
                        let side_m = if m.burst_left.is_multiple_of(2) {
                            arms::WING_L
                        } else {
                            arms::WING_R
                        };
                        new_slugs.push(FoeSlug {
                            pos: m.pos + m.orient * side_m,
                            vel: m.vel + dir * Weapon::Cannon.muzzle_mps(),
                            born_s: t_s,
                        });
                        self.foe_shots = self.foe_shots.wrapping_add(1);
                        m.next_shot_s = if m.burst_left == 0 {
                            t_s + 1.6 + self.unit() as f64 * 1.4
                        } else {
                            t_s + 0.11
                        };
                    }
                }
                Phase::Leaving => {
                    let away = -dir_us;
                    want_fwd = Some(away);
                    thrust = away * 40.0;
                }
                Phase::Wreck => {
                    m.orient = (DQuat::from_scaled_axis(m.spin * dt) * m.orient).normalize();
                }
            }
            if let Some(f) = want_fwd {
                let target = look_at(f, m.orient * DVec3::Y);
                let rate = if m.phase == Phase::Revealing {
                    0.6
                } else {
                    2.2
                };
                m.orient = m.orient.slerp(target, (rate * dt).min(1.0)).normalize();
            }
            m.effort = if m.phase == Phase::Wreck {
                0.0
            } else {
                (thrust.length() / 30.0).clamp(0.0, 1.0) as f32
            };
            m.vel += thrust * dt;
            m.pos += m.vel * dt;
            self.ships[i] = m;
        }
        self.slugs.extend(new_slugs);
        self.ships.retain(|m| match m.phase {
            Phase::Leaving => (own.pos - m.pos).length() < 8_000.0,
            Phase::Wreck => t_s - m.phase_s < WRECK_S,
            _ => (own.pos - m.pos).length() < 12_000.0,
        });

        // Their slugs fly, and some of them land on us.
        let mut k = 0;
        while k < self.slugs.len() {
            let s = self.slugs[k];
            if t_s - s.born_s > arms::SLUG_LIFE_S {
                self.slugs.swap_remove(k);
                continue;
            }
            let a = s.pos;
            let b = s.pos + s.vel * dt;
            if arms::segment_hits_sphere(a, b, own.pos, OWN_R_M).is_some() {
                self.hull = (self.hull - HIT_HULL).max(0.0);
                self.own_hits.push(OwnHit {
                    from: (a - own.pos).normalize_or_zero(),
                    size: 0.55,
                });
                self.slugs.swap_remove(k);
                continue;
            }
            self.slugs[k].pos = b;
            k += 1;
        }
    }

    /// The readout's line: what was said or what is happening, then the
    /// hull if it is hurt while anything is in the air.
    pub fn text(&self) -> Option<String> {
        if let Some((line, _)) = &self.line {
            return Some(line.clone());
        }
        if self.hull < 0.999 && !self.ships.is_empty() {
            let hostile = self.ships.iter().any(|m| m.phase == Phase::Attacking);
            return Some(format!(
                "HULL {:.0}%{}",
                self.hull * 100.0,
                if hostile { "  UNDER FIRE" } else { "" }
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rock(id: RockId) -> Rock {
        Rock {
            id,
            pos: DVec3::new(0.0, 0.0, -900.0),
            vel: DVec3::ZERO,
            radius_m: 40.0,
            seed: 0.3,
            spin: 0.0,
        }
    }

    fn own() -> arms::Ship {
        arms::Ship {
            pos: DVec3::ZERO,
            vel: DVec3::ZERO,
            orient: DQuat::IDENTITY,
            aim: DVec3::NEG_Z,
        }
    }

    fn a_mimic_id(chance: f32) -> RockId {
        (0..2000)
            .map(|i| (i, 3, -2, 1u8))
            .find(|&id| is_mimic(id, chance))
            .expect("some rock is a ship")
    }

    #[test]
    fn a_share_of_rocks_are_ships_and_fragments_never() {
        let n = (0..4000).filter(|&i| is_mimic((i, 1, 1, 0), 0.1)).count();
        assert!((250..550).contains(&n), "about a tenth: {n}");
        assert_eq!(
            (0..4000).filter(|&i| is_mimic((i, 1, 1, 0), 0.0)).count(),
            0
        );
        assert!(!(0..4000).any(|i| is_mimic((i, 1, 1, FRAG_SLOT0), 1.0)));
        assert!(
            (0..400).all(|i| is_mimic((i, 1, 1, 0), 1.0)),
            "every rock at 100%"
        );
        let id = a_mimic_id(0.1);
        assert_eq!(is_mimic(id, 0.1), is_mimic(id, 0.1), "keyed, not rolled");
        let hostile = (0..4000)
            .filter(|&i| mood_of((i, 0, 0, 0), 0.5) == Mood::Hostile)
            .count();
        assert!((1600..2400).contains(&hostile), "{hostile}");
        assert!(hail_text(0.0).len() <= 32 && hail_text(0.99).len() <= 32);
    }

    #[test]
    fn a_struck_mimic_reveals_once_then_hails_and_leaves() {
        let mut ms = Mimics {
            chance: 0.5,
            hostility: 0.0,
            ..Default::default()
        };
        let id = a_mimic_id(0.5);
        let r = rock(id);
        assert!(ms.on_rock_struck(&r, 10.0, DVec3::ZERO));
        assert!(!ms.on_rock_struck(&r, 10.0, DVec3::ZERO), "only once");
        assert_eq!(ms.reveals, 1);
        assert_eq!(ms.ships[0].phase, Phase::Revealing);
        assert!(ms.ships[0].shrouded(10.5));
        assert!(ms.shroud_off(10.5).is_empty());
        assert_eq!(ms.shroud_off(10.0 + REVEAL_S * SHROUD_OFF + 0.01), vec![id]);
        let o = own();
        let mut t = 10.0;
        while t < 10.0 + REVEAL_S + 0.5 {
            ms.step(t, 1.0 / 120.0, &o);
            t += 1.0 / 120.0;
        }
        assert_eq!(ms.ships[0].phase, Phase::Hailing);
        assert_eq!(ms.hails, 1);
        assert!(ms.text().unwrap().starts_with("HAIL:"));
        assert!(ms.ships[0].reveal(t) >= 1.0);
        // It faces us while it talks.
        let nose = ms.ships[0].orient * DVec3::NEG_Z;
        assert!(nose.dot((o.pos - ms.ships[0].pos).normalize()) > 0.95);
        while t < 10.0 + REVEAL_S + HAIL_S + 30.0 {
            ms.step(t, 1.0 / 120.0, &o);
            t += 1.0 / 120.0;
        }
        assert!(
            ms.ships.is_empty() || ms.ships[0].phase == Phase::Leaving,
            "{:?}",
            ms.ships
        );
        assert!(ms.slugs.is_empty(), "a hailer never fires");
    }

    #[test]
    fn a_hostile_mimic_comes_about_fires_and_hits_the_hull() {
        let mut ms = Mimics {
            chance: 0.5,
            hostility: 1.0,
            ..Default::default()
        };
        let id = a_mimic_id(0.5);
        let mut r = rock(id);
        r.pos = DVec3::new(0.0, 0.0, -700.0);
        assert!(ms.on_rock_struck(&r, 0.0, DVec3::ZERO));
        let o = own();
        let mut t = 0.0;
        let mut hits = 0;
        while t < 20.0 {
            ms.step(t, 1.0 / 120.0, &o);
            hits += ms.own_hits.len();
            t += 1.0 / 120.0;
        }
        assert_eq!(ms.ships[0].phase, Phase::Attacking);
        assert!(ms.foe_shots > 0, "it fired");
        assert!(hits > 0, "and some landed");
        assert!(ms.hull < 1.0);
        assert!(ms.text().unwrap().contains("HULL"));
        let range = (ms.ships[0].pos - o.pos).length();
        assert!(range < 2_000.0 && range > 100.0, "holds its range: {range}");
    }

    #[test]
    fn our_slugs_wreck_it_and_the_wreck_is_salvage() {
        let mut ms = Mimics {
            chance: 0.5,
            hostility: 1.0,
            ..Default::default()
        };
        let id = a_mimic_id(0.5);
        let mut r = rock(id);
        r.pos = DVec3::new(0.0, 0.0, -300.0);
        assert!(ms.on_rock_struck(&r, 0.0, DVec3::ZERO));
        let t = REVEAL_S + 1.0;
        let mut arms = arms::Arms::default();
        let mut haul = Haul::default();
        let mut wrecked = false;
        for _ in 0..200 {
            // A slug's `pos` is where it is after its step: this one flew
            // through the ship at -300 this step.
            arms.slugs.push(Slug {
                pos: DVec3::new(0.0, 0.0, -330.0),
                vel: DVec3::new(0.0, 0.0, -6_000.0),
                born_s: t,
                weapon: Weapon::Rail,
            });
            let breaks = ms.take_fire(&mut arms, &mut haul, t, 1.0 / 120.0);
            assert!(arms.slugs.is_empty(), "the slug landed");
            if !breaks.is_empty() {
                wrecked = true;
                break;
            }
        }
        assert!(wrecked);
        assert_eq!(ms.ships[0].phase, Phase::Wreck);
        assert_eq!(ms.ships[0].kind(), 2);
        assert_eq!(ms.wrecks, 1);
        assert!(haul.tonnes[3] > 3.0);
        assert!(haul.text(t, 3.0).unwrap().starts_with("HAUL"));
        assert!(!arms.bursts.is_empty());
        // A shrouded ship is still a rock to the guns.
        let mut ms2 = Mimics {
            chance: 0.5,
            ..Default::default()
        };
        ms2.on_rock_struck(&r, 0.0, DVec3::ZERO);
        arms.slugs.push(Slug {
            pos: DVec3::new(0.0, 0.0, -330.0),
            vel: DVec3::new(0.0, 0.0, -6_000.0),
            born_s: 0.1,
            weapon: Weapon::Rail,
        });
        ms2.take_fire(&mut arms, &mut haul, 0.1, 1.0 / 120.0);
        assert_eq!(arms.slugs.len(), 1);
    }

    #[test]
    fn the_haul_grows_with_energy_by_kind_and_a_break_leaves_a_lump() {
        let mut h = Haul::default();
        let r = rock((1, 1, 1, 0));
        h.on_hit(&r, 1.0e6, false, 1.0);
        let chipped = h.total_t();
        assert!(chipped > 0.02 && chipped < 1.0, "{chipped}");
        assert_eq!(Ore::of_seed(0.3), Ore::Ice);
        assert!(h.tonnes[0] > 0.0 && h.tonnes[1] == 0.0);
        h.on_hit(&r, 1.0e6, true, 2.0);
        assert!(h.total_t() > chipped * 5.0, "the lump: {}", h.total_t());
        assert!(h.text(2.5, 3.0).unwrap().contains("ICE"));
        assert!(h.text(9.0, 3.0).is_none(), "the line goes");
        h.yield_ = 0.0;
        let before = h.total_t();
        h.on_hit(&r, 1.0e6, true, 3.0);
        assert_eq!(h.total_t(), before, "ORE YIELD off");
    }
}
