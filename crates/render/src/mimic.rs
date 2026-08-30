//! The mimics' look: ships coming out of their rock shrouds, hailing,
//! attacking, or dead. The app owns them (crates/app/src/mimic.rs); this
//! draws up to `MIMICS` of them from the shared fighter SDF at arbitrary
//! poses — a hologram hardening into a sun-lit hull by its reveal, engines
//! by its effort, a beacon while it hails, dark and guttering as a wreck.
//! The live rocks come along as occluders. See `shaders/mimic.wgsl`.

use glam::{Quat, Vec3};

use crate::belt::LIVE;
use crate::instrument::InstrumentPass;
use crate::tracer::Occluder;
use crate::CameraFrame;

pub const MIMICS: usize = 4;

/// One ship for the shader, ship frame relative to the eye (m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MimicView {
    pub at: Vec3,
    pub rot: Quat,
    /// 0 rock .. 1 ship.
    pub reveal: f32,
    /// Engine effort 0..1.
    pub effort: f32,
    /// 0 hailing, 1 hostile, 2 wreck.
    pub kind: u8,
    /// Damage 0..1.
    pub wound: f32,
    pub seed: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MimicUniforms {
    right: [f32; 4],
    up: [f32; 4],
    /// xyz forward, w time
    fwd: [f32; 4],
    /// xyz sun (ship frame), w ships in use
    sun: [f32; 4],
    /// exposure, rocks in use, -, -
    look: [f32; 4],
    /// xyz at, w reveal
    at: [[f32; 4]; MIMICS],
    rot: [[f32; 4]; MIMICS],
    /// effort, kind, wound, seed
    info: [[f32; 4]; MIMICS],
    rocks: [[f32; 4]; LIVE],
}

impl MimicUniforms {
    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        Self::new(cam, head, 0.0, Vec3::Z, &[], &[])
    }

    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        time_s: f32,
        sun_ship: Vec3,
        ships: &[MimicView],
        rocks: &[Occluder],
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut at = [[0.0; 4]; MIMICS];
        let mut rot = [[0.0, 0.0, 0.0, 1.0]; MIMICS];
        let mut info = [[0.0; 4]; MIMICS];
        let mut n = 0;
        for m in ships.iter().take(MIMICS) {
            if !m.at.is_finite() {
                continue;
            }
            let r = m.rot.normalize();
            at[n] = v4(m.at, m.reveal.clamp(0.0, 1.0));
            rot[n] = [r.x, r.y, r.z, r.w];
            info[n] = [
                m.effort.clamp(0.0, 1.0),
                m.kind as f32,
                m.wound.clamp(0.0, 1.0),
                m.seed,
            ];
            n += 1;
        }
        let mut rk = [[0.0; 4]; LIVE];
        let mut nr = 0;
        for r in rocks.iter().take(LIVE) {
            if !r.centre.is_finite() || r.radius_m.is_nan() || r.radius_m <= 0.0 {
                continue;
            }
            rk[nr] = v4(r.centre, r.radius_m);
            nr += 1;
        }
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, time_s.rem_euclid(1000.0)),
            sun: v4(sun_ship.normalize_or_zero(), n as f32),
            look: [cam.exposure, nr as f32, 0.0, 0.0],
            at,
            rot,
            info,
            rocks: rk,
        }
    }
}

pub type MimicPass = InstrumentPass;

pub fn mimic_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> MimicPass {
    MimicPass::new_pane_sized(
        device,
        target_format,
        sample_count,
        "mimic",
        crate::shaders::MIMIC,
        std::mem::size_of::<MimicUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mimic_lanes_hold_their_places() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let ships = [
            MimicView {
                at: Vec3::new(0.0, 0.0, -90.0),
                rot: Quat::from_rotation_y(0.3),
                reveal: 0.5,
                effort: 0.7,
                kind: 1,
                wound: 0.2,
                seed: 0.4,
            },
            MimicView {
                at: Vec3::new(f32::NAN, 0.0, 0.0),
                rot: Quat::IDENTITY,
                reveal: 1.0,
                effort: 0.0,
                kind: 0,
                wound: 0.0,
                seed: 0.0,
            },
        ];
        let rocks = [
            Occluder {
                centre: Vec3::new(1.0, 2.0, -50.0),
                radius_m: 10.0,
            },
            Occluder {
                centre: Vec3::ZERO,
                radius_m: 0.0,
            },
        ];
        let u = MimicUniforms::new(&cam, Quat::IDENTITY, 3.0, Vec3::X, &ships, &rocks);
        assert_eq!(u.sun[3], 1.0, "the bad one dropped");
        assert_eq!(u.look[1], 1.0, "and the empty rock");
        assert_eq!(u.at[0], [0.0, 0.0, -90.0, 0.5]);
        assert_eq!(u.info[0], [0.7, 1.0, 0.2, 0.4]);
        assert!((u.rot[0][3] - (0.15f32).cos()).abs() < 1e-5);
        assert_eq!(u.rocks[0], [1.0, 2.0, -50.0, 10.0]);
        assert_eq!(u.fwd[3], 3.0);
        assert_eq!(MimicUniforms::none(&cam, Quat::IDENTITY).sun[3], 0.0);
        assert_eq!(
            std::mem::size_of::<MimicUniforms>(),
            (5 + 3 * MIMICS + LIVE) * 16
        );
    }
}
