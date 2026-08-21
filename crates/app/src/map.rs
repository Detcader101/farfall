//! The system map's projection: the world at log scale, for `map.wgsl`.
//!
//! The Moon is 60 planet radii out and the Sun 23,000: a linear map is
//! either all planet or all nothing. One map unit per decade of distance
//! from the planet, from 10⁵ m (inside the first ring) outward, directions
//! taken in the Moon's orbital plane (XZ).

use glam::DVec3;

/// Distance, metres, at the map's origin ring.
pub const INNER_M: f64 = 1.0e5;

/// A world position in the planet's frame to map units.
pub fn project(pos: DVec3) -> [f32; 2] {
    let flat = DVec3::new(pos.x, 0.0, pos.z);
    let d = flat.length();
    if d < 1.0 {
        return [0.0, 0.0];
    }
    let r = (pos.length() / INNER_M).log10().max(0.0);
    let dir = flat / d;
    [(dir.x * r) as f32, (dir.z * r) as f32]
}

/// A distance to a map radius.
pub fn radius(d_m: f64) -> f32 {
    (d_m / INNER_M).log10().max(0.0) as f32
}

/// Where the map pane sits on the glass: a square (in pixels) on the right
/// of the screen, a little above centre — clear of the menu text on the
/// left and of the instruments along the bottom rim — as `[cx, cy, half_w]`
/// in NDC (its half height is `half_w * aspect`). The rest of the screen is
/// dimmed around it.
pub fn pane_rect(aspect: f32) -> [f32; 3] {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    const RIGHT: f32 = 0.90;
    const CENTRE_Y: f32 = 0.12;
    const HALF_H: f32 = 0.44;
    // Square in pixels; never reaching past the screen's middle-left,
    // where the menu's text lives.
    let half_w = (HALF_H / aspect).min(0.4);
    [RIGHT - half_w, CENTRE_Y, half_w]
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MapUniforms {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

impl MapUniforms {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ship: DVec3,
        moon: DVec3,
        sun: DVec3,
        dest_centre: DVec3,
        dest_arrival_m: f64,
        visibility: f32,
        aspect: f32,
        time_s: f32,
    ) -> Self {
        let s = project(ship);
        let m = project(moon);
        let su = project(sun);
        let dc = project(dest_centre);
        // The ring of arrival, as a map radius about the destination: in
        // log space a ring does not stay a ring, so this is indicative —
        // the log-radius of the arrival distance, shown around the body.
        let ring = radius(dest_arrival_m).max(0.05);
        let pane = pane_rect(aspect);
        Self {
            a: [s[0], s[1], visibility.clamp(0.0, 1.0), aspect],
            b: [m[0], m[1], su[0], su[1]],
            c: [dc[0], dc[1], ring, time_s],
            d: [radius(moon.length()), pane[0], pane[1], pane[2]],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decades_are_units_and_directions_are_kept() {
        assert_eq!(project(DVec3::new(INNER_M, 0.0, 0.0)), [0.0, 0.0]);
        let p = project(DVec3::new(INNER_M * 10.0, 0.0, 0.0));
        assert!((p[0] - 1.0).abs() < 1e-6 && p[1].abs() < 1e-6);
        let q = project(DVec3::new(0.0, 0.0, INNER_M * 1000.0));
        assert!(q[0].abs() < 1e-6 && (q[1] - 3.0).abs() < 1e-6);
        // Inside the first ring: at the origin, never negative.
        assert_eq!(project(DVec3::new(50.0, 0.0, 0.0)), [0.0, 0.0]);
    }

    #[test]
    fn the_pane_is_a_square_on_the_right_clear_of_the_text() {
        for aspect in [1.0f32, 4.0 / 3.0, 16.0 / 10.0, 21.0 / 9.0] {
            let [cx, cy, hw] = pane_rect(aspect);
            let hh = hw * aspect;
            assert!((cx + hw - 0.90).abs() < 1e-6, "not right-anchored");
            assert!(cx - hw >= 0.0, "reaches into the text at {aspect}");
            // Above the bottom-rim instruments, below the top edge.
            assert!(cy - hh >= -0.36 && cy + hh <= 0.95, "{aspect}: {cy} {hh}");
        }
        assert_eq!(pane_rect(f32::NAN), pane_rect(1.0));
    }

    #[test]
    fn farther_is_farther() {
        let a = radius(3.844e6);
        let b = radius(1.496e9);
        assert!(a > 1.0 && b > a && b < 5.0, "{a} {b}");
    }
}
