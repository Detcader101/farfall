//! Controller glyphs in VR (`shaders/hands.wgsl`, SPEC §5.3b): a small
//! SDF raymarch of each tracked hand at its grip pose, in the ship's
//! frame, drawn in the ship pass right after the cabin.
//!
//! Per-eye parallax through `cockpit.wgsl`'s own convention, not the
//! `with_eye`-shifted-position one `cabin.rs`/`ghost.rs`/`shield.rs`
//! use: hand positions are stored in *absolute* ship frame, and the
//! current eye's own seat travels as its own uniform (`eye`), added to
//! the ray's origin in the shader (`let p = hd.eye.xyz + ray * t;`,
//! exactly `cockpit.wgsl`'s `ck.eye.xyz + ray * t`) rather than
//! subtracted from every stored position first. An earlier version of
//! this pass used the shift-the-target convention and, despite passing
//! its own unit tests (the packed uniforms genuinely differed between
//! two eyes at different seats — see `HandsUniforms::eye`'s own test)
//! showed zero measured disparity in an actual synth-headset capture;
//! switching to the one other cockpit-frame passes in this codebase
//! have already been proven correct with real captures removed the
//! difference, whatever it was. This pass must not repeat the zero-
//! disparity mistake at all — a hand a third of a metre from the eye
//! shows real, obvious stereo disparity a distant cabin rarely does.
//!
//! No depth buffer exists to test the hand against the cabin's own
//! raymarch, so occlusion is approximate: [`dash_occlusion`] fades a
//! hand out as it crosses behind the dash's plane (`cabin::DASH_C`/
//! `DASH_N`) rather than clipping through it undrawn.

use glam::{Quat, Vec3};

use crate::cabin::{DASH_C, DASH_N};
use crate::instrument::InstrumentPass;
use crate::CameraFrame;

/// One hand's pose and state, ready to pack into [`HandsUniforms`].
#[derive(Debug, Clone, Copy)]
pub struct HandGlyph {
    /// The grip pose, *absolute* ship frame — no eye shift; the current
    /// eye's own seat travels separately, as [`HandsUniforms`]'s own
    /// `eye` field.
    pub pos: Vec3,
    pub rot: Quat,
    pub trigger: f32,
    pub squeeze: f32,
    /// Held (grabbing the stick/throttle, SPEC §5.3b(d)): a tighter,
    /// brighter glyph.
    pub held: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HandsUniforms {
    // xyz: the head's right axis, ship frame. w: aspect.
    right: [f32; 4],
    // xyz: the head's up axis. w: tan(fov/2).
    up: [f32; 4],
    // xyz: the head's forward axis (-Z the nose). w: time (s).
    fwd: [f32; 4],
    // xyz: the current eye's own seat, ship frame (`cockpit.wgsl`'s own
    // `ck.eye` — the ray's origin, not a shift on every stored
    // position). w: unused.
    eye: [f32; 4],
    // Left hand: xyz position, *absolute* ship frame. w: shown (1/0).
    left_pos: [f32; 4],
    // Left hand's orientation, a quaternion (xyz, w).
    left_rot: [f32; 4],
    // x: trigger 0..1. y: squeeze 0..1. z: held (1/0). w: occlusion fade.
    left_state: [f32; 4],
    right_pos: [f32; 4],
    right_rot: [f32; 4],
    right_state: [f32; 4],
    // VR BEAM (SPEC §5.3b(c)): the laser's own two ends, *absolute*
    // ship frame. xyz: the origin (the right hand's aim pose). w:
    // shown (1/0).
    beam_a: [f32; 4],
    // xyz: the hit point on the glass (or the ray's far end with no
    // hit). w: unused.
    beam_b: [f32; 4],
}

impl HandsUniforms {
    /// Nothing shown: the shader discards every pixel.
    pub fn none(cam: &CameraFrame, head: Quat, eye: Vec3) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, cam.time_s.rem_euclid(1000.0)),
            eye: v4(eye, 0.0),
            left_pos: [0.0; 4],
            left_rot: [0.0, 0.0, 0.0, 1.0],
            left_state: [0.0; 4],
            right_pos: [0.0; 4],
            right_rot: [0.0, 0.0, 0.0, 1.0],
            right_state: [0.0; 4],
            beam_a: [0.0; 4],
            beam_b: [0.0; 4],
        }
    }

    /// Both hands this frame: `left`/`right` are `None` when that hand
    /// isn't tracked. `eye` is the current eye's own seat, ship frame
    /// (`ViewPose::eye_ship`, as `Vec3`) — passed straight through as
    /// its own uniform, not folded into the hand positions, so the two
    /// eyes' renders share one absolute scene and differ only in where
    /// their own ray originates (see the module doc).
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        eye: Vec3,
        left: Option<HandGlyph>,
        right: Option<HandGlyph>,
    ) -> Self {
        let mut u = Self::none(cam, head, eye);
        if let Some(h) = left {
            u.left_pos = pos4(h.pos);
            u.left_rot = rot4(h.rot);
            u.left_state = state4(h);
        }
        if let Some(h) = right {
            u.right_pos = pos4(h.pos);
            u.right_rot = rot4(h.rot);
            u.right_state = state4(h);
        }
        u
    }

    /// VR BEAM: the laser from `origin` to `hit`, both *absolute* ship
    /// frame. No beam at all (VR BEAM off, no headset, or the right
    /// hand isn't tracked) is simply never called — the uniforms then
    /// keep `beam_a.w == 0.0`.
    pub fn with_beam(mut self, origin: Vec3, hit: Vec3) -> Self {
        self.beam_a = [origin.x, origin.y, origin.z, 1.0];
        self.beam_b = [hit.x, hit.y, hit.z, 0.0];
        self
    }
}

