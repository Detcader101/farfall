//! The SHIP bay's hologram: the pilot's own ship in a screen-fixed pane,
//! seen by an orbit camera, its hardpoints lit and whatever is mounted on
//! them drawn in place. See `shaders/hologram.wgsl`.

use glam::Vec3;

use crate::instrument::InstrumentPass;

/// The hardpoints the hologram can show.
pub const HARDPOINTS: usize = 4;

/// What one hardpoint carries, for the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MountView {
    /// Ship frame, metres.
    pub at: Vec3,
    /// 0 empty, 1 cannon, 2 rail.
    pub kind: u8,
}

/// The orbit camera, in the model's frame (ship metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HologramCamera {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub fwd: Vec3,
    pub tan_half_fov: f32,
}

impl HologramCamera {
    /// An orbit about the origin: `yaw` round the hull's up, `pitch`
    /// above the deck, `dist` metres out, looking at the hull's middle.
    pub fn orbit(yaw: f32, pitch: f32, dist: f32, tan_half_fov: f32) -> Self {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        // Yaw 0 looks at the nose from ahead-left three-quarter when the
        // caller wants it; here yaw 0 is dead ahead of the nose (-Z).
        let dir = Vec3::new(-sy * cp, sp, -cy * cp).normalize();
        let eye = dir * dist;
        let fwd = -dir;
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let right = if right.length_squared() < 1e-6 {
            Vec3::X
        } else {
            right
        };
        let up = right.cross(fwd).normalize();
        Self {
            eye,
            right,
            up,
            fwd,
            tan_half_fov,
        }
    }
}

impl HologramCamera {
    /// Where a model point lands on the screen (NDC), for a pane centred
    /// at `pane_centre` with `pane_half_w` (NDC) on a screen of `aspect`;
    /// None behind the eye.
    pub fn project(
        &self,
        at: Vec3,
        aspect: f32,
        pane_centre: [f32; 2],
        pane_half_w: f32,
    ) -> Option<[f32; 2]> {
        let rel = at - self.eye;
        let z = rel.dot(self.fwd);
        if z <= 1e-3 {
            return None;
        }
        let u = rel.dot(self.right) / (z * self.tan_half_fov);
        let v = rel.dot(self.up) / (z * self.tan_half_fov);
        let half_h = pane_half_w * aspect;
        Some([
            pane_centre[0] + u * half_h / aspect,
            pane_centre[1] + v * half_h,
        ])
    }
}

/// A callout: where a hardpoint's label sits on the screen (NDC, the
/// point the leader line runs to), and whether its dropdown is open.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Callout {
    pub at: [f32; 2],
    pub open: bool,
}

/// The picture this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HologramScene {
    pub camera: HologramCamera,
    /// The pane: centre (NDC), half width (NDC), the screen's aspect.
    pub pane_centre: [f32; 2],
    pub pane_half_w: f32,
    pub aspect: f32,
    /// 0..1, the whole thing.
    pub visibility: f32,
    /// The hologram's hue 0..1 and saturation 0..1.
    pub hue: f32,
    pub saturation: f32,
    /// Scanlines per pane height.
    pub scanlines: f32,
    /// Which hardpoint the card has, if any.
    pub selected: Option<usize>,
    pub mounts: [MountView; HARDPOINTS],
    pub time_s: f32,
    pub height_px: f32,
    /// The whole screen is the bay: no pane frame, a deep backdrop.
    pub fullscreen: bool,
    /// One per hardpoint; None draws no leader line.
    pub callouts: [Option<Callout>; HARDPOINTS],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HologramUniforms {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    pane: [f32; 4],
    look: [f32; 4],
    misc: [f32; 4],
    pts: [[f32; 4]; HARDPOINTS],
    rows: [[f32; 4]; HARDPOINTS],
}

impl HologramUniforms {
    pub fn new(scene: &HologramScene) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let c = &scene.camera;
        let mut pts = [[0.0; 4]; HARDPOINTS];
        for (p, m) in pts.iter_mut().zip(scene.mounts.iter()) {
            *p = v4(m.at, m.kind as f32);
        }
        Self {
            eye: v4(c.eye, scene.visibility.clamp(0.0, 1.0)),
            right: v4(c.right, 0.0),
            up: v4(c.up, 0.0),
            fwd: v4(c.fwd, c.tan_half_fov.max(1e-3)),
            pane: [
                scene.pane_centre[0],
                scene.pane_centre[1],
                scene.pane_half_w.max(1e-3),
                scene.aspect,
            ],
            look: [
                scene.hue.rem_euclid(1.0),
                scene.saturation.clamp(0.0, 1.0),
                scene.scanlines.max(0.0),
                scene.selected.map_or(-1.0, |i| i as f32),
            ],
            misc: [
                scene.time_s.rem_euclid(1000.0),
                scene.height_px,
                if scene.fullscreen { 1.0 } else { 0.0 },
                0.0,
            ],
            pts,
            rows: scene.callouts.map(|c| match c {
                Some(c) => [c.at[0], c.at[1], 1.0, if c.open { 1.0 } else { 0.0 }],
                None => [0.0; 4],
            }),
        }
    }
}

