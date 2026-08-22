//! Landing: a coarse look ahead at where the ship will touch down and how
//! hard, for the landing hoops and the readout.
//!
//! Not the sim — a cheap ballistic integration of gravity and nose-on
//! drag at a tenth of a second, the same laws the trajectory shader uses,
//! good enough to say "this is going to hurt" a minute before it does. The
//! verdict is advice: what a hull would take. (Nothing breaks yet.)

use farfall_sim::{atmo_density, ShipState, WorldParams};

/// Seconds to look ahead.
pub const HORIZON_S: f64 = 90.0;
/// A touchdown faster than this into the ground would wreck a hull, m/s.
pub const HARD_INTO_MPS: f64 = 30.0;
/// Sliding on faster than this at touchdown would too, m/s.
pub const HARD_ALONG_MPS: f64 = 120.0;
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
    /// Which body: 0 planet, 1 Moon, 2 Sun.
    pub body: usize,
}

impl Touchdown {
    /// 0 = a landing the hull shrugs off, 1 = certain destruction; in
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
        if self.hard() {
            "HARD"
        } else if self.danger() > 0.5 {
            "FIRM"
        } else {
            "SOFT"
        }
    }
}

/// Coast the ship forward, no engine, until it meets a body or the
/// horizon runs out.
pub fn predict(params: &WorldParams, ship: &ShipState, t_s: f64) -> Option<Touchdown> {
    let cda_over_m = params.ship.cd_area_m2 / params.ship.mass_kg;
    let mut p = ship.pos_m;
    let mut v = ship.vel_mps;
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
            if rel.length() < b.radius_m {
                // Speeds over the ground: relative to the body, which
                // (the Moon) may well be moving.
                let up = rel.normalize_or_zero();
                let v_rel = v - body_vel[body];
                let into = -v_rel.dot(up);
                let along = (v_rel - up * v_rel.dot(up)).length();
                return Some(Touchdown {
                    in_s: (i + 1) as f64 * DT,
                    into_mps: into.max(0.0),
                    along_mps: along,
                    body,
                });
            }
        }
        p = next;
    }
    None
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
        let [_, moon, _] = params.bodies(0.0);
        let up = DVec3::Y;
        // At rest relative to the Moon — which is itself going round the
        // planet at a hundred metres a second.
        let mu = params.planet.mu;
        let moon_vel = (params.moon.centre(mu, 0.01) - params.moon.centre(mu, -0.01)) / 0.02;
        let ship = ShipState {
            pos_m: moon.centre + up * (moon.radius_m + 2_000.0),
            vel_mps: moon_vel,
            orient: glam::DQuat::IDENTITY,
            ang_vel_radps: DVec3::ZERO,
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
    }

    #[test]
    fn an_orbit_touches_nothing_and_a_hover_is_safe() {
        let params = presets::earth_compact();
        let s = presets::circular_orbit(&params, 40_000.0);
        assert_eq!(predict(&params, &s.ship, 0.0), None);
        // Two metres up, creeping down: a landing.
        let mut ship = s.ship;
        ship.pos_m = DVec3::X * (params.planet.radius_m + 2.0);
        ship.vel_mps = -DVec3::X * 1.0;
        let td = predict(&params, &ship, 0.0).unwrap();
        assert!(!td.hard(), "{td:?}");
        assert!(td.danger() < 0.3);
        assert_eq!(td.verdict(), "SOFT");
        assert!(td.in_s < 2.0);
    }
}
