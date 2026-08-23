//! The wormhole drive: a jump to a chosen body at a chosen safe distance,
//! and the sequence the pilot sees on the way.
//!
//! Nothing outruns light in this sim (see `light_limit`), so the way to
//! the Sun is not through speed: the drive folds the distance. The sequence
//! is three phases — CHARGE, the view opening wide; FLIP, the world turned
//! inside out through a mirror sphere and inverted, at whose peak the jump
//! happens; ARRIVE, the new place seen through gassy, watery particles
//! settling — and then it snaps to normal and the ship is where the map
//! said, at the distance the pilot set, never inside anything.

use farfall_sim::{Body, ShipState, WorldParams};
use glam::DVec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Planet,
    Moon,
    Sun,
    Uranus,
}

impl Destination {
    pub const ALL: [Destination; 4] = [
        Destination::Planet,
        Destination::Moon,
        Destination::Sun,
        Destination::Uranus,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Destination::Planet => "PLANET",
            Destination::Moon => "MOON",
            Destination::Sun => "SUN",
            Destination::Uranus => "URANUS",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Destination::Planet => "planet",
            Destination::Moon => "moon",
            Destination::Sun => "sun",
            Destination::Uranus => "uranus",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|d| d.key() == k)
    }

    /// The body itself, from the sim, at sim time `t`.
    pub fn body(self, params: &WorldParams, t_s: f64) -> Body {
        let [planet, moon, sun, uranus] = params.bodies(t_s);
        match self {
            Destination::Planet => planet,
            Destination::Moon => moon,
            Destination::Sun => sun,
            Destination::Uranus => uranus,
        }
    }

    pub fn radius_m(self, params: &WorldParams) -> f64 {
        self.body(params, 0.0).radius_m
    }

    /// Centre of the body in the planet's frame at sim time `t`.
    pub fn centre(self, params: &WorldParams, t_s: f64) -> DVec3 {
        self.body(params, t_s).centre
    }

    fn next(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }
}

/// Where the pilot wants to go, and how close.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub dest: Destination,
    /// Safe distance from the body's surface, in radii of that body.
    pub safe_radii: f64,
}

pub const SAFE_RADII_MIN: f64 = 0.05;
pub const SAFE_RADII_MAX: f64 = 60.0;

impl Default for Plan {
    fn default() -> Self {
        Self {
            dest: Destination::Sun,
            safe_radii: 20.0,
        }
    }
}

impl Plan {
    pub fn cycle_destination(&mut self, forward: bool) {
        self.dest = self.dest.next(forward);
    }

    pub fn adjust_safe(&mut self, forward: bool) {
        // Geometric steps: fine close in, coarse far out.
        let f = if forward { 1.25 } else { 1.0 / 1.25 };
        self.safe_radii = (self.safe_radii * f).clamp(SAFE_RADII_MIN, SAFE_RADII_MAX);
    }

    pub fn set_safe(&mut self, radii: f64) {
        self.safe_radii = if radii.is_finite() {
            radii.clamp(SAFE_RADII_MIN, SAFE_RADII_MAX)
        } else {
            Plan::default().safe_radii
        };
    }

    /// Distance from the body's surface, metres.
    pub fn safe_m(&self, params: &WorldParams) -> f64 {
        self.safe_radii * self.dest.radius_m(params)
    }

    /// Where the jump lands: on the line from the body toward where the
    /// ship is now (so the old place is behind you), at surface + safe
    /// distance — and in a circular orbit about it, prograde along the
    /// ship's heading projected level. Every body pulls, so every body is
    /// arrived at the same way: the drive never drops you into a fall.
    pub fn arrival(&self, params: &WorldParams, ship: &ShipState, t_s: f64) -> (DVec3, DVec3) {
        let body = self.dest.body(params, t_s);
        let mut away = ship.pos_m - body.centre;
        if away.length() < 1.0 {
            away = DVec3::X;
        }
        let away = away.normalize();
        let r = body.radius_m + self.safe_m(params);
        let pos = body.centre + away * r;
        let nose = ship.orient * DVec3::NEG_Z;
        let mut tangent = nose - away * nose.dot(away);
        if tangent.length() < 1e-6 {
            tangent = away.cross(DVec3::Y);
        }
        let vel = tangent.normalize() * (body.mu / r).sqrt();
        (pos, vel)
    }
}

/// The sequence, in seconds.
pub const CHARGE_S: f32 = 1.3;
pub const FLIP_S: f32 = 0.55;
pub const ARRIVE_S: f32 = 1.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Charge,
    Flip,
    Arrive,
}

