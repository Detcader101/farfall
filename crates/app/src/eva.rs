//! On foot: the EVA walker.
//!
//! DISEMBARK, landed, leaves the seat (SPEC §6.5b): a first-person walker
//! beside the ship, and the keyboard and mouse — reserved for the on-foot
//! controller all along — are its controls. The rocks' rule holds here
//! too: the walker is the app's, not the sim's. The sim keeps the ship
//! LANDED exactly where it stands, and the golden hash does not know
//! anyone stepped out.
//!
//! The ground is the ground the gear stands on: the body's analytic
//! sphere at `radius_m` — there is no terrain mesh to query (SPEC §6.7).
//! Everything here works in the body's own frame — feet measured from the
//! body's centre, velocity over its ground — so a moving body carries its
//! walker exactly as it carries its landed ship.

use glam::{DMat3, DQuat, DVec3};

/// The eye above the feet, metres.
pub const EYE_M: f64 = 1.7;
/// A walk and a run over the ground, m/s.
pub const WALK_MPS: f64 = 2.6;
pub const RUN_MPS: f64 = 6.5;
/// Straight up off the ground, m/s — a fair hop under a full g.
pub const JUMP_MPS: f64 = 3.4;
/// Close enough to the hull for the DISEMBARK key to read BOARD.
pub const BOARD_RANGE_M: f64 = 16.0;
/// How far from the ship the walk-out puts the feet: clear of the gear,
/// the whole hull in view.
pub const EXIT_M: f64 = 9.0;
/// The bench's stroll: out this far, looking back at the ship.
pub const BENCH_OUT_M: f64 = 14.0;

/// The gaze stops short of straight up and straight down.
const PITCH_MAX: f64 = 1.45;
/// Radians per mouse count at sensitivity 1 — the cockpit freelook's rate.
const RAD_PER_COUNT: f64 = 0.0025;

/// The movement keys as held this step. Booleans, not axes: boots do not
/// ramp like thrusters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Keys {
    pub fwd: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub run: bool,
    pub jump: bool,
}

/// A pair of boots on a body: where they are, how they move, where the
/// eyes look. All of it in the body's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Walker {
    /// Which body the boots are on.
    pub body: usize,
    /// The feet, metres from the body's centre.
    pub feet_m: DVec3,
    /// Over the body's ground, m/s.
    pub vel_mps: DVec3,
    /// The way the walker faces: unit, tangent to the ground.
    heading: DVec3,
    /// The gaze above (below) the horizon, radians.
    pitch: f64,
    pub grounded: bool,
}

impl Walker {
    /// Step out of a landed ship: feet on the sphere, `out_m` to the
    /// ship's right, turned to face the hull.
    pub fn disembarked(
        body: usize,
        ship_local: DVec3,
        ship_orient: DQuat,
        ground_r: f64,
        out_m: f64,
    ) -> Self {
        let up = ship_local.normalize_or_zero();
        let door = tangent(ship_orient * DVec3::X, up);
        let feet_m = (ship_local + door * out_m).normalize() * ground_r;
        let mut w = Self {
            body,
            feet_m,
            vel_mps: DVec3::ZERO,
            heading: -door,
            pitch: 0.0,
            grounded: true,
        };
        w.face(ship_local);
        w
    }

    /// Turn to face a point of the body — its bearing along the ground;
    /// the pitch stays where it was.
    pub fn face(&mut self, target_local: DVec3) {
        let up = self.up();
        let to = target_local - self.feet_m;
        if (to - up * to.dot(up)).length_squared() > 1e-12 {
            self.heading = tangent(to, up);
        }
    }

    /// Aim the gaze this far above the horizon, radians.
    pub fn tilt(&mut self, pitch: f64) {
        self.pitch = pitch.clamp(-PITCH_MAX, PITCH_MAX);
    }

    /// Local up: away from the body's centre.
    pub fn up(&self) -> DVec3 {
        self.feet_m.normalize_or_zero()
    }

    /// The eye, metres from the body's centre.
    pub fn eye_m(&self) -> DVec3 {
        self.feet_m + self.up() * EYE_M
    }

    /// Mouse motion in counts: mouse right turns right, mouse up looks up.
    /// Always engaged — on foot there is no held button.
    pub fn look(&mut self, dx: f64, dy: f64, sensitivity: f64) {
        let k = RAD_PER_COUNT * sensitivity.clamp(0.1, 5.0);
        let up = self.up();
        self.heading = tangent(DQuat::from_axis_angle(up, -dx * k) * self.heading, up);
        self.pitch = (self.pitch - dy * k).clamp(-PITCH_MAX, PITCH_MAX);
    }

