//! Native OpenXR VR (SPEC §5.3): a Vulkan device born from the OpenXR
//! runtime, wrapped as a `wgpu::Device` through wgpu-hal, driving the same
//! stereo pair the flat renderer already knows how to draw — [`Game::vr`]
//! and [`crate::VrEye`]'s asymmetric tangents are the seam, exactly as the
//! WebXR bridge (`web.rs::xr_frame`, `web/xr.js`) uses them; this module
//! hands them to a runtime instead of a browser compositor.
//!
//! No GPU work happens at import time and nothing here ever launches
//! SteamVR itself — [`init`] only talks to a runtime that is already
//! running (or isn't, in which case it logs why and returns `None`), and
//! every fallible step falls back to the flat renderer rather than panic.
//!
//! ## The seam this module owns
//!
//! The flat/WebXR path already renders each eye as a *symmetric* frustum
//! wide enough to hold the eye's true asymmetric one
//! ([`crate::VrEye::symmetric`]) into its own slice of a shared target,
//! and relies on something downstream to crop the true field back out of
//! it. On the web that crop is the browser's WebGL compositor
//! (`web/xr.js`'s `tangents()` + the quad it draws). Native has no
//! browser, so [`cutout_uv`] is that same crop as a pure, tested Rust
//! function, and [`RealSession::end_frame`] runs it as a real GPU pass
//! (`shaders/xrblit.wgsl`) from the rendered pair into each eye's OpenXR
//! swapchain image.
//!
//! ## Init order
//!
//! Entry → Instance (`khr_vulkan_enable2`) → System (HMD) →
//! `graphics_requirements::<Vulkan>` → a Vulkan instance through
//! `xr_instance.create_vulkan_instance` (extensions from wgpu-hal's own
//! `Instance::desired_extensions`, so the mirror window's surface still
//! works) → the runtime's physical device → a Vulkan device through
//! `xr_instance.create_vulkan_device` (extensions/features from
//! `Adapter::required_device_extensions`/`physical_device_features`) →
//! wrap the lot with `hal::vulkan::Instance::from_raw` /
//! `Instance::expose_adapter` / `Adapter::device_from_raw` →
//! `wgpu::Instance::from_hal` / `create_adapter_from_hal` /
//! `create_device_from_hal` → an OpenXR session on the same Vulkan handles
//! → a LOCAL reference space (OpenXR's +X right, +Y up, −Z forward is the
//! ship's frame exactly, so no fix-up rotation) → two swapchains.

use ash::vk::Handle as _;
use glam::{Quat, Vec3};
use wgpu::hal;

use crate::VrEye;

/// VR HEADSET's own render-scale range (see `settings::VR_SCALE_MIN/MAX`);
/// duplicated as a plain constant so this module has no settings
/// dependency of its own.
const SCALE_MIN: f32 = 0.5;
const SCALE_MAX: f32 = 1.5;

/// Every wgpu usage a per-eye image is ever asked to support — real or
/// synthetic — in one place, so a usage-flag panic (this class has now
/// hit twice: the mirror-pair crash, then the eye-order self-check's
/// own readback) can't recur a third time from a caller adding a new
/// use of the eye image without updating every descriptor that builds
/// one. RENDER_ATTACHMENT: the crop pass draws into it. TEXTURE_BINDING:
/// the mirror-pair path and the label pass sample it. COPY_SRC: the
/// eye-order self-check reads a corner back
/// (`copy_texture_to_buffer`). `eye_texture_usage_covers_every_caller`
/// pins the exact set this covers.
const EYE_TEXTURE_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC);

// ---------------------------------------------------------------------
// Pure maths — the part of this module that runs without a headset, a
// runtime, or a GPU, and is unit-tested accordingly.
// ---------------------------------------------------------------------

/// An OpenXR field of view's four half-angles (radians; left/down
/// negative, right/up positive, as the spec defines them) as [`VrEye`]'s
/// tangents: left, right, up, down, all positive.
pub fn fov_tangents(angle_left: f32, angle_right: f32, angle_up: f32, angle_down: f32) -> [f32; 4] {
    [
        -angle_left.tan(),
        angle_right.tan(),
        angle_up.tan(),
        -angle_down.tan(),
    ]
}

/// The symmetric hull's cut-out rectangle holding one eye's true
/// asymmetric frustum, as a UV rect local to that eye's own render — 0..1
/// each way, `v = 0` at the top (this engine's texture convention; see
/// `shaders/blit.wgsl`'s "clip space has +Y up, texture space has +V
/// down"). Returns `[u0, v0, u1, v1]`. Identical maths to `web/xr.js`'s
/// `tangents()` + quad, just against our own top-down convention instead
/// of a `texImage2D`'d canvas's.
///
/// Exactly `[0, 0, 1, 1]` when the frustum is already symmetric (left ==
/// right and up == down): the crop is then the whole render, unchanged.
pub fn cutout_uv(tan: [f32; 4]) -> [f32; 4] {
    let [l, r, u, d] = tan;
    let tx = l.max(r).max(1e-6);
    let ty = u.max(d).max(1e-6);
    let u0 = (tx - l) / (2.0 * tx);
    let u1 = (tx + r) / (2.0 * tx);
    // "up" is the top of the frustum, and v = 0 is the top of the image.
    let v0 = (ty - u) / (2.0 * ty);
    let v1 = (ty + d) / (2.0 * ty);
    [u0, v0, u1, v1]
}

/// VR RECENTRE (SPEC §5.3): the new LOCAL space's own pose, expressed in
/// the old one, that re-seats the tracked space on the ship's nose. Yaw
/// and position come from the current head; pitch and roll are left out
/// on purpose — the runtime's own LOCAL space is already gravity-level,
/// and a recentre must never tilt the floor.
pub fn recentre_pose(head_orient: Quat, head_pos: Vec3) -> (Quat, Vec3) {
    (yaw_only(head_orient), head_pos)
}

/// The rotation's yaw about +Y alone, pitch and roll discarded.
fn yaw_only(q: Quat) -> Quat {
    let (yaw, _pitch, _roll) = q.to_euler(glam::EulerRot::YXZ);
    Quat::from_rotation_y(yaw)
}

/// Compose a pose given in some space's own frame with that space's own
/// pose in an ambient one, giving the first pose in the ambient frame —
/// ordinary rigid-transform composition. Recentring a session that was
/// already recentred needs this: OpenXR's `pose_in_reference_space`
/// (`RealSession::recentre` below) is always relative to the *runtime's*
/// own natural LOCAL origin, never to whichever LOCAL space happens to
/// be current, so a second recentre that skipped this would drift —
/// composing onto the current space's own located pose is what keeps a
/// long session's repeated recentres exact instead of compounding.
pub fn compose_pose(outer: (Quat, Vec3), inner: (Quat, Vec3)) -> (Quat, Vec3) {
    let (oq, op) = outer;
    let (iq, ip) = inner;
    (oq * iq, op + oq * ip)
}

/// The UV rectangle to sample from the rendered pair (both eyes, side by
/// side, left eye's half first) for eye `eye`'s crop into its own
/// OpenXR swapchain image: that eye's own half of the pair (eye 0 ↔
/// u∈[0,0.5], eye 1 ↔ u∈[0.5,1] — matching the render loop's own
/// `set_viewport((eye * ew), ...)`, `redraw` in lib.rs) composed with
/// its own [`cutout_uv`] crop, from its own tangents. Eye identity is
/// never re-derived here — `eyes[eye]` is trusted, so the only place
/// that can mismatch it is the caller passing the wrong `eyes` array.
pub fn pair_source_rect(eye: usize, eyes: &[VrEye; 2]) -> [f32; 4] {
    let local = cutout_uv(eyes[eye].tan);
    let eye_u0 = eye as f32 * 0.5;
    [
        eye_u0 + local[0] * 0.5,
        local[1],
        eye_u0 + local[2] * 0.5,
        local[3],
    ]
}

/// A centred, aspect-preserving viewport of `content`'s own shape within
/// a `window`-sized destination — the mirror's letterbox, since the
/// window is resizable independently of the headset's own eye size.
/// Returns `(x, y, w, h)` in the destination's pixels.
pub fn letterbox(window: (u32, u32), content: (u32, u32)) -> (u32, u32, u32, u32) {
    let (ww, wh) = (window.0.max(1) as f32, window.1.max(1) as f32);
    let (cw, ch) = (content.0.max(1) as f32, content.1.max(1) as f32);
    let scale = (ww / cw).min(wh / ch);
    let (w, h) = ((cw * scale).max(1.0), (ch * scale).max(1.0));
    let (x, y) = ((ww - w) * 0.5, (wh - h) * 0.5);
    (
        x.round() as u32,
        y.round() as u32,
        w.round() as u32,
        h.round() as u32,
    )
}

/// How much wider the symmetric hull is than the true asymmetric field,
/// along one axis: `2*max(a,b) / (a+b)` for that axis' two tangents (`a`,
/// `b` — left/right, or up/down). 1.0 exactly when the frustum is
/// already symmetric (a == b); grows the more the lenses are canted.
fn hull_over_true(a: f32, b: f32) -> f32 {
    let hull = a.max(b).max(1e-6);
    let true_extent = (a + b).max(1e-6);
    (2.0 * hull) / true_extent
}

/// The per-eye *render* size a headset with canted, asymmetric lenses
/// needs — bigger than the runtime's own `recommended` per-eye size,
/// which is sized for the true asymmetric field, not the wider symmetric
/// hull this engine actually renders (`VrEye::symmetric`) before cropping
/// back down (`cutout_uv`). Undersizing the render leaves fewer pixels in
/// the cropped region than the runtime's own swapchain wants, softening
/// the image on every crop; this is `recommended × (hull_tan/true_tan)`
/// per axis × `vr_scale`, rounded up, so the crop always maps at least
/// 1:1 onto the swapchain. `tans` is both eyes' own tangents — a shared
/// render serves both halves of the pair, so each axis takes whichever
/// eye needs more.
pub fn eye_render_size(recommended: (u32, u32), tans: [[f32; 4]; 2], vr_scale: f32) -> (u32, u32) {
    let (factor_x, factor_y) = hull_over_true_factors(tans);
    let scale = vr_scale.max(0.0);
    (
        ((recommended.0 as f32) * factor_x * scale).ceil().max(1.0) as u32,
        ((recommended.1 as f32) * factor_y * scale).ceil().max(1.0) as u32,
    )
}

/// The per-axis hull-vs-true factors [`eye_render_size`] inflates by —
/// exposed on its own so a caller can log the real number instead of
/// only the render size it produced, since "is this headset's own
/// asymmetry really this many percent" is a question about the live
/// tangents, not something the final pixel count alone answers.
pub fn hull_over_true_factors(tans: [[f32; 4]; 2]) -> (f32, f32) {
    let factor_x = tans
        .iter()
        .map(|t| hull_over_true(t[0], t[1]))
        .fold(0.0f32, f32::max);
    let factor_y = tans
        .iter()
        .map(|t| hull_over_true(t[2], t[3]))
        .fold(0.0f32, f32::max);
    (factor_x, factor_y)
}

