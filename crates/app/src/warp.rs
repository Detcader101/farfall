//! The wormhole drive: a jump to a chosen body at a chosen safe distance,
//! and the sequence the pilot sees on the way.
//!
//! Nothing outruns light in this sim (see `light_limit`), so the way to
//! the Sun is not through speed: the drive folds the distance. The sequence
//! is four phases flowing into each other over eight seconds (the
//! `warp.length` setting scales the whole of it, 50–200%) — CHARGE, space
//! starting to stretch, the stars drawing into threads, a liquid shimmer
//! building at the rim; PULL, the picture warping harder, the chromatic
//! split growing, the view folding toward the nose; FLIP, the world turned
//! inside out through a mirror sphere and inverted, long and liquid, at
//! whose peak the one jump happens; ARRIVE, the destination pouring in as
//! fluid — the new sky un-warping through settling ripples and chromatic
//! fringes to stillness — and then the ship is where the map said, at the
//! distance the pilot set, never inside anything.

use farfall_sim::{Body, ShipState, WorldParams};
use glam::DVec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Planet,
    Moon,
    Sun,
    Uranus,
}

/// Uranus' ring — the asteroid belt lives in it (crates/app/src/belt.rs);
/// fly to Uranus and on into the ring and it is there. Its plane is
/// normal to Uranus' spin axis (bodies.wgsl has the same numbers), from
/// RING_INNER to RING_OUTER radii, RING_HALF_M thick.
pub const RING_AXIS: DVec3 = DVec3::new(0.97, 0.14, 0.2);
pub const RING_INNER: f64 = 1.62;
pub const RING_OUTER: f64 = 1.98;
pub const RING_MID: f64 = 1.80;
pub const RING_HALF_M: f64 = 900.0;

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

    /// The body's own velocity in the planet's frame at sim time `t`.
    pub fn velocity(self, params: &WorldParams, t_s: f64) -> DVec3 {
        let vels = params.body_velocities(t_s);
        let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
        vels[i]
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

    /// Distance from the body's surface, metres. Uranus is arrived at in
    /// its ring whatever the plan says: the belt's middle radius.
    pub fn safe_m(&self, params: &WorldParams) -> f64 {
        if self.dest == Destination::Uranus {
            return (RING_MID - 1.0) * self.dest.radius_m(params);
        }
        self.safe_radii * self.dest.radius_m(params)
    }

    /// Where the jump lands: on the line from the body toward where the
    /// ship is now (so the old place is behind you), at surface + safe
    /// distance — and in a circular orbit about it, prograde along the
    /// ship's heading projected level, riding along with the body itself.
    /// Every body pulls, so every body is arrived at the same way: the
    /// drive never drops you into a fall.
    ///
    /// Uranus is the exception in where, not how: the drive lands you IN
    /// the asteroid belt — on the ring's middle radius, in its plane, on
    /// the side of Uranus you came from, going round with the rocks at
    /// exactly their speed and way. A perfect orbit of the belt: the rocks
    /// about you hang still.
    pub fn arrival(&self, params: &WorldParams, ship: &ShipState, t_s: f64) -> (DVec3, DVec3) {
        let body = self.dest.body(params, t_s);
        let body_vel = self.dest.velocity(params, t_s);
        let mut away = ship.pos_m - body.centre;
        if away.length() < 1.0 {
            away = DVec3::X;
        }
        let away = away.normalize();
        if self.dest == Destination::Uranus {
            let (e1, _, axis) = crate::belt::ring_frame();
            let mut radial = away - axis * away.dot(axis);
            if radial.length() < 1e-6 {
                radial = e1;
            }
            let radial = radial.normalize();
            let r = body.radius_m * RING_MID;
            let pos = body.centre + radial * r;
            // The ring's own motion at its middle (belt::cell_rocks).
            let vel = body_vel + axis.cross(radial) * (body.mu / r).sqrt();
            return (pos, vel);
        }
        let r = body.radius_m + self.safe_m(params);
        let pos = body.centre + away * r;
        let nose = ship.orient * DVec3::NEG_Z;
        let mut tangent = nose - away * nose.dot(away);
        if tangent.length() < 1e-6 {
            tangent = away.cross(DVec3::Y);
        }
        let vel = body_vel + tangent.normalize() * (body.mu / r).sqrt();
        (pos, vel)
    }
}

