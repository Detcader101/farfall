//! The helicopters' look: generic cold-war utility hulls (a pod, a tail
//! boom, skids, two rotors) parked on their pads down on the planet, and
//! the one being flown, seen from the chase rig. Up to [`HELIS`] hulls a
//! frame, each an SDF of our own drawn at an arbitrary pose in the ship's
//! frame, the planet itself the occluder. See `shaders/heli.wgsl`.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

/// How many helicopters the pass draws at once (the nearest).
pub const HELIS: usize = 4;

/// One helicopter for the shader, ship frame relative to the eye (m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeliView {
    pub at: Vec3,
    pub rot: Quat,
    /// Rotor speed 0..1: a parked idle barely turns, a hover blurs.
    pub rotor: f32,
    pub seed: f32,
    /// Draw the pad under it (parked on its pad, not set down in a field).
    pub pad: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HeliUniforms {
    /// xyz the head's right axis (ship frame), w aspect.
    right: [f32; 4],
    /// xyz up, w tan(fov/2).
    up: [f32; 4],
    /// xyz forward, w time (s).
    fwd: [f32; 4],
    /// xyz the Sun's direction (ship frame), w helis in use.
    sun: [f32; 4],
    /// exposure, -, -, -.
    look: [f32; 4],
    /// The occluding body (the planet underfoot): xyz centre, w radius.
    occ: [f32; 4],
    /// xyz at, w rotor speed.
    at: [[f32; 4]; HELIS],
    rot: [[f32; 4]; HELIS],
    /// seed, pad?, -, -.
    info: [[f32; 4]; HELIS],
}

impl HeliUniforms {
    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        Self::new(cam, head, 0.0, Vec3::Z, &[], (Vec3::ZERO, 0.0))
    }

    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        time_s: f32,
        sun_ship: Vec3,
        helis: &[HeliView],
        occluder: (Vec3, f32),
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut at = [[0.0; 4]; HELIS];
        let mut rot = [[0.0, 0.0, 0.0, 1.0]; HELIS];
        let mut info = [[0.0; 4]; HELIS];
        let mut n = 0;
        for h in helis.iter().take(HELIS) {
            if !h.at.is_finite() {
                continue;
            }
            let r = h.rot.normalize();
            at[n] = v4(h.at, h.rotor.clamp(0.0, 1.0));
            rot[n] = [r.x, r.y, r.z, r.w];
            info[n] = [h.seed, if h.pad { 1.0 } else { 0.0 }, 0.0, 0.0];
            n += 1;
        }
        let (oc, or_) = occluder;
        let occ = if oc.is_finite() && or_.is_finite() && or_ > 0.0 {
            v4(oc, or_)
        } else {
            [0.0; 4]
        };
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, time_s.rem_euclid(1000.0)),
            sun: v4(sun_ship.normalize_or_zero(), n as f32),
            look: [cam.exposure, 0.0, 0.0, 0.0],
            occ,
            at,
            rot,
            info,
        }
    }
}

pub type HeliPass = InstrumentPass;

pub fn heli_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> HeliPass {
    HeliPass::new_pane_sized(
        device,
        target_format,
        sample_count,
        "heli",
        crate::shaders::HELI,
        std::mem::size_of::<HeliUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heli_lanes_hold_their_places() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let views = [
            HeliView {
                at: Vec3::new(0.0, -2.0, -60.0),
                rot: Quat::from_rotation_y(0.4),
                rotor: 0.12,
                seed: 0.7,
                pad: true,
            },
            HeliView {
                at: Vec3::new(f32::NAN, 0.0, 0.0),
                rot: Quat::IDENTITY,
                rotor: 1.0,
                seed: 0.0,
                pad: false,
            },
            HeliView {
                at: Vec3::new(5.0, 1.0, -30.0),
                rot: Quat::IDENTITY,
                rotor: 2.5,
                seed: 0.2,
                pad: false,
            },
        ];
        let u = HeliUniforms::new(
            &cam,
            Quat::IDENTITY,
            3.0,
            Vec3::X,
            &views,
            (Vec3::new(0.0, -63_710.0, 0.0), 63_710.0),
        );
        assert_eq!(u.sun[3], 2.0, "the NaN one dropped");
        assert_eq!(u.at[0], [0.0, -2.0, -60.0, 0.12]);
        assert_eq!(u.info[0], [0.7, 1.0, 0.0, 0.0]);
        assert_eq!(u.at[1][3], 1.0, "rotor clamped to 1");
        assert_eq!(u.info[1][1], 0.0, "no pad in a field");
        assert_eq!(u.occ[3], 63_710.0);
        assert_eq!(u.fwd[3], 3.0);
        assert_eq!(HeliUniforms::none(&cam, Quat::IDENTITY).sun[3], 0.0);
        assert_eq!(
            std::mem::size_of::<HeliUniforms>(),
            (6 + 3 * HELIS) * 16,
            "vec4 rows only, std140-safe"
        );
    }
}
