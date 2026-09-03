//! Freelook: the pilot's head, separate from the ship's nose.
//!
//! Yaw and pitch in the ship's frame, driven by the mouse or trackpad while
//! the look is engaged — held (right button) or locked (L) — and easing
//! back to centre when it is released. The camera is the ship's attitude
//! times this; the controls never see it. It is the start of a cockpit you
//! can turn your head in.

use glam::{Quat, Vec2, Vec3};

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

    /// Lock the look and put the head straight at an angle, radians — for
    /// the bench, which has no mouse.
    pub fn aim(&mut self, yaw: f32, pitch: f32) {
        self.locked = true;
        self.target_yaw = yaw.clamp(-YAW_MAX, YAW_MAX);
        self.target_pitch = pitch.clamp(-PITCH_MAX, PITCH_MAX);
        self.yaw = self.target_yaw;
        self.pitch = self.target_pitch;
    }

    /// Lock the look and put the head at any yaw at all (the full circle,
    /// past the shoulder limits) — for the bench's spin, which looks round
    /// the whole cabin.
    pub fn aim_free(&mut self, yaw: f32, pitch: f32) {
        self.locked = true;
        self.target_yaw = yaw;
        self.target_pitch = pitch.clamp(-PITCH_MAX, PITCH_MAX);
        self.yaw = self.target_yaw;
        self.pitch = self.target_pitch;
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

    /// Where a point fixed to the glass — given as the NDC it occupies with
    /// the head centred — lands on screen with the head turned. The glass
    /// is a sphere around the pilot's head, so this is a rotation of the
    /// point's direction and a perspective projection, not a slide: a dial
    /// at the rim swings through a wider arc than one at the centre, and
    /// all of them keep their places relative to each other. Points turned
    /// behind the head are pushed far off screen.
    #[cfg(test)]
    pub fn reproject(&self, ndc: [f32; 2], tan_half_fov: f32, aspect: f32) -> [f32; 2] {
        self.reproject_from(ndc, tan_half_fov, tan_half_fov, aspect)
    }

    /// The same, for a point given in a REFERENCE projection (the glass is
    /// laid out at the pilot's base field of view) and shown through the
    /// live one (thrust widens it, the drive flares it): the point is a
    /// direction fixed to the ship, and only its screen place changes.
    /// Test-only: production code goes through the free function
    /// [`reproject_with`] instead, since VR needs to reproject against a
    /// headset's own head rather than a mouse-driven `Look`.
    #[cfg(test)]
    pub fn reproject_from(
        &self,
        ndc: [f32; 2],
        tan_half_ref: f32,
        tan_half_fov: f32,
        aspect: f32,
    ) -> [f32; 2] {
        reproject_with(self.rotation(), ndc, tan_half_ref, tan_half_fov, aspect)
    }

    /// A screen point (live NDC, this camera's tan(fov/2)) back to the
    /// laid-out glass (reference NDC): the inverse of [`Look::reproject_from`].
    pub fn glass_point(
        &self,
        ndc: [f32; 2],
        tan_half_fov: f32,
        tan_half_ref: f32,
        aspect: f32,
    ) -> [f32; 2] {
        let t = tan_half_fov.max(1e-4);
        let tr = tan_half_ref.max(1e-4);
        let d = Vec3::new(ndc[0] * aspect * t, ndc[1] * t, -1.0).normalize();
        let v = self.rotation() * d;
        let depth = (-v.z).max(0.02);
        [
            (v.x / (depth * tr * aspect)).clamp(-4.0, 4.0),
            (v.y / (depth * tr)).clamp(-4.0, 4.0),
        ]
    }

    /// The point on the glass now under the centre of the screen — where
    /// the pilot is looking — as glass NDC. The inverse of [`Look::reproject`]
    /// at the origin.
    pub fn gaze(&self, tan_half_fov: f32, aspect: f32) -> [f32; 2] {
        let t = tan_half_fov.max(1e-4);
        let d = self.rotation() * Vec3::NEG_Z;
        let depth = (-d.z).max(0.02);
        [
            (d.x / (depth * t * aspect)).clamp(-4.0, 4.0),
            (d.y / (depth * t)).clamp(-4.0, 4.0),
        ]
    }
}