/// The sequence, in seconds, at stock length (`warp.length` = 100%).
pub const CHARGE_S: f32 = 2.0;
pub const PULL_S: f32 = 2.0;
pub const FLIP_S: f32 = 1.5;
pub const ARRIVE_S: f32 = 2.5;
/// The whole of it: eight seconds stock.
pub const SEQUENCE_S: f32 = CHARGE_S + PULL_S + FLIP_S + ARRIVE_S;
/// What `warp.length` may scale the sequence by: half to double.
pub const LENGTH_MIN: f32 = 0.5;
pub const LENGTH_MAX: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Charge,
    Pull,
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
    /// 0..1: the liquid field on the picture (and the gassy particles).
    pub particles: f32,
    /// 0..1: how charged the drive is (for the sound, and a glow).
    pub charge: f32,
    /// 0..1: space drawing into threads — the stars streak away from the
    /// centre of the view, and the picture streaks with them.
    pub stretch: f32,
    /// 0..1: the view folding in toward the nose, on the way to the flip.
    pub pull: f32,
    /// 0..1: the destination pouring in as fluid — settling rings and
    /// chromatic fringes easing to stillness.
    pub reform: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Warp {
    phase: Phase,
    t: f32,
    jumped: bool,
    /// The `warp.length` setting: every phase scaled by this.
    length: f32,
}

impl Default for Warp {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            t: 0.0,
            jumped: false,
            length: 1.0,
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
        // A touch wider, no more: the speed is in the picture, not the lens.
        self.fov_scale *= 1.0 + 0.12 * h;
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

    #[cfg(test)]
    fn phase(&self) -> Phase {
        self.phase
    }

    /// The `warp.length` setting, 50–200%: a scale on every phase.
    /// Ignored mid-sequence, so a menu edit cannot fold a running jump.
    pub fn set_length(&mut self, x: f32) {
        if self.active() {
            return;
        }
        self.length = if x.is_finite() {
            x.clamp(LENGTH_MIN, LENGTH_MAX)
        } else {
            1.0
        };
    }