/// What the renderer needs from the sequence this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Look {
    /// Multiplier on the camera's vertical field of view.
    pub fov_scale: f32,
    /// 0..1: the mirror-sphere inversion of the view.
    pub fisheye: f32,
    /// 0..1: colour inversion.
    pub invert: f32,
    /// 0..1: the gassy particles on arrival.
    pub particles: f32,
    /// 0..1: how charged the drive is (for the sound, and a glow).
    pub charge: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Warp {
    phase: Phase,
    t: f32,
    jumped: bool,
}

impl Default for Warp {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            t: 0.0,
            jumped: false,
        }
    }
}

impl Look {
    /// The hyper drive half-engages the wormhole drive: at level `h`
    /// (0..1) the view opens partway to the charge, the drive glows, and
    /// the field's particles stream — on top of whatever the full drive
    /// is doing.
    pub fn with_hyper(mut self, h: f32) -> Look {
        let h = if h.is_finite() {
            h.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.fov_scale *= 1.0 + 0.45 * h;
        self.charge = self.charge.max(0.7 * h);
        self.particles = self.particles.max(0.6 * h);
        self
    }
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Warp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Start the sequence. Ignored while one is running.
    pub fn engage(&mut self) -> bool {
        if self.active() {
            return false;
        }
        self.phase = Phase::Charge;
        self.t = 0.0;
        self.jumped = false;
        true
    }

    /// The drive slipping of its own accord (the hyper drive overdriven):
    /// straight into the flip, no charge — the jump comes at its peak as
    /// ever, the app decides where to. Ignored mid-sequence.
    pub fn slip(&mut self) -> bool {
        if self.active() {
            return false;
        }
        self.phase = Phase::Flip;
        self.t = 0.0;
        self.jumped = false;
        true
    }

    /// Advance by `dt`. Returns true on the one frame the jump should be
    /// made — the peak of the flip, when the view is fully inside out and
    /// nobody can see the seam.
    pub fn update(&mut self, dt: f32) -> bool {
        let dt = dt.clamp(0.0, 0.25);
        self.t += dt;
        match self.phase {
            Phase::Idle => false,
            Phase::Charge => {
                if self.t >= CHARGE_S {
                    self.phase = Phase::Flip;
                    self.t -= CHARGE_S;
                }
                false
            }
            Phase::Flip => {
                let jump = !self.jumped && self.t >= FLIP_S * 0.5;
                if jump {
                    self.jumped = true;
                }
                if self.t >= FLIP_S {
                    self.phase = Phase::Arrive;
                    self.t -= FLIP_S;
                }
                jump
            }
            Phase::Arrive => {
                if self.t >= ARRIVE_S {
                    self.phase = Phase::Idle;
                    self.t = 0.0;
                }
                false
            }
        }
    }

    pub fn look(&self) -> Look {
        match self.phase {
            Phase::Idle => Look {
                fov_scale: 1.0,
                ..Default::default()
            },
            Phase::Charge => {
                let f = smooth(self.t / CHARGE_S);
                Look {
                    // 70° opens toward 160°.
                    fov_scale: 1.0 + 1.25 * f * f,
                    fisheye: 0.0,
                    invert: 0.0,
                    particles: 0.0,
                    charge: f,
                }
            }
            Phase::Flip => {
                let f = self.t / FLIP_S;
                // A bell over the flip: fully inside out at its middle.
                let bell = (std::f32::consts::PI * f).sin();
                Look {
                    fov_scale: 2.25,
                    fisheye: smooth(bell * 1.3),
                    invert: smooth((bell - 0.35) / 0.5),
                    particles: f,
                    charge: 1.0,
                }
            }
            Phase::Arrive => {
                let f = self.t / ARRIVE_S;
                Look {
                    // Snap back over the first third, then settle.
                    fov_scale: 1.0 + 1.25 * (1.0 - smooth(f * 3.0)),
                    fisheye: 0.0,
                    invert: 0.0,
                    particles: 1.0 - smooth(f),
                    charge: 1.0 - smooth(f * 2.0),
                }
            }
        }
    }
}

#[cfg(test)]
mod hyper_tests {
    use super::*;

    #[test]
    fn a_slip_flips_at_once_and_jumps_at_the_peak() {
        let mut w = Warp::new();
        assert!(w.slip());
        assert!(w.active());
        assert!(!w.slip(), "not twice");
        let mut jumped = 0;
        for _ in 0..400 {
            if w.update(0.01) {
                jumped += 1;
            }
        }
        assert_eq!(jumped, 1);
        assert!(!w.active(), "flip and arrive are over in four seconds");
    }