/// `FARFALL_VR_SCRIPT`: a synthetic bench headset's deterministic head
/// motion — a pure function of bench time, so a comfort/depth regression
/// shows up the same way on every run, on any machine, with no headset
/// attached. `Still` is the default: a synthetic headset with no script
/// is a fixed observer, exactly where a real one's tracking would settle
/// once the pilot stopped moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadScript {
    #[default]
    Still,
    /// A yaw sweep, ±60°.
    Look,
    /// 6-DoF: ±0.25 m sideways, up to 0.2 m forward, no rotation — the
    /// comfort/parallax invariants (overlay depth, dial-face disparity)
    /// depend on eye *position*, which `Look`/`Nod`/`Spin` never move.
    Lean,
    /// A pitch nod, ±25°.
    Nod,
    /// A full yaw turn over the whole bench run.
    Spin,
}

impl HeadScript {
    /// `FARFALL_VR_SCRIPT=still|look|lean|nod|spin`; anything else (or
    /// unset) is `Still`.
    pub fn parse(s: &str) -> Self {
        match s {
            "look" => Self::Look,
            "lean" => Self::Lean,
            "nod" => Self::Nod,
            "spin" => Self::Spin,
            _ => Self::Still,
        }
    }
}

/// The synthetic headset's head pose (orientation, position — ship/
/// LOCAL frame) at bench time `t` seconds, for `script`. `bench_seconds`
/// only shapes `Spin` (one full turn over the run); the oscillating
/// scripts (`Look`/`Lean`/`Nod`) run their own fixed period regardless
/// of run length, so a short bench still exercises the full sweep.
/// Feeds `VrEye` exactly as a real runtime's `locate_views` would — see
/// `SynthSession::begin_frame`.
pub fn synth_head_pose(script: HeadScript, t: f32, bench_seconds: f32) -> (Quat, Vec3) {
    use std::f32::consts::TAU;
    match script {
        HeadScript::Still => (Quat::IDENTITY, Vec3::ZERO),
        HeadScript::Look => {
            let yaw = 60f32.to_radians() * (t * TAU / 8.0).sin();
            (Quat::from_rotation_y(yaw), Vec3::ZERO)
        }
        HeadScript::Lean => {
            let w = t * TAU / 6.0;
            let pos = Vec3::new(0.25 * w.sin(), 0.0, -0.2 * w.cos());
            (Quat::IDENTITY, pos)
        }
        HeadScript::Nod => {
            let pitch = 25f32.to_radians() * (t * TAU / 5.0).sin();
            (Quat::from_rotation_x(pitch), Vec3::ZERO)
        }
        HeadScript::Spin => {
            let period = bench_seconds.max(1.0);
            let yaw = (t / period).fract() * TAU;
            (Quat::from_rotation_y(yaw), Vec3::ZERO)
        }
    }
}

/// The synthetic headset's own two eyes, split by `ipd` about `head`'s
/// own local +X from `pos`/`head` (the current [`synth_head_pose`]) —
/// pulled out of `SynthSession::begin_frame` as a pure function so the
/// eye-position split itself has a direct, GPU-free test: "is the eye
/// VALUE actually different" was one of three candidates a real-runtime
/// capture narrowed a mono-cabin comfort bug to, and this settles it
/// for the synthetic path outright rather than by reading the code
/// again.
pub fn synth_eyes(head: Quat, pos: Vec3, ipd: f32, tan: [[f32; 4]; 2]) -> [VrEye; 2] {
    let half = ipd * 0.5;
    std::array::from_fn(|i| {
        let local_x = if i == 0 { -half } else { half };
        VrEye {
            head,
            pos: pos + head * Vec3::new(local_x, 0.0, 0.0),
            tan: tan[i],
        }
    })
}

#[cfg(test)]
mod pure_math_tests {
    use super::*;
    use std::f32::consts::FRAC_PI_4;

