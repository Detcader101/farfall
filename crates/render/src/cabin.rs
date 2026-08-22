//! The wireframe cabin (`shaders/cockpit.wgsl`): a canopy dome, a sill, a
//! dash and a bulkhead drawn around the pilot's head in the ship's frame.

use crate::instrument::InstrumentPass;
use crate::CameraFrame;
use glam::{Quat, Vec3};

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CabinUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    misc: [f32; 4],
}

impl CabinUniforms {
    /// `head`: the pilot's head rotation in the ship's frame (freelook);
    /// the cabin is fixed to the ship, so the rays are turned by it.
    pub fn new(cam: &CameraFrame, head: Quat, glow: f32, hull: f32, on: f32) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        Self {
            right: v4(head * Vec3::X, glow.clamp(0.0, 3.0)),
            up: v4(head * Vec3::Y, hull.clamp(0.0, 1.0)),
            fwd: v4(head * Vec3::NEG_Z, (cam.fov_y * 0.5).tan()),
            misc: [cam.aspect, cam.time_s, on.clamp(0.0, 1.0), 0.0],
        }
    }
}

pub fn cabin_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> InstrumentPass {
    InstrumentPass::new_pane(
        device,
        target_format,
        sample_count,
        "cockpit",
        crate::shaders::COCKPIT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_turns_the_rays_not_the_cabin() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 2.0,
            exposure: 1.0,
        };
        let still = CabinUniforms::new(&cam, Quat::IDENTITY, 1.0, 0.8, 1.0);
        assert_eq!(&still.fwd[..3], &[0.0, 0.0, -1.0]);
        // Looking right: the forward ray swings toward +X in the ship's
        // frame (the nose is -Z; rotating -Z about +Y by a negative angle
        // swings it toward +X).
        let turned = CabinUniforms::new(&cam, Quat::from_rotation_y(-0.5), 5.0, -1.0, 1.0);
        assert!(turned.fwd[0] > 0.4, "{:?}", turned.fwd);
        assert_eq!(turned.right[3], 3.0);
        assert_eq!(turned.up[3], 0.0);
        assert_eq!(still.misc[2], 1.0);
    }
}
