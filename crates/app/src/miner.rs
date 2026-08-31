//! Miners: the ring is worked. A handful of ships of our own class are
//! already here when we arrive, each picking a rock, flying to it and
//! cutting ore off it with a beam — and what they haul makes them grow:
//! four tiers of size, hull, shield and guns as the haul crosses its
//! thresholds. Neutral until shot at: a small one runs, a grown one comes
//! about and fights harder than a mimic. A wreck gives up its whole haul.
//!
//! Like the mimics none of this touches `farfall-sim`: the population is
//! placed from hashes when the belt goes live, stepped after the belt
//! each fixed step, and it hands the ship its slugs through the mimics'
//! list so the shield, the tracers and the readout need nothing new.

use glam::{DQuat, DVec3};

use crate::arms::{self, Burst, Slug, Weapon};
use crate::belt::{self, Belt, Rock, RockId};
use crate::mimic::{self, look_at, FoeSlug, Haul, Mimics, Ore, HULL_R_M};

/// The most miners in the ring at once (the pass has this many lanes over
/// the mimics' four).
pub const MAX_MINERS: usize = 8;
/// The tiers: the haul (tonnes) that reaches each, the hull's size (metres
/// per SDF unit — 1 is our own fighter), the toughness (J), the share of
/// a hit the shield sheds, and the guns' burst length.
pub const TIERS: usize = 4;
pub const TIER_T: [f64; TIERS] = [0.0, 40.0, 160.0, 480.0];
pub const TIER_SIZE: [f64; TIERS] = [1.0, 1.6, 2.4, 3.4];
pub const TIER_TOUGH_J: [f64; TIERS] = [2.4e6, 6.0e6, 14.0e6, 32.0e6];
pub const TIER_SHIELD: [f64; TIERS] = [0.0, 0.0, 0.40, 0.65];
pub const TIER_BURST: [u32; TIERS] = [3, 5, 7, 9];
/// What a tier-0 miner cuts a second at stock growth, tonnes.
pub const MINE_T_PER_S: f64 = 0.35;
/// The share of a rock's mass a miner takes before it is spent, and the
/// bounds on that, tonnes.
pub const ROCK_SHARE: f64 = 0.001;
pub const SHARE_MIN_T: f64 = 2.0;
pub const SHARE_MAX_T: f64 = 80.0;
/// The beam's stand-off from the rock's surface, metres per unit of size.
pub const STANDOFF_M: f64 = 70.0;
/// A miner looks this far for a rock, and takes none smaller than this.
pub const SEEK_M: f64 = 4_000.0;
pub const MIN_ROCK_M: f64 = 8.0;
/// Where the population is placed about us, metres.
pub const PLACE_MIN_M: f64 = 900.0;
pub const PLACE_MAX_M: f64 = 2_600.0;
/// Past this range a miner is let go.
pub const DROP_M: f64 = 12_000.0;
/// A neutral miner hails us inside this range, once.
pub const HAIL_M: f64 = 300.0;
/// With no rock to be had, a miner looks again after this long.
pub const RETRY_S: f64 = 2.0;
/// A hostile miner holds this range.
pub const HOLD_M: f64 = 320.0;

/// Where a miner is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Looking for a rock.
    Seeking,
    /// Flying to its claim.
    Transit,
    /// The beam on the claim.
    Mining,
    /// Come about, guns on us.
    Attacking,
    /// Running: full burn away, cleared past 8 km.
    Leaving,
    /// Dead: dark, tumbling, the haul taken.
    Wreck,
}

/// What a miner thinks of us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Temper {
    Neutral,
    Hostile,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Miner {
    /// Its identity (a hash of where it was placed).
    pub id: u32,
    pub pos: DVec3,
    pub vel: DVec3,
    pub orient: DQuat,
    /// Tumble for a wreck, rad/s about a fixed axis.
    pub spin: DVec3,
    pub phase: Phase,
    pub phase_s: f64,
    pub temper: Temper,
    /// What it has hauled, tonnes: ice, iron, silicate. Only ever grows.
    pub haul: [f64; 3],
    /// The rock it is working, and what it has had off this one.
    pub claim: Option<RockId>,
    pub taken_t: f64,
    /// Damage taken, joules.
    pub wound_j: f64,
    /// Engine effort 0..1, for the look.
    pub effort: f32,
    pub seed: f32,
    /// Said its piece to us already.
    pub hailed: bool,
    /// The shield's sheen 0..1, lit by a hit it shed, fading.
    pub sheen: f32,
    retry_s: f64,
    next_shot_s: f64,
    burst_left: u32,
}

