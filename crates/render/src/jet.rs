//! The ship from outside: the fighter's exterior, Sun-lit, for the chase
//! view and the holo3PP projection — and its engines' plumes, RCS puffs
//! and the hull's own lit look. See `shaders/jet.wgsl`.

use glam::{Quat, Vec3};

use crate::hologram::MountView;
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
    /// xyz: pitch / yaw / roll demands -1..1 (the RCS puffs); w unused.
    rcs: [f32; 4],
    /// xyz: the nearest body's direction in ship frame (its light on the
    /// belly); w: how bright that fill is, 0..1.
    fill: [f32; 4],
    /// xyz: each hardpoint, ship frame (m) — the one transform table
    /// (bay.rs Hardpoint::pos via fit_views); w: its mount's kind
    /// (0 empty, 1 cannon, 2 rail). The chase view shows the bay's fit.
    hp: [[f32; 4]; 4],
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
            rcs: [0.0; 4],
            fill: [0.0; 4],
            hp: [[0.0; 4]; 4],
        }
    }
    /// The SHIP bay's fit: each hardpoint's place with its mount's kind,
    /// for the mounts on the marched hull.
    pub fn with_fit(mut self, fit: &[MountView; 4]) -> Self {
        for (slot, m) in self.hp.iter_mut().zip(fit.iter()) {
            *slot = [m.at.x, m.at.y, m.at.z, m.kind as f32];
        }
        self
    }
    /// The pass draws only when shown.
    pub fn shown(mut self) -> Self {
        self.glow[1] = 1.0;
        self
    }
    /// The attitude demands -1..1 on pitch, yaw and roll: each lights its
    /// RCS puff at the nose or a wingtip.
    pub fn with_rcs(mut self, pitch: f32, yaw: f32, roll: f32) -> Self {
        let c = |v: f32| {
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        self.rcs = [c(pitch), c(yaw), c(roll), 0.0];
        self
    }
    /// A body's light on the hull: its direction in ship frame and how
    /// much of the sky it fills (0..1) — the planet lights the belly.
    pub fn with_body_fill(mut self, dir_ship: Vec3, fill: f32) -> Self {
        let d = dir_ship.normalize_or_zero();
        self.fill = [d.x, d.y, d.z, fill.clamp(0.0, 1.0)];
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
    // and the bodies end at the hull. Its plumes write with alpha 0, so
    // the same premultiplied blend adds them as light.
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
        .shown()
        .with_rcs(0.5, -2.0, f32::NAN)
        .with_body_fill(Vec3::new(0.0, -2.0, 0.0), 0.7)
        .with_fit(&[
            MountView {
                at: Vec3::new(0.0, -0.45, -4.2),
                kind: 2,
            },
            MountView {
                at: Vec3::new(-2.6, -0.35, -0.6),
                kind: 1,
            },
            MountView {
                at: Vec3::new(2.6, -0.35, -0.6),
                kind: 0,
            },
            MountView {
                at: Vec3::new(0.0, -1.95, 1.4),
                kind: 0,
            },
        ]);
        assert_eq!(std::mem::size_of::<JetUniforms>(), 12 * 16);
        assert_eq!(u.right[3], 2.0, "aspect rides right.w");
        assert_eq!(u.eye, [0.0, 3.0, 22.0, 1.5], "eye + exposure");
        assert_eq!(u.sun[3], 0.5, "effort rides sun.w");
        assert_eq!(u.glow[0], 0.25, "hyper in glow.x");
        assert_eq!(u.glow[1], 1.0, "shown flag");
        assert_eq!(u.fwd[2], -1.0, "forward is -Z");
        assert_eq!(u.rcs, [0.5, -1.0, 0.0, 0.0], "demands clamped, NaN is none");
        assert_eq!(u.fill, [0.0, -1.0, 0.0, 0.7], "the body below, its fill");
        assert_eq!(
            u.hp[0],
            [0.0, -0.45, -4.2, 2.0],
            "the nose rail rides its lane"
        );
        assert_eq!(u.hp[2][3], 0.0, "an empty hardpoint is a bare pylon");
    }
}
