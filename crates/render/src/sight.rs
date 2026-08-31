//! The gun sight: a hologram on the glass where the guns point — the
//! gimballed aim, not the nose — so it follows the gaze in freelook and
//! sits on the gimbal's ring when the gaze goes past it. With it: the
//! gimbal ring itself, the gaze when it is not the aim, each barrel's
//! own line at the convergence range, the heat and the rail's charge
//! as arcs. See `shaders/sight.wgsl`.

use glam::{Quat, Vec3};

use crate::instrument::InstrumentPass;
use crate::CameraFrame;

/// The range the barrels' lines are shown at, metres.
pub const CONVERGE_M: f32 = 300.0;
/// Up to this many barrels get a pip.
pub const BARRELS: usize = 4;
/// Up to this many mimic ships get a marker (crate::mimic's lane count).
pub const MARKS: usize = 4;
/// The marker safe area: an edge arrow sits on this rectangle (NDC), well
/// inside the rim so it survives any glass curvature or overscan.
pub const MARK_EDGE: f32 = 0.88;

/// A direction in the head's frame (nose -Z) on the glass as NDC, if it
/// is in front of the eye at all.
pub fn project(dir_head: Vec3, tan_half_fov: f32, aspect: f32) -> Option<[f32; 2]> {
    let depth = -dir_head.z;
    if depth <= 1e-4 {
        return None;
    }
    let t = tan_half_fov.max(1e-4);
    Some([dir_head.x / (depth * t * aspect), dir_head.y / (depth * t)])
}

/// Where a mimic's marker goes this frame: on the glass over the ship
/// itself, or — off the glass — an arrow on the safe-area rectangle
/// pointing the shortest way round to it. `angle` is the outward bearing,
/// shader convention (0 up, positive clockwise). The maths is here, under
/// test, because "which way is the thing I cannot see" is exactly the sort
/// of sign error that is miserable to debug on a GPU.
pub fn edge_mark(dir_head: Vec3, tan_half_fov: f32, aspect: f32) -> ([f32; 2], f32, f32) {
    if let Some(n) = project(dir_head, tan_half_fov, aspect) {
        if n[0].abs() <= MARK_EDGE && n[1].abs() <= MARK_EDGE {
            return (n, 0.0, 1.0);
        }
    }
    // Off the glass (or behind): the on-screen direction toward it. The
    // projection's x is divided by the aspect, so the screen-space bearing
    // of a head-frame direction is (x / aspect, y) — true whether the
    // depth is positive or not, since behind the eye the sideways sign
    // still says which way to turn.
    let v = glam::Vec2::new(dir_head.x / aspect.max(1e-4), dir_head.y);
    let v = if v.length_squared() < 1e-12 {
        // Dead ahead-behind: point down, any edge is as wrong as another.
        glam::Vec2::new(0.0, -1.0)
    } else {
        v.normalize()
    };
    let k = MARK_EDGE / v.x.abs().max(v.y.abs()).max(1e-4);
    ([v.x * k, v.y * k], v.x.atan2(v.y), 2.0)
}

/// What the sight shows this frame. Directions are in the SHIP frame;
/// the head's turn is applied here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SightScene {
    /// Where the guns point, and where the gaze is.
    pub aim: Vec3,
    pub gaze: Vec3,
    /// The gimbal's half-angle, radians; `clamped` when the gaze is past it.
    pub gimbal_rad: f32,
    pub clamped: bool,
    /// The barrels' places, ship frame (metres).
    pub barrels: [Option<Vec3>; BARRELS],
    /// 0 cannon, 1 rail.
    pub kind: u8,
    pub heat: f32,
    pub charge: f32,
    pub jammed: bool,
    pub empty: bool,
    /// The setting: 0 off .. 2 bright.
    pub strength: f32,
    /// Revealed mimic ships: the direction to each (ship frame) and its
    /// kind (0 hail, 1 hostile, 2 wreck). Shown whatever the sight's
    /// strength: the marker is the way to FIND a ship, not part of the gun.
    pub marks: [Option<(Vec3, u8)>; MARKS],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SightUniforms {
    /// aspect, strength (0 off), time, gimbal ring radius (NDC, vertical)
    a: [f32; 4],
    /// aim NDC xy, gaze NDC xy (aim off-glass: a.y is 0)
    b: [f32; 4],
    /// clamped, heat, charge, kind
    c: [f32; 4],
    /// jammed, empty, nose NDC xy
    d: [f32; 4],
    /// each barrel's pip NDC xy, z = 1 if shown
    pips: [[f32; 4]; BARRELS],
    /// each mimic's marker: NDC xy, outward angle, mode + kind * 4
    /// (mode 0 off, 1 on the ship, 2 an edge arrow)
    marks: [[f32; 4]; MARKS],
}

