//! The other ships' look: mimics coming out of their rock shrouds, hailing,
//! attacking, or dead — and the miners working the ring, growing through
//! their tiers. The app owns them (crates/app/src/mimic.rs, miner.rs);
//! this draws up to `MIMICS` hulls from the shared fighter SDF at
//! arbitrary poses and sizes — a hologram hardening into a sun-lit hull by
//! its reveal, engines by its effort, a beacon while it hails, dark and
//! guttering as a wreck, the tier's parts on a miner and its mining beam
//! with the ore sliding up it. The live rocks come along as occluders.
//! See `shaders/mimic.wgsl`.

use glam::{Quat, Vec3};

use crate::belt::LIVE;
use crate::instrument::InstrumentPass;
use crate::tracer::Occluder;
use crate::CameraFrame;

/// Lanes in the pass: four mimics and eight miners.
pub const MIMICS: usize = 12;

/// One ship for the shader, ship frame relative to the eye (m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MimicView {
    pub at: Vec3,
    pub rot: Quat,
    /// 0 rock .. 1 ship.
    pub reveal: f32,
    /// Engine effort 0..1.
    pub effort: f32,
    /// 0 hailing, 1 hostile, 2 wreck, 3 a miner, 4 a hostile miner.
    pub kind: u8,
    /// Damage 0..1.
    pub wound: f32,
    pub seed: f32,
    /// Metres per SDF unit: 1 is our own fighter's size.
    pub size: f32,
    /// A miner's tier 0..3: which parts are on the hull.
    pub tier: u8,
    /// A shield sheen on the hull, 0..1 (a hit just shed).
    pub shield: f32,
    /// The mining beam's far end, ship frame, while it is on.
    pub beam: Option<Vec3>,
}

impl MimicView {
    /// A plain ship: no parts, no beam, our size.
    pub fn plain(
        at: Vec3,
        rot: Quat,
        reveal: f32,
        effort: f32,
        kind: u8,
        wound: f32,
        seed: f32,
    ) -> Self {
        Self {
            at,
            rot,
            reveal,
            effort,
            kind,
            wound,
            seed,
            size: 1.0,
            tier: 0,
            shield: 0.0,
            beam: None,
        }
    }
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
    /// size, tier, shield sheen, beam on
    more: [[f32; 4]; MIMICS],
    /// xyz the beam's far end, w unused
    beam: [[f32; 4]; MIMICS],
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
        let mut more = [[1.0, 0.0, 0.0, 0.0]; MIMICS];
        let mut beam = [[0.0; 4]; MIMICS];
        let mut n = 0;
        for m in ships.iter().take(MIMICS) {
            if !m.at.is_finite() || !m.size.is_finite() || m.size <= 0.0 {
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
            let b = m.beam.filter(|b| b.is_finite());
            more[n] = [
                m.size,
                m.tier.min(3) as f32,
                m.shield.clamp(0.0, 1.0),
                if b.is_some() { 1.0 } else { 0.0 },
            ];
            beam[n] = v4(b.unwrap_or(Vec3::ZERO), 0.0);
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
            more,
            beam,
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
            MimicView::plain(
                Vec3::new(0.0, 0.0, -90.0),
                Quat::from_rotation_y(0.3),
                0.5,
                0.7,
                1,
                0.2,
                0.4,
            ),
            MimicView::plain(
                Vec3::new(f32::NAN, 0.0, 0.0),
                Quat::IDENTITY,
                1.0,
                0.0,
                0,
                0.0,
                0.0,
            ),
            MimicView {
                size: 2.4,
                tier: 2,
                shield: 0.5,
                beam: Some(Vec3::new(3.0, 0.0, -200.0)),
                ..MimicView::plain(
                    Vec3::new(10.0, 0.0, -120.0),
                    Quat::IDENTITY,
                    1.0,
                    0.3,
                    3,
                    0.0,
                    0.1,
                )
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
        assert_eq!(u.sun[3], 2.0, "the bad one dropped");
        assert_eq!(u.look[1], 1.0, "and the empty rock");
        assert_eq!(u.at[0], [0.0, 0.0, -90.0, 0.5]);
        assert_eq!(u.info[0], [0.7, 1.0, 0.2, 0.4]);
        assert_eq!(
            u.more[0],
            [1.0, 0.0, 0.0, 0.0],
            "a plain ship: our size, no beam"
        );
        assert!((u.rot[0][3] - (0.15f32).cos()).abs() < 1e-5);
        // The miner rides the second lane with its size, tier, sheen and beam.
        assert_eq!(u.more[1], [2.4, 2.0, 0.5, 1.0]);
        assert_eq!(u.beam[1], [3.0, 0.0, -200.0, 0.0]);
        assert_eq!(u.info[1][1], 3.0);
        assert_eq!(u.rocks[0], [1.0, 2.0, -50.0, 10.0]);
        assert_eq!(u.fwd[3], 3.0);
        assert_eq!(MimicUniforms::none(&cam, Quat::IDENTITY).sun[3], 0.0);
        assert_eq!(
            std::mem::size_of::<MimicUniforms>(),
            (5 + 5 * MIMICS + LIVE) * 16
        );
    }
}
