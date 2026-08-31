//! The camera as a thing on the pilot's head, not a tripod: under load
//! the head is pushed and lags and swings back; under thrust and g it
//! trembles; when the guns go the whole view jolts. Small, felt more
//! than watched — a helmet camera, not a gimbal.
//!
//! A damped spring in three axes (yaw, pitch, roll), driven by the felt
//! acceleration in the ship's frame, with a noise tremor on top whose
//! size follows thrust and load, and impulses from recoil. Pure: stepped
//! by the app each frame, read as a rotation in the head's frame.

use glam::{Quat, Vec3};

/// Radians of head deflection per g of sideways load, at full strength.
/// Subtle by default (a third of what it was): the helmet camera is felt,
/// not watched, and the dash must stay readable under load — CAMERA SHAKE
/// on the CABIN page goes to 200% for anyone who wants it back.
const SWAY_RAD_PER_G: f32 = 0.006;
/// Roll per g of sideways load.
const ROLL_RAD_PER_G: f32 = 0.009;
/// The spring: natural frequency (rad/s) and damping ratio. Under-damped
/// so a jolt overshoots and settles, like a neck.
const OMEGA: f32 = 9.0;
const ZETA: f32 = 0.45;
/// Tremor amplitude, radians: per unit of thrust effort, per g of load.
const TREMOR_THRUST: f32 = 0.0006;
const TREMOR_G: f32 = 0.00028;
/// A gun's kick: radians of pitch velocity impulse per m/s the ship is
/// kicked by, and its cap.
const KICK_RAD_PER_MPS: f32 = 0.22;
const KICK_MAX: f32 = 0.07;
/// The most the whole thing may deflect, radians.
const MAX_RAD: f32 = 0.12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shake {
    /// Deflection (yaw, pitch, roll), radians, and its rate.
    pos: Vec3,
    vel: Vec3,
    /// The tremor's phase, seconds.
    phase: f32,
    /// The tremor's current amplitude, smoothed.
    tremor: f32,
    /// The setting: 0 off .. 2 double.
    pub strength: f32,
}

impl Default for Shake {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            phase: 0.0,
            tremor: 0.0,
            strength: 1.0,
        }
    }
}

impl Shake {
    pub fn new(strength: f32) -> Self {
        Self {
            strength,
            ..Default::default()
        }
    }

    /// A gun fired: the ship was kicked by `kick_mps` (its speed change).
    /// The head jolts — pitch mostly, a little yaw by the side it came
    /// from (`side` -1..1).
    pub fn kick(&mut self, kick_mps: f32, side: f32) {
        let j = (kick_mps * KICK_RAD_PER_MPS).min(KICK_MAX) * self.strength;
        self.vel.y += j * 4.0;
        self.vel.x += j * 1.2 * side;
        self.vel.z -= j * 0.8 * side;
    }

    /// One frame: `g_body` is the felt load in g in the ship's frame
    /// (right, up, forward); `effort` the thrust demand 0..1.
    pub fn step(&mut self, dt: f32, g_body: [f32; 3], effort: f32) {
        let dt = dt.clamp(0.0, 0.1);
        let s = self.strength.max(0.0);
        // Where the load pushes the head: sideways g swings it the other
        // way and rolls it; up g nods it down; forward g nods it up.
        let target = Vec3::new(
            -g_body[0] * SWAY_RAD_PER_G,
            (-g_body[1] * 0.6 + g_body[2] * 0.4) * SWAY_RAD_PER_G,
            -g_body[0] * ROLL_RAD_PER_G,
        ) * s;
        // Semi-implicit spring toward it.
        let acc = (target - self.pos) * (OMEGA * OMEGA) - self.vel * (2.0 * ZETA * OMEGA);
        self.vel += acc * dt;
        self.pos += self.vel * dt;
        // The tremor: its amplitude eases toward what the load asks.
        let g = (g_body[0] * g_body[0] + g_body[1] * g_body[1] + g_body[2] * g_body[2]).sqrt();
        let want = (TREMOR_THRUST * effort.clamp(0.0, 1.0) + TREMOR_G * g) * s;
        self.tremor += (want - self.tremor) * (1.0 - (-dt * 6.0).exp());
        self.phase += dt;
    }