    /// This class of bug has hit twice now: the mirror-pair crash (a
    /// missing TEXTURE_BINDING), then the eye-order self-check's own
    /// first real-runtime run (a missing COPY_SRC). Every wgpu usage any
    /// caller needs from a per-eye image must be a part of
    /// EYE_TEXTURE_USAGE, or the descriptor that grants it has silently
    /// fallen out of sync with what actually reads the texture.
    #[test]
    fn eye_texture_usage_covers_every_caller() {
        // xr_composite's crop pass draws into it.
        assert!(EYE_TEXTURE_USAGE.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        // FARFALL_VR_MIRROR=pair and the L/R label both sample it.
        assert!(EYE_TEXTURE_USAGE.contains(wgpu::TextureUsages::TEXTURE_BINDING));
        // The eye-order self-check reads a corner back.
        assert!(EYE_TEXTURE_USAGE.contains(wgpu::TextureUsages::COPY_SRC));
    }

    #[test]
    fn a_symmetric_fov_gives_matching_left_right_and_up_down_tangents() {
        let tan = fov_tangents(-FRAC_PI_4, FRAC_PI_4, FRAC_PI_4, -FRAC_PI_4);
        for t in tan {
            assert!((t - 1.0).abs() < 1e-5, "{tan:?}");
        }
    }

    #[test]
    fn a_symmetric_frustums_cutout_is_the_whole_render() {
        let uv = cutout_uv([1.0, 1.0, 0.8, 0.8]);
        assert!((uv[0] - 0.0).abs() < 1e-6);
        assert!((uv[1] - 0.0).abs() < 1e-6);
        assert!((uv[2] - 1.0).abs() < 1e-6);
        assert!((uv[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_wider_left_frustum_cuts_out_the_left_edge_and_stops_short_of_the_right() {
        // left tangent bigger than right: the symmetric hull is as wide
        // as the left side needs, so the true frustum touches u=0 (the
        // render's own left edge) and never reaches u=1.
        let uv = cutout_uv([2.0, 1.0, 1.0, 1.0]);
        assert!((uv[0] - 0.0).abs() < 1e-6, "{uv:?}");
        assert!(uv[2] < 1.0 - 1e-6, "{uv:?}");
    }

    #[test]
    fn a_wider_right_frustum_cuts_out_the_right_edge_and_starts_short_of_the_left() {
        let uv = cutout_uv([1.0, 2.0, 1.0, 1.0]);
        assert!((uv[2] - 1.0).abs() < 1e-6, "{uv:?}");
        assert!(uv[0] > 1e-6, "{uv:?}");
    }

    #[test]
    fn a_taller_top_frustum_cuts_out_the_top_edge_and_stops_short_of_the_bottom() {
        // up tangent bigger than down: the crop touches v=0 (the top)
        // and stops before v=1 (the bottom).
        let uv = cutout_uv([1.0, 1.0, 2.0, 1.0]);
        assert!((uv[1] - 0.0).abs() < 1e-6, "{uv:?}");
        assert!(uv[3] < 1.0 - 1e-6, "{uv:?}");
    }

    #[test]
    fn recentre_keeps_the_heads_own_position() {
        let pos = Vec3::new(1.0, 2.0, 3.0);
        let (_, out_pos) = recentre_pose(Quat::IDENTITY, pos);
        assert_eq!(out_pos, pos);
    }

    #[test]
    fn recentre_drops_pitch_and_roll_but_keeps_the_yaw() {
        let yaw0 = 0.7_f32;
        let head =
            Quat::from_rotation_y(yaw0) * Quat::from_rotation_x(0.35) * Quat::from_rotation_z(-0.2);
        let (out, _) = recentre_pose(head, Vec3::ZERO);
        let (yaw, pitch, roll) = out.to_euler(glam::EulerRot::YXZ);
        assert!((yaw - yaw0).abs() < 1e-4, "yaw {yaw} vs {yaw0}");
        assert!(pitch.abs() < 1e-6, "pitch leaked: {pitch}");
        assert!(roll.abs() < 1e-6, "roll leaked: {roll}");
    }

    #[test]
    fn a_level_look_recentres_to_the_identity() {
        let (out, _) = recentre_pose(Quat::IDENTITY, Vec3::ZERO);
        assert!(out.angle_between(Quat::IDENTITY) < 1e-6);
    }

    #[test]
    fn composing_onto_the_identity_space_changes_nothing() {
        let inner = (Quat::from_rotation_y(0.4), Vec3::new(1.0, 2.0, 3.0));
        let (q, p) = compose_pose((Quat::IDENTITY, Vec3::ZERO), inner);
        assert!(q.angle_between(inner.0) < 1e-6);
        assert!((p - inner.1).length() < 1e-6);
    }

    #[test]
    fn composing_two_pure_translations_adds_them() {
        let outer = (Quat::IDENTITY, Vec3::new(5.0, 0.0, 0.0));
        let inner = (Quat::IDENTITY, Vec3::new(0.0, 0.0, 2.0));
        let (q, p) = compose_pose(outer, inner);
        assert!(q.angle_between(Quat::IDENTITY) < 1e-6);
        assert!((p - Vec3::new(5.0, 0.0, 2.0)).length() < 1e-6);
    }

    #[test]
    fn a_recentred_space_composed_again_is_the_second_recentres_own_seat() {
        // Recentre once (yaw 90 degrees, moved 1m on X) — this is the
        // pose a *second* recentre must land on, expressed in the
        // runtime's own natural origin, given the same head pose again
        // relative to the now-current (already-recentred) space.
        let first = (Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), Vec3::X);
        // A second recentre with the head dead ahead and unmoved,
        // relative to the space `first` established, must land exactly
        // back on `first` itself in natural terms — recentring twice
        // with no head motion between them is a no-op.
        let (q, p) = compose_pose(first, (Quat::IDENTITY, Vec3::ZERO));
        // `Quat::angle_between` is glam's `acos_approx`, not exact acos —
        // it reads a few tenths of a degree off zero even for the same
        // value composed with itself, so the tolerance here is looser
        // than the other pure-math tests', which never round-trip
        // through it this way.
        assert!(
            q.angle_between(first.0) < 1e-3,
            "q={q:?} first.0={:?}",
            first.0
        );
        assert!((p - first.1).length() < 1e-6);
    }

    #[test]
    fn a_window_the_same_shape_as_its_content_fills_it_exactly() {
        let (x, y, w, h) = letterbox((1000, 500), (2000, 1000));
        assert_eq!((x, y, w, h), (0, 0, 1000, 500));
    }

    #[test]
    fn a_wide_window_letterboxes_a_taller_contents_sides() {
        // Content is 1:1, the window is wider: bars go left and right.
        let (x, y, w, h) = letterbox((800, 400), (400, 400));
        assert_eq!((y, h), (0, 400), "fills the window's height");
        assert!(x > 0 && w < 800, "{x},{w}");
        assert_eq!(x * 2 + w, 800, "centred");
    }

    #[test]
    fn a_tall_window_letterboxes_a_wider_contents_top_and_bottom() {
        let (x, y, w, h) = letterbox((400, 800), (400, 400));
        assert_eq!((x, w), (0, 400), "fills the window's width");
        assert!(y > 0 && h < 800, "{y},{h}");
        assert_eq!(y * 2 + h, 800, "centred");
    }

    /// Two distinguishable eyes: index 0 skewed wide on its own left
    /// (as a real left eye's outboard/temporal edge is), index 1 wide
    /// on its own right — so a mismatch (either eye's crop coming from
    /// the wrong half, or from the other eye's fov) shows up as a wrong
    /// *shape* of crop, not just a wrong number.
    fn distinguishable_eyes() -> [VrEye; 2] {
        let e = |tan: [f32; 4]| VrEye {
            head: Quat::IDENTITY,
            pos: Vec3::ZERO,
            tan,
        };
        [e([2.0, 1.0, 1.0, 1.0]), e([1.0, 2.0, 1.0, 1.0])]
    }

    #[test]
    fn eye_0s_crop_samples_the_pairs_left_half() {
        let eyes = distinguishable_eyes();
        let rect = pair_source_rect(0, &eyes);
        assert!(
            rect[0] >= 0.0 && rect[2] <= 0.5,
            "{rect:?} left of the midline"
        );
    }

    #[test]
    fn eye_1s_crop_samples_the_pairs_right_half() {
        let eyes = distinguishable_eyes();
        let rect = pair_source_rect(1, &eyes);
        assert!(
            rect[0] >= 0.5 && rect[2] <= 1.0,
            "{rect:?} right of the midline"
        );
    }

    #[test]
    fn each_eyes_crop_uses_that_same_eyes_own_fov_not_the_others() {
        let eyes = distinguishable_eyes();
        // Eye 0's tan is wider on its own left (index 0 of its tan):
        // within its own half, its crop should touch that half's own
        // left edge (u local 0) and stop short on the right — the
        // mirror-image shape `a_wider_left_frustum_...` already pins
        // for `cutout_uv` alone. If eye 1's tan (wide on ITS right) were
        // used instead, the crop would touch the OTHER edge.
        let rect0 = pair_source_rect(0, &eyes);
        assert!(
            (rect0[0] - 0.0).abs() < 1e-6,
            "{rect0:?}: eye 0 should touch u=0"
        );
        assert!(
            rect0[2] < 0.5 - 1e-6,
            "{rect0:?}: eye 0 should stop short of the midline"
        );
        let rect1 = pair_source_rect(1, &eyes);
        assert!(
            (rect1[2] - 1.0).abs() < 1e-6,
            "{rect1:?}: eye 1 should touch u=1"
        );
        assert!(
            rect1[0] > 0.5 + 1e-6,
            "{rect1:?}: eye 1 should start past the midline"
        );
    }

    /// SPEC §5.3: the regression guard for the crop-uniform race a
    /// synth capture caught (a9238a5 → 7c64062's own stereo-disparity
    /// self-check still failed, 0.07% differing — both eye textures
    /// cropped from the SAME half of the pair). `xr_composite` used one
    /// shared `XrBlitPass` for both eyes' crop draws; `update()`'s two
    /// calls (eye 0's rect, then eye 1's) both landed in that one
    /// buffer before the single `queue.submit()`, so both draws read
    /// eye 1's rect. This test alone cannot see that runtime race (it
    /// is now impossible by construction — `VrPair::to_swapchain` is
    /// `[XrBlitPass; 2]`), but it does pin the one thing that could
    /// silently make the fix moot: that the *input* rects genuinely
    /// differ for two eyes shaped like the real Index-mirrored fov this
    /// engine actually ships (`fov_tangents`, angles mirrored left-for-
    /// right the way `SynthSession::eye_tan` builds them) — not only
    /// the exaggerated `distinguishable_eyes()` fixture above.
    #[test]
    fn a_realistic_mirrored_eye_pair_gets_two_different_crop_rects() {
        let tan_of = |left_deg: f32, right_deg: f32, up_deg: f32, down_deg: f32| {
            fov_tangents(
                (-left_deg).to_radians(),
                right_deg.to_radians(),
                up_deg.to_radians(),
                (-down_deg).to_radians(),
            )
        };
        let eyes = [
            VrEye {
                head: Quat::IDENTITY,
                pos: Vec3::new(-0.032, 0.0, 0.0),
                tan: tan_of(54.0, 46.0, 55.0, 55.0),
            },
            VrEye {
                head: Quat::IDENTITY,
                pos: Vec3::new(0.032, 0.0, 0.0),
                tan: tan_of(46.0, 54.0, 55.0, 55.0),
            },
        ];
        let rect0 = pair_source_rect(0, &eyes);
        let rect1 = pair_source_rect(1, &eyes);
        assert_ne!(
            rect0, rect1,
            "a real Index-shaped eye pair must crop with two different rects, \
             not the same one drawn twice"
        );
    }

    #[test]
    fn a_symmetric_headsets_render_needs_no_inflation() {
        let symmetric = [1.0, 1.0, 1.0, 1.0];
        let (w, h) = eye_render_size((2016, 2240), [symmetric, symmetric], 1.0);
        assert_eq!((w, h), (2016, 2240));
    }

    #[test]
    fn a_canted_headsets_render_is_inflated_by_the_hull_over_true_ratio() {
        // left=2, right=1: hull=2, true=3, factor=4/3.
        let tan = [2.0, 1.0, 1.0, 1.0];
        let (w, _) = eye_render_size((900, 1000), [tan, tan], 1.0);
        assert_eq!(w, (900.0f32 * (4.0 / 3.0)).ceil() as u32);
    }

    #[test]
    fn the_render_size_takes_whichever_eye_needs_more_per_axis() {
        // Eye 0 needs more width, eye 1 needs more height — a shared
        // render must satisfy both, not just whichever eye came first.
        let wide = [3.0, 1.0, 1.0, 1.0]; // x factor 3/2, y factor 1
        let tall = [1.0, 1.0, 3.0, 1.0]; // x factor 1, y factor 3/2
        let (w, h) = eye_render_size((1000, 1000), [wide, tall], 1.0);
        assert_eq!(w, (1000.0f32 * 1.5).ceil() as u32, "eye 0's width need");
        assert_eq!(h, (1000.0f32 * 1.5).ceil() as u32, "eye 1's height need");
    }

    #[test]
    fn vr_scale_multiplies_the_render_size_directly() {
        let symmetric = [1.0, 1.0, 1.0, 1.0];
        let (w, h) = eye_render_size((1000, 1000), [symmetric, symmetric], 1.5);
        assert_eq!((w, h), (1500, 1500));
    }

    #[test]
    fn the_render_never_undersamples_the_crop_it_feeds() {
        // For any tan set, the cropped fraction of the render times the
        // render's own size must reach at least the recommended size —
        // the whole point of the inflation.
        let tan = [2.0, 1.0, 1.5, 0.5];
        let recommended = (1832, 1920);
        let (rw, rh) = eye_render_size(recommended, [tan, tan], 1.0);
        let crop = cutout_uv(tan);
        let cropped_w = (rw as f32) * (crop[2] - crop[0]);
        let cropped_h = (rh as f32) * (crop[3] - crop[1]);
        assert!(
            cropped_w >= recommended.0 as f32 - 1.0,
            "{cropped_w} < {}",
            recommended.0
        );
        assert!(
            cropped_h >= recommended.1 as f32 - 1.0,
            "{cropped_h} < {}",
            recommended.1
        );
    }

    #[test]
    fn head_script_parses_the_five_names_and_anything_else_is_still() {
        assert_eq!(HeadScript::parse("look"), HeadScript::Look);
        assert_eq!(HeadScript::parse("lean"), HeadScript::Lean);
        assert_eq!(HeadScript::parse("nod"), HeadScript::Nod);
        assert_eq!(HeadScript::parse("spin"), HeadScript::Spin);
        assert_eq!(HeadScript::parse("still"), HeadScript::Still);
        assert_eq!(HeadScript::parse("bogus"), HeadScript::Still);
        assert_eq!(HeadScript::parse(""), HeadScript::Still);
        assert_eq!(HeadScript::default(), HeadScript::Still);
    }

    #[test]
    fn still_never_moves() {
        for t in [0.0, 1.3, 50.0] {
            let (q, p) = synth_head_pose(HeadScript::Still, t, 20.0);
            assert!(q.angle_between(Quat::IDENTITY) < 1e-6);
            assert_eq!(p, Vec3::ZERO);
        }
    }

    /// A real-runtime capture narrowed a mono-cabin comfort bug to one
    /// of three candidates, including "the synth VrView's own eye
    /// positions might not actually differ." They do: eye 0 sits at
    /// exactly `-ipd/2` and eye 1 at `+ipd/2` along the head's own
    /// local +X, for any head/position the current script has produced
    /// — this settles that candidate outright for the synthetic path.
    #[test]
    fn synth_eyes_split_by_the_full_ipd_about_the_head() {
        let ipd = 0.064;
        let tan = [[1.0; 4]; 2];
        for (head, pos) in [
            (Quat::IDENTITY, Vec3::ZERO),
            (Quat::from_rotation_y(0.7), Vec3::new(0.1, 0.0, -0.2)),
        ] {
            let eyes = synth_eyes(head, pos, ipd, tan);
            let sep = (eyes[1].pos - eyes[0].pos).length();
            assert!(
                (sep - ipd).abs() < 1e-6,
                "eyes must be exactly ipd apart: {:?} vs {:?} (sep {sep})",
                eyes[0].pos,
                eyes[1].pos
            );
            // Eye 0 is the more negative along the head's own local +X
            // (a level head: world x too) — the runtime-order convention
            // this whole branch's diagnostics assume.
            let local = |p: Vec3| head.inverse() * (p - pos);
            assert!(
                local(eyes[0].pos).x < local(eyes[1].pos).x,
                "eye 0 must be the left (more negative local x) eye"
            );
        }
    }

    #[test]
    fn look_sweeps_yaw_to_exactly_60_degrees_at_its_peak_and_no_further() {
        let mut max_yaw = 0.0f32;
        let mut t = 0.0f32;
        while t < 8.0 {
            let (q, _) = synth_head_pose(HeadScript::Look, t, 20.0);
            let (yaw, _, _) = q.to_euler(glam::EulerRot::YXZ);
            max_yaw = max_yaw.max(yaw.abs());
            t += 0.01;
        }
        assert!(
            (max_yaw.to_degrees() - 60.0).abs() < 0.5,
            "peak yaw {} deg",
            max_yaw.to_degrees()
        );
    }

    #[test]
    fn lean_moves_position_on_two_axes_and_never_rotates() {
        let (q0, p0) = synth_head_pose(HeadScript::Lean, 0.0, 20.0);
        let (_, p1) = synth_head_pose(HeadScript::Lean, 1.5, 20.0);
        assert!(
            q0.angle_between(Quat::IDENTITY) < 1e-6,
            "lean never turns the head"
        );
        assert!(p0 != p1, "position must actually move over time");
        let mut max_x = 0.0f32;
        let mut max_z = 0.0f32;
        let mut t = 0.0f32;
        while t < 6.0 {
            let (_, p) = synth_head_pose(HeadScript::Lean, t, 20.0);
            max_x = max_x.max(p.x.abs());
            max_z = max_z.max(p.z.abs());
            assert_eq!(p.y, 0.0, "lean does not lift or drop the head");
            t += 0.01;
        }
        assert!((max_x - 0.25).abs() < 0.01, "sideways peak {max_x}");
        assert!((max_z - 0.2).abs() < 0.01, "forward peak {max_z}");
    }

    #[test]
    fn nod_pitches_to_exactly_25_degrees_at_its_peak() {
        let mut max_pitch = 0.0f32;
        let mut t = 0.0f32;
        while t < 5.0 {
            let (q, _) = synth_head_pose(HeadScript::Nod, t, 20.0);
            let (_, pitch, _) = q.to_euler(glam::EulerRot::YXZ);
            max_pitch = max_pitch.max(pitch.abs());
            t += 0.01;
        }
        assert!(
            (max_pitch.to_degrees() - 25.0).abs() < 0.5,
            "peak pitch {} deg",
            max_pitch.to_degrees()
        );
    }

    #[test]
    fn spin_completes_exactly_one_full_turn_over_the_bench() {
        let bench_seconds = 20.0;
        let (start, _) = synth_head_pose(HeadScript::Spin, 0.0, bench_seconds);
        let (quarter, _) = synth_head_pose(HeadScript::Spin, bench_seconds * 0.25, bench_seconds);
        let (mid, _) = synth_head_pose(HeadScript::Spin, bench_seconds * 0.5, bench_seconds);
        assert!(start.angle_between(Quat::IDENTITY) < 1e-4);
        assert!(
            quarter.angle_between(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)) < 1e-3,
            "{quarter:?}"
        );
        assert!(
            mid.angle_between(Quat::from_rotation_y(std::f32::consts::PI)) < 1e-3,
            "{mid:?}"
        );
        // A whole number of periods later, the turn has come all the way
        // back round to the start — one full turn per `bench_seconds`,
        // not a partial or a multiple of one.
        let (back_around, _) =
            synth_head_pose(HeadScript::Spin, bench_seconds * 2.0, bench_seconds);
        assert!(
            back_around.angle_between(Quat::IDENTITY) < 1e-3,
            "{back_around:?}"
        );
    }
}

// ---------------------------------------------------------------------
// The runtime: OpenXR + Vulkan + wgpu-hal. Everything below this line
// touches a real loader/runtime/GPU and cannot run in a unit test; it is
// exercised by compiling (`cargo check`) and, eventually, by a headset.
// ---------------------------------------------------------------------

/// Why VR init stopped short of a session — always logged, never a panic.
#[derive(Debug)]
pub enum Error {
    Loader(String),
    Instance(String),
    NoHmd(String),
    Vulkan(String),
    Device(String),
    Session(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, msg) = match self {
            Error::Loader(m) => ("loader", m),
            Error::Instance(m) => ("instance", m),
            Error::NoHmd(m) => ("no headset", m),
            Error::Vulkan(m) => ("vulkan", m),
            Error::Device(m) => ("device", m),
            Error::Session(m) => ("session", m),
        };
        write!(f, "{kind}: {msg}")
    }
}

/// The wgpu handles a VR session hands to `init_gpu`, in place of the
/// ones `request_adapter`/`request_device` would have produced.
pub struct VrDevice {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// One eye's OpenXR swapchain: its images, each wrapped once as a
/// `wgpu::TextureView` (session lifetime — re-wrapping per frame would
/// leak wgpu-hal identities).
struct EyeSwapchain {
    handle: openxr::Swapchain<openxr::Vulkan>,
    /// Parallel to `views` — kept so the eye-order self-check can read
    /// back the currently-acquired image (`copy_texture_to_buffer` needs
    /// the `wgpu::Texture` itself, not a view of it).
    textures: Vec<wgpu::Texture>,
    views: Vec<wgpu::TextureView>,
    /// Set by `acquire`, consumed by `release`.
    acquired: Option<usize>,
}

impl EyeSwapchain {
    fn acquire(&mut self) -> &wgpu::TextureView {
        let index = self.handle.acquire_image().expect("xr: acquire_image");
        self.handle
            .wait_image(openxr::Duration::INFINITE)
            .expect("xr: wait_image");
        self.acquired = Some(index as usize);
        &self.views[index as usize]
    }

    fn release(&mut self) {
        if self.acquired.take().is_some() {
            let _ = self.handle.release_image();
        }
    }

    /// The image `acquire` most recently handed out — `None` before the
    /// first `acquire` of a session.
    fn acquired_texture(&self) -> Option<&wgpu::Texture> {
        self.acquired.map(|i| &self.textures[i])
    }
}

/// What a frame's `begin_frame` found.
pub enum Frame {
    /// The session is not `READY`/`SYNCHRONIZED`/`VISIBLE`/`FOCUSED` yet
    /// (e.g. between launch and the runtime attaching): nothing to draw
    /// or present this call.
    Idle,
    /// A frame is open (`frame_stream.begin()` was called) and must be
    /// matched by [`RealSession::end_frame`] or [`RealSession::skip_frame`].
    Open {
        should_render: bool,
        eyes: [VrEye; 2],
    },
    /// The runtime is gone (instance loss, or the session exited):
    /// caller drops this `RealSession` and falls back to flat.
    Lost,
}

/// A native VR session, real or synthetic — the one type every other
/// module reaches for (`Gpu::xr: Option<XrSession>`), so a synthetic
/// bench headset (`FARFALL_VR=synth`) runs the exact same call sequence
/// (`begin_frame` → `acquire_eye` → `end_frame`/`skip_frame`) and the
/// exact same `xr_composite` crop/label/mirror pass a real one does —
/// the comfort and eye-order self-checks it exists to let a bench run
/// without a headset are only honest if nothing downstream of this type
/// can tell the difference.
pub enum XrSession {
    Real(Box<RealSession>),
    Synth(Box<SynthSession>),
}

impl XrSession {
    /// The runtime's own recommended per-eye size, unscaled.
    pub fn eye_size(&self) -> (u32, u32) {
        match self {
            Self::Real(s) => s.eye_size(),
            Self::Synth(s) => s.eye_size(),
        }
    }

    /// See [`RealSession::recommended_size`].
    pub fn recommended_size(&self) -> (u32, u32) {
        match self {
            Self::Real(s) => s.recommended_size(),
            Self::Synth(s) => s.recommended_size(),
        }
    }

    /// The runtime's own current display rate, Hz — `None` off an
    /// unsupporting runtime or build; a synthetic headset always
    /// reports a fixed 90 Hz (SPEC §5.3, `FARFALL_VR=synth`).
    pub fn display_refresh_hz(&self) -> Option<f32> {
        match self {
            Self::Real(s) => s.display_refresh_hz(),
            Self::Synth(s) => s.display_refresh_hz(),
        }
    }

    /// Wall-clock time the last `begin_frame` spent waiting on the
    /// runtime's own pacing — always zero for a synthetic headset,
    /// which free-runs (SPEC §5.3): there is no compositor to wait on.
    pub fn last_wait_ms(&self) -> f32 {
        match self {
            Self::Real(s) => s.last_wait_ms(),
            Self::Synth(s) => s.last_wait_ms(),
        }
    }

    /// See [`RealSession::begin_frame`].
    pub fn begin_frame(&mut self, force_render: bool) -> Frame {
        match self {
            Self::Real(s) => s.begin_frame(force_render),
            Self::Synth(s) => s.begin_frame(force_render),
        }
    }

    /// See [`RealSession::skip_frame`].
    pub fn skip_frame(&mut self) {
        match self {
            Self::Real(s) => s.skip_frame(),
            Self::Synth(s) => s.skip_frame(),
        }
    }

    /// This eye's swapchain image to render into.
    pub fn acquire_eye(&mut self, eye: usize) -> &wgpu::TextureView {
        match self {
            Self::Real(s) => s.acquire_eye(eye),
            Self::Synth(s) => s.acquire_eye(eye),
        }
    }

    /// The `wgpu::Texture` behind this eye's currently-acquired image —
    /// the eye-order self-check's readback target.
    pub fn acquired_eye_texture(&self, eye: usize) -> Option<&wgpu::Texture> {
        match self {
            Self::Real(s) => s.acquired_eye_texture(eye),
            Self::Synth(s) => s.acquired_eye_texture(eye),
        }
    }

    /// See [`RealSession::end_frame`].
    pub fn end_frame(&mut self) {
        match self {
            Self::Real(s) => s.end_frame(),
            Self::Synth(s) => s.end_frame(),
        }
    }

    /// VR RECENTRE — a no-op for a synthetic headset, whose head pose is
    /// already a deterministic pure function of bench time
    /// ([`synth_head_pose`]), not a runtime's own drifting tracking
    /// origin; there is nothing here for a recentre to correct.
    pub fn recentre(&mut self, head: VrEye) {
        if let Self::Real(s) = self {
            s.recentre(head);
        }
    }

    /// Whether this is the synthetic bench headset, not a real runtime
    /// — for the stamp line's `synth=1`.
    pub fn is_synth(&self) -> bool {
        matches!(self, Self::Synth(_))
    }
}

/// A running native VR session: the OpenXR side of the seam, plus the
/// two swapchains the flat-rendered pair is cropped into each frame.
pub struct RealSession {
    instance: openxr::Instance,
    session: openxr::Session<openxr::Vulkan>,
    frame_wait: openxr::FrameWaiter,
    frame_stream: openxr::FrameStream<openxr::Vulkan>,
    space: openxr::Space,
    /// The runtime's own, never-recentred LOCAL origin — see the note at
    /// its creation in `try_init`.
    natural_local: openxr::Space,
    blend_mode: openxr::EnvironmentBlendMode,
    eyes: [EyeSwapchain; 2],
    eye_size: (u32, u32),
    /// The runtime's own recommended per-eye size, unscaled — the input
    /// to [`eye_render_size`]; see the note where `eye_size` is set.
    recommended_size: (u32, u32),
    /// The runtime's own current display rate, Hz, read once at session
    /// start if `fb_display_refresh_rate` was offered — `None` off an
    /// unsupporting runtime, not a failure. Never set (SPEC §5.3);
    /// bench uses it for `hz=`/`headroom_ms=`, reporting `hz=unknown`
    /// rather than assuming 90 when it is `None`.
    display_refresh_hz: Option<f32>,
    /// Wall-clock time the last `begin_frame` spent inside
    /// `FrameWaiter::wait` — the runtime's own pacing, not this app's
    /// (SPEC §5.3: never assumed, never set, only ever paced by this
    /// wait). Read by the bench stamp's `xr_wait_ms`.
    last_wait_ms: f32,
    event_storage: openxr::EventDataBuffer,
    session_running: bool,
    frame_open: bool,
    /// Set once the first located frame has logged its diagnostic line
    /// (see `begin_frame`) — never repeated, so a long session's log
    /// isn't spammed once a frame.
    logged_eye_diagnostic: bool,
    /// Set once a `SessionState::FOCUSED` event has been seen, never
    /// cleared — the gate for the one-shot auto-recentre below. Losing
    /// and regaining focus (the SteamVR dashboard, say) must not
    /// re-seat LOCAL a second time out from under a flying pilot.
    became_focused: bool,
    /// Set once the auto-recentre has run (see `begin_frame`): LOCAL is
    /// wherever the runtime put it at session start, which can be
    /// heavily yawed from where the pilot is actually facing if the
    /// headset was lying on the desk when the session came up — this
    /// runs the same maths as VR RECENTRE, once, the first time a frame
    /// is both FOCUSED and has a real located head, so the seat is
    /// never left silently wrong for however long it takes the pilot to
    /// notice and press HOME themselves (SPEC §5.3).
    auto_recentred: bool,
    /// Whether the runtime itself asked for this frame's content
    /// (`XrFrameState::should_render`), consulted by `end_frame`: a
    /// forced render (bench, a desk-side capture) always renders the
    /// pair, but only *submits* it to the OpenXR swapchain — a real
    /// `CompositionLayerProjection` rather than an empty layer list —
    /// when the runtime actually asked for it.
    runtime_wants_this_frame: bool,
    predicted_display_time: openxr::Time,
    /// The last frame's views, held so `end_frame` can build the
    /// composition layer after the caller has rendered.
    last_views: [openxr::View; 2],
}

const VIEW_TYPE: openxr::ViewConfigurationType = openxr::ViewConfigurationType::PRIMARY_STEREO;

/// Vulkan colour formats worth asking the runtime for, most preferred
/// first, each paired with the `wgpu::TextureFormat` it is — sRGB 8-bit,
/// which every desktop OpenXR runtime lists one flavour of.
const SWAPCHAIN_FORMATS: &[(ash::vk::Format, wgpu::TextureFormat)] = &[
    (
        ash::vk::Format::B8G8R8A8_SRGB,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ),
    (
        ash::vk::Format::R8G8B8A8_SRGB,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ),
];

/// Try to stand up a native VR session: an OpenXR runtime, a Vulkan
/// device born from it, and two swapchains. `render_scale` is
/// `settings::vr_scale`, a factor on the runtime's own recommended
/// per-eye size. `None` on any failure — logged at warn level — and the
/// caller falls back to the flat renderer; this function never panics
/// and never launches a runtime that is not already running.
pub fn init(render_scale: f32) -> Option<(VrDevice, XrSession, wgpu::TextureFormat)> {
    match try_init(render_scale) {
        Ok((vr, session, fmt)) => Some((vr, XrSession::Real(Box::new(session)), fmt)),
        Err(e) => {
            log::warn!("VR: {e}; falling back to the flat view");
            None
        }
    }
}

fn try_init(render_scale: f32) -> Result<(VrDevice, RealSession, wgpu::TextureFormat), Error> {
    let render_scale = render_scale.clamp(SCALE_MIN, SCALE_MAX);

    #[cfg(target_os = "windows")]
    let loader_path = std::env::var("FARFALL_OPENXR_LOADER").ok().or_else(|| {
        Some(
            r"C:\Program Files (x86)\Steam\steamapps\common\SteamVR\bin\win64\openxr_loader.dll"
                .to_string(),
        )
    });
    #[cfg(not(target_os = "windows"))]
    let loader_path = std::env::var("FARFALL_OPENXR_LOADER").ok();

    let entry = unsafe { openxr::Entry::load() }.or_else(|default_err| {
        let Some(path) = loader_path.as_deref() else {
            return Err(Error::Loader(format!(
                "no default OpenXR loader found ({default_err}); \
                 set FARFALL_OPENXR_LOADER to point at one"
            )));
        };
        unsafe { openxr::Entry::load_from(std::path::Path::new(path)) }.map_err(|e| {
            Error::Loader(format!(
                "no default loader ({default_err}); the SteamVR one at \
                 {path} didn't load either ({e})"
            ))
        })
    })?;

    let available = entry
        .enumerate_extensions()
        .map_err(|e| Error::Instance(format!("enumerate_extensions: {e}")))?;
    if !available.khr_vulkan_enable2 {
        return Err(Error::Instance(
            "the active OpenXR runtime has no khr_vulkan_enable2".into(),
        ));
    }
    let mut exts = openxr::ExtensionSet::default();
    exts.khr_vulkan_enable2 = true;
    // Optional: only ever read from (log the runtime's current rate for
    // anything time-based, e.g. the queued bench mode's headroom calc)
    // — SPEC §5.3 is explicit that native VR never *sets* a refresh
    // rate, it only ever paces by wait_frame.
    let have_refresh_rate = available.fb_display_refresh_rate;
    exts.fb_display_refresh_rate = have_refresh_rate;
    let xr_instance = entry
        .create_instance(
            &openxr::ApplicationInfo {
                application_name: "FARFALL",
                application_version: 0,
                engine_name: "FARFALL",
                engine_version: 0,
                api_version: openxr::Version::new(1, 0, 0),
            },
            &exts,
            &[],
        )
        .map_err(|e| Error::Instance(format!("create_instance: {e}")))?;

    let system = xr_instance
        .system(openxr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .map_err(|e| Error::NoHmd(format!("system: {e}")))?;
    let blend_mode = xr_instance
        .enumerate_environment_blend_modes(system, VIEW_TYPE)
        .map_err(|e| Error::NoHmd(format!("enumerate_environment_blend_modes: {e}")))?
        .first()
        .copied()
        .ok_or_else(|| Error::NoHmd("no environment blend mode offered".into()))?;

    let reqs = xr_instance
        .graphics_requirements::<openxr::Vulkan>(system)
        .map_err(|e| Error::Vulkan(format!("graphics_requirements: {e}")))?;
    let vk_target = ash::vk::make_api_version(
        0,
        reqs.min_api_version_supported.major() as u32,
        reqs.min_api_version_supported.minor() as u32,
        0,
    )
    .max(ash::vk::make_api_version(0, 1, 1, 0));

    // SAFETY (this whole block): every call follows the exact sequence
    // XR_KHR_vulkan_enable2 requires — instance, then that system's
    // physical device, then a device on it — with each `ash` wrapper
    // built from the raw handle the previous OpenXR call returned, and
    // the wgpu-hal instance/device below take ownership per their own
    // safety contracts (`drop_callback: None`), matching the OpenXR
    // spec's rule that the *application*, not the runtime, owns and
    // eventually destroys the Vulkan objects.
    unsafe {
        let vk_entry = ash::Entry::load()
            .map_err(|e| Error::Vulkan(format!("no Vulkan loader on this system: {e}")))?;
        let instance_flags = if cfg!(debug_assertions) {
            wgpu::wgt::InstanceFlags::debugging()
        } else {
            wgpu::wgt::InstanceFlags::empty()
        };
        let wanted_instance_exts =
            hal::vulkan::Instance::desired_extensions(&vk_entry, vk_target, instance_flags)
                .map_err(|e| Error::Vulkan(format!("desired_extensions: {e}")))?;
        let instance_ext_ptrs: Vec<*const std::os::raw::c_char> =
            wanted_instance_exts.iter().map(|e| e.as_ptr()).collect();
        let app_info = ash::vk::ApplicationInfo::default()
            .application_version(0)
            .engine_version(0)
            .api_version(vk_target);
        let vk_instance_ci = ash::vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&instance_ext_ptrs);
        #[allow(clippy::missing_transmute_annotations)]
        let raw_vk_instance = xr_instance
            .create_vulkan_instance(
                system,
                std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                &vk_instance_ci as *const _ as *const _,
            )
            .map_err(|e| Error::Vulkan(format!("create_vulkan_instance (xr): {e}")))?
            .map_err(|vkr| Error::Vulkan(format!("create_vulkan_instance (vk): {vkr}")))?;
        let vk_instance = ash::Instance::load(
            vk_entry.static_fn(),
            ash::vk::Instance::from_raw(raw_vk_instance as _),
        );

        let vk_physical_device = ash::vk::PhysicalDevice::from_raw(
            xr_instance
                .vulkan_graphics_device(system, vk_instance.handle().as_raw() as _)
                .map_err(|e| Error::Vulkan(format!("vulkan_graphics_device: {e}")))?
                as _,
        );

        // Wrap the instance in wgpu-hal now, so `Instance::expose_adapter`
        // does the same feature/extension inspection wgpu itself would
        // have done for any other adapter.
        let hal_instance = hal::vulkan::Instance::from_raw(
            vk_entry.clone(),
            vk_instance.clone(),
            vk_target,
            0,
            None,
            wanted_instance_exts,
            instance_flags,
            wgpu::wgt::MemoryBudgetThresholds::default(),
            false,
            None,
        )
        .map_err(|e| Error::Vulkan(format!("hal::vulkan::Instance::from_raw: {e}")))?;
        let exposed = hal_instance
            .expose_adapter(vk_physical_device)
            .ok_or_else(|| {
                Error::Vulkan("the runtime's physical device isn't one wgpu-hal can drive".into())
            })?;

        let features = exposed.features;
        let device_exts = exposed.adapter.required_device_extensions(features);
        let mut phd_features = exposed
            .adapter
            .physical_device_features(&device_exts, features);
        let device_ext_ptrs: Vec<*const std::os::raw::c_char> =
            device_exts.iter().map(|e| e.as_ptr()).collect();
        let queue_family_index = vk_instance
            .get_physical_device_queue_family_properties(vk_physical_device)
            .into_iter()
            .enumerate()
            .find_map(|(i, info)| {
                info.queue_flags
                    .contains(ash::vk::QueueFlags::GRAPHICS)
                    .then_some(i as u32)
            })
            .ok_or_else(|| Error::Vulkan("no graphics queue family".into()))?;
        let queue_priorities = [1.0f32];
        let queue_ci = [ash::vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priorities)];
        let mut vk_device_ci = ash::vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_ci)
            .enabled_extension_names(&device_ext_ptrs);
        vk_device_ci = phd_features.add_to_device_create(vk_device_ci);
        #[allow(clippy::missing_transmute_annotations)]
        let raw_vk_device = xr_instance
            .create_vulkan_device(
                system,
                std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                vk_physical_device.as_raw() as _,
                &vk_device_ci as *const _ as *const _,
            )
            .map_err(|e| Error::Device(format!("create_vulkan_device (xr): {e}")))?
            .map_err(|vkr| Error::Device(format!("create_vulkan_device (vk): {vkr}")))?;
        let vk_device = ash::Device::load(
            vk_instance.fp_v1_0(),
            ash::vk::Device::from_raw(raw_vk_device as _),
        );

        let hal_open_device = exposed
            .adapter
            .device_from_raw(
                vk_device.clone(),
                None,
                &device_exts,
                features,
                &wgpu::wgt::Limits::default(),
                &wgpu::wgt::MemoryHints::MemoryUsage,
                queue_family_index,
                0,
            )
            .map_err(|e| Error::Device(format!("device_from_raw: {e}")))?;

        let wgpu_instance = wgpu::Instance::from_hal::<hal::api::Vulkan>(hal_instance);
        let wgpu_adapter = wgpu_instance.create_adapter_from_hal(exposed);
        let (wgpu_device, wgpu_queue) = wgpu_adapter
            .create_device_from_hal(
                hal_open_device,
                &wgpu::DeviceDescriptor {
                    label: Some("farfall VR device"),
                    required_features: features,
                    required_limits: wgpu::wgt::Limits::default(),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                },
            )
            .map_err(|e| Error::Device(format!("create_device_from_hal: {e}")))?;

        // The OpenXR session rides the same three handles.
        let (session, frame_wait, frame_stream) = xr_instance
            .create_session::<openxr::Vulkan>(
                system,
                &openxr::vulkan::SessionCreateInfo {
                    instance: vk_instance.handle().as_raw() as _,
                    physical_device: vk_physical_device.as_raw() as _,
                    device: vk_device.handle().as_raw() as _,
                    queue_family_index,
                    queue_index: 0,
                },
            )
            .map_err(|e| Error::Session(format!("create_session: {e}")))?;

        // Read-only: log the runtime's current display rate if it will
        // tell us (never assumed to be 90 Hz, and never set — the Index
        // is switchable between 80/90/120/144).
        let display_refresh_hz = have_refresh_rate
            .then(|| session.get_display_refresh_rate().ok())
            .flatten();
        match display_refresh_hz {
            Some(hz) => log::info!("VR: runtime's current display rate is {hz} Hz"),
            None if have_refresh_rate => {
                log::warn!(
                    "VR: fb_display_refresh_rate offered but get_display_refresh_rate failed"
                )
            }
            None => log::info!("VR: runtime doesn't offer fb_display_refresh_rate; rate unknown"),
        }

        // LOCAL: gravity-level, seated at session start — exactly the
        // ship's own frame (+X right, +Y up, −Z the nose), no fix-up.
        // `natural_local` is a second handle on that exact same origin,
        // kept for the life of the session so a recentre — which always
        // has to land relative to *it*, never to whichever LOCAL space
        // happens to be current — can locate the current one against it
        // and compose (`compose_pose`) instead of drifting on a second
        // recentre.
        let space = session
            .create_reference_space(openxr::ReferenceSpaceType::LOCAL, openxr::Posef::IDENTITY)
            .map_err(|e| Error::Session(format!("create_reference_space: {e}")))?;
        let natural_local = session
            .create_reference_space(openxr::ReferenceSpaceType::LOCAL, openxr::Posef::IDENTITY)
            .map_err(|e| Error::Session(format!("create_reference_space (natural): {e}")))?;

        let view_configs = xr_instance
            .enumerate_view_configuration_views(system, VIEW_TYPE)
            .map_err(|e| Error::Session(format!("enumerate_view_configuration_views: {e}")))?;
        let (rw, rh) = (
            view_configs[0].recommended_image_rect_width,
            view_configs[0].recommended_image_rect_height,
        );
        let recommended_size = (rw, rh);
        // The swapchain itself stays at the runtime's own recommended
        // size × VR RENDER SCALE — undistorted by the hull-vs-true
        // inflation, which is entirely the render's own problem
        // (eye_render_size, applied per frame once real tangents are
        // known from the first locate_views; see redraw's native_vr
        // path). Log the two numbers now so a mismatch is visible
        // without a headset: recommended, and what the swapchain itself
        // ended up at.
        let eye_size = (
            ((rw as f32) * render_scale).round().max(1.0) as u32,
            ((rh as f32) * render_scale).round().max(1.0) as u32,
        );
        log::info!(
            "VR: runtime recommends {rw}x{rh} per eye; swapchain at {}x{} (scale {render_scale}); \
             the render itself is sized per-frame from the real tangents (see eye_render_size)",
            eye_size.0,
            eye_size.1,
        );
        // The runtime's own "recommended" size already carries whatever
        // supersampling percentage its compositor (SteamVR) has set —
        // this engine's own hull inflation and VR RENDER SCALE both
        // multiply on top of it, so an aggressive compositor setting
        // compounds instead of being a separate, informed choice. A
        // real Index panel is 1440x1600 per eye; recommend backing off
        // once the runtime's ask is comfortably past what that panel
        // could ever resolve.
        const INDEX_PANEL: (f32, f32) = (1440.0, 1600.0);
        let over = (rw as f32 / INDEX_PANEL.0).max(rh as f32 / INDEX_PANEL.1);
        if over > 1.4 {
            log::info!(
                "VR: the runtime's recommended {rw}x{rh} is {:.0}% of the Index panel's own \
                 1440x1600 — SteamVR's own resolution slider looks to be around {:.0}%; \
                 consider 100% there before raising VR RENDER SCALE",
                over * 100.0,
                over * 100.0,
            );
        }

        let offered = session
            .enumerate_swapchain_formats()
            .map_err(|e| Error::Session(format!("enumerate_swapchain_formats: {e}")))?;
        let (vk_format, wgpu_format) = SWAPCHAIN_FORMATS
            .iter()
            .find(|(vkf, _)| offered.contains(&(vkf.as_raw() as u32)))
            .copied()
            .ok_or_else(|| {
                Error::Session(format!(
                    "no sRGB 8-bit swapchain format offered ({offered:?})"
                ))
            })?;

        let hal_device_guard = wgpu_device
            .as_hal::<hal::api::Vulkan>()
            .ok_or_else(|| Error::Device("wgpu device has no Vulkan hal backend".into()))?;
        let make_eye = |eye: usize| -> Result<EyeSwapchain, Error> {
            let handle = session
                .create_swapchain(&openxr::SwapchainCreateInfo {
                    create_flags: openxr::SwapchainCreateFlags::EMPTY,
                    // COLOR_ATTACHMENT: the crop pass draws into it.
                    // SAMPLED: the mirror-pair/label paths sample it.
                    // TRANSFER_SRC: the eye-order self-check's own
                    // texture-to-buffer readback — the Vulkan-level
                    // mirror of EYE_TEXTURE_USAGE's COPY_SRC below; the
                    // VkImage must allow it before wgpu's own
                    // TextureUsages::COPY_SRC can mean anything.
                    usage_flags: openxr::SwapchainUsageFlags::COLOR_ATTACHMENT
                        | openxr::SwapchainUsageFlags::SAMPLED
                        | openxr::SwapchainUsageFlags::TRANSFER_SRC,
                    format: vk_format.as_raw() as u32,
                    sample_count: 1,
                    width: eye_size.0,
                    height: eye_size.1,
                    face_count: 1,
                    array_size: 1,
                    mip_count: 1,
                })
                .map_err(|e| Error::Session(format!("create_swapchain: {e}")))?;
            let images = handle
                .enumerate_images()
                .map_err(|e| Error::Session(format!("enumerate_images: {e}")))?;
            let (textures, views): (Vec<_>, Vec<_>) = images
                .into_iter()
                .enumerate()
                .map(|(i, raw)| {
                    let label = format!("xr swapchain eye {eye} image {i}");
                    let vk_image = ash::vk::Image::from_raw(raw);
                    let hal_desc = hal::TextureDescriptor {
                        label: Some(label.as_str()),
                        size: wgpu::wgt::Extent3d {
                            width: eye_size.0,
                            height: eye_size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::wgt::TextureDimension::D2,
                        format: wgpu_format,
                        usage: wgpu::wgt::TextureUses::COLOR_TARGET
                            | wgpu::wgt::TextureUses::RESOURCE
                            | wgpu::wgt::TextureUses::COPY_SRC,
                        memory_flags: hal::MemoryFlags::empty(),
                        view_formats: Vec::new(),
                    };
                    let hal_texture = hal_device_guard.texture_from_raw(
                        vk_image,
                        &hal_desc,
                        None,
                        hal::vulkan::TextureMemory::External,
                    );
                    // EYE_TEXTURE_USAGE: every wgpu usage anything ever
                    // does with this image (the crop draw,
                    // mirror-pair/label sampling, the eye-order
                    // self-check's own readback) — the VkImage/hal_desc
                    // above already grants the matching Vulkan-level
                    // usages, so wgpu's own validation is the only thing
                    // that was ever missing here (twice, now).
                    let texture = wgpu_device.create_texture_from_hal::<hal::api::Vulkan>(
                        hal_texture,
                        &wgpu::TextureDescriptor {
                            label: Some(&label),
                            size: wgpu::Extent3d {
                                width: eye_size.0,
                                height: eye_size.1,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu_format,
                            usage: EYE_TEXTURE_USAGE,
                            view_formats: &[],
                        },
                        wgpu::wgt::TextureUses::UNINITIALIZED,
                    );
                    let view = texture.create_view(&wgpu::TextureViewDescriptor {
                        label: Some(&label),
                        ..Default::default()
                    });
                    (texture, view)
                })
                .unzip();
            Ok(EyeSwapchain {
                handle,
                textures,
                views,
                acquired: None,
            })
        };
        let eyes = [make_eye(0)?, make_eye(1)?];
        drop(hal_device_guard);

        // Self-check: FARFALL_VR_MIRROR=pair and the per-eye label both
        // sample these wrapped swapchain images, and a wgpu usage-flag
        // mismatch on them is a validation error, not a graceful
        // failure — it crashed the whole session the first time this
        // shipped (ece7006). Try to build the exact kind of bind group
        // that path needs, right now while a fallback to flat is still
        // cheap, instead of finding out mid-session.
        {
            let scope = wgpu_device.push_error_scope(wgpu::ErrorFilter::Validation);
            let probe_layout =
                wgpu_device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("xr swapchain sample probe"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    }],
                });
            let _probe = wgpu_device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("xr swapchain sample probe"),
                layout: &probe_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&eyes[0].views[0]),
                }],
            });
            if let Some(e) = pollster::block_on(scope.pop()) {
                return Err(Error::Session(format!(
                    "swapchain images can't be sampled (the mirror-pair/label path would \
                     crash on this): {e}"
                )));
            }
        }

        log::info!(
            "VR: session up on {} runtime, {}x{} per eye, format {wgpu_format:?}",
            xr_instance
                .properties()
                .map(|p| p.runtime_name)
                .unwrap_or_else(|_| "?".into()),
            eye_size.0,
            eye_size.1,
        );

        let session_for_teardown = RealSession {
            instance: xr_instance,
            session,
            frame_wait,
            frame_stream,
            space,
            natural_local,
            blend_mode,
            eyes,
            eye_size,
            recommended_size,
            display_refresh_hz,
            last_wait_ms: 0.0,
            event_storage: openxr::EventDataBuffer::new(),
            session_running: false,
            frame_open: false,
            logged_eye_diagnostic: false,
            became_focused: false,
            auto_recentred: false,
            runtime_wants_this_frame: false,
            predicted_display_time: openxr::Time::from_nanos(0),
            last_views: [openxr::View {
                pose: openxr::Posef::IDENTITY,
                fov: openxr::Fovf {
                    angle_left: 0.0,
                    angle_right: 0.0,
                    angle_up: 0.0,
                    angle_down: 0.0,
                },
            }; 2],
        };

        Ok((
            VrDevice {
                instance: wgpu_instance,
                adapter: wgpu_adapter,
                device: wgpu_device,
                queue: wgpu_queue,
            },
            session_for_teardown,
            wgpu_format,
        ))
    }
}

