//! The debris: shards of rock off a hit or a break, tumbling and cooling,
//! drawn over the belt as lit solids. See `shaders/debris.wgsl`.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

pub const SHARDS: usize = 64;
/// Rocks that can hide a shard (the belt's live set).
pub const ROCKS: usize = 48;

/// One shard, ship frame (metres), for the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShardView {
    pub at: Vec3,
    /// Half its longest side, metres.
    pub size: f32,
    /// Its tumble so far: an axis and an angle (rad).
    pub axis: Vec3,
    pub angle: f32,
    /// Age over life, 0..1.
    pub age01: f32,
    pub seed: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Occluder {
    pub centre: Vec3,
    pub radius_m: f32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DebrisScene<'a> {
    pub shards: &'a [ShardView],
    pub rocks: &'a [Occluder],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DebrisUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    /// exposure, ring fill, shards in use, rocks in use
    look: [f32; 4],
    /// xyz sun (ship frame), w ember strength
    sun: [f32; 4],
    /// xyz at, w size
    at: [[f32; 4]; SHARDS],
    /// xyz axis, w angle
    tumble: [[f32; 4]; SHARDS],
    /// age01, seed, -, -
    info: [[f32; 4]; SHARDS],
    /// xyz centre, w radius
    rocks: [[f32; 4]; ROCKS],
}

impl DebrisUniforms {
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        ring_fill: f32,
        ember: f32,
        sun_ship: Vec3,
        scene: &DebrisScene,
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut u = Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, cam.time_s.rem_euclid(1000.0)),
            look: [
                cam.exposure,
                ring_fill,
                scene.shards.len().min(SHARDS) as f32,
                scene.rocks.len().min(ROCKS) as f32,
            ],
            sun: v4(sun_ship.normalize_or_zero(), ember.max(0.0)),
            at: [[0.0; 4]; SHARDS],
            tumble: [[0.0; 4]; SHARDS],
            info: [[0.0; 4]; SHARDS],
            rocks: [[0.0; 4]; ROCKS],
        };
        for (i, s) in scene.shards.iter().take(SHARDS).enumerate() {
            u.at[i] = v4(s.at, s.size);
            u.tumble[i] = v4(s.axis.normalize_or_zero(), s.angle);
            u.info[i] = [s.age01.clamp(0.0, 1.0), s.seed, 0.0, 0.0];
        }
        for (i, r) in scene.rocks.iter().take(ROCKS).enumerate() {
            u.rocks[i] = v4(r.centre, r.radius_m);
        }
        u
    }

    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        Self::new(cam, head, 0.0, 0.0, Vec3::Y, &DebrisScene::default())
    }

    pub fn count(&self) -> usize {
        self.look[2] as usize
    }
}

pub type DebrisPass = InstrumentPass;

pub fn debris_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> DebrisPass {
    // Solids over the belt: alpha, not light.
    DebrisPass::new_pane_sized(
        device,
        target_format,
        sample_count,
        "debris",
        crate::shaders::DEBRIS,
        std::mem::size_of::<DebrisUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shards_pack_and_the_overflow_is_dropped() {
        assert_eq!(
            std::mem::size_of::<DebrisUniforms>(),
            16 * (5 + 3 * SHARDS + ROCKS)
        );
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 3.0,
            exposure: 1.2,
        };
        let shards: Vec<ShardView> = (0..70)
            .map(|i| ShardView {
                at: Vec3::new(i as f32, 0.0, -50.0),
                size: 0.5,
                axis: Vec3::new(0.0, 2.0, 0.0),
                angle: 1.0,
                age01: 0.25,
                seed: 0.5,
            })
            .collect();
        let rocks = [Occluder {
            centre: Vec3::new(0.0, 0.0, -400.0),
            radius_m: 30.0,
        }];
        let u = DebrisUniforms::new(
            &cam,
            Quat::IDENTITY,
            0.4,
            1.0,
            Vec3::X,
            &DebrisScene {
                shards: &shards,
                rocks: &rocks,
            },
        );
        assert_eq!(u.count(), SHARDS);
        assert_eq!(u.look[3], 1.0);
        assert_eq!(u.tumble[3], [0.0, 1.0, 0.0, 1.0], "the axis is a unit");
        assert_eq!(u.info[0][0], 0.25);
        assert_eq!(u.rocks[0][3], 30.0);
        assert_eq!(DebrisUniforms::none(&cam, Quat::IDENTITY).count(), 0);
    }
}
