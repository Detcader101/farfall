//! Landing: a coarse look ahead at where the ship will touch down and how
//! hard, for the landing hoops, the pad marker and the readout; the record
//! of how the last touchdown went; the flight computer's LANDING ASSIST;
//! and the readout's lines for every state of the ground.
//!
//! Not the sim — a cheap ballistic integration of gravity and nose-on
//! drag at a tenth of a second, the same laws the trajectory shader uses,
//! good enough to say "this is going to hurt" a minute before it does. The
//! verdict is advice: what the gear will take. The sim (farfall-sim's
//! `Ground`) is the one that decides whether the ship actually lands.

use farfall_sim::{
    atmo_density, Controls, Ground, ShipState, WorldParams, DT as SIM_DT, GEAR_HEIGHT_M,
    TOUCHDOWN_INTO_MPS,
};
use glam::{DMat3, DQuat, DVec3};

/// Seconds to look ahead.
pub const HORIZON_S: f64 = 90.0;
/// A touchdown faster than this into the ground is a hard one: the gear
/// will not take it and the ship will not land — the sim's own limit.
pub const HARD_INTO_MPS: f64 = TOUCHDOWN_INTO_MPS;
/// Sliding on faster than this at touchdown would wreck a hull, m/s.
pub const HARD_ALONG_MPS: f64 = 120.0;
/// Seconds before touchdown at which the gear cue shows.
pub const GEAR_CUE_S: f64 = 15.0;
/// Tilt off the surface normal, degrees, past which the cue says LEVEL —
/// the sim's UPRIGHT_COS, 15°, is where a touchdown stops being a landing.
pub const LEVEL_DEG: f64 = 15.0;
/// The readout panel's width in glyphs: no line may be longer.
pub const LINE_COLS: usize = 32;
/// The bodies' names, by the sim's body index.
pub const BODY_NAMES: [&str; farfall_sim::BODIES] = ["PLANET", "MOON", "SUN", "URANUS"];
const DT: f64 = 0.1;

/// What the ship is heading for, if anything, within the horizon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Touchdown {
    /// Seconds from now.
    pub in_s: f64,
    /// Speed into the ground at contact, m/s (positive = down).
    pub into_mps: f64,
    /// Speed along the ground at contact, m/s.
    pub along_mps: f64,
    /// Which body: 0 planet, 1 Moon, 2 Sun, 3 Uranus.
    pub body: usize,
    /// Where, world frame, metres: the point at the gear's height over the
    /// surface. (Camera-relative subtraction is the caller's, in f64.)
    pub pos: DVec3,
    /// The surface normal there.
    pub up: DVec3,
    /// Metres of path between here and there.
    pub path_m: f64,
}

impl Touchdown {
    /// 0 = a landing the gear shrugs off, 1 = certain destruction; in
    /// between, the hoops go from calm to red.
    pub fn danger(&self) -> f32 {
        if self.body == 2 {
            return 1.0;
        }
        let into = self.into_mps / HARD_INTO_MPS;
        let along = self.along_mps / HARD_ALONG_MPS;
        into.max(along).clamp(0.0, 1.0) as f32
    }

    pub fn hard(&self) -> bool {
        self.body == 2 || self.into_mps > HARD_INTO_MPS || self.along_mps > HARD_ALONG_MPS
    }

    /// One word for the readout.
    pub fn verdict(&self) -> &'static str {
        verdict(self.hard(), self.danger())
    }

    /// The word for what to do now, if anything: FLARE when the descent
    /// would break the gear and the ground is close; LEVEL when the hull
    /// is tilted past what the gear will take; GEAR DOWN in the last
    /// seconds of a good approach.
    pub fn cue(&self, tilt_deg: f64) -> &'static str {
        if self.into_mps > HARD_INTO_MPS && self.in_s <= 20.0 {
            "FLARE"
        } else if tilt_deg > LEVEL_DEG && self.in_s <= 30.0 {
            "LEVEL"
        } else if self.in_s <= GEAR_CUE_S {
            "GEAR DOWN"
        } else {
            ""
        }
    }
}

