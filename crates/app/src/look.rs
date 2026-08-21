//! Freelook: the pilot's head, separate from the ship's nose.
//!
//! Yaw and pitch in the ship's frame, driven by the mouse or trackpad while
//! the look is engaged — held (right button) or locked (L) — and easing
//! back to centre when it is released. The camera is the ship's attitude
//! times this; the controls never see it. It is the start of a cockpit you
//! can turn your head in.

use glam::Quat;

/// How far the head turns, radians: well past the shoulders sideways,
/// short of straight up.
const YAW_MAX: f32 = 2.6;
const PITCH_MAX: f32 = 1.4;
/// Radians per mouse count at sensitivity 1.
const RAD_PER_COUNT: f32 = 0.0025;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Where the head is, smoothed.
    yaw: f32,
    pitch: f32,
    /// Where the mouse has put it.
    target_yaw: f32,
    target_pitch: f32,
    /// Right button held.
    held: bool,
    /// Toggled on with L.
    locked: bool,
    pub sensitivity: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            target_yaw: 0.0,
            target_pitch: 0.0,
            held: false,
            locked: false,
            sensitivity: 1.0,
        }
    }
}

impl Look {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn engaged(&self) -> bool {
        self.held || self.locked
    }

    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// L: lock the look on, or release it (and let it recentre).
    pub fn toggle_lock(&mut self) {
        self.locked = !self.locked;
    }

    /// Mouse motion in counts. Ignored unless engaged, so a stray trackpad
    /// brush in flight moves nothing. Mouse right = look right; mouse up =
    /// look up.
    pub fn motion(&mut self, dx: f32, dy: f32) {
        if !self.engaged() {
            return;
        }
        let k = RAD_PER_COUNT * self.sensitivity.clamp(0.1, 5.0);
        self.target_yaw = (self.target_yaw + dx * k).clamp(-YAW_MAX, YAW_MAX);
        self.target_pitch = (self.target_pitch - dy * k).clamp(-PITCH_MAX, PITCH_MAX);
    }

    /// Ease toward the mouse while engaged, or back to centre when not.
    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.25);
        if !self.engaged() {
            self.target_yaw = 0.0;
            self.target_pitch = 0.0;
        }
        let tau = if self.engaged() { 0.04 } else { 0.22 };
        let a = 1.0 - (-dt / tau).exp();
        self.yaw += (self.target_yaw - self.yaw) * a;
        self.pitch += (self.target_pitch - self.pitch) * a;
        if !self.engaged() && self.yaw.abs() < 1e-4 && self.pitch.abs() < 1e-4 {
            self.yaw = 0.0;
            self.pitch = 0.0;
        }
    }

    /// Head rotation in the ship's frame: yaw about the ship's up, then
    /// pitch about the turned right axis. Looking right is a NEGATIVE
    /// rotation about +Y (the nose is −Z: rotating −Z about +Y by a
    /// negative angle swings it toward +X).
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(-self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    /// How far things fixed to the glass shift on screen, in NDC, for a
    /// camera with this tan(fov/2) and aspect: look right and the dials
    /// slide left.
    pub fn glass_shift(&self, tan_half_fov: f32, aspect: f32) -> [f32; 2] {
        [
            -self.yaw.tan().clamp(-4.0, 4.0) / (tan_half_fov * aspect),
            -self.pitch.tan().clamp(-4.0, 4.0) / tan_half_fov,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn mouse_does_nothing_unless_engaged() {
        let mut l = Look::new();
        l.motion(100.0, 50.0);
        l.update(0.1);
        assert_eq!((l.yaw, l.pitch), (0.0, 0.0));
        l.set_held(true);
        l.motion(100.0, 0.0);
        for _ in 0..50 {
            l.update(0.05);
        }
        assert!(l.yaw > 0.2);
    }

    #[test]
    fn looking_right_swings_the_view_right_and_the_glass_left() {
        let mut l = Look::new();
        l.set_held(true);
        l.motion(200.0, 0.0);
        for _ in 0..50 {
            l.update(0.05);
        }
        let forward = l.rotation() * Vec3::NEG_Z;
        assert!(forward.x > 0.0, "{forward}");
        assert!(l.glass_shift(0.7, 1.6)[0] < 0.0);
        // Mouse up looks up.
        l.motion(0.0, -200.0);
        for _ in 0..50 {
            l.update(0.05);
        }
        assert!((l.rotation() * Vec3::NEG_Z).y > 0.0);
    }

    #[test]
    fn release_recentres_and_limits_hold() {
        let mut l = Look::new();
        l.set_held(true);
        l.motion(1.0e6, 1.0e6);
        for _ in 0..100 {
            l.update(0.05);
        }
        assert!(l.yaw.abs() <= YAW_MAX + 1e-6 && l.pitch.abs() <= PITCH_MAX + 1e-6);
        l.set_held(false);
        for _ in 0..200 {
            l.update(0.05);
        }
        assert_eq!((l.yaw, l.pitch), (0.0, 0.0));
    }

    #[test]
    fn lock_keeps_looking_without_the_button() {
        let mut l = Look::new();
        l.toggle_lock();
        l.motion(50.0, 0.0);
        for _ in 0..50 {
            l.update(0.05);
        }
        assert!(l.yaw > 0.05);
        l.toggle_lock();
        for _ in 0..200 {
            l.update(0.05);
        }
        assert_eq!(l.yaw, 0.0);
    }
}