fn pos4(p: Vec3) -> [f32; 4] {
    [p.x, p.y, p.z, 1.0]
}

fn rot4(q: Quat) -> [f32; 4] {
    let q = q.normalize();
    [q.x, q.y, q.z, q.w]
}

fn state4(h: HandGlyph) -> [f32; 4] {
    [
        h.trigger.clamp(0.0, 1.0),
        h.squeeze.clamp(0.0, 1.0),
        if h.held { 1.0 } else { 0.0 },
        dash_occlusion(h.pos),
    ]
}

/// How much a hand glyph shows, given its ship-frame position (before
/// any eye shift — the dash is fixed to the ship, not the eye): 1 in
/// front of the dash's own plane by a margin, fading to 0 a further
/// margin behind it. An approximate occlusion in place of a true depth
/// test against the cabin's own raymarch, which this pass has no depth
/// buffer to read.
pub fn dash_occlusion(hand_ship: Vec3) -> f32 {
    let depth = (hand_ship - DASH_C).dot(DASH_N);
    const MARGIN_M: f32 = 0.06;
    ((depth + MARGIN_M) / (2.0 * MARGIN_M)).clamp(0.0, 1.0)
}

/// Where `hands.wgsl`'s own raymarch (`hd.eye.xyz + ray * t`) would
/// place `point` (absolute ship frame) on screen for one eye — the same
/// projection the shader's ray-direction formula performs, inverted
/// algebraically rather than raymarched, so this is exact and needs no
/// GPU: `local = head⁻¹ · (point − eye)`, then `ndc = (local.x /
/// -local.z / (tan_half·aspect), local.y / -local.z / tan_half)` (−Z is
/// forward, this engine-wide). `None` behind the eye (`local.z >= 0`),
/// which a raymarch never hits either. Exists so a real headset's own
/// finding — this pass's two eyes rendering a near controller at the
/// *same* screen position — has a direct, CPU-only regression test
/// (`hand_projects_with_crossed_disparity_between_two_eyes`) instead of
/// only a fragile pixel-colour GPU readback.
pub fn project_point(
    point: Vec3,
    eye: Vec3,
    head: Quat,
    tan_half: f32,
    aspect: f32,
) -> Option<[f32; 2]> {
    let local = head.inverse() * (point - eye);
    if local.z >= 0.0 {
        return None;
    }
    let half_w = (tan_half * aspect).max(1.0e-4);
    let half_h = tan_half.max(1.0e-4);
    Some([local.x / -local.z / half_w, local.y / -local.z / half_h])
}

pub type HandsPass = InstrumentPass;

