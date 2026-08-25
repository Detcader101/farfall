//! The scars: where a slug hit a rock and did not break it, a crater that
//! glows — white-hot, then orange, then the dull red of cooling stone —
//! painted on the rock's face and riding with it. See `shaders/scar.wgsl`.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

pub const SCARS: usize = 32;
pub const ROCKS: usize = 48;

/// One scar for the picture, ship frame (metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScarView {
    /// The rock it is on: its centre and radius.
    pub centre: Vec3,
    pub radius_m: f32,
    /// Where on the rock: a unit direction from the centre.
    pub dir: Vec3,
    /// The crater's radius, metres.
    pub size_m: f32,
    /// How hot it still is, 0..1 (see [`scar_heat`]).
    pub heat: f32,
    pub seed: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Occluder {
    pub centre: Vec3,
    pub radius_m: f32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ScarScene<'a> {
    pub scars: &'a [ScarView],
    pub rocks: &'a [Occluder],
}

/// How hot a scar is `age_s` after the hit, cooling over `cool_s`: it
/// loses its white in the first tenth, its orange over the middle, and
/// the last of the red at the end.
pub fn scar_heat(age_s: f32, cool_s: f32) -> f32 {
    let cool = cool_s.max(0.1);
    if age_s < 0.0 || age_s >= cool {
        return 0.0;
    }
    let x = age_s / cool;
    ((-3.0 * x).exp() * (1.0 - x)).clamp(0.0, 1.0)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScarUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    /// exposure, glow, scars in use, rocks in use
    look: [f32; 4],
    /// xyz rock centre, w rock radius
    centres: [[f32; 4]; SCARS],
    /// xyz direction on the rock, w crater radius
    dirs: [[f32; 4]; SCARS],
    /// heat, seed, -, -
    info: [[f32; 4]; SCARS],
    rocks: [[f32; 4]; ROCKS],
}

impl ScarUniforms {
    pub fn new(cam: &CameraFrame, head: Quat, glow: f32, scene: &ScarScene) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut u = Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, cam.time_s.rem_euclid(1000.0)),
            look: [
                cam.exposure,
                glow.max(0.0),
                scene.scars.len().min(SCARS) as f32,
                scene.rocks.len().min(ROCKS) as f32,
            ],
            centres: [[0.0; 4]; SCARS],
            dirs: [[0.0; 4]; SCARS],
            info: [[0.0; 4]; SCARS],
            rocks: [[0.0; 4]; ROCKS],
        };
        for (i, s) in scene.scars.iter().take(SCARS).enumerate() {
            u.centres[i] = v4(s.centre, s.radius_m);
            u.dirs[i] = v4(s.dir.normalize_or_zero(), s.size_m);
            u.info[i] = [s.heat.clamp(0.0, 1.0), s.seed, 0.0, 0.0];
        }
        for (i, r) in scene.rocks.iter().take(ROCKS).enumerate() {
            u.rocks[i] = v4(r.centre, r.radius_m);
        }
        u
    }

    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        Self::new(cam, head, 0.0, &ScarScene::default())
    }

    pub fn count(&self) -> usize {
        self.look[2] as usize
    }
}

pub type ScarPass = InstrumentPass;

pub fn scar_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> ScarPass {
    ScarPass::new_sized(
        device,
        target_format,
        sample_count,
        "scar",
        crate::shaders::SCAR,
        std::mem::size_of::<ScarUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scar_cools_white_to_nothing_over_its_time() {
        assert_eq!(scar_heat(0.0, 12.0), 1.0);
        assert!(scar_heat(1.0, 12.0) > 0.6, "still white-hot early");
        assert!(
            scar_heat(6.0, 12.0) > 0.05 && scar_heat(6.0, 12.0) < 0.2,
            "{}",
            scar_heat(6.0, 12.0)
        );
        assert_eq!(scar_heat(12.0, 12.0), 0.0);
        assert_eq!(scar_heat(-1.0, 12.0), 0.0);
        let mut last = 2.0;
        for i in 0..120 {
            let h = scar_heat(i as f32 * 0.1, 12.0);
            assert!(h <= last, "cooling never warms");
            last = h;
        }
        assert!(
            scar_heat(2.0, 60.0) > scar_heat(2.0, 4.0),
            "a long cool time keeps it hot longer"
        );
    }

    #[test]
    fn scars_pack_on_their_rocks() {
        assert_eq!(
            std::mem::size_of::<ScarUniforms>(),
            16 * (4 + 3 * SCARS + ROCKS)
        );
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 3.0,
            exposure: 1.2,
        };
        let scars = [ScarView {
            centre: Vec3::new(0.0, 0.0, -200.0),
            radius_m: 25.0,
            dir: Vec3::new(0.0, 0.0, 3.0),
            size_m: 2.0,
            heat: 0.5,
            seed: 0.1,
        }];
        let u = ScarUniforms::new(
            &cam,
            Quat::IDENTITY,
            1.0,
            &ScarScene {
                scars: &scars,
                rocks: &[],
            },
        );
        assert_eq!(u.count(), 1);
        assert_eq!(u.dirs[0], [0.0, 0.0, 1.0, 2.0]);
        assert_eq!(u.centres[0][3], 25.0);
        assert_eq!(u.info[0][0], 0.5);
        assert_eq!(ScarUniforms::none(&cam, Quat::IDENTITY).count(), 0);
    }
}
