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
//! function, and [`XrSession::end_frame`] runs it as a real GPU pass
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
/// (`XrSession::recentre` below) is always relative to the *runtime's*
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

#[cfg(test)]
mod pure_math_tests {
    use super::*;
    use std::f32::consts::FRAC_PI_4;

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
}

/// What a frame's `begin_frame` found.
pub enum Frame {
    /// The session is not `READY`/`SYNCHRONIZED`/`VISIBLE`/`FOCUSED` yet
    /// (e.g. between launch and the runtime attaching): nothing to draw
    /// or present this call.
    Idle,
    /// A frame is open (`frame_stream.begin()` was called) and must be
    /// matched by [`XrSession::end_frame`] or [`XrSession::skip_frame`].
    Open {
        should_render: bool,
        eyes: [VrEye; 2],
    },
    /// The runtime is gone (instance loss, or the session exited):
    /// caller drops this `XrSession` and falls back to flat.
    Lost,
}

/// A running native VR session: the OpenXR side of the seam, plus the
/// two swapchains the flat-rendered pair is cropped into each frame.
pub struct XrSession {
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
    event_storage: openxr::EventDataBuffer,
    session_running: bool,
    frame_open: bool,
    /// Set once the first located frame has logged its diagnostic line
    /// (see `begin_frame`) — never repeated, so a long session's log
    /// isn't spammed once a frame.
    logged_eye_diagnostic: bool,
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
        Ok(ok) => Some(ok),
        Err(e) => {
            log::warn!("VR: {e}; falling back to the flat view");
            None
        }
    }
}

fn try_init(render_scale: f32) -> Result<(VrDevice, XrSession, wgpu::TextureFormat), Error> {
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
        let eye_size = (
            ((rw as f32) * render_scale).round().max(1.0) as u32,
            ((rh as f32) * render_scale).round().max(1.0) as u32,
        );

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
        let make_eye = || -> Result<EyeSwapchain, Error> {
            let handle = session
                .create_swapchain(&openxr::SwapchainCreateInfo {
                    create_flags: openxr::SwapchainCreateFlags::EMPTY,
                    usage_flags: openxr::SwapchainUsageFlags::COLOR_ATTACHMENT
                        | openxr::SwapchainUsageFlags::SAMPLED,
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
            let views = images
                .into_iter()
                .map(|raw| {
                    let vk_image = ash::vk::Image::from_raw(raw);
                    let hal_desc = hal::TextureDescriptor {
                        label: Some("xr swapchain image"),
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
                            | wgpu::wgt::TextureUses::RESOURCE,
                        memory_flags: hal::MemoryFlags::empty(),
                        view_formats: Vec::new(),
                    };
                    let hal_texture = hal_device_guard.texture_from_raw(
                        vk_image,
                        &hal_desc,
                        None,
                        hal::vulkan::TextureMemory::External,
                    );
                    let texture = wgpu_device.create_texture_from_hal::<hal::api::Vulkan>(
                        hal_texture,
                        &wgpu::TextureDescriptor {
                            label: Some("xr swapchain image"),
                            size: wgpu::Extent3d {
                                width: eye_size.0,
                                height: eye_size.1,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu_format,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            view_formats: &[],
                        },
                        wgpu::wgt::TextureUses::UNINITIALIZED,
                    );
                    texture.create_view(&wgpu::TextureViewDescriptor::default())
                })
                .collect();
            Ok(EyeSwapchain {
                handle,
                views,
                acquired: None,
            })
        };
        let eyes = [make_eye()?, make_eye()?];
        drop(hal_device_guard);

        log::info!(
            "VR: session up on {} runtime, {}x{} per eye, format {wgpu_format:?}",
            xr_instance
                .properties()
                .map(|p| p.runtime_name)
                .unwrap_or_else(|_| "?".into()),
            eye_size.0,
            eye_size.1,
        );

        let session_for_teardown = XrSession {
            instance: xr_instance,
            session,
            frame_wait,
            frame_stream,
            space,
            natural_local,
            blend_mode,
            eyes,
            eye_size,
            event_storage: openxr::EventDataBuffer::new(),
            session_running: false,
            frame_open: false,
            logged_eye_diagnostic: false,
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

impl XrSession {
    pub fn eye_size(&self) -> (u32, u32) {
        self.eye_size
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
        let state = match self.frame_wait.wait() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("VR: frame_wait: {e}");
                return Frame::Lost;
            }
        };
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
        if !self.logged_eye_diagnostic {
            self.logged_eye_diagnostic = true;
            log::info!(
                "VR: eye 0 (should be left) x={:+.4}m, eye 1 (should be right) x={:+.4}m \
                 — eye 0 should be the more negative",
                eyes[0].pos.x,
                eyes[1].pos.x,
            );
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

impl Drop for XrSession {
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