    /// A phase's duration at the set length.
    fn dur(&self, base: f32) -> f32 {
        base * self.length
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
                if self.t >= self.dur(CHARGE_S) {
                    self.phase = Phase::Pull;
                    self.t -= self.dur(CHARGE_S);
                }
                false
            }
            Phase::Pull => {
                if self.t >= self.dur(PULL_S) {
                    self.phase = Phase::Flip;
                    self.t -= self.dur(PULL_S);
                }
                false
            }
            Phase::Flip => {
                let jump = !self.jumped && self.t >= self.dur(FLIP_S) * 0.5;
                if jump {
                    self.jumped = true;
                }
                if self.t >= self.dur(FLIP_S) {
                    self.phase = Phase::Arrive;
                    self.t -= self.dur(FLIP_S);
                }
                jump
            }
            Phase::Arrive => {
                if self.t >= self.dur(ARRIVE_S) {
                    self.phase = Phase::Idle;
                    self.t = 0.0;
                }
                false
            }
        }
    }

    /// Every phase's look flows into the next without a seam: each field
    /// at a phase's end equals its value at the next phase's start (the
    /// `never_seams` test walks the whole sequence).
    pub fn look(&self) -> Look {
        match self.phase {
            Phase::Idle => Look {
                fov_scale: 1.0,
                ..Default::default()
            },
            Phase::Charge => {
                // Space starts to stretch: the stars draw into threads and
                // a liquid shimmer builds at the rim. The lens itself only
                // eases a touch wider — the stretch does the talking.
                let f = smooth(self.t / self.dur(CHARGE_S));
                Look {
                    fov_scale: 1.0 + 0.10 * f,
                    fisheye: 0.0,
                    invert: 0.0,
                    particles: 0.25 * f,
                    charge: f,
                    stretch: 0.6 * f,
                    pull: 0.0,
                    reform: 0.0,
                }
            }
            Phase::Pull => {
                // The picture warps harder: the chromatic split grows and
                // the view begins to fold toward the nose.
                let f = smooth(self.t / self.dur(PULL_S));
                Look {
                    fov_scale: 1.10 + 0.25 * f,
                    fisheye: 0.0,
                    invert: 0.0,
                    particles: 0.25 + 0.35 * f,
                    charge: 1.0,
                    stretch: 0.6 + 0.4 * f,
                    pull: 0.7 * f,
                    reform: 0.0,
                }
            }
            Phase::Flip => {
                let f = (self.t / self.dur(FLIP_S)).clamp(0.0, 1.0);
                // A bell over the flip: fully inside out at its middle,
                // where the one jump happens and nobody can see the seam.
                let bell = (std::f32::consts::PI * f).sin();
                let out = smooth(f);
                Look {
                    fov_scale: 1.35,
                    fisheye: smooth(bell * 1.3),
                    invert: smooth((bell - 0.35) / 0.5),
                    particles: 0.6 + 0.3 * bell,
                    charge: 1.0,
                    stretch: 1.0 - out,
                    pull: 0.7 * (1.0 - out),
                    // The far half of the turn: the new place already
                    // pouring in behind the inversion.
                    reform: smooth((f - 0.5) * 2.0),
                }
            }
            Phase::Arrive => {
                // The destination pours in as fluid: the new sky un-warps
                // through settling rings and fringes to stillness.
                let f = (self.t / self.dur(ARRIVE_S)).clamp(0.0, 1.0);
                Look {
                    // Snap back over the first third, then settle.
                    fov_scale: 1.0 + 0.35 * (1.0 - smooth(f * 3.0)),
                    fisheye: 0.0,
                    invert: 0.0,
                    particles: 0.6 * (1.0 - smooth(f)),
                    charge: 1.0 - smooth(f * 2.0),
                    stretch: 0.0,
                    pull: 0.0,
                    reform: 1.0 - smooth(f),
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
        for _ in 0..450 {
            if w.update(0.01) {
                jumped += 1;
            }
        }
        assert_eq!(jumped, 1);
        assert!(!w.active(), "flip and arrive are over inside 4.5 seconds");
    }

    #[test]
    fn the_hyper_drive_is_half_a_charge() {
        let idle = Warp::new().look();
        let h = idle.with_hyper(1.0);
        assert!(h.fov_scale > 1.05 && h.fov_scale < 1.2, "{}", h.fov_scale);
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
                // Uranus is arrived at in its ring, whatever the plan says.
                let want = if dest == Destination::Uranus {
                    r * RING_MID
                } else {
                    r * (1.0 + radii)
                };
                assert!((d - want).abs() < 1e-3 * d, "{dest:?} {radii}: {d}");
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
            // Circular about the body — in the body's own frame, so a
            // moving body is arrived at riding along with it.
            let vel = vel - dest.velocity(&p, 0.0);
            assert!(
                (vel.length() - (body.mu / r).sqrt()).abs() < 1e-6,
                "{dest:?} not circular"
            );
            assert!(vel.dot(rel.normalize()).abs() < 1e-6, "{dest:?} not level");
        }
    }

    /// Uranus: the drive lands you in the belt — the ring's middle radius,
    /// in its plane, going round with the rocks at their exact speed and
    /// way, so the rocks about you hang still.
    #[test]
    fn uranus_is_arrived_at_in_the_belt_riding_with_the_rocks() {
        let p = presets::earth_compact();
        let s = presets::circular_orbit(&p, 12_000.0);
        let plan = Plan {
            dest: Destination::Uranus,
            safe_radii: 20.0,
        };
        let t = 1234.5;
        let (pos, vel) = plan.arrival(&p, &s.ship, t);
        let uranus = Destination::Uranus.body(&p, t);
        let (_, _, axis) = crate::belt::ring_frame();
        let rel = pos - uranus.centre;
        assert!(
            rel.dot(axis).abs() < 1e-3,
            "in the ring's plane: {}",
            rel.dot(axis)
        );
        assert!(
            (rel.length() - uranus.radius_m * RING_MID).abs() < 1e-3,
            "on the ring's middle radius"
        );
        // The ring turns rigidly at the rate of its middle: a rock at the
        // arrival point moves at exactly this.
        let ring_vel = Destination::Uranus.velocity(&p, t)
            + axis.cross(rel) * crate::belt::ring_rate_radps(&uranus);
        assert!(
            (vel - ring_vel).length() < 1e-6,
            "riding with the rocks: {vel:?} vs {ring_vel:?}"
        );
        // And the map's arrival marker agrees: safe_m puts it in the ring.
        assert!(
            (uranus.radius_m + plan.safe_m(&p) - rel.length()).abs() < 1e-3,
            "the plan's arrival distance is the ring's"
        );
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
            (jump_at - (CHARGE_S + PULL_S + FLIP_S * 0.5)).abs() < 0.02,
            "{jump_at}"
        );
        assert!(
            max_fov > 1.25 && max_fov < 1.5,
            "a touch wider, no more: {max_fov}"
        );
        assert_eq!(w.look().fov_scale, 1.0);
        assert!((t - SEQUENCE_S).abs() < 0.03, "{t}");
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

    #[test]
    fn the_sequence_is_eight_seconds_and_the_length_setting_scales_it() {
        assert_eq!(SEQUENCE_S, 8.0, "stock is eight seconds");
        for len in [LENGTH_MIN, 1.0, LENGTH_MAX] {
            let mut w = Warp::new();
            w.set_length(len);
            assert!(w.engage());
            // A menu edit mid-jump changes nothing.
            w.set_length(1.0);
            let mut t = 0.0;
            let mut jump_at = -1.0;
            while w.active() {
                if w.update(0.01) {
                    jump_at = t;
                }
                t += 0.01;
            }
            assert!((t - SEQUENCE_S * len).abs() < 0.03, "{len}: {t}");
            assert!(
                (jump_at - (CHARGE_S + PULL_S + FLIP_S * 0.5) * len).abs() < 0.02,
                "the jump scales with the length: {len}: {jump_at}"
            );
        }
        let mut w = Warp::new();
        w.set_length(f32::NAN);
        assert!(w.engage());
        let mut t = 0.0;
        while w.active() {
            w.update(0.01);
            t += 0.01;
        }
        assert!((t - SEQUENCE_S).abs() < 0.03, "a NaN length is stock: {t}");
    }

    #[test]
    fn the_phases_flow_in_order_and_every_one_is_seen() {
        let mut w = Warp::new();
        assert!(w.engage());
        let order = |p: Phase| match p {
            Phase::Charge => 0,
            Phase::Pull => 1,
            Phase::Flip => 2,
            Phase::Arrive => 3,
            Phase::Idle => 4,
        };
        let mut last = 0;
        let mut seen = [false; 4];
        while w.active() {
            let o = order(w.phase());
            if o < 4 {
                seen[o] = true;
            }
            assert!(o >= last, "phases never run backwards");
            last = o;
            w.update(0.01);
        }
        assert_eq!(seen, [true; 4], "charge, pull, flip and arrive all seen");
    }

    #[test]
    fn the_look_flows_between_phases_without_a_seam() {
        let mut w = Warp::new();
        assert!(w.engage());
        let mut prev = w.look();
        while w.active() {
            w.update(0.01);
            let l = w.look();
            for (a, b, name) in [
                (prev.fov_scale, l.fov_scale, "fov"),
                (prev.fisheye, l.fisheye, "fisheye"),
                (prev.invert, l.invert, "invert"),
                (prev.particles, l.particles, "particles"),
                (prev.charge, l.charge, "charge"),
                (prev.stretch, l.stretch, "stretch"),
                (prev.pull, l.pull, "pull"),
                (prev.reform, l.reform, "reform"),
            ] {
                assert!((a - b).abs() < 0.1, "{name} jumps {a} -> {b}");
            }
            prev = l;
        }
        // And the sequence ends still: the idle look.
        assert_eq!(
            w.look(),
            Look {
                fov_scale: 1.0,
                ..Default::default()
            }
        );
    }
}
