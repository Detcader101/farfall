//! HOLD — the smart lock on the flight controls. Engaged on the thing
//! under the gun sight (a rock, a ship out of a rock; whatever else can be
//! flown against later), the flight computer takes the throttle: it
//! matches the target's velocity and keeps the relative position it was
//! engaged at, so the ship hangs off a tumbling stone as if hovering, and
//! the guns can work. The stick still speaks — thrust inputs move the hold
//! point (a commanded relative velocity), and the pilot's torque adds over
//! the computer's — and with HOLD FACING the nose is kept on the target.
//!
//! It drives nothing but the ship's own thrust and torque demands, through
//! `sim::Controls`, capped like the pilot's are: the sim is untouched.

use farfall_sim as sim;
use glam::DVec3;

use crate::arms::Ship;
use crate::belt::{Belt, RockId};
use crate::mimic::Mimics;

/// How far the lock reaches, metres, and how far off the aim it will take
/// something (the cosine of the cone's half angle).
pub const REACH_M: f64 = 6_000.0;
pub const CONE_COS: f64 = 0.94;
/// The hold's position loop: the velocity it wants is this much of the
/// error a second (times the HOLD GAIN setting), capped.
pub const POS_GAIN: f64 = 0.7;
pub const CLOSE_MAX_MPS: f64 = 80.0;
/// The velocity loop: acceleration per m/s of velocity error.
pub const VEL_GAIN: f64 = 2.5;
/// How fast the stick moves the hold point, m/s at full deflection.
pub const NUDGE_MPS: f64 = 25.0;
/// The facing loop: torque demand per radian of error, and damping per
/// rad/s of body rate.
pub const FACE_KP: f64 = 2.2;
pub const FACE_KD: f64 = 1.4;

/// What the lock is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Rock(RockId),
    Ship(RockId),
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Rock(_) => "ROCK",
            Target::Ship(_) => "SHIP",
        }
    }
}

/// The target's state this step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracked {
    pub pos: DVec3,
    pub vel: DVec3,
    pub radius_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    /// The lock, and the relative position it keeps (world frame, m).
    pub target: Option<(Target, DVec3)>,
    /// HOLD GAIN, the setting (1 stock): how hard it holds.
    pub gain: f32,
    /// HOLD FACING: keep the nose on the target.
    pub face: bool,
    /// The last thrust demand it made, body frame -1..1 (for the engines'
    /// voice and the camera).
    pub demand: DVec3,
    /// The last range and closing speed, for the readout.
    pub range_m: f64,
    pub closing_mps: f64,
}

impl Default for Hold {
    fn default() -> Self {
        Self {
            target: None,
            gain: 1.0,
            face: true,
            demand: DVec3::ZERO,
            range_m: 0.0,
            closing_mps: 0.0,
        }
    }
}

impl Hold {
    pub fn engaged(&self) -> bool {
        self.target.is_some()
    }

    /// Take the lock on whatever is nearest the aim within reach: ships
    /// and rocks alike, the one closest to the line. Returns whether
    /// anything was taken.
    pub fn engage(&mut self, own: &Ship, aim: DVec3, belt: &Belt, mimics: &Mimics) -> bool {
        let aim = aim.normalize_or_zero();
        let mut best: Option<(Target, DVec3, f64)> = None;
        let mut consider = |t: Target, pos: DVec3, radius_m: f64| {
            let to = pos - own.pos;
            let range = to.length();
            if !(1.0..=REACH_M).contains(&range) {
                return;
            }
            let dir = to / range;
            // Off the aim by the angle less the target's own size.
            let cos = dir.dot(aim);
            let size = (radius_m / range).min(0.5);
            let miss = (1.0 - cos) - size * size * 0.5;
            if cos + size < CONE_COS {
                return;
            }
            if best.is_none_or(|b| miss < b.2) {
                best = Some((t, to, miss));
            }
        };
        for m in mimics.ships.iter() {
            consider(Target::Ship(m.id), m.pos, crate::mimic::HULL_R_M);
        }
        for r in belt.rocks.iter() {
            consider(Target::Rock(r.id), r.pos, r.radius_m);
        }
        self.target = best.map(|(t, off, _)| (t, off));
        self.engaged()
    }

    pub fn release(&mut self) {
        self.target = None;
        self.demand = DVec3::ZERO;
    }

    /// Where the target is now, or None if it is gone (the lock drops).
    pub fn track(&self, belt: &Belt, mimics: &Mimics) -> Option<Tracked> {
        let (t, _) = self.target?;
        match t {
            Target::Rock(id) => belt.rocks.iter().find(|r| r.id == id).map(|r| Tracked {
                pos: r.pos,
                vel: r.vel,
                radius_m: r.radius_m,
            }),
            Target::Ship(id) => mimics.ships.iter().find(|m| m.id == id).map(|m| Tracked {
                pos: m.pos,
                vel: m.vel,
                radius_m: crate::mimic::HULL_R_M,
            }),
        }
    }