impl SightUniforms {
    pub fn new(cam: &CameraFrame, head: Quat, scene: &SightScene) -> Self {
        let tan = (cam.fov_y * 0.5).tan();
        let inv = head.inverse();
        let to_glass = |d: Vec3| project(inv * d.normalize_or_zero(), tan, cam.aspect);
        let aim = to_glass(scene.aim);
        let gaze = to_glass(scene.gaze).unwrap_or([0.0, 0.0]);
        let nose = to_glass(Vec3::NEG_Z).unwrap_or([0.0, 9.0]);
        // The gimbal ring: the cone's half-angle as a radius on the glass,
        // in vertical NDC units (the shader corrects for aspect).
        let ring = scene.gimbal_rad.tan() / tan;
        let strength = if aim.is_some() {
            scene.strength.clamp(0.0, 2.0)
        } else {
            0.0
        };
        let aim = aim.unwrap_or([0.0, 0.0]);
        let mut marks = [[0.0; 4]; MARKS];
        for (m, s) in marks.iter_mut().zip(scene.marks.iter()) {
            if let Some((dir, kind)) = s {
                let (pos, ang, mode) = edge_mark(inv * dir.normalize_or_zero(), tan, cam.aspect);
                *m = [pos[0], pos[1], ang, mode + *kind as f32 * 4.0];
            }
        }
        let mut pips = [[0.0; 4]; BARRELS];
        for (p, b) in pips.iter_mut().zip(scene.barrels.iter()) {
            if let Some(m) = b {
                let far = *m + scene.aim.normalize_or_zero() * CONVERGE_M;
                if let Some(n) = to_glass(far) {
                    *p = [n[0], n[1], 1.0, 0.0];
                }
            }
        }
        Self {
            a: [cam.aspect, strength, cam.time_s.rem_euclid(1000.0), ring],
            b: [aim[0], aim[1], gaze[0], gaze[1]],
            c: [
                if scene.clamped { 1.0 } else { 0.0 },
                scene.heat.clamp(0.0, 1.0),
                scene.charge.clamp(0.0, 1.0),
                scene.kind as f32,
            ],
            d: [
                if scene.jammed { 1.0 } else { 0.0 },
                if scene.empty { 1.0 } else { 0.0 },
                nose[0],
                nose[1],
            ],
            pips,
            marks,
        }
    }

    pub fn none(cam: &CameraFrame) -> Self {
        Self {
            a: [cam.aspect, 0.0, 0.0, 1.0],
            b: [0.0; 4],
            c: [0.0; 4],
            d: [0.0; 4],
            pips: [[0.0; 4]; BARRELS],
            marks: [[0.0; 4]; MARKS],
        }
    }

    pub fn shown(&self) -> bool {
        self.a[1] > 0.0
    }
}

pub type SightPass = InstrumentPass;