impl RealSession {
    /// The swapchain's own per-eye size (recommended × VR RENDER SCALE).
    pub fn eye_size(&self) -> (u32, u32) {
        self.eye_size
    }

    /// The runtime's own recommended per-eye size, unscaled — feed this
    /// and the current frame's real tangents to [`eye_render_size`] for
    /// the size the *render* itself wants, which is not the same thing.
    pub fn recommended_size(&self) -> (u32, u32) {
        self.recommended_size
    }

    /// Wall-clock milliseconds the most recent `begin_frame` spent
    /// inside `FrameWaiter::wait` — the runtime's own pacing.
    pub fn last_wait_ms(&self) -> f32 {
        self.last_wait_ms
    }

    /// The runtime's own current display rate, Hz — `None` if it never
    /// offered `fb_display_refresh_rate`.
    pub fn display_refresh_hz(&self) -> Option<f32> {
        self.display_refresh_hz
    }

    // VR HANDS (fable/vr-hands): minimal accessors so `xr_input::XrInput`
    // can be built and polled from outside this module — the instance
    // and session to attach the action set to (once, right after
    // `init`), and the current space/predicted display time to locate
    // the hands in each frame, exactly as `begin_frame` locates the
    // eyes. Only `RealSession` has any of these: a synthetic session
    // (`SynthSession`) has no instance/session/space at all, so hand
    // input is only ever attached to a real one — see `xr_input`'s own
    // `hands_mode`. No new state here; these borrow what `try_init`/
    // `begin_frame` already own.

