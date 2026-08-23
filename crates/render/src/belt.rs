//! The asteroid belt's live rocks, ray-traced near the ship. The app owns
//! the rocks (crates/app/src/belt.rs); this is their look.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

pub const LIVE: usize = 48;

/// One rock for the shader: centre relative to the head (ship frame, m),
/// radius, seed and spin phase.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RockView {
    pub centre: Vec3,
    pub radius_m: f32,
    pub seed: f32,
    pub phase: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BeltUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    sun: [f32; 4],
    look: [f32; 4],
    rocks: [[f32; 4]; LIVE],
    spins: [[f32; 4]; LIVE],
}

impl BeltUniforms {
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        time_s: f32,
        sun_ship: Vec3,
        ring_light: f32,
        rocks: &[RockView],
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut rk = [[0.0; 4]; LIVE];
        let mut sp = [[0.0; 4]; LIVE];
        let mut n = 0;
        for (i, r) in rocks.iter().take(LIVE).enumerate() {
            if !r.centre.is_finite() || r.radius_m <= 0.0 {
                continue;
            }
            rk[n] = v4(r.centre, r.radius_m);
            sp[n] = [r.seed, r.phase, 0.0, 0.0];
            n += 1;
            let _ = i;
        }
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, time_s.rem_euclid(1000.0)),
            sun: v4(sun_ship.normalize_or_zero(), n as f32),
            look: [cam.exposure, ring_light.clamp(0.0, 2.0), 0.0, 0.0],
            rocks: rk,
            spins: sp,
        }
    }
}

pub type BeltPass = InstrumentPass;

pub fn belt_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> BeltPass {
    BeltPass::new_pane_sized(
        device,
        target_format,
        sample_count,
        "belt",
        crate::shaders::BELT,
        std::mem::size_of::<BeltUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rocks_are_packed_and_bad_ones_dropped() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let rocks = [
            RockView {
                centre: Vec3::new(0.0, 0.0, -300.0),
                radius_m: 20.0,
                seed: 0.3,
                phase: 1.0,
            },
            RockView {
                centre: Vec3::new(f32::NAN, 0.0, 0.0),
                radius_m: 5.0,
                seed: 0.1,
                phase: 0.0,
            },
        ];
        let u = BeltUniforms::new(&cam, Quat::IDENTITY, 2.0, Vec3::X, 1.0, &rocks);
        assert_eq!(u.sun[3], 1.0);
        assert_eq!(u.rocks[0], [0.0, 0.0, -300.0, 20.0]);
        assert_eq!(std::mem::size_of::<BeltUniforms>(), (5 + 2 * LIVE) * 16);
    }
}
