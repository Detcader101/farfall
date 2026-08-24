//! The quantum after-image: the ship's own geometry left behind on a WARP
//! STOP, sliding on down the vector the ship no longer has, fading. See
//! `shaders/ghost.wgsl`.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

/// How long the image lasts, seconds.
pub const GHOST_LIFE_S: f32 = 1.8;
/// Where the image is at a given age: it starts a few lengths ahead and
/// slides on, fast at first — the ship's own motion, seen leaving.
pub fn ghost_distance_m(age_s: f32) -> f32 {
    let a = age_s.max(0.0);
    12.0 + 70.0 * (1.0 - (-a * 1.6).exp())
}
/// The fade: a quick bloom, a long tail.
pub fn ghost_fade(age_s: f32) -> f32 {
    if !(0.0..GHOST_LIFE_S).contains(&age_s) {
        return 0.0;
    }
    let bloom = (age_s / 0.08).min(1.0);
    let tail = 1.0 - age_s / GHOST_LIFE_S;
    bloom * tail * tail
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GhostUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    at: [f32; 4],
    rot: [f32; 4],
    dir: [f32; 4],
}

impl GhostUniforms {
    /// No image: the pass discards everything.
    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, 0.0),
            at: [0.0; 4],
            rot: [0.0, 0.0, 0.0, 1.0],
            dir: [0.0; 4],
        }
    }

    /// The image `age_s` after the stop. `dir_ship`: the old velocity's
    /// direction in the ship's CURRENT frame; `rot_rel`: the image's
    /// attitude relative to the ship's current one; `strength`: the
    /// setting (1 stock).
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        time_s: f32,
        age_s: f32,
        dir_ship: Vec3,
        rot_rel: Quat,
        strength: f32,
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let d = dir_ship.normalize_or_zero();
        let at = d * ghost_distance_m(age_s);
        let r = rot_rel.normalize();
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, time_s.rem_euclid(1000.0)),
            at: v4(at, ghost_fade(age_s)),
            rot: [r.x, r.y, r.z, r.w],
            dir: v4(d, strength.clamp(0.0, 2.0)),
        }
    }

    /// See the image from an eye away from the pilot's seat (the chase
    /// view): the image's origin moves the other way. Ship frame, metres.
    pub fn with_eye(mut self, eye_ship: Vec3) -> Self {
        self.at[0] -= eye_ship.x;
        self.at[1] -= eye_ship.y;
        self.at[2] -= eye_ship.z;
        self
    }
}

pub type GhostPass = InstrumentPass;

pub fn ghost_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> GhostPass {
    GhostPass::new_sized(
        device,
        target_format,
        sample_count,
        "ghost",
        crate::shaders::GHOST,
        std::mem::size_of::<GhostUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_slides_away_and_fades() {
        assert!(ghost_distance_m(0.0) > 5.0);
        assert!(ghost_distance_m(1.0) > ghost_distance_m(0.2));
        assert!(ghost_distance_m(10.0) < 90.0, "it does not run off forever");
        assert_eq!(ghost_fade(-0.1), 0.0);
        assert!(ghost_fade(0.1) > 0.8);
        assert!(ghost_fade(1.0) < ghost_fade(0.5));
        assert_eq!(ghost_fade(GHOST_LIFE_S), 0.0);
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let u = GhostUniforms::new(
            &cam,
            Quat::IDENTITY,
            3.0,
            0.5,
            Vec3::new(0.0, 0.0, -2.0),
            Quat::from_rotation_y(0.3),
            1.0,
        );
        assert!(u.at[2] < -10.0 && u.at[0] == 0.0, "{:?}", u.at);
        assert!(u.at[3] > 0.0 && u.at[3] < 1.0);
        assert_eq!(u.dir[3], 1.0);
        assert_eq!(GhostUniforms::none(&cam, Quat::IDENTITY).at[3], 0.0);
    }
}
