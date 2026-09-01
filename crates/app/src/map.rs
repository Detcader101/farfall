//! The system map's projection and camera: the world at log scale in three
//! dimensions, for `map.wgsl`.
//!
//! The Moon is 60 planet radii out and the Sun 23,000: a linear map is
//! either all planet or all nothing. One map unit per decade of distance
//! from the planet, from 10⁵ m (inside the first ring) outward, in the
//! direction the thing really lies — so height above the Moon's orbital
//! plane (XZ) survives, and the map can show it on a pole. A camera orbits
//! the planet; the pilot drags it round and zooms it.

use glam::{DQuat, DVec3, Vec3};

/// Distance, metres, at the map's origin.
pub const INNER_M: f64 = 1.0e5;

/// A world position in the planet's frame to map units, all three axes.
pub fn project3(pos: DVec3) -> Vec3 {
    let d = pos.length();
    if d < 1.0 {
        return Vec3::ZERO;
    }
    let r = (d / INNER_M).log10().max(0.0);
    ((pos / d) * r).as_vec3()
}

/// A distance to a map radius.
pub fn radius(d_m: f64) -> f32 {
    (d_m / INNER_M).log10().max(0.0) as f32
}

/// Where the map pane sits on the glass: a square (in pixels) centred on
/// `centre` (the pilot's anchor for it), as `[cx, cy, half_w]` in NDC (its
/// half height is `half_w * aspect`). The rest of the screen is dimmed
/// around it.
pub fn pane_rect(aspect: f32, centre: [f32; 2]) -> [f32; 3] {
    pane_rect_sized(aspect, centre, PANE_HALF_H)
}

/// The full map pane's half height, NDC.
pub const PANE_HALF_H: f32 = 0.44;
/// The mini map's half height, NDC: a gauge, not a screen.
pub const MINI_HALF_H: f32 = 0.17;
/// Where the mini map sits on the glass: the top-right corner, clear of
/// the arch (the readout has the top-left).
pub const MINI_ANCHOR: [f32; 2] = [0.80, 0.78];

/// A pane of a given half height (see [`pane_rect`]).
pub fn pane_rect_sized(aspect: f32, centre: [f32; 2], half_h: f32) -> [f32; 3] {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    let half_w = (half_h / aspect).min(0.4);
    let c = |v: f32| {
        if v.is_finite() {
            v.clamp(-0.95, 0.95)
        } else {
            0.0
        }
    };
    [c(centre[0]), c(centre[1]), half_w]
}

/// The mini map's centre pulled back on screen. The pane is an
/// instrument, not scenery: a turned head may swing its glass anchor
/// past the rim, but the pane itself stays whole on the screen, frame
/// and all, at every head angle.
pub fn mini_centre_on_screen(aspect: f32, centre: [f32; 2], half_h: f32) -> [f32; 2] {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    let half_w = (half_h / aspect).min(0.4);
    // A hair of margin keeps the frame line off the very edge.
    let margin = 0.01;
    let c = |v: f32, half: f32| {
        let reach = (1.0 - half - margin).max(0.0);
        if v.is_finite() {
            v.clamp(-reach, reach)
        } else {
            0.0
        }
    };
    [c(centre[0], half_w), c(centre[1], half_w * aspect)]
}

pub const RINGS_MAX: u32 = 6;
pub const ZOOM_MIN: f32 = 0.35;
pub const ZOOM_MAX: f32 = 6.0;

/// The orbiting camera: yaw round the planet, pitch above the plane, zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.55,
            zoom: 1.0,
        }
    }
}

impl MapView {
    /// Radians per pixel of drag.
    const DRAG_RATE: f32 = 0.006;

    /// A drag of the mouse: right turns the map one way, up tips it over.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * Self::DRAG_RATE;
        self.pitch = (self.pitch + dy * Self::DRAG_RATE).clamp(-1.45, 1.45);
    }

    /// Wheel notches (or key taps): positive zooms in.
    pub fn zoom_by(&mut self, notches: f32) {
        self.zoom = (self.zoom * 1.15f32.powf(notches)).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// Eye position and basis, looking at the planet from `distance /
    /// zoom` away.
    pub fn camera(&self) -> (Vec3, Vec3, Vec3, Vec3) {
        let dist = 9.0 / self.zoom;
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let eye = Vec3::new(cp * sy, sp, cp * cy) * dist;
        let fwd = (-eye).normalize();
        let mut right = Vec3::Y.cross(fwd);
        if right.length() < 1e-4 {
            right = Vec3::X;
        }
        let right = right.normalize();
        let up = fwd.cross(right).normalize();
        (eye, right, up, fwd)
    }
}