    /// The deflection this frame, in the head's frame.
    pub fn rotation(&self) -> Quat {
        let t = self.phase;
        // Three incommensurate sines per axis: a jitter that never repeats.
        let jit = |a: f32, b: f32, c: f32| {
            ((t * a).sin() + (t * b + 1.3).sin() * 0.6 + (t * c + 2.1).sin() * 0.35) / 1.95
        };
        let n = Vec3::new(
            jit(41.0, 67.0, 9.0),
            jit(53.0, 29.0, 7.0),
            jit(23.0, 71.0, 5.0),
        ) * self.tremor;
        let d = (self.pos + n).clamp(Vec3::splat(-MAX_RAD), Vec3::splat(MAX_RAD));
        Quat::from_rotation_y(d.x) * Quat::from_rotation_x(d.y) * Quat::from_rotation_z(d.z)
    }

    /// How far the view is deflected right now, radians.
    #[cfg(test)]
    pub fn amount(&self) -> f32 {
        self.pos.length() + self.tremor
    }

    /// The bench: park the head at a deflection, for a capture.
    pub fn park(&mut self, yaw: f32, pitch: f32, roll: f32) {
        self.pos = Vec3::new(yaw, pitch, roll);
        self.vel = Vec3::ZERO;
        self.tremor = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut Shake, secs: f32, g: [f32; 3], effort: f32) {
        let n = (secs / 0.005) as usize;
        for _ in 0..n {
            s.step(0.005, g, effort);
        }
    }

    #[test]
    fn still_and_unloaded_the_camera_is_still() {
        let mut s = Shake::default();
        run(&mut s, 2.0, [0.0; 3], 0.0);
        assert!(s.amount() < 1e-6);
        assert!(s.rotation().angle_between(Quat::IDENTITY) < 1e-5);
    }

    #[test]
    fn a_sideways_load_sways_the_head_the_other_way_and_it_settles() {
        let mut s = Shake::default();
        run(&mut s, 0.12, [4.0, 0.0, 0.0], 0.0);
        let early = s.pos.x;
        run(&mut s, 3.0, [4.0, 0.0, 0.0], 0.0);
        let target = -4.0 * SWAY_RAD_PER_G;
        assert!(
            (s.pos.x - target).abs() < 1e-3,
            "settled at {} for {target}",
            s.pos.x
        );
        assert!(early > target, "still on its way at first: {early}");
        assert!(s.pos.z < 0.0, "rolled with it");
        // Off the load: it comes back, with a little overshoot.
        let mut peak = 0.0f32;
        for _ in 0..600 {
            s.step(0.005, [0.0; 3], 0.0);
            peak = peak.max(s.pos.x);
        }
        assert!(peak > 0.0, "overshoots past centre on the way back");
        assert!(s.pos.x.abs() < 1e-3);
    }

    #[test]
    fn thrust_and_load_bring_a_tremor_and_the_setting_scales_everything() {
        let mut s = Shake::default();
        run(&mut s, 1.0, [0.0, 0.0, 1.5], 1.0);
        let full = s.amount();
        assert!(s.tremor > 0.0006, "{}", s.tremor);
        let mut half = Shake::new(0.5);
        run(&mut half, 1.0, [0.0, 0.0, 1.5], 1.0);
        assert!((half.amount() * 2.0 - full).abs() < 1e-4);
        let mut off = Shake::new(0.0);
        run(&mut off, 1.0, [3.0, 2.0, 1.5], 1.0);
        off.kick(7.0, 0.0);
        run(&mut off, 0.1, [3.0, 2.0, 1.5], 1.0);
        assert!(off.amount() < 1e-6, "off is off");
        // The rotation actually moves frame to frame under tremor.
        let a = s.rotation();
        s.step(0.02, [0.0, 0.0, 1.5], 1.0);
        assert!(s.rotation().angle_between(a) > 1e-5);
    }

    #[test]
    fn a_gun_kicks_the_view_the_rail_more_and_it_is_capped() {
        let mut cannon = Shake::default();
        cannon.kick(0.07, 1.0);
        let mut rail = Shake::default();
        rail.kick(7.0, 0.0);
        let mut silly = Shake::default();
        silly.kick(700.0, 0.0);
        assert!(rail.vel.y > cannon.vel.y * 3.0);
        assert_eq!(rail.vel.y, silly.vel.y, "capped");
        assert!(cannon.vel.x > 0.0, "the right wing's gun yaws it a little");
        // The jolt is a bounce: up, then back near centre within a second.
        let mut peak = 0.0f32;
        for _ in 0..200 {
            rail.step(0.005, [0.0; 3], 0.0);
            peak = peak.max(rail.pos.y);
        }
        assert!(peak > 0.01 && peak <= MAX_RAD, "{peak}");
        assert!(
            rail.pos.y.abs() < peak * 0.3,
            "settling: {} after a peak of {peak}",
            rail.pos.y
        );
    }
}