fn verdict(hard: bool, danger: f32) -> &'static str {
    if hard {
        "HARD"
    } else if danger > 0.5 {
        "FIRM"
    } else {
        "SOFT"
    }
}

/// Coast the ship forward, no engine, until it meets a body (at the gear's
/// height, where the sim's contact is) or the horizon runs out.
pub fn predict(params: &WorldParams, ship: &ShipState, t_s: f64) -> Option<Touchdown> {
    let cda_over_m = params.ship.cd_area_m2 / params.ship.mass_kg;
    let mut p = ship.pos_m;
    let mut v = ship.vel_mps;
    let mut path = 0.0;
    let steps = (HORIZON_S / DT) as usize;
    for i in 0..steps {
        let t = t_s + i as f64 * DT;
        let mut a = farfall_sim::gravity_all(params, t, p);
        let r = p.length();
        if r - params.planet.radius_m < params.planet.atmo_top_m {
            let rho = atmo_density(&params.planet, r);
            a -= v * (0.5 * rho * v.length() * cda_over_m);
        }
        v += a * DT;
        let next = p + v * DT;
        let body_vel = params.body_velocities(t + DT);
        for (body, b) in params.bodies(t + DT).iter().enumerate() {
            let rel = next - b.centre;
            let stance = b.radius_m + GEAR_HEIGHT_M;
            if rel.length() < stance {
                // Speeds over the ground: relative to the body, which
                // (the Moon) may well be moving.
                let up = rel.normalize_or_zero();
                let v_rel = v - body_vel[body];
                let into = -v_rel.dot(up);
                let along = (v_rel - up * v_rel.dot(up)).length();
                let pos = b.centre + up * stance;
                return Some(Touchdown {
                    in_s: (i + 1) as f64 * DT,
                    into_mps: into.max(0.0),
                    along_mps: along,
                    body,
                    pos,
                    up,
                    path_m: path + (pos - p).length(),
                });
            }
        }
        path += (next - p).length();
        p = next;
    }
    None
}

/// How the last touchdown went: judged from the tick the ship met the
/// ground, by the app, from the sim's own before-and-after — presentation
/// memory, not world state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Record {
    pub into_mps: f64,
    pub along_mps: f64,
    pub body: usize,
}

impl Record {
    /// The record of this step, if this is the step that put the ship on
    /// the ground. `t_s` is the sim time at `before`.
    pub fn judge(
        params: &WorldParams,
        t_s: f64,
        before: &ShipState,
        after: &ShipState,
    ) -> Option<Record> {
        if before.ground != Ground::Flight {
            return None;
        }
        let body = match after.ground {
            Ground::Flight => return None,
            Ground::Down { body, .. } | Ground::Landed { body, .. } => body,
        };
        let b = params.bodies(t_s + SIM_DT)[body];
        let up = (after.pos_m - b.centre).normalize_or_zero();
        let v_rel = before.vel_mps - params.body_velocities(t_s)[body];
        let into = -v_rel.dot(up);
        Some(Record {
            into_mps: into.max(0.0),
            along_mps: (v_rel - up * v_rel.dot(up)).length(),
            body,
        })
    }

    pub fn verdict(&self) -> &'static str {
        let hard =
            self.body == 2 || self.into_mps > HARD_INTO_MPS || self.along_mps > HARD_ALONG_MPS;
        let danger = (self.into_mps / HARD_INTO_MPS).max(self.along_mps / HARD_ALONG_MPS) as f32;
        verdict(hard, danger)
    }

    /// A typical soft landing, for the bench's parked capture.
    pub fn sample() -> Record {
        Record {
            into_mps: 1.8,
            along_mps: 0.4,
            body: 0,
        }
    }
}

/// How far the ship's own up is off the surface normal, degrees.
pub fn tilt_deg(orient: DQuat, up_world: DVec3) -> f64 {
    let own = orient * DVec3::Y;
    own.dot(up_world).clamp(-1.0, 1.0).acos().to_degrees()
}