    /// The raw handles `xr_input::OpenXrHands::new` attaches its action
    /// set to (and clones its own session from). Call once, right after
    /// `init` returns — see that module's doc comment for why the
    /// ordering matters.
    pub fn raw_handles(&self) -> (&openxr::Instance, &openxr::Session<openxr::Vulkan>) {
        (&self.instance, &self.session)
    }

    /// The current (possibly recentred) LOCAL space hands are located
    /// against — the same space `begin_frame` locates the eyes in.
    pub fn space(&self) -> &openxr::Space {
        &self.space
    }

    /// This frame's predicted display time, set by the last
    /// `begin_frame` call.
    pub fn predicted_display_time(&self) -> openxr::Time {
        self.predicted_display_time
    }

    /// Poll session-state events and, if the runtime wants a frame,
    /// block for it (`FrameWaiter::wait`) and open it
    /// (`FrameStream::begin`). Every `Open` result must be matched by
    /// [`Self::end_frame`] or [`Self::skip_frame`] before the next call.
    ///
    /// `force_render`: render (and return `should_render: true`) even
    /// when the runtime's own `should_render` is false — a bench run or
    /// a desk-side capture needs a picture whether the compositor
    /// currently wants one submitted or not (the session commonly sits
    /// VISIBLE-but-unfocused with the headset on the desk, which is not
    /// the same as `should_render` being false, but this covers the
    /// case even if it is). The frame is still rendered either way;
    /// `end_frame` alone decides, from what the runtime actually asked
    /// for, whether to submit it as a real composition layer or an
    /// empty one — never sending a layer the runtime didn't ask for.
    pub fn begin_frame(&mut self, force_render: bool) -> Frame {
        loop {
            match self.instance.poll_event(&mut self.event_storage) {
                Ok(Some(openxr::Event::SessionStateChanged(e))) => match e.state() {
                    openxr::SessionState::READY => {
                        if self.session.begin(VIEW_TYPE).is_err() {
                            return Frame::Lost;
                        }
                        self.session_running = true;
                    }
                    openxr::SessionState::FOCUSED => {
                        self.became_focused = true;
                    }
                    openxr::SessionState::STOPPING => {
                        let _ = self.session.end();
                        self.session_running = false;
                    }
                    openxr::SessionState::EXITING | openxr::SessionState::LOSS_PENDING => {
                        return Frame::Lost;
                    }
                    _ => {}
                },
                Ok(Some(openxr::Event::InstanceLossPending(_))) => return Frame::Lost,
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => {
                    log::warn!("VR: poll_event: {e}");
                    return Frame::Lost;
                }
            }
        }
        if !self.session_running {
            return Frame::Idle;
        }
        let wait_start = std::time::Instant::now();
        let state = match self.frame_wait.wait() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("VR: frame_wait: {e}");
                return Frame::Lost;
            }
        };
        self.last_wait_ms = wait_start.elapsed().as_secs_f32() * 1000.0;
        if self.frame_stream.begin().is_err() {
            return Frame::Lost;
        }
        self.frame_open = true;
        self.predicted_display_time = state.predicted_display_time;
        self.runtime_wants_this_frame = state.should_render;
        // No render this call, one way or another: `nothing_to_render`
        // is the one payload for every "skip, but the frame is still
        // open and must still be matched by end_frame/skip_frame" case
        // below — should_render was false and nothing forced it anyway,
        // or locate_views could not be trusted. None of these tear the
        // session down (that is Frame::Lost's job, for the runtime
        // actually going away); a frame that briefly can't be located is
        // not a lost session, and reusing last frame's stale views to
        // render anyway is exactly the kind of thing that reads as
        // "wrong" once it is worn.
        let nothing_to_render = || Frame::Open {
            should_render: false,
            eyes: [VrEye {
                head: Quat::IDENTITY,
                pos: Vec3::ZERO,
                tan: [1.0, 1.0, 1.0, 1.0],
            }; 2],
        };
        if !state.should_render && !force_render {
            return nothing_to_render();
        }
        let (_flags, views) =
            match self
                .session
                .locate_views(VIEW_TYPE, self.predicted_display_time, &self.space)
            {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("VR: locate_views: {e}; skipping this frame");
                    self.runtime_wants_this_frame = false;
                    return nothing_to_render();
                }
            };
        if views.len() < 2 {
            log::warn!(
                "VR: runtime offered {} views, need 2; skipping this frame",
                views.len()
            );
            self.runtime_wants_this_frame = false;
            return nothing_to_render();
        }
        self.last_views = [views[0], views[1]];
        let eyes = std::array::from_fn(|i| {
            let v = self.last_views[i];
            VrEye {
                head: Quat::from_xyzw(
                    v.pose.orientation.x,
                    v.pose.orientation.y,
                    v.pose.orientation.z,
                    v.pose.orientation.w,
                )
                .normalize(),
                pos: Vec3::new(v.pose.position.x, v.pose.position.y, v.pose.position.z),
                tan: fov_tangents(
                    v.fov.angle_left,
                    v.fov.angle_right,
                    v.fov.angle_up,
                    v.fov.angle_down,
                ),
            }
        });
        // A one-shot diagnostic, not a guess: view 0 is the eye array
        // index every downstream mapping (the pair's left half, the
        // left OpenXR swapchain, the left composition-layer view) is
        // built to mean "left" — per spec, guaranteed, but never
        // confirmed against this specific runtime/headset until it is
        // actually worn. If eye 0's X here is not the more negative of
        // the two, that is the runtime disagreeing with the spec, not a
        // bug in the mapping itself; see SPEC §5.3.
        //
        // The X-only figure alone reads badly when the head is heavily
        // yawed relative to LOCAL (a small apparent gap that looks like
        // a mis-tracked IPD but is really geometry): the full |eye1 −
        // eye0| separation and the head's own yaw are logged alongside
        // it so a small number here is legible as "turned", not "wrong".
        if !self.logged_eye_diagnostic {
            self.logged_eye_diagnostic = true;
            let ipd_m = (eyes[1].pos - eyes[0].pos).length();
            let (yaw, _pitch, _roll) = eyes[0].head.to_euler(glam::EulerRot::YXZ);
            log::info!(
                "VR: eye 0 (should be left) x={:+.4}m, eye 1 (should be right) x={:+.4}m \
                 — eye 0 should be the more negative; full IPD {ipd_m:.4}m; head yaw \
                 {:+.1}° relative to LOCAL",
                eyes[0].pos.x,
                eyes[1].pos.x,
                yaw.to_degrees(),
            );
        }
        // Auto-recentre, once: LOCAL is wherever the runtime put it at
        // session start, which is wherever the headset physically lay —
        // on the desk, yawed however many degrees off the pilot's actual
        // heading — until this runs. Waiting for FOCUSED (not merely a
        // located head) normally means it fires once the compositor has
        // actually handed the session the frame, not mid-transition
        // while the runtime's own splash or dashboard still owns the
        // view — but `force_render` (bench, a desk-side capture) is a
        // headset left flat on the desk, which never reaches FOCUSED at
        // all (a9869ff's own capture: +86.7° off LOCAL, uncorrected).
        // Those runs already accept whatever pose the runtime hands
        // back for `force_render` to work at all, so recentring on the
        // first LOCATED frame regardless of focus costs nothing a desk
        // capture wasn't already trusting, and is the only way it ever
        // looks at the nose instead of wherever the desk happened to
        // point.
        if (self.became_focused || force_render) && !self.auto_recentred {
            self.auto_recentred = true;
            let (yaw, _pitch, _roll) = eyes[0].head.to_euler(glam::EulerRot::YXZ);
            log::info!(
                "VR: auto-recentring on the first {} frame \
                 (head yaw {:+.1}° from LOCAL, pos {:.3?})",
                if self.became_focused {
                    "focused, located"
                } else {
                    "located (forced render, not yet focused)"
                },
                yaw.to_degrees(),
                eyes[0].pos,
            );
            self.recentre(eyes[0]);
        }
        Frame::Open {
            should_render: true,
            eyes,
        }
    }

    /// `should_render` was false: end the frame with no layers, as the
    /// spec requires every `begin()` to be matched.
    pub fn skip_frame(&mut self) {
        if !self.frame_open {
            return;
        }
        self.frame_open = false;
        let _ = self
            .frame_stream
            .end(self.predicted_display_time, self.blend_mode, &[]);
    }

    /// This eye's swapchain image to render into, acquired and waited on.
    pub fn acquire_eye(&mut self, eye: usize) -> &wgpu::TextureView {
        self.eyes[eye].acquire()
    }

    /// The `wgpu::Texture` behind this eye's currently-acquired image —
    /// the eye-order self-check's readback target. `None` before the
    /// first `acquire_eye` of the session.
    pub fn acquired_eye_texture(&self, eye: usize) -> Option<&wgpu::Texture> {
        self.eyes[eye].acquired_texture()
    }

    /// Release both swapchain images and end the frame — with a stereo
    /// projection layer at this frame's located poses/fovs when the
    /// runtime actually asked for one (`should_render` was true), or
    /// with none at all when it was a forced render (bench, a desk-side
    /// capture) the runtime itself did not request: the swapchain images
    /// are still cropped and released above, so the pair, the mirror
    /// and any capture still have a real frame to show, but nothing is
    /// submitted to a compositor that did not ask for it.
    pub fn end_frame(&mut self) {
        for e in &mut self.eyes {
            e.release();
        }
        if !self.frame_open {
            return;
        }
        self.frame_open = false;
        if !self.runtime_wants_this_frame {
            if let Err(e) = self
                .frame_stream
                .end(self.predicted_display_time, self.blend_mode, &[])
            {
                log::warn!("VR: frame_stream.end (forced render, no layer): {e}");
            }
            return;
        }
        let rect = openxr::Rect2Di {
            offset: openxr::Offset2Di { x: 0, y: 0 },
            extent: openxr::Extent2Di {
                width: self.eye_size.0 as i32,
                height: self.eye_size.1 as i32,
            },
        };
        let views = [
            openxr::CompositionLayerProjectionView::new()
                .pose(self.last_views[0].pose)
                .fov(self.last_views[0].fov)
                .sub_image(
                    openxr::SwapchainSubImage::new()
                        .swapchain(&self.eyes[0].handle)
                        .image_array_index(0)
                        .image_rect(rect),
                ),
            openxr::CompositionLayerProjectionView::new()
                .pose(self.last_views[1].pose)
                .fov(self.last_views[1].fov)
                .sub_image(
                    openxr::SwapchainSubImage::new()
                        .swapchain(&self.eyes[1].handle)
                        .image_array_index(0)
                        .image_rect(rect),
                ),
        ];
        let layer = openxr::CompositionLayerProjection::new()
            .space(&self.space)
            .views(&views);
        if let Err(e) =
            self.frame_stream
                .end(self.predicted_display_time, self.blend_mode, &[&layer])
        {
            log::warn!("VR: frame_stream.end: {e}");
        }
    }

    /// VR RECENTRE: re-seat the LOCAL space on `head`'s yaw and position
    /// (SPEC §5.3; see [`recentre_pose`]). Only meaningful once a frame
    /// has located a head; a call before then is a harmless no-op.
    pub fn recentre(&mut self, head: VrEye) {
        // `head` is already relative to `self.space`, which may itself
        // be an earlier recentre — but `create_reference_space`'s pose
        // is always relative to the runtime's *natural* LOCAL origin.
        // Locate the current space against that natural one and compose
        // (`compose_pose`) onto it, so a second recentre lands exactly
        // instead of drifting by however far the first one moved.
        let located = match self
            .space
            .locate(&self.natural_local, self.predicted_display_time)
        {
            Ok(l) => l,
            Err(e) => {
                log::warn!("VR: recentre: locate (current space in natural): {e}");
                return;
            }
        };
        let space_in_natural = (
            Quat::from_xyzw(
                located.pose.orientation.x,
                located.pose.orientation.y,
                located.pose.orientation.z,
                located.pose.orientation.w,
            )
            .normalize(),
            Vec3::new(
                located.pose.position.x,
                located.pose.position.y,
                located.pose.position.z,
            ),
        );
        let head_in_space = recentre_pose(head.head, head.pos);
        let (q, pos) = compose_pose(space_in_natural, head_in_space);
        let pose = openxr::Posef {
            orientation: openxr::Quaternionf {
                x: q.x,
                y: q.y,
                z: q.z,
                w: q.w,
            },
            position: openxr::Vector3f {
                x: pos.x,
                y: pos.y,
                z: pos.z,
            },
        };
        match self
            .session
            .create_reference_space(openxr::ReferenceSpaceType::LOCAL, pose)
        {
            Ok(space) => self.space = space,
            Err(e) => log::warn!("VR: recentre: create_reference_space: {e}"),
        }
    }
}