    /// One fixed step of the lock: the pilot's thrust moves the hold
    /// point, the computer's demand replaces it on the way to the sim, the
    /// facing torque adds over the pilot's. `max_thrust` is the ship's
    /// per-axis thrust, m/s² (body). Call only while engaged.
    pub fn apply(
        &mut self,
        controls: &mut sim::Controls,
        own: &Ship,
        ang_vel_body: DVec3,
        tracked: &Tracked,
        max_thrust: DVec3,
        dt: f64,
    ) {
        let Some((_, offset)) = self.target.as_mut() else {
            return;
        };
        // The stick: a commanded relative velocity, moving the hold point.
        let nudge = own.orient * (controls.thrust_body * NUDGE_MPS);
        *offset += nudge * dt;
        let want_pos = tracked.pos + *offset;
        let err = want_pos - own.pos;
        let gain = POS_GAIN * self.gain.clamp(0.2, 3.0) as f64;
        let close = (err * gain).clamp_length_max(CLOSE_MAX_MPS);
        let want_vel = tracked.vel + close;
        let accel = (want_vel - own.vel) * VEL_GAIN;
        let body = own.orient.inverse() * accel;
        let demand = DVec3::new(
            body.x / max_thrust.x.max(1e-6),
            body.y / max_thrust.y.max(1e-6),
            body.z / max_thrust.z.max(1e-6),
        )
        .clamp(DVec3::splat(-1.0), DVec3::splat(1.0));
        self.demand = demand;
        controls.thrust_body = demand;
        // The assist steers the velocity toward the nose; the brake dumps
        // it; the overdrive would overshoot: all off under the hold.
        controls.assist = false;
        controls.brake = false;
        controls.boost = false;

        let to = tracked.pos - own.pos;
        self.range_m = to.length();
        self.closing_mps = -(own.vel - tracked.vel).dot(to.normalize_or_zero());
        if self.face {
            // The rotation that takes the nose onto the target, as an
            // axis-angle in the body frame, damped by the body rate.
            let tb = (own.orient.inverse() * to).normalize_or_zero();
            let nose = DVec3::NEG_Z;
            let axis = nose.cross(tb);
            let angle = nose.dot(tb).clamp(-1.0, 1.0).acos();
            let rot = if axis.length() > 1e-6 {
                axis.normalize() * angle
            } else if angle > 1.0 {
                DVec3::Y * angle
            } else {
                DVec3::ZERO
            };
            let torque = rot * FACE_KP - ang_vel_body * FACE_KD;
            controls.torque_body =
                (controls.torque_body + torque).clamp(DVec3::splat(-1.0), DVec3::splat(1.0));
            controls.despin = false;
        }
    }