/// Everything the shader needs, 15 vec4s.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MapUniforms {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    pane: [f32; 4],
    planet: [f32; 4],
    moon: [f32; 4],
    sun: [f32; 4],
    ship: [f32; 4],
    ship_right: [f32; 4],
    ship_up: [f32; 4],
    ship_fwd: [f32; 4],
    dest: [f32; 4],
    misc: [f32; 4],
    uranus: [f32; 4],
}

pub const UNIFORM_BYTES: u64 = std::mem::size_of::<MapUniforms>() as u64;

/// The world as the map needs it.
pub struct MapWorld {
    pub ship: DVec3,
    pub ship_orient: DQuat,
    pub moon: DVec3,
    pub sun: DVec3,
    pub uranus: DVec3,
    pub dest_centre: DVec3,
    pub dest_arrival_m: f64,
}

/// What the pilot has set.
pub struct MapLook {
    pub view: MapView,
    pub rings: u32,
    pub grid: bool,
    /// The dart's craft: 0 the fighter, 1 the helicopter (SPEC §6.5b).
    pub craft: f32,
    pub visibility: f32,
    pub aspect: f32,
    pub time_s: f32,
    /// The pane's centre on the glass.
    pub centre: [f32; 2],
    /// The pane's half height (NDC): the full map, or the mini map.
    pub half_h: f32,
    /// Dim the rest of the screen round the pane (the full map does; a
    /// gauge does not).
    pub dim: bool,
}