impl Drop for RealSession {
    fn drop(&mut self) {
        // Matched `begin()`s must not outlive the frame; an unmatched one
        // here means the app is exiting mid-frame, which the runtime
        // tolerates far better than a dangling `frame_stream.begin()`.
        if self.frame_open {
            let _ = self
                .frame_stream
                .end(self.predicted_display_time, self.blend_mode, &[]);
        }
        if self.session_running {
            let _ = self.session.end();
        }
    }
}

// ---------------------------------------------------------------------
// The synthetic bench headset (FARFALL_VR=synth, SPEC §5.3): needs no
// OpenXR runtime, so this half of the module compiles and runs on any
// machine, with a plain wgpu device — no ash/hal, no `#[cfg(test)]`
// wall between it and a real GPU test, unlike everything above it.
// ---------------------------------------------------------------------

/// A synthetic Valve-Index-shaped headset needing no OpenXR runtime
/// (`FARFALL_VR=synth`): the whole render/comfort/label pipeline
/// (`xr_composite`, the mirror, the eye-order and overlay-depth self-
/// checks) can be benched on the desktop, on any machine, any time —
/// comfort regressions get caught by a bench row before a human ever
/// wears the real thing. Its two "swapchain" images are ordinary wgpu
/// textures built with the identical format/usage/label pattern
/// `RealSession::try_init`'s `make_eye` uses, so `xr_composite`'s
/// crop+label+mirror pass is genuinely the same code, not a stand-in.
pub struct SynthSession {
    eye_size: (u32, u32),
    recommended_size: (u32, u32),
    /// Parallel to `views` — kept for the eye-order self-check's
    /// readback (`copy_texture_to_buffer` needs the `wgpu::Texture`
    /// itself), matching `EyeSwapchain`'s own reason for keeping both.
    textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    script: HeadScript,
    bench_seconds: f32,
    start: std::time::Instant,
    /// Set once the first frame has logged its eye-position diagnostic
    /// — `RealSession`'s own equivalent, so a synth bench log shows the
    /// same evidence a real headset's does.
    logged_eye_diagnostic: bool,
}

