//! The right hand's laser (SPEC §5.3b(c)): the aim ray hits a virtual
//! glass [`VR_GLASS_M`] in front of the current eye's own seat — the
//! exact plane that eye's own symmetric render already treats as its
//! screen (`Game::cursor_on_glass`, `text_screen_anchor`'s `on_glass`
//! use the same `cam.fov_y`/`cam.aspect` convention) — landing on the
//! same screen-NDC `Game::cursor_screen` already speaks, so the beam
//! drives the panels through the exact code the mouse does.
//!
//! Pure geometry only; not gated to native OpenXR (unlike `xr_input`)
//! since the maths itself is target-independent — `Game::cursor_screen`
//! and the per-frame click tick call it on every build, and it is
//! simply never reached when `Game::vr` is `None` (every non-native-VR
//! build, always).

use glam::{Quat, Vec3};

/// Where the panels are laid out, ship frame: this many metres in front
/// of the current eye along its own forward axis. `vr.hud-distance`
/// from `fable/vr`, once this branch rebases onto it, is this same
/// number under a setting; until then it is fixed here, in one place.
pub const VR_GLASS_M: f32 = 1.0;

/// Intersect a ray (ship frame) with the current eye's own glass plane.
/// `eye`/`head` are that eye's seat and orientation (`ViewPose::eye_ship`
/// as `Vec3`, `ViewPose::head`); `tan_half_fov`/`aspect` its symmetric
/// camera's (`CameraFrame::fov_y`/`aspect` — the same wide symmetric
/// frame everything else in the pass draws into, cropped afterward
/// without disturbing relative NDC positions; see `xr.rs`'s module doc).
/// Returns the hit's screen NDC (`Game::cursor_screen`'s own convention:
/// x right, y up, both -1..1) and the hit point in ship frame, for the
/// beam/dot to draw to — `None` if the ray points away from the plane,
/// is parallel to it, or lands outside the frustum.
pub fn ray_hits_glass(
    origin: Vec3,
    dir: Vec3,
    eye: Vec3,
    head: Quat,
    tan_half_fov: f32,
    aspect: f32,
) -> Option<([f32; 2], Vec3)> {
    let inv = head.inverse();
    let local_origin = inv * (origin - eye);
    let local_dir = inv * dir;
    // The plane sits at local z = -VR_GLASS_M (this engine's forward is
    // -Z); a ray that never reaches it (pointing away, or exactly
    // parallel) has no hit.
    if local_dir.z >= -1.0e-5 {
        return None;
    }
    let t = (-VR_GLASS_M - local_origin.z) / local_dir.z;
    if t <= 0.0 {
        return None;
    }
    let local_hit = local_origin + local_dir * t;
    let half_w = (VR_GLASS_M * tan_half_fov * aspect).max(1.0e-4);
    let half_h = (VR_GLASS_M * tan_half_fov).max(1.0e-4);
    let ndc = [local_hit.x / half_w, local_hit.y / half_h];
    if ndc[0].abs() > 1.0 || ndc[1].abs() > 1.0 {
        return None;
    }
    Some((ndc, origin + dir * t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_straight_ahead_hits_the_plane_dead_centre() {
        let hit = ray_hits_glass(
            Vec3::ZERO,
            Vec3::NEG_Z,
            Vec3::ZERO,
            Quat::IDENTITY,
            0.7,
            1.5,
        );
        let (ndc, p) = hit.expect("straight ahead must hit");
        assert!(ndc[0].abs() < 1e-5 && ndc[1].abs() < 1e-5, "{ndc:?}");
        assert!((p - Vec3::new(0.0, 0.0, -VR_GLASS_M)).length() < 1e-5);
    }

    #[test]
    fn a_ray_pointing_away_from_the_plane_misses() {
        assert!(
            ray_hits_glass(Vec3::ZERO, Vec3::Z, Vec3::ZERO, Quat::IDENTITY, 0.7, 1.5).is_none()
        );
    }

    #[test]
    fn a_ray_parallel_to_the_plane_misses() {
        assert!(
            ray_hits_glass(Vec3::ZERO, Vec3::X, Vec3::ZERO, Quat::IDENTITY, 0.7, 1.5).is_none()
        );
    }

    #[test]
    fn a_ray_toward_the_frustums_edge_lands_near_ndc_one() {
        // Aimed at exactly the plane's right edge (x = tan_half*aspect at
        // z = -1): should land at ndc.x close to +1.
        let tan_half = 0.7_f32;
        let aspect = 1.5_f32;
        let edge_x = tan_half * aspect * VR_GLASS_M;
        let dir = Vec3::new(edge_x, 0.0, -VR_GLASS_M).normalize();
        let (ndc, _) = ray_hits_glass(
            Vec3::ZERO,
            dir,
            Vec3::ZERO,
            Quat::IDENTITY,
            tan_half,
            aspect,
        )
        .expect("hits within the frustum");
        assert!((ndc[0] - 1.0).abs() < 1e-4, "{ndc:?}");
        assert!(ndc[1].abs() < 1e-5, "{ndc:?}");
    }

    #[test]
    fn a_ray_past_the_frustums_edge_misses() {
        let tan_half = 0.7_f32;
        let aspect = 1.5_f32;
        let past_x = tan_half * aspect * VR_GLASS_M * 1.5;
        let dir = Vec3::new(past_x, 0.0, -VR_GLASS_M).normalize();
        assert!(ray_hits_glass(
            Vec3::ZERO,
            dir,
            Vec3::ZERO,
            Quat::IDENTITY,
            tan_half,
            aspect
        )
        .is_none());
    }

    #[test]
    fn a_turned_head_carries_the_plane_with_it() {
        // A head yawed 90 degrees right: its own "straight ahead" (local
        // -Z) is now world +X — a ray toward world +X should hit dead
        // centre of ITS glass, not miss because it isn't world-forward.
        let head = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
        let (ndc, _) = ray_hits_glass(Vec3::ZERO, Vec3::X, Vec3::ZERO, head, 0.7, 1.5)
            .expect("the eye's own forward, wherever it looks, hits centre");
        assert!(ndc[0].abs() < 1e-4 && ndc[1].abs() < 1e-4, "{ndc:?}");
    }

    #[test]
    fn a_hand_off_to_one_side_of_the_eye_still_resolves_correctly() {
        // Origin offset from the eye: the maths must use the ray, not
        // assume it starts at the eye (a hand is never exactly there).
        let hit = ray_hits_glass(
            Vec3::new(0.3, 0.0, 0.0),
            Vec3::new(-0.3, 0.0, -1.0).normalize(),
            Vec3::ZERO,
            Quat::IDENTITY,
            0.7,
            1.5,
        );
        let (ndc, p) = hit.expect("an angled ray from off-centre still hits");
        assert!(ndc[0].abs() < 1e-4, "aimed back to centre: {ndc:?}");
        assert!((p.x).abs() < 1e-4, "{p:?}");
    }
}
