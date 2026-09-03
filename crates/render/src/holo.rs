//! The holo3PP: a volumetric hologram in the cabin — the ship in
//! miniature at its true attitude, with the velocity vector, the nearest
//! body and the Sun around it at their true bearings. Third person without
//! ever leaving first person, and not a screen: a 3D object over an
//! emitter in the dash, with real parallax. See `shaders/holo.wgsl`.

use glam::{Quat, Vec3};

use crate::cabin::{anchor_direction, socket_centre};
use crate::instrument::InstrumentPass;
use crate::CameraFrame;

/// How far the hologram's underside floats above its emitter (m).
pub const HOLO_LIFT_M: f32 = 0.03;
/// The hologram's radius at size 1.0 (m).
pub const HOLO_RADIUS_M: f32 = 0.7;
/// The space the hologram's rim stands for at range 1, metres: a mark
/// nearer than this sits in from the rim by its share; one further sits
/// on the rim. HOLO RANGE multiplies it (and shrinks the ship).
pub const HOLO_REACH_M: f32 = 1500.0;
/// The most marks the hologram shows.
pub const MARKS: usize = 8;

/// What the hologram shows this frame, all in the ship's frame.
#[derive(Debug, Clone, Copy)]
pub struct HoloScene {
    /// Velocity direction (relative to the nearest body).
    pub vel_dir: Vec3,
    /// Speed, m/s, relative to the nearest body.
    pub speed_mps: f32,
    /// The nearest body's bearing and the sine of its angular radius.
    pub body_dir: Vec3,
    pub body_sin: f32,
    /// The Sun's bearing.
    pub sun_dir: Vec3,
    /// Engines 0..1 and the chaos drive's field 0..1.
    pub effort: f32,
    pub hyper: f32,
    /// HOLO RANGE: how much space the hologram shows round the ship, 1
    /// (the ship fills it) .. 4 (four times the room, the ship a quarter
    /// the size).
    pub range: f32,
    /// Other ships, relative to this one in its frame (m), with their
    /// kind: 0 hailing, 1 hostile, 2 wreck.
    pub marks: [Option<(Vec3, u8)>; MARKS],
    /// The miniature's craft: 0 the fighter, 1 the helicopter
    /// (common.wgsl sd_craft_exterior — SPEC §6.5c).
    pub craft: f32,
}

/// Where a mark sits in the little scene, as a share of the rim (0 at
/// the ship, 1 on the rim): its distance over the reach, capped — a
/// ship beyond the reach is on the rim, in its true direction.
pub fn mark_reach(dist_m: f32, range: f32) -> f32 {
    (dist_m / (HOLO_REACH_M * range.max(0.25))).clamp(0.0, 1.0)
}

/// The rod's length for a speed: a log scale, so a walking pace and an
/// interplanetary one both read — 1 m/s a stub, 1 km/s half, 1000 km/s
/// full.
pub fn arrow_length(speed_mps: f32) -> f32 {
    ((1.0 + speed_mps.max(0.0)).log10() / 6.0).clamp(0.0, 1.0)
}

/// Where the hologram's centre sits for a glass anchor: over the socket
/// that anchor's direction meets in the dash, lifted its radius clear.
pub fn holo_centre(anchor: [f32; 2], tan_half_fov: f32, aspect: f32, radius_m: f32) -> Vec3 {
    let socket = socket_centre(anchor_direction(anchor, tan_half_fov, aspect));
    socket + Vec3::new(0.0, radius_m + HOLO_LIFT_M, 0.0)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HoloUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    centre: [f32; 4],
    vel: [f32; 4],
    body: [f32; 4],
    sun: [f32; 4],
    misc: [f32; 4],
    /// xyz: a mark's direction times its reach (unit sphere), w: its
    /// kind + 1 (0: none).
    marks: [[f32; 4]; MARKS],
}

impl HoloUniforms {
    /// `head`: the view's rotation in the ship's frame; `centre`: the
    /// hologram's centre in that frame (m), `radius_m` its radius;
    /// `shown`: draw at all.
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        centre: Vec3,
        radius_m: f32,
        scene: &HoloScene,
        shown: bool,
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, cam.time_s.rem_euclid(1000.0)),
            centre: v4(centre, radius_m.max(0.01)),
            vel: v4(
                scene.vel_dir.normalize_or_zero(),
                arrow_length(scene.speed_mps),
            ),
            body: v4(
                scene.body_dir.normalize_or_zero(),
                scene.body_sin.clamp(0.0, 1.0),
            ),
            // w carries shown and the craft in one: 0 skips, 1 the
            // fighter, 2 the helicopter.
            sun: v4(
                scene.sun_dir.normalize_or_zero(),
                if shown { 1.0 + scene.craft } else { 0.0 },
            ),
            misc: [
                scene.effort.clamp(0.0, 1.0),
                scene.hyper.clamp(0.0, 1.0),
                HOLO_LIFT_M,
                scene.range.clamp(0.25, 8.0),
            ],
            marks: scene.marks.map(|m| match m {
                Some((rel, kind)) => {
                    let d = rel.length();
                    let at = rel.normalize_or_zero() * mark_reach(d, scene.range);
                    [at.x, at.y, at.z, kind as f32 + 1.0]
                }
                None => [0.0; 4],
            }),
        }
    }
}

pub type HoloPass = InstrumentPass;