impl Miner {
    /// A miner put in the ring by hand (the bench, the tests): at `pos`
    /// with `haul_t` tonnes aboard (its tier follows), in `phase`.
    pub fn planted(
        pos: DVec3,
        vel: DVec3,
        orient: DQuat,
        haul_t: f64,
        phase: Phase,
        temper: Temper,
        seed: f32,
    ) -> Self {
        Self {
            id: 0xB0A7,
            pos,
            vel,
            orient,
            spin: DVec3::new(0.2, 0.7, 0.3),
            phase,
            phase_s: 0.0,
            temper,
            haul: [haul_t * 0.5, haul_t * 0.3, haul_t * 0.2],
            claim: None,
            taken_t: 0.0,
            wound_j: 0.0,
            effort: 0.0,
            seed,
            hailed: false,
            sheen: 0.0,
            retry_s: 0.0,
            next_shot_s: 0.0,
            burst_left: 0,
        }
    }

    pub fn total_t(&self) -> f64 {
        self.haul.iter().sum()
    }

    /// The tier its haul has reached, 0..3. The haul only grows, so a
    /// miner never comes down a tier.
    pub fn tier(&self) -> usize {
        let t = self.total_t();
        TIER_T.iter().rposition(|&th| t >= th).unwrap_or(0)
    }
    pub fn size(&self) -> f64 {
        TIER_SIZE[self.tier()]
    }
    pub fn tough_j(&self) -> f64 {
        TIER_TOUGH_J[self.tier()]
    }
    /// The hull's hit sphere, metres.
    pub fn hull_r_m(&self) -> f64 {
        HULL_R_M * self.size()
    }
    /// How hurt, 0..1.
    pub fn wound(&self) -> f32 {
        (self.wound_j / self.tough_j()).clamp(0.0, 1.0) as f32
    }
    /// The look's kind lane: 3 a miner at work, 4 one turned on us, 2 a
    /// wreck (the mimics' code for one).
    pub fn kind(&self) -> u8 {
        match (self.phase, self.temper) {
            (Phase::Wreck, _) => 2,
            (_, Temper::Hostile) => 4,
            (_, Temper::Neutral) => 3,
        }
    }
    /// Whether the beam is on.
    pub fn mining(&self) -> bool {
        self.phase == Phase::Mining && self.claim.is_some()
    }
    /// The readout's word for what it has aboard.
    pub fn haul_text(&self) -> String {
        let t = self.total_t();
        if t >= 100.0 {
            format!("{t:.0} T")
        } else {
            format!("{t:.1} T")
        }
    }
}

/// The rock a miner at `pos` would take: the nearest live rock big enough
/// within reach that is not shrouded (a mimic), not another miner's
/// claim, and never the one the pilot is holding station on.
pub fn choose_target(
    pos: DVec3,
    belt: &Belt,
    claims: &[RockId],
    held: Option<RockId>,
    mimic_chance: f32,
) -> Option<RockId> {
    let mut best: Option<(f64, RockId)> = None;
    for r in belt.rocks.iter() {
        if r.radius_m < MIN_ROCK_M {
            continue;
        }
        if held == Some(r.id) || claims.contains(&r.id) {
            continue;
        }
        if mimic::is_mimic(r.id, mimic_chance) {
            continue;
        }
        let d = (r.pos - pos).length();
        if d > SEEK_M {
            continue;
        }
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, r.id));
        }
    }
    best.map(|(_, id)| id)
}

/// What a rock gives a miner before it is spent, tonnes.
pub fn rock_share_t(rock: &Rock) -> f64 {
    let mass_t = 2_000.0 * (4.0 / 3.0) * std::f64::consts::PI * rock.radius_m.powi(3) / 1_000.0;
    (mass_t * ROCK_SHARE).clamp(SHARE_MIN_T, SHARE_MAX_T)
}

/// What a miner says when we come close, by its seed and its tier.
pub fn hail_text(seed: f32, tier: usize) -> &'static str {
    // The readout is 32 columns wide.
    const SMALL: [&str; 4] = [
        "MINER: CLAIM STAKED. KEEP OFF",
        "MINER: ICE RUNS DEEP HERE. LUCK",
        "MINER: WE PAID FOR THIS ROCK",
        "MINER: NO TROUBLE. JUST ORE",
    ];
    const BIG: [&str; 2] = [
        "MINER: HOLD IS FULL. GOING HOME",
        "MINER: BIG RIG. STAY CLEAR",
    ];
    if tier >= 2 {
        BIG[((seed.clamp(0.0, 0.999) * BIG.len() as f32) as usize).min(BIG.len() - 1)]
    } else {
        SMALL[((seed.clamp(0.0, 0.999) * SMALL.len() as f32) as usize).min(SMALL.len() - 1)]
    }
}

/// The miners in the ring and the settings they answer to.
#[derive(Debug, Clone, PartialEq)]
pub struct Miners {
    pub ships: Vec<Miner>,
    /// MINERS: how many are placed, 0..8.
    pub count: u32,
    /// MINER GROWTH: the haul rate's multiplier.
    pub growth: f32,
    /// The population has been placed since the ring was last empty.
    pub placed: bool,
    pub wrecks: u32,
    seq: u32,
}