impl MapUniforms {
    pub fn new(w: &MapWorld, l: &MapLook) -> Self {
        let (eye, right, up, fwd) = l.view.camera();
        let tan_half = (40.0f32).to_radians().tan();
        let pane = pane_rect_sized(l.aspect, l.centre, l.half_h);
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let q = w.ship_orient.as_quat();
        // The ring of arrival as a map radius about the destination: in log
        // space a ring does not stay a ring, so this is indicative.
        let ring = radius(w.dest_arrival_m).max(0.04);
        Self {
            eye: v4(eye, l.visibility.clamp(0.0, 1.0)),
            right: v4(right, if l.dim { 1.0 } else { 0.0 }),
            up: v4(up, 0.0),
            fwd: v4(fwd, tan_half),
            pane: [pane[0], pane[1], pane[2], l.aspect],
            planet: [0.0, 0.0, 0.0, 0.10],
            moon: v4(project3(w.moon), 0.05),
            sun: v4(project3(w.sun), 0.22),
            ship: v4(project3(w.ship), 0.16),
            ship_right: v4(q * Vec3::X, 0.0),
            // The dart's craft rides the up axis's spare lane: 0 the
            // fighter, 1 the helicopter (SPEC §6.5b).
            ship_up: v4(q * Vec3::Y, l.craft),
            ship_fwd: v4(q * Vec3::NEG_Z, 0.0),
            dest: v4(project3(w.dest_centre), ring),
            misc: [
                l.time_s,
                l.rings.min(RINGS_MAX) as f32,
                radius(w.moon.length()),
                if l.grid { 1.0 } else { 0.0 },
            ],
            uranus: v4(project3(w.uranus), 0.14),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decades_are_units_and_directions_are_kept() {
        assert_eq!(project3(DVec3::new(INNER_M, 0.0, 0.0)), Vec3::ZERO);
        let p = project3(DVec3::new(INNER_M * 10.0, 0.0, 0.0));
        assert!((p - Vec3::X).length() < 1e-6);
        let q = project3(DVec3::new(0.0, 0.0, INNER_M * 1000.0));
        assert!((q - Vec3::Z * 3.0).length() < 1e-6);
        // Height survives: a body above the plane maps above the plane.
        let h = project3(DVec3::new(0.6, 0.8, 0.0) * INNER_M * 100.0);
        assert!((h.y - 1.6).abs() < 1e-5 && (h.x - 1.2).abs() < 1e-5, "{h}");
        // Inside the first ring: at the origin, never negative.
        assert_eq!(project3(DVec3::new(50.0, 0.0, 0.0)), Vec3::ZERO);
    }

    /// Looking back once clipped the mini map half off the top-left
    /// corner: wherever the head swings its anchor, the whole pane —
    /// frame included — must still fit on the screen.
    #[test]
    fn the_mini_pane_stays_whole_on_screen_at_every_head_angle() {
        for aspect in [1.0f32, 4.0 / 3.0, 16.0 / 9.0] {
            for centre in [[-50.0f32, 3.0], [0.95, 0.94], [1.4, -2.0], [f32::NAN, 0.2]] {
                for half_h in [MINI_HALF_H * 0.25, MINI_HALF_H, MINI_HALF_H * 4.0] {
                    let c = mini_centre_on_screen(aspect, centre, half_h);
                    let [cx, cy, hw] = pane_rect_sized(aspect, c, half_h);
                    let hh = hw * aspect;
                    assert!(cx - hw >= -1.0 && cx + hw <= 1.0, "{aspect} {centre:?}");
                    assert!(cy - hh >= -1.0 && cy + hh <= 1.0, "{aspect} {centre:?}");
                }
            }
        }
        // A pane already on the screen is left exactly where it is.
        assert_eq!(
            mini_centre_on_screen(1.5, [0.5, -0.3], MINI_HALF_H),
            [0.5, -0.3]
        );
    }

    #[test]
    fn the_pane_is_a_square_where_the_pilot_put_it() {
        for aspect in [1.0f32, 4.0 / 3.0, 16.0 / 10.0, 21.0 / 9.0] {
            let [cx, cy, hw] = pane_rect(aspect, [0.42, 0.12]);
            let hh = hw * aspect;
            assert!((cx - 0.42).abs() < 1e-6 && (cy - 0.12).abs() < 1e-6);
            assert!(hh <= 0.45 && hw <= 0.45, "{aspect}: {hw} {hh}");
        }
        // Garbage and the far beyond are kept on the glass.
        assert_eq!(pane_rect(f32::NAN, [f32::NAN, 9.0])[..2], [0.0, 0.95]);
    }

    #[test]
    fn farther_is_farther() {
        let a = radius(3.844e6);
        let b = radius(1.496e9);
        assert!(a > 1.0 && b > a && b < 5.0, "{a} {b}");
    }

    #[test]
    fn the_camera_orbits_the_planet_and_zooms_within_limits() {
        let mut v = MapView::default();
        let (eye, right, up, fwd) = v.camera();
        assert!(
            (fwd - (-eye).normalize()).length() < 1e-6,
            "not looking at the planet"
        );
        assert!(right.dot(up).abs() < 1e-5 && right.dot(fwd).abs() < 1e-5);
        assert!(up.y > 0.0, "camera upside down");
        let d0 = eye.length();
        v.zoom_by(3.0);
        assert!(v.camera().0.length() < d0, "zoom in did not come closer");
        v.zoom_by(100.0);
        assert_eq!(v.zoom, ZOOM_MAX);
        v.zoom_by(-1000.0);
        assert_eq!(v.zoom, ZOOM_MIN);
        // Pitch is bounded short of the poles; yaw is free.
        v.drag(0.0, 1.0e5);
        assert!(v.pitch <= 1.45);
        let y = v.yaw;
        v.drag(100.0, 0.0);
        assert!(v.yaw > y);
    }

    #[test]
    fn uniforms_are_fourteen_vec4s_and_carry_the_attitude() {
        assert_eq!(UNIFORM_BYTES, 15 * 16);
        let w = MapWorld {
            ship: DVec3::new(1.0e5 * 10.0, 0.0, 0.0),
            ship_orient: DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
            moon: DVec3::new(0.0, 0.0, 3.844e6),
            sun: DVec3::new(0.62, 0.42, -0.66) * 1.496e9,
            uranus: DVec3::new(0.62, 0.42, -0.66) * 1.496e9 + DVec3::X * 2.87e10,
            dest_centre: DVec3::ZERO,
            dest_arrival_m: 2.0e5,
        };
        let l = MapLook {
            view: MapView::default(),
            rings: 99,
            grid: true,
            craft: 1.0,
            visibility: 2.0,
            aspect: 1.5,
            time_s: 3.0,
            centre: [0.4, 0.1],
            half_h: PANE_HALF_H,
            dim: true,
        };
        let u = MapUniforms::new(&w, &l);
        assert_eq!(u.eye[3], 1.0);
        assert_eq!(u.misc[1], RINGS_MAX as f32);
        assert_eq!(u.ship_up[3], 1.0, "the dart's craft rides the spare lane");
        assert!(u.sun[1] > 0.0, "the Sun sits above the plane");
        assert!(u.uranus[0] > u.sun[0], "Uranus lies beyond the Sun");
        // Nose (-Z) turned about +Y by 90°: points along -X.
        assert!((u.ship_fwd[0] + 1.0).abs() < 1e-6, "{:?}", u.ship_fwd);
        assert!((u.ship[0] - 1.0).abs() < 1e-6);
    }
}