pub fn holo_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> HoloPass {
    // Light, not glass: the hologram adds over the cabin.
    HoloPass::new_sized(
        device,
        target_format,
        sample_count,
        "holo",
        crate::shaders::HOLO,
        std::mem::size_of::<HoloUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cabin::DASH_N;

    fn scene() -> HoloScene {
        HoloScene {
            vel_dir: Vec3::new(0.0, 0.0, -2.0),
            speed_mps: 1000.0,
            body_dir: Vec3::new(0.0, -3.0, 0.0),
            body_sin: 0.8,
            sun_dir: Vec3::X,
            effort: 0.5,
            hyper: 0.25,
            range: 1.0,
            marks: [None; MARKS],
            craft: 1.0,
        }
    }

    /// The uniform block is a wire format for holo.wgsl: pin the lanes.
    #[test]
    fn holo_lanes_hold_their_places() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 2.0,
            time_s: 3.0,
            exposure: 1.5,
        };
        let u = HoloUniforms::new(
            &cam,
            Quat::IDENTITY,
            Vec3::new(0.5, -0.3, -1.2),
            0.15,
            &scene(),
            true,
        );
        assert_eq!(std::mem::size_of::<HoloUniforms>(), (8 + MARKS) * 16);
        assert_eq!(u.right[3], 2.0, "aspect rides right.w");
        assert_eq!(u.centre, [0.5, -0.3, -1.2, 0.15]);
        assert_eq!(&u.vel[..3], &[0.0, 0.0, -1.0], "directions are unit");
        assert!(
            (u.vel[3] - 0.5).abs() < 0.01,
            "1 km/s is half a rod: {}",
            u.vel[3]
        );
        assert_eq!(u.body[3], 0.8);
        assert_eq!(u.sun[3], 2.0, "shown, and the craft rides with it");
        assert_eq!(u.misc[0], 0.5, "effort");
        assert_eq!(u.misc[1], 0.25, "hyper");
        let off = HoloUniforms::new(&cam, Quat::IDENTITY, Vec3::ZERO, 0.15, &scene(), false);
        assert_eq!(off.sun[3], 0.0, "hidden discards");
        assert_eq!(u.misc[3], 1.0, "range rides misc.w");
        assert_eq!(u.marks[0], [0.0; 4], "no marks");
    }

    /// A ship 300 m off at range 1 sits a fifth of the way out in its
    /// true direction; one 30 km off sits on the rim; at range 4 the
    /// near one comes in four times closer. The kind rides w as kind+1.
    #[test]
    fn marks_sit_at_their_bearing_by_their_distance_over_the_reach() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 2.0,
            time_s: 3.0,
            exposure: 1.5,
        };
        let mut s = scene();
        s.marks[0] = Some((Vec3::new(300.0, 0.0, 0.0), 1));
        s.marks[1] = Some((Vec3::new(0.0, 0.0, -30_000.0), 0));
        s.marks[2] = Some((Vec3::new(0.0, 600.0, 0.0), 2));
        let u = HoloUniforms::new(&cam, Quat::IDENTITY, Vec3::ZERO, 0.15, &s, true);
        assert!((u.marks[0][0] - 0.2).abs() < 1e-5, "{:?}", u.marks[0]);
        assert_eq!(u.marks[0][3], 2.0, "hostile is kind 1, lane 2");
        assert!((u.marks[1][2] + 1.0).abs() < 1e-5, "on the rim ahead");
        assert_eq!(u.marks[1][3], 1.0, "a hail is lane 1");
        assert_eq!(u.marks[2][3], 3.0, "a wreck is lane 3");
        assert_eq!(u.marks[3][3], 0.0, "no fourth mark");
        s.range = 4.0;
        let u4 = HoloUniforms::new(&cam, Quat::IDENTITY, Vec3::ZERO, 0.15, &s, true);
        assert!((u4.marks[0][0] - 0.05).abs() < 1e-5, "{:?}", u4.marks[0]);
        assert_eq!(u4.misc[3], 4.0);
        assert_eq!(mark_reach(0.0, 1.0), 0.0);
        assert_eq!(mark_reach(1e9, 1.0), 1.0);
    }

    /// The rod: a log of the speed, clamped — a stub at walking pace,
    /// full at a thousand kilometres a second.
    #[test]
    fn the_velocity_rod_grows_with_the_log_of_the_speed() {
        assert_eq!(arrow_length(0.0), 0.0);
        assert!(arrow_length(1.0) < 0.1);
        assert!((arrow_length(1.0e6) - 1.0).abs() < 1e-3);
        assert_eq!(arrow_length(1.0e9), 1.0);
    }

    /// The hologram stands on the dash: its centre sits its radius (and a
    /// hair) above the socket under its anchor, and a lower-right anchor
    /// lands it right of the pilot and below the sill — out of the view.
    #[test]
    fn the_hologram_stands_over_its_socket_out_of_the_view() {
        let c = holo_centre([0.55, -0.55], 0.55, 1.6, 0.15);
        assert!(c.x > 0.3, "right of the pilot: {c}");
        assert!(c.y < -0.2, "below the sill: {c}");
        let socket = c - Vec3::new(0.0, 0.15 + HOLO_LIFT_M, 0.0);
        let on_dash = (socket - crate::cabin::DASH_C).dot(DASH_N).abs();
        assert!(on_dash < 1e-3, "the socket is in the dash plane: {on_dash}");
    }
}