/// A synthetic Index's own recommended per-eye size, before `vr_scale`
/// and the hull-vs-true inflation `eye_render_size` applies on top — the
/// same two steps a real runtime's own recommendation goes through
/// (`RealSession::try_init`'s own `eye_size`/`recommended_size` split).
const SYNTH_RECOMMENDED_SIZE: (u32, u32) = (2016, 2240);

/// A synthetic Index's own display rate, Hz — always reported, unlike a
/// real runtime's `fb_display_refresh_rate` (which may be absent).
const SYNTH_HZ: f32 = 90.0;

/// A synthetic Index's own IPD, metres — a stock value, split ±half
/// about the head's own position along its local +X.
const SYNTH_IPD_M: f32 = 0.064;

/// A synthetic Index's own per-eye field of view, degrees — magnitudes
/// only, as `(left, right, up, down)`; [`fov_tangents`] applies OpenXR's
/// own left/down sign. The right eye is this mirrored left-for-right,
/// same up/down, as a real Index's own lenses are.
const SYNTH_FOV_LEFT_EYE_DEG: (f32, f32, f32, f32) = (54.0, 46.0, 55.0, 55.0);

impl SynthSession {
    fn eye_tan(eye: usize) -> [f32; 4] {
        let (l, r, u, d) = SYNTH_FOV_LEFT_EYE_DEG;
        let (l, r) = if eye == 0 { (l, r) } else { (r, l) };
        fov_tangents(
            (-l).to_radians(),
            r.to_radians(),
            u.to_radians(),
            (-d).to_radians(),
        )
    }

