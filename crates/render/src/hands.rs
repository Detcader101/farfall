//! Controller glyphs in VR (`shaders/hands.wgsl`, SPEC §5.3b): a small
//! SDF raymarch of each tracked hand at its grip pose, in the ship's
//! frame, drawn in the ship pass right after the cabin — the same
//! per-eye parallax every other cockpit-frame pass gets from
//! `with_eye` (`cabin.rs`, `ghost.rs`, `shield.rs`): this pass must not
//! repeat the zero-disparity mistake, since a hand a third of a metre
//! from the eye shows real, obvious stereo disparity a distant cabin
//! rarely does.
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
    /// The grip pose, ship frame, already `with_eye`-shifted by the
    /// caller if this is being built per-eye (see [`HandsUniforms::new`]).
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
    // Left hand: xyz position (ship frame, eye-shifted); w: shown (1/0).
    left_pos: [f32; 4],
    // Left hand's orientation, a quaternion (xyz, w).
    left_rot: [f32; 4],
    // x: trigger 0..1. y: squeeze 0..1. z: held (1/0). w: occlusion fade.
    left_state: [f32; 4],
    right_pos: [f32; 4],
    right_rot: [f32; 4],
    right_state: [f32; 4],
}

impl HandsUniforms {
    /// Nothing shown: the shader discards every pixel.
    pub fn none(cam: &CameraFrame, head: Quat) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        Self {
            right: v4(head * Vec3::X, cam.aspect),
            up: v4(head * Vec3::Y, (cam.fov_y * 0.5).tan()),
            fwd: v4(head * Vec3::NEG_Z, cam.time_s.rem_euclid(1000.0)),
            left_pos: [0.0; 4],
            left_rot: [0.0, 0.0, 0.0, 1.0],
            left_state: [0.0; 4],
            right_pos: [0.0; 4],
            right_rot: [0.0, 0.0, 0.0, 1.0],
            right_state: [0.0; 4],
        }
    }

    /// Both hands this frame, in the current eye's own seat: `left`/
    /// `right` are `None` when that hand isn't tracked. Callers pass
    /// grip positions already shifted by `-eye_ship` (see `cabin.rs`'s
    /// own `with_eye` convention) so the two eyes see real parallax.
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        left: Option<HandGlyph>,
        right: Option<HandGlyph>,
    ) -> Self {
        let mut u = Self::none(cam, head);
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
        let u = HandsUniforms::none(&cam, Quat::IDENTITY);
        assert_eq!(u.left_pos[3], 0.0);
        assert_eq!(u.right_pos[3], 0.0);
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
        let u = HandsUniforms::new(&cam, Quat::IDENTITY, Some(glyph), None);
        assert_eq!(u.left_pos[3], 1.0, "shown");
        assert_eq!(&u.left_pos[..3], &[0.1, -0.6, -0.4]);
        assert_eq!(u.left_state[0], 0.7, "trigger");
        assert_eq!(u.left_state[2], 1.0, "held");
        assert_eq!(u.right_pos[3], 0.0, "the other hand is untouched");
    }
}