pub type HologramPass = InstrumentPass;

pub fn hologram_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> HologramPass {
    // A pane like the map's: it dims the screen and draws over it.
    HologramPass::new_pane_sized(
        device,
        target_format,
        sample_count,
        "hologram",
        crate::shaders::HOLOGRAM,
        std::mem::size_of::<HologramUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> HologramScene {
        let mut mounts = [MountView {
            at: Vec3::ZERO,
            kind: 0,
        }; HARDPOINTS];
        mounts[1] = MountView {
            at: Vec3::new(-2.6, -0.35, -0.6),
            kind: 1,
        };
        HologramScene {
            camera: HologramCamera::orbit(0.75, 0.3, 24.0, 0.45),
            pane_centre: [0.1, -0.2],
            pane_half_w: 0.3,
            aspect: 1.6,
            visibility: 0.8,
            hue: 0.52,
            saturation: 1.0,
            scanlines: 120.0,
            selected: Some(1),
            mounts,
            time_s: 2.0,
            height_px: 1200.0,
            fullscreen: true,
            callouts: [
                Some(Callout {
                    at: [0.6, 0.5],
                    open: true,
                }),
                None,
                None,
                None,
            ],
        }
    }

    /// A model point ahead of the camera projects to where the pane puts
    /// it: the origin lands on the pane's centre, a point to the camera's
    /// right lands right of it, and a point behind the eye is None.
    #[test]
    fn the_camera_projects_into_the_pane() {
        let c = HologramCamera::orbit(0.0, 0.0, 20.0, 0.5);
        let centre = c.project(Vec3::ZERO, 1.6, [0.2, -0.1], 0.3).unwrap();
        assert!((centre[0] - 0.2).abs() < 1e-5 && (centre[1] + 0.1).abs() < 1e-5);
        let right = c.project(c.right * 5.0, 1.6, [0.2, -0.1], 0.3).unwrap();
        assert!(right[0] > 0.2 && (right[1] + 0.1).abs() < 1e-5);
        assert!(c.project(c.eye - c.fwd, 1.6, [0.0, 0.0], 0.3).is_none());
    }

    /// The uniform block is a wire format for hologram.wgsl: pin the lanes.
    #[test]
    fn hologram_lanes_hold_their_places() {
        let u = HologramUniforms::new(&scene());
        assert_eq!(
            std::mem::size_of::<HologramUniforms>(),
            16 * (7 + 2 * HARDPOINTS)
        );
        assert_eq!(u.misc[2], 1.0, "fullscreen");
        assert_eq!(u.rows[0], [0.6, 0.5, 1.0, 1.0], "a callout, open");
        assert_eq!(u.rows[1][2], 0.0, "no callout");
        assert_eq!(u.eye[3], 0.8, "visibility rides eye.w");
        assert_eq!(u.fwd[3], 0.45, "tan half fov rides fwd.w");
        assert_eq!(u.pane, [0.1, -0.2, 0.3, 1.6]);
        assert_eq!(u.look, [0.52, 1.0, 120.0, 1.0]);
        assert_eq!(u.pts[1], [-2.6, -0.35, -0.6, 1.0]);
        assert_eq!(u.pts[0][3], 0.0);
        let mut s = scene();
        s.selected = None;
        s.visibility = 0.0;
        let off = HologramUniforms::new(&s);
        assert_eq!(off.look[3], -1.0);
        assert_eq!(off.eye[3], 0.0, "hidden discards");
    }

    /// The orbit camera looks at the hull from `dist` out: the eye is that
    /// far away, forward points at the origin, and the basis is
    /// orthonormal; yaw 0 pitch 0 looks down the nose from ahead.
    #[test]
    fn the_orbit_camera_looks_at_the_hull() {
        let c = HologramCamera::orbit(0.0, 0.0, 20.0, 0.5);
        assert!(
            (c.eye - Vec3::new(0.0, 0.0, -20.0)).length() < 1e-5,
            "{:?}",
            c.eye
        );
        assert!((c.fwd - Vec3::Z).length() < 1e-5);
        for (yaw, pitch) in [(0.75, 0.3), (3.0, -1.0), (-2.0, 1.1)] {
            let c = HologramCamera::orbit(yaw, pitch, 24.0, 0.5);
            assert!((c.eye.length() - 24.0).abs() < 1e-4);
            assert!(
                (c.fwd + c.eye.normalize()).length() < 1e-5,
                "looks at the origin"
            );
            assert!(c.right.dot(c.up).abs() < 1e-5 && c.right.dot(c.fwd).abs() < 1e-5);
            assert!(c.up.y > 0.0, "up stays up");
        }
    }
}