    #[test]
    fn the_hyper_drive_is_half_a_charge() {
        let idle = Warp::new().look();
        let h = idle.with_hyper(1.0);
        assert!(h.fov_scale > 1.3 && h.fov_scale < 1.6, "{}", h.fov_scale);
        assert!(h.charge > 0.5 && h.charge < 1.0);
        assert!(h.particles > 0.0);
        let none = idle.with_hyper(0.0);
        assert_eq!(none, idle);
        assert_eq!(idle.with_hyper(f32::NAN), idle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farfall_sim::presets;

    #[test]
    fn arrival_is_at_the_safe_distance_and_never_inside() {
        let p = presets::earth_compact();
        let s = presets::circular_orbit(&p, 12_000.0);
        for dest in Destination::ALL {
            for radii in [SAFE_RADII_MIN, 1.0, 20.0, SAFE_RADII_MAX] {
                let plan = Plan {
                    dest,
                    safe_radii: radii,
                };
                let (pos, _) = plan.arrival(&p, &s.ship, 0.0);
                let centre = dest.centre(&p, 0.0);
                let d = (pos - centre).length();
                let r = dest.radius_m(&p);
                assert!(
                    (d - r * (1.0 + radii)).abs() < 1e-3 * d,
                    "{dest:?} {radii}: {d}"
                );
                assert!(d > r, "{dest:?} inside the body");
            }
        }
    }

    #[test]
    fn every_body_is_arrived_at_in_a_circular_orbit() {
        let p = presets::earth_compact();
        let s = presets::circular_orbit(&p, 12_000.0);
        for dest in Destination::ALL {
            let plan = Plan {
                dest,
                safe_radii: 0.2,
            };
            let (pos, vel) = plan.arrival(&p, &s.ship, 0.0);
            let body = dest.body(&p, 0.0);
            let rel = pos - body.centre;
            let r = rel.length();
            assert!(
                (vel.length() - (body.mu / r).sqrt()).abs() < 1e-6,
                "{dest:?} not circular"
            );
            assert!(vel.dot(rel.normalize()).abs() < 1e-6, "{dest:?} not level");
        }
    }

    #[test]
    fn the_sun_is_far_and_big() {
        let p = presets::earth_compact();
        let s = presets::circular_orbit(&p, 12_000.0);
        let plan = Plan::default();
        let (pos, _) = plan.arrival(&p, &s.ship, 0.0);
        assert!(pos.length() > 1.0e9);
        assert!(Destination::Sun.radius_m(&p) > 100.0 * p.planet.radius_m);
    }

    #[test]
    fn the_sequence_jumps_exactly_once_at_the_flips_peak() {
        let mut w = Warp::new();
        assert!(w.engage());
        assert!(!w.engage(), "re-engaging mid-sequence must be refused");
        let mut jumps = 0;
        let mut t = 0.0;
        let mut jump_at = -1.0;
        let mut max_fov: f32 = 0.0;
        while w.active() {
            if w.update(0.01) {
                jumps += 1;
                jump_at = t;
                let l = w.look();
                assert!(
                    l.fisheye > 0.9 && l.invert > 0.9,
                    "jump while visible: {l:?}"
                );
            }
            max_fov = max_fov.max(w.look().fov_scale);
            t += 0.01;
        }
        assert_eq!(jumps, 1);
        assert!(
            (jump_at - (CHARGE_S + FLIP_S * 0.5)).abs() < 0.02,
            "{jump_at}"
        );
        assert!(max_fov > 2.0);
        assert_eq!(w.look().fov_scale, 1.0);
        assert!((t - (CHARGE_S + FLIP_S + ARRIVE_S)).abs() < 0.03, "{t}");
    }

    #[test]
    fn plan_edits_are_bounded_and_cycle() {
        let mut plan = Plan::default();
        for _ in 0..100 {
            plan.adjust_safe(true);
        }
        assert_eq!(plan.safe_radii, SAFE_RADII_MAX);
        for _ in 0..200 {
            plan.adjust_safe(false);
        }
        assert_eq!(plan.safe_radii, SAFE_RADII_MIN);
        let start = plan.dest;
        for _ in 0..Destination::ALL.len() {
            plan.cycle_destination(true);
        }
        assert_eq!(plan.dest, start);
        plan.set_safe(f64::NAN);
        assert_eq!(plan.safe_radii, Plan::default().safe_radii);
    }
}