/// LANDING ASSIST: proportional gain on the levelling error (radians) and
/// on the spin, into torque demand. At 3.0 a 20° tilt asks for full
/// authority; at 3.0 a third of a rad/s of spin does. On the ship's
/// torque (1.7 rad/s² pitch, 0.8 roll) that is ζ ≈ 0.8 about roll, the
/// slow axis, and overdamped about pitch: it settles in a few seconds
/// and never hunts. (At 0.8 it was ζ ≈ 0.2 and still 3° out after six.)
pub const ASSIST_KP: f64 = 3.0;
pub const ASSIST_KD: f64 = 3.0;

/// The flight computer holding the hull level over the ground on the way
/// in: pitch and roll demand toward the surface normal, damped by the
/// spin, blended per axis by how much the pilot is *not* commanding that
/// axis — full stick means no fighting the pilot. Yaw is left alone; a
/// heading is the pilot's business.
pub fn assist(controls: &mut Controls, orient: DQuat, ang_vel: DVec3, up_world: DVec3) {
    let up_body = orient.conjugate() * up_world;
    // The rotation that takes the ship's +Y to the surface normal is about
    // Y × up, by its angle: as a small-angle vector that is the error.
    let err = DVec3::Y.cross(up_body);
    let demand = DVec3::new(
        err.x * ASSIST_KP - ang_vel.x * ASSIST_KD,
        0.0,
        err.z * ASSIST_KP - ang_vel.z * ASSIST_KD,
    );
    let pilot = controls.torque_body;
    let gain = DVec3::ONE - pilot.abs().clamp(DVec3::ZERO, DVec3::ONE);
    controls.torque_body = (pilot + demand * gain).clamp(DVec3::splat(-1.0), DVec3::splat(1.0));
}

/// The odometer phase that puts a hoop exactly on the touchdown, `path_m`
/// of path away: the hoops then converge on the pad rather than on an
/// arbitrary grid line, and still stream past as the ship closes.
pub fn hoop_phase(path_m: f64, spacing_m: f32) -> f32 {
    let s = f64::from(spacing_m.max(1.0));
    (s - path_m.rem_euclid(s)) as f32
}

/// A ship parked on its gear on `body`, for the bench: the Sun well up
/// the sky, the nose along the ground toward it, still. The sim will hold
/// it there.
pub fn parked(params: &WorldParams, body: usize) -> ShipState {
    let b = params.bodies(0.0)[body];
    let sun = params.sun.dir.normalize();
    let up = (sun + DVec3::Y * 0.8).normalize();
    let nose = (sun - up * sun.dot(up)).normalize_or_zero();
    let right = nose.cross(up).normalize();
    ShipState {
        pos_m: b.centre + up * (b.radius_m + GEAR_HEIGHT_M),
        vel_mps: params.body_velocities(0.0)[body],
        orient: DQuat::from_mat3(&DMat3::from_cols(right, up, -nose)),
        ang_vel_radps: DVec3::ZERO,
        ground: Ground::Landed { body, up },
    }
}

/// Everything the readout needs to say about the ground.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    /// LANDING mode (G) is on.
    pub mode: bool,
    pub ground: Ground,
    pub touchdown: Option<Touchdown>,
    /// Vertical speed over the nearest body, m/s, up positive.
    pub vspeed_mps: f64,
    /// Altitude over the nearest body's surface, metres.
    pub altitude_m: f64,
    /// Speed over the ground of the body under the ship, m/s.
    pub ground_speed_mps: f64,
    pub tilt_deg: f64,
    pub record: Option<Record>,
    /// A message with a moment to it — DISEMBARK's answer.
    pub notice: Option<&'a str>,
    /// The name of the DISEMBARK key.
    pub disembark_key: &'a str,
}

fn alt_text(m: f64) -> String {
    if m < 1000.0 {
        format!("{m:.0}M")
    } else {
        format!("{:.1}KM", m / 1000.0)
    }
}