    /// The swapchain's own per-eye size (recommended × VR RENDER SCALE)
    /// — see [`RealSession::eye_size`].
    pub fn eye_size(&self) -> (u32, u32) {
        self.eye_size
    }

    /// See [`RealSession::recommended_size`].
    pub fn recommended_size(&self) -> (u32, u32) {
        self.recommended_size
    }

    pub fn display_refresh_hz(&self) -> Option<f32> {
        Some(SYNTH_HZ)
    }

    /// Always zero: free-running, nothing here waits on a compositor.
    pub fn last_wait_ms(&self) -> f32 {
        0.0
    }

    /// The deterministic head pose at the current bench time
    /// ([`synth_head_pose`]), split into two eyes at the stock IPD about
    /// the head's own local +X — position and orientation feed `VrEye`
    /// exactly as a real runtime's `locate_views` would.
    pub fn begin_frame(&mut self, _force_render: bool) -> Frame {
        let t = self.start.elapsed().as_secs_f32();
        let (head, pos) = synth_head_pose(self.script, t, self.bench_seconds);
        let tan = [Self::eye_tan(0), Self::eye_tan(1)];
        let eyes = synth_eyes(head, pos, SYNTH_IPD_M, tan);
        if !self.logged_eye_diagnostic {
            self.logged_eye_diagnostic = true;
            let ipd_m = (eyes[1].pos - eyes[0].pos).length();
            log::info!(
                "VR: synth eye 0 x={:+.4}m, eye 1 x={:+.4}m — eye 0 should be the more \
                 negative; full IPD {ipd_m:.4}m",
                eyes[0].pos.x,
                eyes[1].pos.x,
            );
        }
        Frame::Open {
            should_render: true,
            eyes,
        }
    }

    pub fn skip_frame(&mut self) {}

    pub fn acquire_eye(&mut self, eye: usize) -> &wgpu::TextureView {
        &self.views[eye]
    }

    /// Always available — a synthetic session has no ring buffer, just
    /// the one persistent image per eye.
    pub fn acquired_eye_texture(&self, eye: usize) -> Option<&wgpu::Texture> {
        Some(&self.textures[eye])
    }

    pub fn end_frame(&mut self) {}
}

/// Build a synthetic headset: no OpenXR runtime, no real GPU interop —
/// `device` is the ordinary flat-path wgpu device (`request_flat_device`
/// in lib.rs), since a synthetic session never needs one born from a
/// runtime's own Vulkan instance. `render_scale`/`bench_seconds` mirror
/// the real path's own knobs (`FARFALL_VR_SCALE`, the bench's own run
/// length — `Spin`'s only input, see `synth_head_pose`).
pub fn init_synth(
    device: &wgpu::Device,
    render_scale: f32,
    script: HeadScript,
    bench_seconds: f32,
) -> (XrSession, wgpu::TextureFormat) {
    let render_scale = render_scale.clamp(SCALE_MIN, SCALE_MAX);
    let eye_size = (
        ((SYNTH_RECOMMENDED_SIZE.0 as f32) * render_scale)
            .round()
            .max(1.0) as u32,
        ((SYNTH_RECOMMENDED_SIZE.1 as f32) * render_scale)
            .round()
            .max(1.0) as u32,
    );
    // SWAPCHAIN_FORMATS' own first (most-preferred) choice — the same
    // format a real runtime almost always offers, so a synth bench row
    // exercises the identical sRGB round-trip xrblit.wgsl relies on.
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let mut textures: Vec<wgpu::Texture> = Vec::with_capacity(2);
    let mut views: Vec<wgpu::TextureView> = Vec::with_capacity(2);
    for eye in 0..2 {
        let label = format!("xr swapchain eye {eye} image 0");
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&label),
            size: wgpu::Extent3d {
                width: eye_size.0,
                height: eye_size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: EYE_TEXTURE_USAGE,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&label),
            ..Default::default()
        });
        textures.push(texture);
        views.push(view);
    }
    log::info!(
        "VR: synthetic headset up ({}x{} per eye, {SYNTH_HZ} Hz, script {script:?})",
        eye_size.0,
        eye_size.1,
    );
    let session = SynthSession {
        eye_size,
        recommended_size: SYNTH_RECOMMENDED_SIZE,
        textures: textures.try_into().unwrap_or_else(|v: Vec<_>| {
            panic!(
                "synth headset builds exactly 2 eye textures, got {}",
                v.len()
            )
        }),
        views: views
            .try_into()
            .unwrap_or_else(|_| panic!("synth headset builds exactly 2 eye views")),
        script,
        bench_seconds,
        start: std::time::Instant::now(),
        logged_eye_diagnostic: false,
    };
    (XrSession::Synth(Box::new(session)), format)
}