/// [`Look::reproject_from`]'s own maths, against an arbitrary head
/// rotation instead of a mouse-driven [`Look`]'s: the glass is a sphere
/// around whichever head is looking through it, and a headset's is a
/// real 3-DOF orientation `Look` (yaw/pitch only) cannot represent. A
/// point fixed to the glass in a REFERENCE projection (the pilot's base
/// field of view, head centred) lands here in the live one.
pub fn reproject_with(
    head: Quat,
    ndc: [f32; 2],
    tan_half_ref: f32,
    tan_half_fov: f32,
    aspect: f32,
) -> [f32; 2] {
    let tr = tan_half_ref.max(1e-4);
    let t = tan_half_fov.max(1e-4);
    let d = Vec3::new(ndc[0] * aspect * tr, ndc[1] * tr, -1.0).normalize();
    let v = head.inverse() * d;
    let depth = -v.z;
    if depth < 0.02 {
        let off = Vec2::new(v.x, v.y).normalize_or_zero() * 50.0;
        return [off.x, off.y];
    }
    [v.x / (depth * t * aspect), v.y / (depth * t)]
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(l.reproject([0.0, 0.0], 0.7, 1.6)[0] < 0.0);
        // Mouse up looks up.
        l.motion(0.0, -200.0);
        for _ in 0..50 {
            l.update(0.05);
        }
        assert!((l.rotation() * Vec3::NEG_Z).y > 0.0);
    }

    fn turned(dx: f32, dy: f32) -> Look {
        let mut l = Look::new();
        l.set_held(true);
        l.motion(dx, dy);
        for _ in 0..100 {
            l.update(0.05);
        }
        l
    }

    #[test]
    fn reprojection_is_identity_at_rest_and_matches_the_shift_at_centre() {
        let l = Look::new();
        for p in [[0.0, 0.0], [0.7, -0.6], [-0.9, 0.02]] {
            let q = l.reproject(p, 0.6, 1.5);
            assert!((q[0] - p[0]).abs() < 1e-5 && (q[1] - p[1]).abs() < 1e-5);
        }
        // A pure yaw slides the centre of the glass by tan(yaw) in view
        // units; a pure pitch likewise.
        let l = turned(150.0, 0.0);
        let centre = l.reproject([0.0, 0.0], 0.6, 1.5);
        let want = -l.yaw.tan() / (0.6 * 1.5);
        assert!(
            (centre[0] - want).abs() < 1e-4 && centre[1].abs() < 1e-5,
            "{centre:?}"
        );
        let l = turned(0.0, -80.0);
        let centre = l.reproject([0.0, 0.0], 0.6, 1.5);
        let want = -l.pitch.tan() / 0.6;
        assert!(
            (centre[1] - want).abs() < 1e-4 && centre[0].abs() < 1e-5,
            "{centre:?}"
        );
    }

    #[test]
    fn the_rim_swings_further_than_the_centre_and_stays_ordered() {
        // Look right: everything slides left, and a dial on the right rim
        // (now nearer the centre of view) moves less in angle but its
        // neighbours keep their order left-to-right.
        let l = turned(200.0, 0.0);
        let a = l.reproject([-0.8, 0.0], 0.6, 1.5);
        let b = l.reproject([0.0, 0.0], 0.6, 1.5);
        let c = l.reproject([0.8, 0.0], 0.6, 1.5);
        assert!(a[0] < b[0] && b[0] < c[0], "{a:?} {b:?} {c:?}");
        assert!(b[0] < 0.0);
        // Perspective: the far-left one, swung toward the edge of vision,
        // has moved further in NDC than the centre one.
        assert!((a[0] + 0.8).abs() > (b[0]).abs(), "{a:?} {b:?}");
    }

    #[test]
    fn a_wider_fov_pulls_a_ship_fixed_point_toward_the_centre() {
        let l = Look::new();
        let narrow = l.reproject_from([0.6, -0.4], 0.5, 0.5, 1.5);
        let wide = l.reproject_from([0.6, -0.4], 0.5, 0.8, 1.5);
        assert!((narrow[0] - 0.6).abs() < 1e-5 && (narrow[1] + 0.4).abs() < 1e-5);
        assert!(
            wide[0] < 0.6 && wide[0] > 0.0 && wide[1] > -0.4 && wide[1] < 0.0,
            "{wide:?}"
        );
        assert!((wide[0] - 0.6 * 0.5 / 0.8).abs() < 1e-5);
    }

    #[test]
    fn a_screen_point_maps_back_to_the_glass_it_came_from() {
        let l = turned(90.0, -40.0);
        for p in [[0.3, -0.5], [-0.7, 0.2]] {
            let screen = l.reproject_from(p, 0.7, 0.5, 1.6);
            let back = l.glass_point(screen, 0.5, 0.7, 1.6);
            assert!(
                (back[0] - p[0]).abs() < 1e-4 && (back[1] - p[1]).abs() < 1e-4,
                "{back:?}"
            );
        }
    }

    #[test]
    fn the_gaze_is_where_the_head_points_and_inverts_the_reprojection() {
        let l = Look::new();
        assert_eq!(l.gaze(0.6, 1.5), [0.0, 0.0]);
        let l = turned(120.0, -60.0);
        let g = l.gaze(0.6, 1.5);
        // Looking right and up: the glass point under the centre is to the
        // right and above.
        assert!(g[0] > 0.0 && g[1] > 0.0, "{g:?}");
        let back = l.reproject(g, 0.6, 1.5);
        assert!(back[0].abs() < 1e-4 && back[1].abs() < 1e-4, "{back:?}");
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