impl Default for Miners {
    fn default() -> Self {
        Self {
            ships: Vec::new(),
            count: 4,
            growth: 1.0,
            placed: false,
            wrecks: 0,
            seq: 11,
        }
    }
}

impl Miners {
    fn unit(&mut self) -> f32 {
        self.seq = self.seq.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seq >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Place the population: once the belt has rocks about us and none
    /// has been placed since it was last empty, `count` miners go in at
    /// hashed bearings and ranges, riding the ring, a few already grown.
    /// The ring left (no rocks, no ships), the next visit places anew.
    pub fn populate(&mut self, own: &arms::Ship, belt: &Belt) {
        if belt.rocks.is_empty() {
            if self.ships.is_empty() {
                self.placed = false;
            }
            return;
        }
        if self.placed {
            return;
        }
        self.placed = true;
        let Some(near) = belt
            .rocks
            .iter()
            .min_by(|a, b| {
                let da = (a.pos - own.pos).length_squared();
                let db = (b.pos - own.pos).length_squared();
                da.partial_cmp(&db).unwrap()
            })
            .copied()
        else {
            return;
        };
        let cell = near.id;
        for k in 0..(self.count as usize).min(MAX_MINERS) {
            let h = |salt: u32| {
                belt::unit(belt::hash(
                    cell.0,
                    cell.1,
                    cell.2,
                    salt ^ (0x31_1E00 + k as u32 * 131),
                ))
            };
            let dir = DVec3::new(h(1) - 0.5, (h(2) - 0.5) * 0.4, h(3) - 0.5).normalize_or_zero();
            let dir = if dir == DVec3::ZERO { DVec3::X } else { dir };
            let range = PLACE_MIN_M + h(4) * (PLACE_MAX_M - PLACE_MIN_M);
            let pos = own.pos + dir * range;
            let drift = DVec3::new(h(5) - 0.5, h(6) - 0.5, h(7) - 0.5) * 4.0;
            // Most have little aboard; some were here long before us.
            let haul_t = h(8).powi(3) * 220.0;
            let seed = h(9) as f32;
            let id = belt::hash(cell.0, cell.1, cell.2, 0x3113 ^ k as u32);
            let mut m = Miner::planted(
                pos,
                near.vel + drift,
                look_at(-dir, DVec3::Y),
                haul_t,
                Phase::Seeking,
                Temper::Neutral,
                seed,
            );
            m.id = id;
            m.haul = match Ore::of_seed(seed) {
                Ore::Ice => [haul_t * 0.7, haul_t * 0.1, haul_t * 0.2],
                Ore::Iron => [haul_t * 0.2, haul_t * 0.6, haul_t * 0.2],
                Ore::Silicate => [haul_t * 0.2, haul_t * 0.2, haul_t * 0.6],
            };
            self.ships.push(m);
        }
    }

    /// Every rock a miner is working.
    pub fn claims(&self) -> Vec<RockId> {
        self.ships.iter().filter_map(|m| m.claim).collect()
    }

    /// One fixed step for every miner. `own`: our ship; `held`: the rock
    /// the pilot is holding station on, which no miner touches; the
    /// miners' slugs, hails and lines go out through `out` (the mimics')
    /// so the shield, the tracers and the readout need nothing new.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        t_s: f64,
        dt: f64,
        own: &arms::Ship,
        belt: &mut Belt,
        held: Option<RockId>,
        mimic_chance: f32,
        out: &mut Mimics,
    ) {
        let growth = self.growth.clamp(0.25, 4.0) as f64;
        for i in 0..self.ships.len() {
            let mut m = self.ships[i];
            let size = m.size();
            let tier = m.tier();
            let to_us = own.pos - m.pos;
            let range = to_us.length();
            let dir_us = to_us / range.max(1.0);
            let rel_v = own.vel - m.vel;
            let mut thrust = DVec3::ZERO;
            let mut want_fwd: Option<DVec3> = None;
            m.sheen = (m.sheen - dt as f32 * 1.6).max(0.0);
            // A word for us as we come close, once.
            if m.temper == Temper::Neutral
                && !m.hailed
                && range < HAIL_M
                && matches!(m.phase, Phase::Seeking | Phase::Transit | Phase::Mining)
            {
                m.hailed = true;
                out.hails = out.hails.wrapping_add(1);
                out.line = Some((hail_text(m.seed, tier).to_string(), t_s + mimic::HAIL_S));
            }
            // The claim: still in the belt, and not the pilot's.
            let claim_rock = m
                .claim
                .and_then(|id| belt.rocks.iter().position(|r| r.id == id))
                .map(|k| (k, belt.rocks[k]));
            if let Some(id) = m.claim {
                let gone = claim_rock.is_none();
                if held == Some(id) || gone {
                    if held == Some(id) && m.phase == Phase::Mining {
                        out.line = Some(("MINER: YOUR ROCK. WE STAND OFF".to_string(), t_s + 3.0));
                    }
                    m.claim = None;
                    m.taken_t = 0.0;
                    if matches!(m.phase, Phase::Transit | Phase::Mining) {
                        m.phase = Phase::Seeking;
                        m.phase_s = t_s;
                        m.retry_s = t_s + if gone { 0.0 } else { RETRY_S };
                    }
                }
            }
            let accel_max = 14.0 / size.sqrt();
            // Station off a rock: the stand-off point on the line from the
            // rock to the ship, the rock's velocity matched.
            let station = |m: &Miner, rock: &Rock| -> (DVec3, DVec3) {
                let away = (m.pos - rock.pos).normalize_or_zero();
                let away = if away == DVec3::ZERO { DVec3::Y } else { away };
                let want_pos = rock.pos + away * (rock.radius_m + STANDOFF_M * size);
                let want_vel = rock.vel + ((want_pos - m.pos) * 0.15).clamp_length_max(45.0);
                let accel = ((want_vel - m.vel) * 0.9).clamp_length_max(accel_max);
                (accel, want_pos)
            };
            match m.phase {
                Phase::Seeking => {
                    if m.claim.is_none() && t_s >= m.retry_s {
                        let claims = self.claims();
                        match choose_target(m.pos, belt, &claims, held, mimic_chance) {
                            Some(id) => {
                                m.claim = Some(id);
                                m.taken_t = 0.0;
                                m.phase = Phase::Transit;
                                m.phase_s = t_s;
                            }
                            None => m.retry_s = t_s + RETRY_S,
                        }
                    }
                }
                Phase::Transit => {
                    if let Some((_, rock)) = claim_rock {
                        let (accel, want_pos) = station(&m, &rock);
                        thrust = accel;
                        want_fwd = Some((rock.pos - m.pos).normalize_or_zero());
                        let off = (want_pos - m.pos).length();
                        let rel = (m.vel - rock.vel).length();
                        if off < 6.0 * size && rel < 2.5 {
                            m.phase = Phase::Mining;
                            m.phase_s = t_s;
                        }
                    }
                }
                Phase::Mining => {
                    if let Some((k, rock)) = claim_rock {
                        let (accel, _) = station(&m, &rock);
                        thrust = accel;
                        want_fwd = Some((rock.pos - m.pos).normalize_or_zero());
                        let rate = MINE_T_PER_S * (1.0 + 0.5 * tier as f64) * growth;
                        let take = rate * dt;
                        let ore = Ore::of_seed(rock.seed);
                        let slot = match ore {
                            Ore::Ice => 0,
                            Ore::Iron => 1,
                            Ore::Silicate => 2,
                        };
                        m.haul[slot] += take;
                        m.taken_t += take;
                        // The cut is real: each step wounds the rock through
                        // the belt's own toughness — it cracks as it is worked
                        // (the guns' wounds add to the same tally) and breaks,
                        // into fragments, as the last of its share goes.
                        let dir = (rock.pos - m.pos).normalize_or_zero();
                        let at = rock.pos - dir * rock.radius_m;
                        let share = rock_share_t(&rock);
                        let cut_j = Belt::toughness_j(rock.radius_m) * (take / share) * 1.001;
                        let spent = belt.strike(k, cut_j, 0.0, at, dir).destroyed;
                        if spent || m.taken_t >= share {
                            m.claim = None;
                            m.taken_t = 0.0;
                            m.phase = Phase::Seeking;
                            m.phase_s = t_s;
                            m.retry_s = t_s;
                        }
                    }
                }
                Phase::Attacking => {
                    // The mimic's fight, harder with the tier: the guns on
                    // us, the range held, a weave; longer bursts, and the
                    // rail from the biggest.
                    let side = dir_us.cross(DVec3::Y).normalize_or_zero();
                    let weave = (t_s * 0.5 + m.seed as f64 * 6.0).sin();
                    let hold = (range - HOLD_M) * 0.05;
                    thrust = dir_us * hold.clamp(-accel_max, accel_max)
                        + rel_v * 0.35
                        + side * weave * (4.0 / size);
                    let rail = tier >= 3 && m.burst_left == 0 && m.seed > 0.5;
                    let speed = if rail {
                        Weapon::Rail.muzzle_mps()
                    } else {
                        Weapon::Cannon.muzzle_mps()
                    };
                    let flight = range / speed;
                    let aim = (own.pos + rel_v * flight - m.pos).normalize_or_zero();
                    want_fwd = Some(aim);
                    let nose = m.orient * DVec3::NEG_Z;
                    let on = nose.dot(aim) > 0.9985;
                    if range < mimic::FIRE_M && on && t_s >= m.next_shot_s {
                        if m.burst_left == 0 {
                            m.burst_left = TIER_BURST[tier] + (self.unit() * 3.0) as u32;
                        }
                        m.burst_left -= 1;
                        let spread = DVec3::new(
                            (self.unit() as f64 - 0.5) * 0.016,
                            (self.unit() as f64 - 0.5) * 0.016,
                            0.0,
                        );
                        let dir = (aim + m.orient * spread).normalize_or_zero();
                        let side_m = if m.burst_left.is_multiple_of(2) {
                            arms::WING_L
                        } else {
                            arms::WING_R
                        };
                        out.slugs.push(FoeSlug {
                            pos: m.pos + m.orient * (side_m * size),
                            vel: m.vel + dir * speed,
                            born_s: t_s,
                        });
                        out.foe_shots = out.foe_shots.wrapping_add(1);
                        m.next_shot_s = if m.burst_left == 0 {
                            t_s + 1.4 + self.unit() as f64 * 1.2
                        } else {
                            t_s + 0.11
                        };
                    }
                }
                Phase::Leaving => {
                    let away = -dir_us;
                    want_fwd = Some(away);
                    thrust = away * (40.0 / size.sqrt());
                }
                Phase::Wreck => {
                    m.orient = (DQuat::from_scaled_axis(m.spin * dt) * m.orient).normalize();
                }
            }
            if let Some(f) = want_fwd {
                let target = look_at(f, m.orient * DVec3::Y);
                let rate = 1.8 / size.sqrt();
                m.orient = m.orient.slerp(target, (rate * dt).min(1.0)).normalize();
            }
            m.effort = if m.phase == Phase::Wreck {
                0.0
            } else {
                (thrust.length() / accel_max).clamp(0.0, 1.0) as f32
            };
            m.vel += thrust * dt;
            m.pos += m.vel * dt;
            self.ships[i] = m;
        }
        self.ships.retain(|m| match m.phase {
            Phase::Leaving => (own.pos - m.pos).length() < 8_000.0,
            Phase::Wreck => t_s - m.phase_s < mimic::WRECK_S,
            _ => (own.pos - m.pos).length() < DROP_M,
        });
    }