    /// The walker's orientation in the body's frame: right-handed, +X
    /// right, +Y up, the gaze at −Z — the ship's own convention — leaning
    /// with the planet wherever the feet stand.
    pub fn orientation(&self) -> DQuat {
        let up = self.up();
        let fwd = tangent(self.heading, up);
        let right = fwd.cross(up).normalize();
        DQuat::from_mat3(&DMat3::from_cols(right, up, -fwd)) * DQuat::from_rotation_x(self.pitch)
    }

    /// One fixed step over the ground. The legs set the pace along the
    /// tangent — boots grip, no skating — gravity (the body's own μ/r²)
    /// pulls the rest, and the sphere-exact ground puts the feet back on
    /// the surface and takes the inward velocity.
    pub fn step(&mut self, k: &Keys, mu: f64, ground_r: f64, dt: f64) {
        let up = self.up();
        self.heading = tangent(self.heading, up);
        if self.grounded {
            let right = self.heading.cross(up).normalize();
            let mut wish = DVec3::ZERO;
            if k.fwd {
                wish += self.heading;
            }
            if k.back {
                wish -= self.heading;
            }
            if k.right {
                wish += right;
            }
            if k.left {
                wish -= right;
            }
            let pace = if k.run { RUN_MPS } else { WALK_MPS };
            self.vel_mps = up * self.vel_mps.dot(up) + wish.normalize_or_zero() * pace;
            if k.jump {
                self.vel_mps += up * JUMP_MPS;
                self.grounded = false;
            }
        }
        let r2 = self.feet_m.length_squared().max(1.0);
        self.vel_mps -= self.feet_m / r2.sqrt() * (mu / r2) * dt;
        self.feet_m += self.vel_mps * dt;
        let r = self.feet_m.length();
        if r <= ground_r {
            let up = self.feet_m / r.max(1e-9);
            self.feet_m = up * ground_r;
            let into = self.vel_mps.dot(up);
            if into < 0.0 {
                self.vel_mps -= up * into;
            }
            self.grounded = true;
        } else if r > ground_r + 0.05 {
            self.grounded = false;
        }
    }
}

/// The tangent part of `v` on the ground whose up is `up`, unit — or an
/// arbitrary tangent when `v` stands straight up.
fn tangent(v: DVec3, up: DVec3) -> DVec3 {
    let t = v - up * v.dot(up);
    if t.length_squared() > 1e-12 {
        t.normalize()
    } else {
        up.any_orthonormal_vector()
    }
}

