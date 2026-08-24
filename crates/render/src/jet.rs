//! The ship from outside: the fighter's exterior, Sun-lit, for the chase
//! view and the holo3PP projection. See `shaders/jet.wgsl`.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct JetUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    eye: [f32; 4],
    sun: [f32; 4],
    glow: [f32; 4],
}

impl JetUniforms {
    /// Nothing to draw: the pass discards everything.
    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        Self::new(cam, head, Vec3::new(0.0, 3.2, 22.0), Vec3::Y, 0.0, 0.0)
    }
    /// `head`: the view's rotation in the ship's frame; `eye_ship`: where
    /// the eye sits in that frame (m); `effort`: engines 0..1; `hyper`:
    /// the chaos drive's field 0..1.
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        eye_ship: Vec3,
        sun_ship: Vec3,
        effort: f32,
        hyper: f32,
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, cam.time_s.rem_euclid(1000.0)),
            eye: v4(eye_ship, cam.exposure),
            sun: v4(sun_ship.normalize_or_zero(), effort.clamp(0.0, 1.0)),
            glow: [hyper.clamp(0.0, 1.0), 0.0, 0.0, 0.0],
        }
    }
    /// The pass draws only when shown.
    pub fn shown(mut self) -> Self {
        self.glow[1] = 1.0;
        self
    }
}

pub type JetPass = InstrumentPass;

pub fn jet_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> JetPass {
    // A pane, not a glow: where the ship is hit it is opaque — the stars
    // and the bodies end at the hull.
    JetPass::new_pane_sized(
        device,
        target_format,
        sample_count,
        "jet",
        crate::shaders::JET,
        std::mem::size_of::<JetUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The uniform block is a wire format: the WGSL struct reads these
    /// lanes by position. Pin them.
    #[test]
    fn jet_lanes_hold_their_places() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 2.0,
            time_s: 3.0,
            exposure: 1.5,
        };
        let u = JetUniforms::new(
            &cam,
            Quat::IDENTITY,
            Vec3::new(0.0, 3.0, 22.0),
            Vec3::X,
            0.5,
            0.25,
        )
        .shown();
        assert_eq!(std::mem::size_of::<JetUniforms>(), 6 * 16);
        assert_eq!(u.right[3], 2.0, "aspect rides right.w");
        assert_eq!(u.eye, [0.0, 3.0, 22.0, 1.5], "eye + exposure");
        assert_eq!(u.sun[3], 0.5, "effort rides sun.w");
        assert_eq!(u.glow[0], 0.25, "hyper in glow.x");
        assert_eq!(u.glow[1], 1.0, "shown flag");
        assert_eq!(u.fwd[2], -1.0, "forward is -Z");
    }
}