    /// The readout's line while engaged.
    pub fn text(&self) -> Option<String> {
        let (t, _) = self.target?;
        let range = if self.range_m >= 1_000.0 {
            format!("{:.1} KM", self.range_m / 1_000.0)
        } else {
            format!("{:.0} M", self.range_m)
        };
        Some(format!(
            "HOLD {} {range}  {:+.0} M/S",
            t.name(),
            self.closing_mps
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::belt::Rock;
    use glam::DQuat;

    fn nose_of(orient: DQuat) -> DVec3 {
        orient * DVec3::NEG_Z
    }

    fn world() -> (sim::WorldParams, sim::WorldState) {
        let params = sim::presets::earth_compact();
        let mut state = sim::presets::circular_orbit(&params, 400_000.0);
        state.ship.ang_vel_radps = DVec3::ZERO;
        (params, state)
    }

    fn rock_at(pos: DVec3, vel: DVec3) -> Belt {
        let mut belt = Belt::default();
        belt.rocks.push(Rock {
            id: (1, 2, 3, 0),
            pos,
            vel,
            radius_m: 20.0,
            seed: 0.5,
            spin: 0.0,
        });
        belt
    }

    fn fly(
        params: &sim::WorldParams,
        state: &mut sim::WorldState,
        hold: &mut Hold,
        belt: &mut Belt,
        seconds: f64,
        stick: DVec3,
    ) {
        let steps = (seconds / sim::DT) as usize;
        for _ in 0..steps {
            for r in belt.rocks.iter_mut() {
                r.pos += r.vel * sim::DT;
            }
            let own = Ship {
                pos: state.ship.pos_m,
                vel: state.ship.vel_mps,
                orient: state.ship.orient,
                aim: nose_of(state.ship.orient),
            };
            let tracked = hold.track(belt, &Mimics::default()).unwrap();
            let mut controls = sim::Controls {
                thrust_body: stick,
                assist: true,
                ..Default::default()
            };
            hold.apply(
                &mut controls,
                &own,
                state.ship.ang_vel_radps,
                &tracked,
                params.ship.max_thrust_mps2,
                sim::DT,
            );
            *state = sim::step(params, state, controls);
        }
    }

    #[test]
    fn the_lock_takes_what_is_under_the_aim_and_nothing_out_of_the_cone() {
        let (_, state) = world();
        let own = Ship {
            pos: state.ship.pos_m,
            vel: state.ship.vel_mps,
            orient: state.ship.orient,
            aim: nose_of(state.ship.orient),
        };
        let nose = own.aim;
        let side = nose.cross(DVec3::Y).normalize();
        let mut belt = rock_at(own.pos + nose * 800.0 + side * 40.0, own.vel);
        belt.rocks.push(Rock {
            id: (9, 9, 9, 0),
            pos: own.pos + side * 800.0,
            vel: own.vel,
            radius_m: 50.0,
            seed: 0.1,
            spin: 0.0,
        });
        let mut hold = Hold::default();
        assert!(hold.engage(&own, nose, &belt, &Mimics::default()));
        assert!(matches!(hold.target, Some((Target::Rock((1, 2, 3, 0)), _))));
        assert!(hold.text().unwrap().starts_with("HOLD ROCK"));
        // Aimed away from both: nothing.
        let mut h2 = Hold::default();
        assert!(!h2.engage(&own, -nose, &belt, &Mimics::default()));
        assert!(h2.text().is_none());
        // The rock gone: the track drops.
        belt.rocks.clear();
        assert!(hold.track(&belt, &Mimics::default()).is_none());
    }

    #[test]
    fn the_hold_matches_a_drifting_rock_and_keeps_its_station() {
        let (params, mut state) = world();
        let nose = nose_of(state.ship.orient);
        let side = nose.cross(DVec3::Y).normalize();
        // A rock ahead, drifting across and away at a walking pace.
        let drift = side * 4.0 + nose * 2.0;
        let mut belt = rock_at(state.ship.pos_m + nose * 300.0, state.ship.vel_mps + drift);
        let mut hold = Hold::default();
        let own = Ship {
            pos: state.ship.pos_m,
            vel: state.ship.vel_mps,
            orient: state.ship.orient,
            aim: nose,
        };
        assert!(hold.engage(&own, nose, &belt, &Mimics::default()));
        fly(&params, &mut state, &mut hold, &mut belt, 25.0, DVec3::ZERO);
        let rock = belt.rocks[0];
        let rel_v = (state.ship.vel_mps - rock.vel).length();
        let range = (rock.pos - state.ship.pos_m).length();
        assert!(rel_v < 0.5, "velocity matched: {rel_v} m/s");
        assert!((range - 300.0).abs() < 15.0, "station kept: {range} m");
        assert!(hold.demand.length() < 0.3, "settled: {:?}", hold.demand);
        assert!(hold.closing_mps.abs() < 0.5);
        // The nose stays on it.
        let to = (rock.pos - state.ship.pos_m).normalize();
        assert!(
            nose_of(state.ship.orient).dot(to) > 0.995,
            "facing: {}",
            nose_of(state.ship.orient).dot(to)
        );
    }

    #[test]
    fn the_stick_moves_the_hold_point_and_the_nose_finds_a_target_off_axis() {
        let (params, mut state) = world();
        let nose = nose_of(state.ship.orient);
        let side = nose.cross(DVec3::Y).normalize();
        // Off the nose by 25 degrees: the lock still takes it, and the
        // ship comes round.
        let dir = (nose * 25f64.to_radians().cos() + side * 25f64.to_radians().sin()).normalize();
        let mut belt = rock_at(state.ship.pos_m + dir * 400.0, state.ship.vel_mps);
        let mut hold = Hold::default();
        let own = Ship {
            pos: state.ship.pos_m,
            vel: state.ship.vel_mps,
            orient: state.ship.orient,
            aim: nose,
        };
        assert!(hold.engage(&own, nose, &belt, &Mimics::default()));
        fly(&params, &mut state, &mut hold, &mut belt, 20.0, DVec3::ZERO);
        let to = (belt.rocks[0].pos - state.ship.pos_m).normalize();
        assert!(
            nose_of(state.ship.orient).dot(to) > 0.99,
            "{}",
            nose_of(state.ship.orient).dot(to)
        );
        // Full forward on the stick for 10 s closes at the nudge rate.
        let before = (belt.rocks[0].pos - state.ship.pos_m).length();
        fly(
            &params,
            &mut state,
            &mut hold,
            &mut belt,
            10.0,
            DVec3::new(0.0, 0.0, -1.0),
        );
        let after = (belt.rocks[0].pos - state.ship.pos_m).length();
        assert!(
            before - after > NUDGE_MPS * 10.0 * 0.6,
            "closed from {before} to {after}"
        );
        // Let go: it settles on the hold point (the ship trails it a
        // little while it moves, and closes that once it stops).
        fly(&params, &mut state, &mut hold, &mut belt, 10.0, DVec3::ZERO);
        let settled = (belt.rocks[0].pos - state.ship.pos_m).length();
        let station = hold.target.unwrap().1.length();
        assert!(
            (settled - station).abs() < 10.0,
            "{after} -> {settled} vs {station}"
        );
        assert!(settled < after, "and it is closer than while moving");
        assert!((state.ship.vel_mps - belt.rocks[0].vel).length() < 1.0);
    }
}