/// The readout's lines for this state of the ground — none at all in
/// plain flight with the mode off. Every line fits [`LINE_COLS`].
pub fn lines(v: &View) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match v.ground {
        Ground::Landed { body, .. } => {
            out.push(format!("LANDED ON {}", BODY_NAMES[body]));
            out.push(match v.record {
                Some(r) => format!(
                    "{}  DOWN {:.1}  ALONG {:.1} M/S",
                    r.verdict(),
                    r.into_mps,
                    r.along_mps
                ),
                None => "ENGINES IDLE".to_string(),
            });
            out.push(match v.notice {
                Some(n) => n.to_string(),
                None => format!("{} DISEMBARK", v.disembark_key),
            });
        }
        Ground::Down { body, clean } => {
            if clean {
                out.push(format!(
                    "ROLLING ON {}  {:.0} M/S",
                    BODY_NAMES[body], v.ground_speed_mps
                ));
                out.push(format!(
                    "SLOW UNDER {:.0} M/S TO SETTLE",
                    farfall_sim::TOUCHDOWN_ALONG_MPS
                ));
            } else {
                out.push(format!("DOWN ON {}  HARD", BODY_NAMES[body]));
                if let Some(r) = v.record {
                    out.push(format!(
                        "DOWN {:.0}  ALONG {:.0} M/S",
                        r.into_mps, r.along_mps
                    ));
                }
                out.push("NOT LANDED  LIFT OFF, RE-LAND".to_string());
            }
            if let Some(n) = v.notice {
                out.push(n.to_string());
            }
        }
        Ground::Flight => {
            if v.mode {
                match v.touchdown {
                    Some(t) => {
                        out.push(format!(
                            "LAND {:<4} IN {:>3.0}S  ALT {}",
                            t.verdict(),
                            t.in_s,
                            alt_text(v.altitude_m)
                        ));
                        out.push(format!(
                            "VS {:+.1}  ALONG {:.0}  {}",
                            v.vspeed_mps,
                            t.along_mps,
                            t.cue(v.tilt_deg)
                        ));
                    }
                    None => {
                        out.push(format!("LAND  NO TOUCHDOWN IN {HORIZON_S:.0}S"));
                        out.push(format!(
                            "VS {:+.1}  ALT {}",
                            v.vspeed_mps,
                            alt_text(v.altitude_m)
                        ));
                    }
                }
            }
            if let Some(n) = v.notice {
                out.push(n.to_string());
            }
        }
    }
    for line in out.iter_mut() {
        if line.len() > LINE_COLS {
            line.truncate(LINE_COLS);
        }
        while line.ends_with(' ') {
            line.pop();
        }
    }
    out
}

