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
    /// `up_world`: gravity's up at the ship. The shader works in camera
    /// space, so it is rotated here.
    pub fn new(cam: &CameraFrame, up_world: Vec3, visibility: f32, height_px: f32) -> Self {
        let (right, up, forward) = cam.basis();
        let u = up_world.normalize_or_zero();
        let up_cam = [u.dot(right), u.dot(up), u.dot(forward)];
        Self {
            a: [up_cam[0], up_cam[1], up_cam[2], visibility.clamp(0.0, 1.0)],
            b: [(cam.fov_y * 0.5).tan(), cam.aspect, height_px, cam.time_s],
            c: [0.0; 4],
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
    InstrumentPass::new(
        device,
        target_format,
        sample_count,
        "gyro",
        crate::shaders::GYRO,
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
