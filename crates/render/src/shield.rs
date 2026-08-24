//! The force field: a shell round the ship that shows only where it has
//! been hit — a ripple of blue light spreading from each impact, the
//! field's honeycomb showing through around it. See `shaders/shield.wgsl`.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

/// How many impacts the shell remembers at once.
pub const IMPACTS: usize = 8;

/// One strike on the shell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Impact {
    /// Unit direction from the shell's centre, ship frame.
    pub dir: Vec3,
    /// When, seconds on the frame clock.
    pub at_s: f32,
    /// 0..1, a grain to a pebble.
    pub size: f32,
}

/// The shell's geometry in the ship's frame: its centre (a little ahead
/// of and below the head, about the hull) and radius, metres.
pub const SHELL_CENTRE: Vec3 = Vec3::new(0.0, -0.4, -1.2);
pub const SHELL_RADIUS_M: f32 = 4.2;
/// How fast a ripple crosses the shell, m/s, and the honeycomb's cell, m.
pub const RIPPLE_MPS: f32 = 5.0;
pub const CELL_M: f32 = 0.32;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShieldUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    shell: [f32; 4],
    look: [f32; 4],
    /// x: the hyper drive's field 0..1 — the whole shell ablating; yzw unused.
    flow: [f32; 4],
    hits: [[f32; 4]; IMPACTS],
}

impl ShieldUniforms {
    /// `head`: the pilot's head in the ship's frame; `strength`: the
    /// SHIELD setting (0 off); `impacts`: the latest, newest first is
    /// fine — the shader treats them alike.
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        time_s: f32,
        strength: f32,
        impacts: &[Impact],
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut hits = [[0.0; 4]; IMPACTS];
        let mut n = 0;
        for (slot, im) in hits.iter_mut().zip(impacts.iter()) {
            let d = im.dir.normalize_or_zero();
            if d == Vec3::ZERO || !im.at_s.is_finite() {
                continue;
            }
            // w packs the time and the size: time + 1000 × size (the time
            // is kept under 1000 by the caller's clock wrap).
            let size = (im.size.clamp(0.0, 1.0) * 1000.0).round();
            *slot = [d.x, d.y, d.z, im.at_s.rem_euclid(1000.0) + 1000.0 * size];
            n += 1;
        }
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, time_s.rem_euclid(1000.0)),
            shell: v4(SHELL_CENTRE, SHELL_RADIUS_M),
            look: [strength.clamp(0.0, 2.0), RIPPLE_MPS, CELL_M, n as f32],
            flow: [0.0; 4],
            hits,
        }
    }

    /// See the shell from an eye away from the pilot's seat (the chase
    /// view): its centre moves the other way. Ship frame, metres.
    pub fn with_eye(mut self, eye_ship: Vec3) -> Self {
        self.shell[0] -= eye_ship.x;
        self.shell[1] -= eye_ship.y;
        self.shell[2] -= eye_ship.z;
        self
    }

    /// Under the hyper drive the whole shell ablates: the field lights
    /// from the nose back, streaming, at this level 0..1.
    pub fn with_hyper(mut self, hyper: f32) -> Self {
        self.flow[0] = if hyper.is_finite() {
            hyper.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self
    }
}

pub type ShieldPass = InstrumentPass;

pub fn shield_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> ShieldPass {
    ShieldPass::new_sized(
        device,
        target_format,
        sample_count,
        "shield",
        crate::shaders::SHIELD,
        std::mem::size_of::<ShieldUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impacts_are_packed_with_their_time_and_size() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let hits = [
            Impact {
                dir: Vec3::new(0.0, 0.0, -2.0),
                at_s: 1234.5,
                size: 0.25,
            },
            Impact {
                dir: Vec3::ZERO,
                at_s: 1.0,
                size: 1.0,
            },
        ];
        let u = ShieldUniforms::new(&cam, Quat::IDENTITY, 1236.0, 1.0, &hits);
        assert_eq!(u.look[3], 1.0, "a zero direction is no impact");
        assert_eq!(&u.hits[0][..3], &[0.0, 0.0, -1.0]);
        // 234.5 s into the wrap + 1000 × 250.
        assert!((u.hits[0][3] - (234.5 + 250_000.0)).abs() < 0.01);
        assert!((u.fwd[3] - 236.0).abs() < 1e-4);
        assert_eq!(u.shell[3], SHELL_RADIUS_M);
        assert_eq!(std::mem::size_of::<ShieldUniforms>(), (6 + IMPACTS) * 16);
        assert_eq!(u.with_hyper(0.5).flow[0], 0.5);
    }
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    /// The shader's arithmetic, mirrored: a ripple 0.1 s old from a hit
    /// dead ahead must be found by a ray just off the nose.
    #[test]
    fn a_fresh_ripple_lies_where_the_ray_ahead_meets_the_shell() {
        let ray = Vec3::new(0.0, 0.0, -1.0);
        let c = SHELL_CENTRE;
        let rad = SHELL_RADIUS_M;
        let b = ray.dot(c);
        let disc = b * b - (c.dot(c) - rad * rad);
        assert!(disc > 0.0);
        let t = b + disc.sqrt();
        let p = ray * t;
        let n = (p - c) / rad;
        let hit = Vec3::new(0.0, 0.0, -1.0);
        let d = n.dot(hit).clamp(-1.0, 1.0).acos() * rad;
        // The nose's shell point is a little above the hit direction's
        // (the shell is centred below the head): within a metre.
        assert!(d < 1.0, "{d}");
    }
}