/// The answer to DISEMBARK.
pub fn disembark_notice(ground: Ground) -> &'static str {
    match ground {
        Ground::Landed { .. } => "DISEMBARK  NOT YET",
        Ground::Down { .. } => "DISEMBARK  NOT LANDED",
        Ground::Flight => "DISEMBARK  LAND FIRST",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farfall_sim::presets;
    use glam::DVec3;

    #[test]
    fn a_free_fall_arrives_at_sqrt_2gh() {
        let params = presets::earth_compact();
        // Straight down from 2 km, in (effectively) vacuum: the Moon.
        let [_, moon, _, _] = params.bodies(0.0);
        let up = DVec3::Y;
        // At rest relative to the Moon — which is itself going round the
        // planet at a hundred metres a second.
        let mu = params.planet.mu;
        let moon_vel = (params.moon.centre(mu, 0.01) - params.moon.centre(mu, -0.01)) / 0.02;
        let ship = ShipState {
            pos_m: moon.centre + up * (moon.radius_m + GEAR_HEIGHT_M + 2_000.0),
            vel_mps: moon_vel,
            orient: glam::DQuat::IDENTITY,
            ang_vel_radps: DVec3::ZERO,
            ground: Ground::Flight,
        };
        let td = predict(&params, &ship, 0.0).expect("should hit the Moon");
        assert_eq!(td.body, 1);
        let g = moon.mu / (moon.radius_m * moon.radius_m);
        let want = (2.0 * g * 2_000.0).sqrt();
        // g falls off over the 2 km, so a shade under sqrt(2 g h).
        assert!(
            td.into_mps < want && td.into_mps > want * 0.9,
            "{} vs {want}",
            td.into_mps
        );
        assert!(td.along_mps < 3.0, "{}", td.along_mps);
        assert!(
            td.hard(),
            "{} m/s into the Moon is not a landing",
            td.into_mps
        );
        assert_eq!(td.danger(), 1.0);
        assert_eq!(td.verdict(), "HARD");
        // The pad is under the ship at the gear's height. The path is in
        // the planet's frame — the frame the hoops are drawn in — so over
        // the Moon it is the drop plus the Moon's own motion meanwhile.
        assert!(td.up.dot(up) > 0.9999);
        // ...over where the Moon will be then, not where it is now.
        let then = params.moon.centre(mu, td.in_s);
        assert!(((td.pos - then).length() - moon.radius_m - GEAR_HEIGHT_M).abs() < 1e-3);
        let carried = moon_vel.length() * td.in_s;
        assert!(
            td.path_m > 2_000.0 && td.path_m < 2_000.0 + carried,
            "{} m of path for a 2 km drop carried {carried:.0} m sideways",
            td.path_m
        );
        // Fifty seconds out there is no word yet; in the last twenty, with
        // a descent the gear will not take, the word is FLARE.
        assert_eq!(td.cue(0.0), "");
        let soon = Touchdown { in_s: 10.0, ..td };
        assert_eq!(soon.cue(0.0), "FLARE");
        // Over the still planet, a straight drop's path is the drop.
        let mut still = ship;
        still.pos_m = DVec3::Y * (params.planet.radius_m + GEAR_HEIGHT_M + 2_000.0);
        still.vel_mps = DVec3::ZERO;
        let td = predict(&params, &still, 0.0).expect("should hit the planet");
        assert_eq!(td.body, 0);
        assert!((td.path_m - 2_000.0).abs() < 20.0, "{}", td.path_m);
    }

    #[test]
    fn an_orbit_touches_nothing_and_a_hover_is_safe() {
        let params = presets::earth_compact();
        let s = presets::circular_orbit(&params, 40_000.0);
        assert_eq!(predict(&params, &s.ship, 0.0), None);
        // A hand's breadth up, creeping down: a landing (gravity has the
        // last word on the speed — two metres of drop is already FIRM on
        // a 12 m/s gear).
        let mut ship = s.ship;
        ship.pos_m = DVec3::X * (params.planet.radius_m + GEAR_HEIGHT_M + 0.2);
        ship.vel_mps = -DVec3::X * 1.0;
        let td = predict(&params, &ship, 0.0).unwrap();
        assert!(!td.hard(), "{td:?}");
        assert!(td.danger() < 0.3, "{}", td.danger());
        assert_eq!(td.verdict(), "SOFT");
        assert!(td.in_s < 2.0);
        assert_eq!(td.cue(0.0), "GEAR DOWN");
        assert_eq!(td.cue(20.0), "LEVEL");
        ship.pos_m = DVec3::X * (params.planet.radius_m + GEAR_HEIGHT_M + 2.0);
        assert_eq!(predict(&params, &ship, 0.0).unwrap().verdict(), "FIRM");
    }

    /// The verdict is the sim's: a descent the gear takes is SOFT or FIRM,
    /// one it will not is HARD — the same 12 m/s line the sim lands on.
    #[test]
    fn the_verdict_agrees_with_what_the_gear_takes() {
        let r = |into: f64| Record {
            into_mps: into,
            along_mps: 0.0,
            body: 0,
        };
        assert_eq!(r(2.0).verdict(), "SOFT");
        assert_eq!(r(TOUCHDOWN_INTO_MPS * 0.8).verdict(), "FIRM");
        assert_eq!(r(TOUCHDOWN_INTO_MPS * 1.1).verdict(), "HARD");
    }

    /// The touchdown record comes from the tick the ground was met, and
    /// says how fast the ship was going when it met it.
    #[test]
    fn a_touchdown_is_judged_from_the_tick_it_happens() {
        let params = presets::earth_compact();
        let mut state = presets::circular_orbit(&params, 0.0);
        state.ship.pos_m = DVec3::Y * (params.planet.radius_m + GEAR_HEIGHT_M + 3.0);
        state.ship.vel_mps = -DVec3::Y * 5.0;
        state.ship.orient = DQuat::IDENTITY;
        let mut record = None;
        for _ in 0..240 {
            let before = state.ship;
            let t = state.time_s;
            state = farfall_sim::step(&params, &state, Controls::default());
            if let Some(r) = Record::judge(&params, t, &before, &state.ship) {
                assert!(record.is_none(), "judged twice");
                record = Some(r);
            }
        }
        let r = record.expect("never touched down");
        assert_eq!(r.body, 0);
        // Three metres of fall on top of the 5 m/s: v² = 25 + 2·g·3.
        let want = (25.0 + 2.0 * 9.81 * 3.0_f64).sqrt();
        assert!((r.into_mps - want).abs() < 0.5, "{} vs {want}", r.into_mps);
        assert!(r.along_mps < 0.5, "{}", r.along_mps);
        // Nine m/s on a twelve m/s gear: FIRM — and the sim landed it.
        assert_eq!(r.verdict(), "FIRM");
        assert!(matches!(state.ship.ground, Ground::Landed { body: 0, .. }));
    }

    /// LANDING ASSIST levels a rolled ship over the ground and holds it
    /// there without hunting, and gives way to the pilot on any axis the
    /// pilot is using.
    #[test]
    fn the_assist_levels_the_hull_and_yields_to_the_pilot() {
        let params = presets::earth_compact();
        // Above the air, nose along the ground, rolled 30° and pitched 10°.
        let mut state = presets::circular_orbit(&params, 40_000.0);
        state.ship.vel_mps = DVec3::ZERO;
        let up = state.ship.pos_m.normalize();
        let level = DQuat::from_rotation_arc(DVec3::Y, up);
        state.ship.orient = level
            * DQuat::from_rotation_z(30f64.to_radians())
            * DQuat::from_rotation_x(10f64.to_radians());
        assert!(tilt_deg(state.ship.orient, up) > 25.0);
        let mut worst_spin: f64 = 0.0;
        for _ in 0..(6 * 120) {
            let mut c = Controls::default();
            let up = state.ship.pos_m.normalize();
            assist(&mut c, state.ship.orient, state.ship.ang_vel_radps, up);
            assert_eq!(c.torque_body.y, 0.0, "yaw is the pilot's");
            state = farfall_sim::step(&params, &state, c);
            worst_spin = worst_spin.max(state.ship.ang_vel_radps.length());
        }
        let up = state.ship.pos_m.normalize();
        let tilt = tilt_deg(state.ship.orient, up);
        assert!(tilt < 1.0, "still {tilt:.1}° off level");
        assert!(
            state.ship.ang_vel_radps.length() < 0.02,
            "still turning at {:?}",
            state.ship.ang_vel_radps
        );
        assert!(worst_spin < 1.5, "it whipped round at {worst_spin} rad/s");
        // The pilot's full roll is the pilot's: no assist on that axis.
        let mut c = Controls {
            torque_body: DVec3::new(0.0, 0.0, 1.0),
            ..Default::default()
        };
        let rolled = level * DQuat::from_rotation_z(30f64.to_radians());
        assist(&mut c, rolled, DVec3::ZERO, up);
        assert_eq!(c.torque_body.z, 1.0);
        assert!(c.torque_body.x.abs() < 1e-9, "{:?}", c.torque_body);
    }

    /// The hoop phase puts a hoop exactly on the touchdown, whatever the
    /// spacing and however far the path runs.
    #[test]
    fn the_hoops_are_phased_onto_the_touchdown() {
        for spacing in [100.0f32, 250.0, 500.0, 1000.0] {
            for path in [0.0, 12.5, 249.0, 250.0, 251.0, 3_777.7, 12_345.6] {
                let x = f64::from(hoop_phase(path, spacing));
                let s = f64::from(spacing);
                // The shader's rule: the first hoop ahead is at
                // spacing - fract(x / spacing) * spacing.
                let ahead = s - (x / s).fract() * s;
                let off = (path - ahead).rem_euclid(s);
                let off = off.min(s - off);
                assert!(
                    off < 1e-3,
                    "spacing {spacing} path {path}: hoop {off} m off the pad"
                );
            }
        }
    }

    /// The bench's parked ship is landed, level and still, and the sim
    /// keeps it exactly there.
    #[test]
    fn a_parked_ship_is_landed_level_and_still() {
        let params = presets::earth_compact();
        let ship = parked(&params, 0);
        let up = ship.pos_m.normalize();
        assert!(matches!(ship.ground, Ground::Landed { body: 0, .. }));
        assert!((ship.pos_m.length() - params.planet.radius_m - GEAR_HEIGHT_M).abs() < 1e-6);
        assert!(tilt_deg(ship.orient, up) < 1e-6);
        // The nose is along the ground.
        let nose = ship.orient * DVec3::NEG_Z;
        assert!(nose.dot(up).abs() < 1e-9);
        let mut state = presets::circular_orbit(&params, 0.0);
        state.ship = ship;
        let later = (0..600).fold(state, |s, _| {
            farfall_sim::step(&params, &s, Controls::default())
        });
        assert_eq!(later.ship.pos_m, ship.pos_m);
    }

    /// Every line the readout can say about the ground fits the panel, in
    /// every state, at the widest numbers a landing can show.
    #[test]
    fn every_readout_line_fits_the_panel_in_every_state() {
        let td = Touchdown {
            in_s: 90.0,
            into_mps: 147.0,
            along_mps: 1234.0,
            body: 3,
            pos: DVec3::ZERO,
            up: DVec3::Y,
            path_m: 0.0,
        };
        let record = Record {
            into_mps: 147.3,
            along_mps: 1234.5,
            body: 3,
        };
        let grounds = [
            Ground::Flight,
            Ground::Down {
                body: 3,
                clean: false,
            },
            Ground::Down {
                body: 3,
                clean: true,
            },
            Ground::Landed {
                body: 3,
                up: DVec3::Y,
            },
        ];
        let mut seen = 0;
        for ground in grounds {
            for mode in [false, true] {
                for touchdown in [None, Some(td)] {
                    for notice in [None, Some("DISEMBARK  LAND FIRST")] {
                        let v = View {
                            mode,
                            ground,
                            touchdown,
                            vspeed_mps: -123.4,
                            altitude_m: 12_345.0,
                            ground_speed_mps: 1234.0,
                            tilt_deg: 20.0,
                            record: Some(record),
                            notice,
                            disembark_key: "RSHIFT",
                        };
                        for line in lines(&v) {
                            seen += 1;
                            assert!(
                                line.len() <= LINE_COLS,
                                "{line:?} is {} glyphs in {ground:?}",
                                line.len()
                            );
                            assert!(!line.ends_with(' '));
                        }
                    }
                }
            }
        }
        assert!(seen > 30);
        // And the words are the right ones.
        let landed = View {
            mode: false,
            ground: Ground::Landed {
                body: 1,
                up: DVec3::Y,
            },
            touchdown: None,
            vspeed_mps: 0.0,
            altitude_m: GEAR_HEIGHT_M,
            ground_speed_mps: 0.0,
            tilt_deg: 0.0,
            record: Some(Record::sample()),
            notice: None,
            disembark_key: "I",
        };
        let l = lines(&landed);
        assert_eq!(l[0], "LANDED ON MOON");
        assert_eq!(l[1], "SOFT  DOWN 1.8  ALONG 0.4 M/S");
        assert_eq!(l[2], "I DISEMBARK");
        let flying = View {
            ground: Ground::Flight,
            ..landed
        };
        assert!(lines(&flying).is_empty(), "nothing to say in plain flight");
        assert_eq!(
            disembark_notice(Ground::Landed {
                body: 0,
                up: DVec3::Y
            }),
            "DISEMBARK  NOT YET"
        );
    }
}