    /// Our slugs meet the miners: the ones that land are taken out of the
    /// list, wound the ship (less what its shield sheds), and leave a
    /// burst. A neutral miner shot at runs if it is small and fights if
    /// it has grown. Past its toughness it is a wreck and its whole haul
    /// is ours. Slugs are tested over the step they just flew (`dt`).
    pub fn take_fire(
        &mut self,
        arms: &mut arms::Arms,
        haul: &mut Haul,
        out: &mut Mimics,
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
                if let Some(f) = arms::segment_hits_sphere(a, b, m.pos, m.hull_r_m()) {
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
            let tier = m.tier();
            let ship = &mut self.ships[mi];
            let mass = 14_000.0 * ship.size().powi(3);
            ship.vel += rel.normalize_or_zero() * (momentum / mass);
            let was_wreck = ship.phase == Phase::Wreck;
            let shed = TIER_SHIELD[tier];
            if shed > 0.0 && !was_wreck {
                ship.sheen = 1.0;
            }
            ship.wound_j += energy.max(0.0) * (1.0 - shed);
            arms.bangs = arms.bangs.wrapping_add(1);
            arms.bang_size = 0.6 + 0.2 * tier as f32;
            let mut kind = if s.weapon == Weapon::Rail { 3 } else { 1 };
            if !was_wreck && ship.wound_j >= ship.tough_j() {
                ship.phase = Phase::Wreck;
                ship.phase_s = t_s;
                ship.effort = 0.0;
                ship.claim = None;
                ship.spin = DVec3::new(
                    (seed as f64 - 0.5) * 1.0,
                    0.3 + seed as f64 * 0.8,
                    (seed2 as f64 - 0.5) * 0.6,
                ) / ship.size().sqrt();
                self.wrecks = self.wrecks.wrapping_add(1);
                arms.breaks = arms.breaks.wrapping_add(1);
                // The haul is ours, and the salvage.
                let y = haul.yield_ as f64;
                for (slot, t) in ship.haul.iter().enumerate() {
                    haul.tonnes[slot] += t * y;
                }
                haul.on_salvage(t_s);
                out.line = Some((format!("WRECK: {} HAUL TAKEN", ship.haul_text()), t_s + 4.0));
                kind = 2;
                breaks.push((at, ship.vel, 1.0));
            } else if !was_wreck && ship.temper == Temper::Neutral && ship.phase != Phase::Leaving {
                ship.claim = None;
                ship.phase_s = t_s;
                if tier == 0 {
                    ship.phase = Phase::Leaving;
                    out.line = Some(("MINER: WE ARE GONE".to_string(), t_s + 3.0));
                } else {
                    ship.temper = Temper::Hostile;
                    ship.phase = Phase::Attacking;
                    out.line = Some(("MINER: WRONG ROCK, PILOT".to_string(), t_s + 3.0));
                }
            } else if !was_wreck
                && ship.temper == Temper::Hostile
                && ship.phase == Phase::Attacking
                && tier < 2
                && ship.wound() > 0.7
            {
                // A small one hurt enough runs.
                ship.phase = Phase::Leaving;
                ship.phase_s = t_s;
                out.line = Some(("THE MINER BREAKS OFF".to_string(), t_s + 3.0));
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rock(id: RockId, pos: DVec3, radius_m: f64) -> Rock {
        Rock {
            id,
            pos,
            vel: DVec3::ZERO,
            radius_m,
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

    fn a_belt(rocks: &[Rock]) -> Belt {
        let mut b = Belt::default();
        b.rocks.extend_from_slice(rocks);
        b
    }

    fn a_miner_at(pos: DVec3, haul_t: f64) -> Miner {
        Miner::planted(
            pos,
            DVec3::ZERO,
            DQuat::IDENTITY,
            haul_t,
            Phase::Seeking,
            Temper::Neutral,
            0.3,
        )
    }

    fn run(
        ms: &mut Miners,
        belt: &mut Belt,
        out: &mut Mimics,
        from: f64,
        secs: f64,
        held: Option<RockId>,
    ) -> f64 {
        let o = own();
        let mut t = from;
        while t < from + secs {
            ms.step(t, 1.0 / 120.0, &o, belt, held, 0.0, out);
            t += 1.0 / 120.0;
        }
        t
    }

    #[test]
    fn a_miner_grows_through_its_tiers_as_its_haul_crosses_the_thresholds() {
        let mut m = a_miner_at(DVec3::ZERO, 0.0);
        assert_eq!(m.tier(), 0);
        assert_eq!(m.size(), 1.0, "a tier-0 miner is exactly our fighter");
        m.haul = [39.9, 0.0, 0.0];
        assert_eq!(m.tier(), 0);
        m.haul = [40.0, 0.0, 0.0];
        assert_eq!(m.tier(), 1);
        m.haul = [100.0, 60.0, 0.0];
        assert_eq!(m.tier(), 2);
        assert!(m.size() > 2.0 && m.hull_r_m() > HULL_R_M * 2.0);
        m.haul = [200.0, 200.0, 100.0];
        assert_eq!(m.tier(), 3);
        assert!(
            m.tough_j() > TIER_TOUGH_J[0] * 10.0,
            "and it is much harder to kill"
        );
        assert!(TIER_SIZE.windows(2).all(|w| w[1] > w[0]));
        assert!(TIER_T.windows(2).all(|w| w[1] > w[0]));
        assert!(hail_text(0.0, 0).len() <= 32 && hail_text(0.99, 3).len() <= 32);
        assert_eq!(m.kind(), 3);
    }

    #[test]
    fn a_miner_seeks_a_rock_flies_to_it_mines_it_with_the_beam_and_seeks_again_when_it_is_spent() {
        let id = (5, 5, 5, 0);
        let mut belt = a_belt(&[rock(id, DVec3::new(0.0, 0.0, -600.0), 12.0)]);
        let mut ms = Miners {
            growth: 4.0,
            ..Default::default()
        };
        ms.ships.push(a_miner_at(DVec3::new(0.0, 0.0, -300.0), 0.0));
        let mut out = Mimics::default();
        let t = run(&mut ms, &mut belt, &mut out, 0.0, 0.1, None);
        assert_eq!(ms.ships[0].phase, Phase::Transit);
        assert_eq!(ms.ships[0].claim, Some(id));
        assert_eq!(ms.claims(), vec![id]);
        // It gets there, holds off the rock, and the beam goes on.
        let mut t = t;
        while ms.ships[0].phase != Phase::Mining && t < 120.0 {
            t = run(&mut ms, &mut belt, &mut out, t, 0.5, None);
        }
        let m = ms.ships[0];
        assert_eq!(m.phase, Phase::Mining, "{m:?}");
        assert!(m.mining());
        let off = (m.pos - belt.rocks[0].pos).length();
        assert!(
            (off - (12.0 + STANDOFF_M)).abs() < 12.0,
            "stands off the rock: {off} m"
        );
        let nose = m.orient * DVec3::NEG_Z;
        assert!(
            nose.dot((belt.rocks[0].pos - m.pos).normalize()) > 0.98,
            "nose on it"
        );
        assert!(m.total_t() > 0.0, "and it is hauling");
        let share = rock_share_t(&belt.rocks[0]);
        // A 12 m rock gives its share (ice, by the seed) and then breaks.
        while belt.rocks.iter().any(|r| r.id == id) && t < 300.0 {
            t = run(&mut ms, &mut belt, &mut out, t, 0.5, None);
        }
        let m = ms.ships[0];
        assert!(m.haul[0] >= share * 0.99, "{:?}", m.haul);
        assert!(
            belt.rocks.iter().all(|r| r.id != id) && belt.dead.contains(&id),
            "the rock is spent"
        );
        assert!(
            matches!(m.phase, Phase::Seeking | Phase::Transit),
            "{:?}",
            m.phase
        );
    }

    #[test]
    fn a_miner_picks_the_nearest_rock_worth_mining_and_not_a_claim_or_a_shroud() {
        let near_small = rock((1, 1, 1, 0), DVec3::new(0.0, 0.0, -100.0), 4.0);
        let claimed = rock((2, 2, 2, 0), DVec3::new(0.0, 0.0, -200.0), 20.0);
        let good = rock((3, 3, 3, 0), DVec3::new(0.0, 0.0, -400.0), 20.0);
        let far = rock((4, 4, 4, 0), DVec3::new(0.0, 0.0, -9_000.0), 200.0);
        let belt = a_belt(&[near_small, claimed, good, far]);
        assert_eq!(
            choose_target(DVec3::ZERO, &belt, &[claimed.id], None, 0.0),
            Some(good.id)
        );
        // Every rock a ship in a shroud: nothing to mine.
        assert_eq!(choose_target(DVec3::ZERO, &belt, &[], None, 1.0), None);
        // Nothing claimed: the nearer big one.
        assert_eq!(
            choose_target(DVec3::ZERO, &belt, &[], None, 0.0),
            Some(claimed.id)
        );
    }

    #[test]
    fn a_miner_never_mines_the_rock_the_pilot_is_holding_station_on() {
        let held = (7, 7, 7, 0);
        let other = (8, 8, 8, 0);
        let mut belt = a_belt(&[
            rock(held, DVec3::new(0.0, 0.0, -300.0), 20.0),
            rock(other, DVec3::new(0.0, 0.0, -900.0), 20.0),
        ]);
        // Choosing: the held rock is passed over for the farther one.
        assert_eq!(
            choose_target(DVec3::ZERO, &belt, &[], Some(held), 0.0),
            Some(other)
        );
        // Mid-mining: the pilot takes HOLD on its rock and it backs off.
        let mut ms = Miners::default();
        ms.ships.push(a_miner_at(DVec3::new(0.0, 0.0, -200.0), 0.0));
        let mut out = Mimics::default();
        let t = run(&mut ms, &mut belt, &mut out, 0.0, 40.0, None);
        assert_eq!(ms.ships[0].claim, Some(held), "it took the near rock first");
        assert_eq!(ms.ships[0].phase, Phase::Mining);
        let t = run(&mut ms, &mut belt, &mut out, t, 0.1, Some(held));
        assert_ne!(ms.ships[0].claim, Some(held));
        assert!(!ms.ships[0].mining());
        assert!(out.text().unwrap().contains("STAND OFF"));
        // And it never comes back to it while the hold is on.
        let _ = run(&mut ms, &mut belt, &mut out, t, 60.0, Some(held));
        assert_eq!(ms.ships[0].claim, Some(other), "{:?}", ms.ships[0]);
    }

    #[test]
    fn a_shot_small_miner_runs_and_a_grown_one_fights_back_harder_until_it_is_a_wreck_that_drops_its_haul(
    ) {
        let mut arms = arms::Arms::default();
        let mut haul = Haul::default();
        let mut out = Mimics::default();
        // A cannon slug that flew through the miner at -300 this step.
        let slug = |t: f64| Slug {
            pos: DVec3::new(0.0, 0.0, -305.0),
            vel: DVec3::new(0.0, 0.0, -1_400.0),
            born_s: t,
            weapon: Weapon::Cannon,
        };
        // Tier 0: one cannon slug and it runs (a rail slug would wreck it:
        // 252 MJ against 2.4).
        let mut ms = Miners::default();
        ms.ships.push(a_miner_at(DVec3::new(0.0, 0.0, -300.0), 0.0));
        arms.slugs.push(slug(1.0));
        ms.take_fire(&mut arms, &mut haul, &mut out, 1.0, 1.0 / 120.0);
        assert!(arms.slugs.is_empty(), "it landed");
        assert_eq!(ms.ships[0].phase, Phase::Leaving);
        assert_eq!(ms.ships[0].temper, Temper::Neutral);
        // Tier 2: it turns, its shield sheds part of the hit, and it
        // shoots back with longer bursts than a mimic.
        let mut ms = Miners::default();
        ms.ships
            .push(a_miner_at(DVec3::new(0.0, 0.0, -300.0), 200.0));
        assert_eq!(ms.ships[0].tier(), 2);
        arms.slugs.push(slug(1.0));
        ms.take_fire(&mut arms, &mut haul, &mut out, 1.0, 1.0 / 120.0);
        let m = ms.ships[0];
        assert_eq!(m.phase, Phase::Attacking);
        assert_eq!(m.temper, Temper::Hostile);
        assert_eq!(m.kind(), 4);
        assert!(m.sheen > 0.9, "the shield lit");
        let full = 0.5 * Weapon::Cannon.slug_kg() * 1_400.0f64.powi(2);
        assert!(
            m.wound_j < full * 0.7 && m.wound_j > full * 0.4,
            "shed: {}",
            m.wound_j
        );
        let mut belt = Belt::default();
        let o = own();
        let mut t = 1.0;
        let mut hits = 0;
        while t < 25.0 {
            ms.step(t, 1.0 / 120.0, &o, &mut belt, None, 0.0, &mut out);
            out.step(t, 1.0 / 120.0, &o);
            hits += out.own_hits.len();
            t += 1.0 / 120.0;
        }
        assert!(out.foe_shots > 0, "it fired");
        assert!(hits > 0, "and some landed");
        assert!(
            out.foe_shots >= TIER_BURST[2],
            "a long burst: {}",
            out.foe_shots
        );
        // Slugs until it is a wreck: the haul (200 t) is ours.
        let before = haul.total_t();
        let mut wrecked = false;
        for _ in 0..400 {
            // A cannon slug that flew through the miner this step: it ends
            // 5 m past its centre, having started short of the hull.
            let m = ms.ships[0];
            arms.slugs.push(Slug {
                pos: m.pos + DVec3::new(0.0, 0.0, -5.0),
                vel: DVec3::new(0.0, 0.0, -1_400.0),
                born_s: t,
                weapon: Weapon::Cannon,
            });
            if !ms
                .take_fire(&mut arms, &mut haul, &mut out, t, 1.0 / 120.0)
                .is_empty()
            {
                wrecked = true;
                break;
            }
        }
        assert!(wrecked);
        assert_eq!(ms.ships[0].phase, Phase::Wreck);
        assert_eq!(ms.ships[0].kind(), 2);
        assert!(haul.total_t() - before > 200.0, "{}", haul.total_t());
        assert!(out.text().unwrap().starts_with("WRECK: 200 T"));
    }

    #[test]
    fn the_population_is_placed_once_in_the_ring_and_again_after_leaving_it() {
        let mut ms = Miners {
            count: 5,
            ..Default::default()
        };
        let o = own();
        let empty = Belt::default();
        ms.populate(&o, &empty);
        assert!(ms.ships.is_empty(), "nothing without rocks");
        let belt = a_belt(&[rock((3, 4, 5, 1), DVec3::new(0.0, 0.0, -500.0), 30.0)]);
        ms.populate(&o, &belt);
        assert_eq!(ms.ships.len(), 5);
        assert!(ms.ships.iter().all(|m| {
            let r = (m.pos - o.pos).length();
            (PLACE_MIN_M..=PLACE_MAX_M).contains(&r)
        }));
        let ids: std::collections::HashSet<u32> = ms.ships.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 5, "each its own");
        let same = {
            let mut m2 = Miners {
                count: 5,
                ..Default::default()
            };
            m2.populate(&o, &belt);
            m2.ships.iter().map(|m| m.pos).collect::<Vec<_>>()
        };
        assert_eq!(
            same,
            ms.ships.iter().map(|m| m.pos).collect::<Vec<_>>(),
            "placed by hash, not by the dice"
        );
        ms.populate(&o, &belt);
        assert_eq!(ms.ships.len(), 5, "once");
        // A hail as we come close, once.
        let mut out = Mimics::default();
        let mut belt2 = a_belt(&[rock((3, 4, 5, 1), DVec3::new(0.0, 0.0, -500.0), 30.0)]);
        let mut m = ms.ships[0];
        m.pos = DVec3::new(0.0, 0.0, -200.0);
        ms.ships = vec![m];
        ms.step(0.0, 1.0 / 120.0, &o, &mut belt2, None, 0.0, &mut out);
        assert_eq!(out.hails, 1);
        assert!(out.text().unwrap().starts_with("MINER:"));
        ms.step(0.1, 1.0 / 120.0, &o, &mut belt2, None, 0.0, &mut out);
        assert_eq!(out.hails, 1);
        // The ring left: the population goes, and comes back placed anew.
        ms.ships.clear();
        ms.populate(&o, &empty);
        assert!(!ms.placed);
        ms.populate(&o, &belt);
        assert_eq!(ms.ships.len(), 5);
    }
}
