//! The arms' light: slugs in the air as tracer streaks, muzzle flashes,
//! and the bursts where they land — sparks off a rock, and a rock coming
//! apart in a cloud of grit and embers. The app owns the slugs and bursts
//! (crates/app/src/arms.rs); this is their look. See `shaders/tracer.wgsl`.
//! The live rocks come along too, so the light hides behind them.

use glam::{Quat, Vec3};

use crate::belt::LIVE;
use crate::instrument::InstrumentPass;
use crate::CameraFrame;

pub const SLUGS: usize = 32;
pub const BURSTS: usize = 16;

/// A slug for the shader: its head and tail relative to the head of the
/// pilot (ship frame, m), kind and age.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlugView {
    pub head: Vec3,
    pub tail: Vec3,
    pub kind: u8,
    pub age_s: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurstView {
    pub at: Vec3,
    pub age_s: f32,
    pub kind: u8,
    pub size: f32,
    pub seed: f32,
}

/// A rock as an occluder: centre and radius, ship frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Occluder {
    pub centre: Vec3,
    pub radius_m: f32,
}

/// What is in the air this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracerScene<'a> {
    pub slugs: &'a [SlugView],
    pub bursts: &'a [BurstView],
    pub rocks: &'a [Occluder],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TracerUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    /// exposure, glow (the setting), slugs in use, bursts in use
    look: [f32; 4],
    /// xyz sun direction (ship frame), w rocks in use
    sun: [f32; 4],
    heads: [[f32; 4]; SLUGS],
    tails: [[f32; 4]; SLUGS],
    bursts: [[f32; 4]; BURSTS],
    binfo: [[f32; 4]; BURSTS],
    rocks: [[f32; 4]; LIVE],
}

impl TracerUniforms {
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        time_s: f32,
        glow: f32,
        sun_ship: Vec3,
        scene: &TracerScene,
    ) -> Self {
        let (slugs, bursts, rocks) = (scene.slugs, scene.bursts, scene.rocks);
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut heads = [[0.0; 4]; SLUGS];
        let mut tails = [[0.0; 4]; SLUGS];
        let mut ns = 0;
        for s in slugs.iter().take(SLUGS) {
            if !s.head.is_finite() || !s.tail.is_finite() {
                continue;
            }
            heads[ns] = v4(s.head, s.kind as f32);
            tails[ns] = v4(s.tail, s.age_s.max(0.0));
            ns += 1;
        }
        let mut bs = [[0.0; 4]; BURSTS];
        let mut bi = [[0.0; 4]; BURSTS];
        let mut nb = 0;
        for b in bursts.iter().take(BURSTS) {
            if !b.at.is_finite() || b.age_s < 0.0 {
                continue;
            }
            bs[nb] = v4(b.at, b.age_s);
            bi[nb] = [b.size.max(0.05), b.kind as f32, b.seed, 0.0];
            nb += 1;
        }
        let mut rk = [[0.0; 4]; LIVE];
        let mut nr = 0;
        for r in rocks.iter().take(LIVE) {
            if !r.centre.is_finite() || r.radius_m <= 0.0 {
                continue;
            }
            rk[nr] = v4(r.centre, r.radius_m);
            nr += 1;
        }
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, time_s.rem_euclid(1000.0)),
            look: [cam.exposure, glow.clamp(0.0, 3.0), ns as f32, nb as f32],
            sun: v4(sun_ship.normalize_or_zero(), nr as f32),
            heads,
            tails,
            bursts: bs,
            binfo: bi,
            rocks: rk,
        }
    }

    /// Nothing in the air: the pass discards everything.
    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        Self::new(cam, head, 0.0, 1.0, Vec3::Z, &TracerScene::default())
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.look[2] as usize,
            self.look[3] as usize,
            self.sun[3] as usize,
        )
    }
}

pub type TracerPass = InstrumentPass;

pub fn tracer_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> TracerPass {
    TracerPass::new_sized(
        device,
        target_format,
        sample_count,
        "tracer",
        crate::shaders::TRACER,
        std::mem::size_of::<TracerUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_counts_what_it_was_given_and_drops_the_broken() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let slugs = vec![
            SlugView {
                head: Vec3::new(0.0, 0.0, -50.0),
                tail: Vec3::new(0.0, 0.0, -30.0),
                kind: 1,
                age_s: 0.2,
            },
            SlugView {
                head: Vec3::NAN,
                tail: Vec3::ZERO,
                kind: 0,
                age_s: 0.0,
            },
        ];
        let bursts = vec![BurstView {
            at: Vec3::new(3.0, 0.0, -80.0),
            age_s: 0.1,
            kind: 2,
            size: 1.5,
            seed: 0.3,
        }];
        let rocks = vec![Occluder {
            centre: Vec3::new(0.0, 0.0, -200.0),
            radius_m: 20.0,
        }];
        let scene = TracerScene {
            slugs: &slugs,
            bursts: &bursts,
            rocks: &rocks,
        };
        let u = TracerUniforms::new(&cam, Quat::IDENTITY, 5.0, 1.0, Vec3::X, &scene);
        assert_eq!(u.counts(), (1, 1, 1));
        assert_eq!(u.heads[0][3], 1.0, "the kind rides with the head");
        assert_eq!(u.binfo[0][1], 2.0);
        assert_eq!(u.rocks[0][3], 20.0);
        assert_eq!(
            TracerUniforms::none(&cam, Quat::IDENTITY).counts(),
            (0, 0, 0)
        );
        assert_eq!(std::mem::size_of::<TracerUniforms>() % 16, 0);
    }
}
