//! Attitude instruments: the gyro ball (`gyro.wgsl`) and the head-up
//! horizon (`horizon.wgsl`). Both are [`InstrumentPass`]es; the numbers
//! come from the app, which owns the sim state.

use crate::instrument::InstrumentPass;
use crate::CameraFrame;
use glam::{Quat, Vec3};

/// The ship's attitude against gravity, radians.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Attitude {
    /// Nose above the level plane.
    pub pitch: f32,
    /// Right wing down positive.
    pub roll: f32,
    /// Prograde azimuth relative to the nose across the level plane, to
    /// the right positive.
    pub drift: f32,
}

impl Attitude {
    /// From the ship's orientation (body→world, nose −Z, up +Y), the local
    /// gravitational up, and the velocity — all world frame.
    pub fn from_world(orient: Quat, up_world: Vec3, vel_world: Vec3) -> Self {
        let up = up_world.normalize_or_zero();
        let up_b = orient.conjugate() * up;
        let pitch = (-up_b.z).clamp(-1.0, 1.0).asin();
        // Rolling right tips the hull's up to the right, so the world's up
        // appears tipped LEFT in the body frame: negate.
        let roll = (-up_b.x).atan2(up_b.y);
        let nose = orient * Vec3::NEG_Z;
        let h_nose = nose - up * nose.dot(up);
        let h_vel = vel_world - up * vel_world.dot(up);
        let drift = if h_nose.length_squared() > 1e-9 && h_vel.length_squared() > 1e-6 {
            // Positive when the velocity is clockwise from the nose seen
            // from above: to the right.
            let s = h_nose.cross(h_vel).dot(up);
            (-s).atan2(h_nose.dot(h_vel))
        } else {
            0.0
        };
        Self { pitch, roll, drift }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GyroUniforms {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
    place: crate::cabin::Placement,
}

impl GyroUniforms {
    /// JET: a solid shaded ball in its bowl.
    pub fn jet(mut self, jet: bool) -> Self {
        self.d[0] = if jet { 1.0 } else { 0.0 };
        self
    }

    /// Set into the dash: the ball drawn in the dash's plane.
    pub fn placed(mut self, place: Option<crate::cabin::Placement>) -> Self {
        self.place = place.unwrap_or(crate::cabin::Placement::GLASS);
        self
    }
}

impl GyroUniforms {
    pub fn new(
        att: Attitude,
        visibility: f32,
        aspect: f32,
        height_px: f32,
        anchor_ndc: [f32; 2],
        sway: [f32; 2],
        time_s: f32,
    ) -> Self {
        Self {
            a: [att.pitch, att.roll, visibility.clamp(0.0, 1.0), aspect],
            b: [att.drift, height_px, anchor_ndc[0], anchor_ndc[1]],
            c: [sway[0], sway[1], time_s, 0.0],
            d: [0.0; 4],
            place: crate::cabin::Placement::GLASS,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HorizonUniforms {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

impl HorizonUniforms {
    /// `up_world`: gravity's up at the ship; `nose_world`: where the ship
    /// points — the ladder hangs on the nose, not on the pilot's turned
    /// head. The shader works in camera space, so both are rotated here.
    pub fn new(
        cam: &CameraFrame,
        up_world: Vec3,
        nose_world: Vec3,
        visibility: f32,
        height_px: f32,
        ladder: bool,
    ) -> Self {
        let (right, up, forward) = cam.basis();
        let to_cam = |v: Vec3| {
            let v = v.normalize_or_zero();
            [v.dot(right), v.dot(up), v.dot(forward)]
        };
        let up_cam = to_cam(up_world);
        let nose_cam = to_cam(nose_world);
        Self {
            a: [up_cam[0], up_cam[1], up_cam[2], visibility.clamp(0.0, 1.0)],
            b: [(cam.fov_y * 0.5).tan(), cam.aspect, height_px, cam.time_s],
            c: [
                if ladder { 1.0 } else { 0.0 },
                nose_cam[0],
                nose_cam[1],
                nose_cam[2],
            ],
            d: [0.0; 4],
        }
    }
}

/// Relevance fade for the horizon: a level reference matters near the
/// world. Out in space it is clutter.
#[derive(Debug, Clone, Copy, Default)]
pub struct HorizonFade {
    level: f32,
}

impl HorizonFade {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, dt: f32, altitude_m: f32) -> f32 {
        let dt = dt.clamp(1e-4, 0.25);
        let target = ((45_000.0 - altitude_m) / 20_000.0).clamp(0.0, 1.0);
        let alpha = 1.0 - (-dt / 0.6).exp();
        self.level += (target - self.level) * alpha;
        self.level.clamp(0.0, 1.0)
    }

    pub fn level(&self) -> f32 {
        self.level
    }
}

pub fn gyro_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> InstrumentPass {
    InstrumentPass::new_sized(
        device,
        target_format,
        sample_count,
        "gyro",
        crate::shaders::GYRO,
        std::mem::size_of::<GyroUniforms>() as u64,
    )
}

pub fn horizon_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> InstrumentPass {
    InstrumentPass::new(
        device,
        target_format,
        sample_count,
        "horizon",
        crate::shaders::HORIZON,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_hangs_on_the_nose_in_camera_space() {
        let cam = CameraFrame {
            orient: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        // The head turned 90° (a +Y rotation swings the view to -X); the
        // nose (world -Z) is then square off to the camera's right, not
        // ahead of it.
        let u = HorizonUniforms::new(&cam, Vec3::Y, Vec3::NEG_Z, 1.0, 100.0, true);
        assert!(u.c[1].abs() > 0.99, "{:?}", u.c);
        assert!(u.c[3].abs() < 1e-5);
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn level_flight_reads_zero() {
        let att = Attitude::from_world(Quat::IDENTITY, Vec3::Y, Vec3::NEG_Z);
        assert!(near(att.pitch, 0.0) && near(att.roll, 0.0) && near(att.drift, 0.0));
    }

    #[test]
    fn nose_up_is_positive_pitch() {
        // Pitch up: +X rotation swings the nose (-Z) toward +Y.
        let q = Quat::from_rotation_x(0.3);
        let att = Attitude::from_world(q, Vec3::Y, Vec3::NEG_Z);
        assert!(near(att.pitch, 0.3), "{}", att.pitch);
    }

    #[test]
    fn right_wing_down_is_positive_roll() {
        // Roll right: -Z rotation tips the up vector (+Y) toward +X.
        let q = Quat::from_rotation_z(-0.4);
        assert!((q * Vec3::Y).x > 0.0);
        let att = Attitude::from_world(q, Vec3::Y, Vec3::NEG_Z);
        assert!(near(att.roll, 0.4), "{}", att.roll);
    }

    #[test]
    fn velocity_to_the_right_is_positive_drift() {
        let vel = Vec3::new(0.5, 0.0, -1.0);
        let att = Attitude::from_world(Quat::IDENTITY, Vec3::Y, vel);
        assert!(att.drift > 0.0, "{}", att.drift);
        assert!(near(att.drift, 0.5f32.atan2(1.0)));
    }

    /// The ball's horizon is the line q.y = y0 in a frame q = R(roll)·p
    /// (gyro.wgsl). Rolled right, the right end of the horizon must RISE —
    /// that is what a pilot sees out of the window. This is the shader's
    /// line equation, kept here so the sign cannot drift silently.
    #[test]
    fn ball_horizon_rises_to_the_right_when_rolled_right() {
        let roll = 0.3f32;
        let (sr, cr) = roll.sin_cos();
        // q.y for a point p: -sr·p.x + cr·p.y. On the horizon q.y = 0.
        let y_at = |px: f32| sr * px / cr;
        assert!(y_at(1.0) > y_at(-1.0));
    }

    #[test]
    fn horizon_fades_out_in_space() {
        let mut f = HorizonFade::new();
        for _ in 0..100 {
            f.update(0.1, 1_000.0);
        }
        assert!(f.level() > 0.99);
        for _ in 0..100 {
            f.update(0.1, 80_000.0);
        }
        assert!(f.level() < 0.01);
    }
}

/// The design guide's numbers: the glass ruled, the dials' anchors and
/// reach, the gaze.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GuideUniforms {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

impl GuideUniforms {
    /// `anchors`: up to four dial anchors (NDC); `gaze`: where the head
    /// points on the glass; `reach`: the pick-up distance (NDC).
    pub fn new(
        aspect: f32,
        on: bool,
        safe_edge: f32,
        gaze: [f32; 2],
        reach: f32,
        looking: bool,
        anchors: &[[f32; 2]],
    ) -> Self {
        let mut slots = [[-10.0f32, -10.0]; 4];
        for (slot, a) in slots.iter_mut().zip(anchors.iter()) {
            *slot = *a;
        }
        Self {
            a: [aspect, if on { 1.0 } else { 0.0 }, safe_edge, 0.0],
            b: [gaze[0], gaze[1], reach, if looking { 1.0 } else { 0.0 }],
            c: [slots[0][0], slots[0][1], slots[1][0], slots[1][1]],
            d: [slots[2][0], slots[2][1], slots[3][0], slots[3][1]],
        }
    }
}

pub fn guide_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> InstrumentPass {
    InstrumentPass::new(
        device,
        target_format,
        sample_count,
        "guide",
        crate::shaders::GUIDE,
    )
}