/// The suit's readout lines: the mode, and the way back to the ship —
/// with the DISEMBARK key reading as BOARD once the hull is in range.
pub fn lines(ship_m: f64, key: &str) -> Vec<String> {
    let dist = if ship_m < 1000.0 {
        format!("{ship_m:.0}M")
    } else {
        format!("{:.1}KM", ship_m / 1000.0)
    };
    let ship = if ship_m <= BOARD_RANGE_M {
        format!("SHIP {dist}  {key} BOARD")
    } else {
        format!("SHIP {dist}")
    };
    vec!["EVA  SUIT OK".to_string(), ship]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Earth-compact's surface: g = μ/R² comes out at 9.81.
    const R: f64 = 63_710.0;
    const MU: f64 = 9.81 * R * R;
    const DT: f64 = 1.0 / 120.0;

    fn standing() -> Walker {
        let up = DVec3::new(0.3, 0.8, -0.5).normalize();
        let orient = DQuat::from_mat3(&DMat3::from_cols(DVec3::X, DVec3::Y, DVec3::Z));
        let ship = up * (R + 2.5);
        Walker::disembarked(0, ship, orient, R, EXIT_M)
    }

    #[test]
    fn a_standing_walker_stays_on_the_ground_through_a_thousand_steps() {
        let mut w = standing();
        for _ in 0..1000 {
            w.step(&Keys::default(), MU, R, DT);
        }
        assert!(w.grounded);
        assert!(
            (w.feet_m.length() - R).abs() < 1e-6,
            "{}",
            w.feet_m.length()
        );
        assert!(w.vel_mps.length() < 1e-6, "{}", w.vel_mps.length());
    }

    #[test]
    fn walking_forward_carries_along_the_ground_not_off_it() {
        let mut w = standing();
        let start = w.feet_m;
        let keys = Keys {
            fwd: true,
            ..Keys::default()
        };
        let steps = 600; // five seconds
        for _ in 0..steps {
            w.step(&keys, MU, R, DT);
        }
        let walked = (w.feet_m - start).length();
        let want = WALK_MPS * DT * steps as f64;
        assert!((walked - want).abs() < want * 0.01, "{walked} vs {want}");
        assert!(
            (w.feet_m.length() - R).abs() < 0.05,
            "{}",
            w.feet_m.length()
        );
        assert!(w.grounded);
    }

    #[test]
    fn running_outpaces_walking() {
        let mut walk = standing();
        let mut run = standing();
        for _ in 0..240 {
            walk.step(
                &Keys {
                    fwd: true,
                    ..Keys::default()
                },
                MU,
                R,
                DT,
            );
            run.step(
                &Keys {
                    fwd: true,
                    run: true,
                    ..Keys::default()
                },
                MU,
                R,
                DT,
            );
        }
        let s = standing().feet_m;
        assert!((run.feet_m - s).length() > (walk.feet_m - s).length() * 2.0);
    }

    #[test]
    fn a_jump_leaves_the_ground_and_gravity_brings_it_back() {
        let mut w = standing();
        w.step(
            &Keys {
                jump: true,
                ..Keys::default()
            },
            MU,
            R,
            DT,
        );
        assert!(!w.grounded);
        let mut peak: f64 = 0.0;
        let mut air = 0;
        for _ in 0..240 {
            // two seconds is plenty at 9.81
            if !w.grounded {
                air += 1;
            }
            peak = peak.max(w.feet_m.length() - R);
            w.step(&Keys::default(), MU, R, DT);
        }
        assert!(w.grounded, "back down");
        // v²/2g ≈ 0.59 m for 3.4 m/s under 9.81.
        assert!(peak > 0.4 && peak < 0.8, "peak {peak}");
        assert!(air > 30, "hang time: {air} steps");
        assert!((w.feet_m.length() - R).abs() < 1e-6);
    }

    #[test]
    fn mouse_right_turns_the_walker_right_and_up_looks_up() {
        let mut w = standing();
        let up = w.up();
        let before = w.orientation();
        let right_was = before * DVec3::X;
        w.look(400.0, 0.0, 1.0);
        let fwd = w.orientation() * DVec3::NEG_Z;
        assert!(
            fwd.dot(right_was) > 0.2,
            "turned right: {}",
            fwd.dot(right_was)
        );
        assert!(fwd.dot(up).abs() < 1e-9, "and stayed level");
        w.look(0.0, -400.0, 1.0);
        let fwd = w.orientation() * DVec3::NEG_Z;
        assert!(fwd.dot(up) > 0.2, "looked up: {}", fwd.dot(up));
    }

    #[test]
    fn the_gaze_stays_short_of_the_poles() {
        let mut w = standing();
        w.look(0.0, -1.0e6, 1.0);
        let fwd = w.orientation() * DVec3::NEG_Z;
        assert!(fwd.dot(w.up()) < 0.999, "never straight up");
        w.look(0.0, 2.0e6, 1.0);
        let fwd = w.orientation() * DVec3::NEG_Z;
        assert!(fwd.dot(w.up()) > -0.999, "never straight down");
    }

    #[test]
    fn the_walk_out_stands_beside_the_ship_facing_the_hull() {
        let up = DVec3::new(0.1, 0.9, 0.2).normalize();
        let ship = up * (R + 2.5);
        let nose = tangent(DVec3::new(-0.7, 0.1, 0.4), up);
        let right = nose.cross(up).normalize();
        let orient = DQuat::from_mat3(&DMat3::from_cols(right, up, -nose));
        let w = Walker::disembarked(0, ship, orient, R, EXIT_M);
        assert!((w.feet_m.length() - R).abs() < 1e-6, "feet on the sphere");
        // Measured along the ground: the ship's origin itself sits up on
        // its gear.
        let out = (w.feet_m - ship.normalize() * R).length();
        assert!((out - EXIT_M).abs() < 0.2, "beside the ship: {out}");
        let fwd = w.orientation() * DVec3::NEG_Z;
        let to_ship = (ship - w.eye_m()).normalize();
        assert!(
            fwd.dot(to_ship) > 0.9,
            "facing the hull: {}",
            fwd.dot(to_ship)
        );
        assert!(w.grounded);
    }

    #[test]
    fn the_camera_leans_with_the_planet() {
        let w = standing();
        let o = w.orientation();
        let cam_up = o * DVec3::Y;
        assert!(cam_up.dot(w.up()) > 0.999, "up is the planet's up");
        let fwd = o * DVec3::NEG_Z;
        assert!(fwd.dot(w.up()).abs() < 1e-9, "level gaze on the horizon");
        assert!(
            ((o * DVec3::X).cross(cam_up).dot(-fwd) - 1.0).abs() < 1e-9,
            "right-handed"
        );
    }

    #[test]
    fn the_suit_lines_name_the_ship_and_offer_the_key_in_range() {
        let near = lines(9.0, "I");
        assert_eq!(near[0], "EVA  SUIT OK");
        assert_eq!(near[1], "SHIP 9M  I BOARD");
        let far = lines(230.0, "I");
        assert_eq!(far[1], "SHIP 230M");
        let miles = lines(2340.0, "I");
        assert_eq!(miles[1], "SHIP 2.3KM");
        for l in near.iter().chain(far.iter()).chain(miles.iter()) {
            assert!(l.len() <= 32, "fits the panel: {l}");
        }
    }
}