pub fn sight_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> SightPass {
    SightPass::new_sized(
        device,
        target_format,
        sample_count,
        "sight",
        crate::shaders::SIGHT,
        std::mem::size_of::<SightUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> CameraFrame {
        CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        }
    }

    fn scene(aim: Vec3, gaze: Vec3, clamped: bool) -> SightScene {
        SightScene {
            aim,
            gaze,
            gimbal_rad: 0.6,
            clamped,
            barrels: [
                Some(Vec3::new(-2.6, -0.35, -0.6)),
                Some(Vec3::new(2.6, -0.35, -0.6)),
                None,
                None,
            ],
            kind: 0,
            heat: 0.3,
            charge: 0.0,
            jammed: false,
            empty: false,
            strength: 1.0,
            marks: [None; MARKS],
        }
    }

    /// A mimic on the glass gets its marker over the ship; one off the
    /// glass gets an arrow ON the safe-area rectangle pointing its way —
    /// including one dead astern, which must still land on an edge.
    #[test]
    fn a_mimic_off_the_glass_gets_an_edge_arrow_pointing_its_way() {
        let (tan, aspect) = (0.7, 1.5);
        // Ahead and a touch right: on the glass, over the ship.
        let (pos, _, mode) = edge_mark(Vec3::new(0.2, 0.0, -1.0), tan, aspect);
        assert_eq!(mode, 1.0);
        assert!(pos[0] > 0.0 && pos[0] < MARK_EDGE && pos[1] == 0.0, "{pos:?}");
        // Hard right, past the glass: pinned to the right edge, arrow
        // pointing right (angle +90 degrees, shader convention).
        let (pos, ang, mode) = edge_mark(Vec3::new(1.0, 0.0, -0.1), tan, aspect);
        assert_eq!(mode, 2.0);
        assert!((pos[0] - MARK_EDGE).abs() < 1e-5, "{pos:?}");
        assert!((ang - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{ang}");
        // Behind and left: pinned to the LEFT edge — the sideways sign
        // survives the eye, so the arrow says which way round to turn.
        let (pos, _, mode) = edge_mark(Vec3::new(-0.4, 0.0, 1.0), tan, aspect);
        assert_eq!(mode, 2.0);
        assert!((pos[0] + MARK_EDGE).abs() < 1e-5, "{pos:?}");
        // Above: pinned to the top edge, arrow up (angle 0).
        let (pos, ang, mode) = edge_mark(Vec3::new(0.0, 1.0, -0.1), tan, aspect);
        assert_eq!(mode, 2.0);
        assert!((pos[1] - MARK_EDGE).abs() < 1e-5, "{pos:?}");
        assert!(ang.abs() < 1e-4, "{ang}");
        // Dead astern: still an arrow, still on the rectangle.
        let (pos, _, mode) = edge_mark(Vec3::Z, tan, aspect);
        assert_eq!(mode, 2.0);
        assert!(pos[0].abs() <= MARK_EDGE + 1e-5 && pos[1].abs() <= MARK_EDGE + 1e-5);
        assert!(pos[0].abs().max(pos[1].abs()) > MARK_EDGE - 1e-5, "{pos:?}");
    }

    /// The marker rides the uniforms whatever the gun sight's strength:
    /// it is the way to find a ship, not part of the gun.
    #[test]
    fn markers_survive_a_sight_turned_off() {
        let c = cam();
        let mut s = scene(Vec3::NEG_Z, Vec3::NEG_Z, false);
        s.strength = 0.0;
        s.marks[0] = Some((Vec3::new(1.0, 0.0, 0.2), 1));
        let u = SightUniforms::new(&c, Quat::IDENTITY, &s);
        // Mode 2 (edge arrow) + kind 1 (hostile) * 4.
        assert_eq!(u.marks[0][3], 6.0);
        assert!((u.marks[0][0] - MARK_EDGE).abs() < 1e-5, "{:?}", u.marks[0]);
        assert_eq!(u.marks[1][3], 0.0);
    }

    #[test]
    fn the_glass_projection_puts_the_nose_at_the_centre_and_drops_what_is_behind() {
        assert_eq!(project(Vec3::NEG_Z, 0.7, 1.5), Some([0.0, 0.0]));
        let up = project(Vec3::new(0.0, 0.7, -1.0), 0.7, 1.5).unwrap();
        assert!((up[1] - 1.0).abs() < 1e-5 && up[0].abs() < 1e-6, "{up:?}");
        assert_eq!(project(Vec3::Z, 0.7, 1.5), None);
    }

    #[test]
    fn the_sight_follows_the_head_and_the_barrels_converge_on_the_aim() {
        assert_eq!(
            std::mem::size_of::<SightUniforms>(),
            16 * (4 + BARRELS + MARKS)
        );
        let c = cam();
        // Looking straight ahead: the sight is at the centre, the wing
        // barrels' pips converge just either side of it, closer than the
        // wings are.
        let u = SightUniforms::new(&c, Quat::IDENTITY, &scene(Vec3::NEG_Z, Vec3::NEG_Z, false));
        assert!(u.shown());
        assert_eq!(u.b[0], 0.0);
        assert!(u.pips[0][2] == 1.0 && u.pips[0][0] < 0.0 && u.pips[1][0] > 0.0);
        assert!(u.pips[0][0].abs() < 0.05, "{:?}", u.pips[0]);
        assert_eq!(u.pips[2][2], 0.0);
        // The head turns 20 degrees left: the aim (still the gaze) sits at
        // the centre of the glass and the nose is off to the right.
        let head = Quat::from_rotation_y(20f32.to_radians());
        let aim = head * Vec3::NEG_Z;
        let u = SightUniforms::new(&c, head, &scene(aim, aim, false));
        assert!(u.b[0].abs() < 1e-5, "{:?}", u.b);
        assert!(u.d[2] > 0.2, "the nose to the right: {:?}", u.d);
        // Past the gimbal the aim is clamped: it sits off-centre, toward
        // the nose, and the flag is up.
        let head = Quat::from_rotation_y(60f32.to_radians());
        let gaze = head * Vec3::NEG_Z;
        let aim = Quat::from_rotation_y(0.6) * Vec3::NEG_Z;
        let u = SightUniforms::new(&c, head, &scene(aim, gaze, true));
        assert_eq!(u.c[0], 1.0);
        assert!(u.b[0] > 0.1 && u.b[2].abs() < 1e-5, "{:?}", u.b);
        // Behind the head: nothing shown.
        let u = SightUniforms::new(&c, Quat::IDENTITY, &scene(Vec3::Z, Vec3::Z, false));
        assert!(!u.shown());
        assert!(!SightUniforms::none(&c).shown());
    }
}