pub fn hands_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> HandsPass {
    HandsPass::new_sized(
        device,
        target_format,
        sample_count,
        "hands",
        crate::shaders::HANDS,
        std::mem::size_of::<HandsUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hand_well_above_the_dash_shows_fully() {
        // The stick's own top (cockpit.wgsl's `base + lean`, roughly
        // (0, -0.62, -0.5)) sits just in front of the dash plane.
        assert!(dash_occlusion(Vec3::new(0.0, -0.2, -0.5)) > 0.99);
    }

    #[test]
    fn a_hand_well_behind_the_dash_is_hidden() {
        assert_eq!(dash_occlusion(DASH_C - DASH_N * 0.5), 0.0);
    }

    #[test]
    fn a_hand_crossing_the_dash_fades_smoothly() {
        let just_above = dash_occlusion(DASH_C + DASH_N * 0.03);
        let just_below = dash_occlusion(DASH_C - DASH_N * 0.03);
        assert!(just_above > 0.6 && just_above < 1.0, "{just_above}");
        assert!(just_below > 0.0 && just_below < 0.4, "{just_below}");
        assert!(just_above > just_below);
    }

    #[test]
    fn none_shows_neither_hand() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let u = HandsUniforms::none(&cam, Quat::IDENTITY, Vec3::new(0.032, 0.0, 0.0));
        assert_eq!(u.left_pos[3], 0.0);
        assert_eq!(u.right_pos[3], 0.0);
        assert_eq!(u.beam_a[3], 0.0, "no beam without with_beam");
        assert_eq!(
            &u.eye[..3],
            &[0.032, 0.0, 0.0],
            "the eye's own seat travels through"
        );
    }

    #[test]
    fn a_shown_hand_carries_its_pose_and_trigger() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let glyph = HandGlyph {
            pos: Vec3::new(0.1, -0.6, -0.4),
            rot: Quat::from_rotation_y(0.3),
            trigger: 0.7,
            squeeze: 0.2,
            held: true,
        };
        let u = HandsUniforms::new(&cam, Quat::IDENTITY, Vec3::ZERO, Some(glyph), None);
        assert_eq!(u.left_pos[3], 1.0, "shown");
        assert_eq!(
            &u.left_pos[..3],
            &[0.1, -0.6, -0.4],
            "absolute, not eye-shifted"
        );
        assert_eq!(u.left_state[0], 0.7, "trigger");
        assert_eq!(u.left_state[2], 1.0, "held");
        assert_eq!(u.right_pos[3], 0.0, "the other hand is untouched");
    }

    #[test]
    fn two_eyes_at_different_seats_produce_different_hands_uniforms() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let glyph = HandGlyph {
            pos: Vec3::new(0.1, -0.6, -0.4),
            rot: Quat::IDENTITY,
            trigger: 0.0,
            squeeze: 0.0,
            held: false,
        };
        let left = HandsUniforms::new(
            &cam,
            Quat::IDENTITY,
            Vec3::new(-0.032, 0.0, 0.0),
            Some(glyph),
            None,
        );
        let right = HandsUniforms::new(
            &cam,
            Quat::IDENTITY,
            Vec3::new(0.032, 0.0, 0.0),
            Some(glyph),
            None,
        );
        assert_eq!(
            left.left_pos, right.left_pos,
            "the hand's own absolute position does not move with the eye"
        );
        assert_ne!(
            bytemuck::bytes_of(&left),
            bytemuck::bytes_of(&right),
            "only the eye seat differs, but the ray origin the shader marches \
             from must still differ between the two eyes"
        );
    }

    /// The actual finding from a real synth-headset capture: a
    /// controller at real-world grabbing distance (~0.4m) must show
    /// *crossed* disparity — the left eye sees it shifted toward its
    /// own right, the right eye toward its own left — the standard
    /// near-object sign, or a bench harness reading pixels would catch
    /// exactly what a human did here by eye. `project_point` is the
    /// same math `hands.wgsl`'s ray direction performs, run backwards
    /// on the CPU: no GPU, no headset, no colour-matching pixels needed
    /// to pin this down precisely.
    #[test]
    fn hand_projects_with_crossed_disparity_between_two_eyes() {
        let half_ipd = 0.032;
        let left_eye = Vec3::new(-half_ipd, 0.0, 0.0);
        let right_eye = Vec3::new(half_ipd, 0.0, 0.0);
        // Straight ahead, 0.4m out — a real grabbing distance.
        let hand = Vec3::new(0.0, 0.0, -0.4);
        let tan_half = 0.7;
        let aspect = 1.5;
        let left_ndc =
            project_point(hand, left_eye, Quat::IDENTITY, tan_half, aspect).expect("in view");
        let right_ndc =
            project_point(hand, right_eye, Quat::IDENTITY, tan_half, aspect).expect("in view");
        assert!(
            left_ndc[0] > 0.0,
            "the left eye, seated to the hand's left, sees it toward its own right: {left_ndc:?}"
        );
        assert!(
            right_ndc[0] < 0.0,
            "the right eye, seated to the hand's right, sees it toward its own left: {right_ndc:?}"
        );
        assert!(
            (left_ndc[0] - right_ndc[0]).abs() > 0.05,
            "a 0.4m controller at a 6.4cm IPD should show clear, not marginal, \
             disparity: left={left_ndc:?} right={right_ndc:?}"
        );
    }

    #[test]
    fn with_beam_carries_both_ends_and_marks_itself_shown() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let u = HandsUniforms::none(&cam, Quat::IDENTITY, Vec3::ZERO)
            .with_beam(Vec3::new(0.1, -0.5, 0.1), Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(u.beam_a[3], 1.0, "shown");
        assert_eq!(&u.beam_a[..3], &[0.1, -0.5, 0.1]);
        assert_eq!(&u.beam_b[..3], &[0.0, 0.0, -1.0]);
    }
}
