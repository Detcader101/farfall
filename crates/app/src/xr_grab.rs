//! The virtual stick and throttle (SPEC §5.3b(d)): a grab-and-drag
//! interaction on the cockpit's own control column, feeding the SAME
//! `Controls` path the HOTAS does (`InputState::set_vr_stick`, summed
//! in `input.rs` alongside the real stick with the physical one always
//! winning once it moves — see `InputState::summed`'s own doc comment).
//!
//! State machine ported from Hotham's grab pattern (Hotham, Apache-2.0/
//! MIT, `examples/grab_object.rs`; see `docs/RESEARCH-VR-OSS.md` §3):
//! squeeze past [`GRAB_SQUEEZE_ON`] within [`GRAB_REACH_M`] of the
//! control enters the grab; squeeze dropping below [`GRAB_SQUEEZE_OFF`]
//! releases it. The gap between the two thresholds is deliberate
//! hysteresis, straight from that same research note: a squeeze resting
//! near one shared threshold would otherwise grab and release every
//! other frame.
//!
//! Pure and target-independent: everything here is `glam` maths, gated
//! by nothing, exercised without a headset.

use glam::Vec3;

/// Enter a grab once squeeze crosses this...
pub const GRAB_SQUEEZE_ON: f32 = 0.6;
/// ...and release once it drops below this.
pub const GRAB_SQUEEZE_OFF: f32 = 0.4;

/// How close the grip must be to a control to grab it, metres.
pub const GRAB_REACH_M: f32 = 0.12;

/// Displacement inside this radius of the anchor does nothing — a
/// resting hand's own tremor must not read as input.
pub const DEAD_ZONE_M: f32 = 0.015;
/// Same idea for the grip's twist (yaw), radians.
pub const DEAD_ZONE_RAD: f32 = 0.05;

/// -1..1 axis per metre of displacement past the dead zone.
pub const SENSITIVITY_PER_M: f32 = 6.0;
/// -1..1 axis per radian of twist past the dead zone.
pub const YAW_SENSITIVITY_PER_RAD: f32 = 2.0;

/// The stick's own grip, ship frame, at rest — `cockpit.wgsl`'s `top +
/// vec3(0, 0.04, 0)` (the grip ellipsoid's centre) with `ck.stick` at
/// zero, so the lean is zero too: `base` (0,-1,-0.45), `top` (0,-0.62,
/// -0.5), grip (0,-0.58,-0.5).
pub const STICK_REST: Vec3 = Vec3::new(0.0, -0.58, -0.5);
/// The throttle lever's own centre, ship frame, at rest — `cockpit.wgsl`'s
/// `vec3(-0.74, -0.53, tz)` with `ck.stick.w` (the lever axis) at zero,
/// so `tz = -0.3`.
pub const THROTTLE_REST: Vec3 = Vec3::new(-0.74, -0.53, -0.3);

/// Which hand is holding a [`Grab`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

/// One hand's live state, as `Grab::update` wants it: grip position
/// (ship frame), squeeze 0..1, and the grip's own yaw (radians, about
/// +Y — the twist axis) for the stick's yaw input.
pub type HandState = (Vec3, f32, f32);

/// One control's grab state: which hand (if any) is holding it, and
/// where that hand was when it grabbed — every frame's displacement is
/// measured from there, not from the control's own rest position, so
/// the grab does not jump the moment it is taken.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Grab {
    holder: Option<Hand>,
    anchor_pos: Vec3,
    anchor_yaw: f32,
}

impl Grab {
    pub const fn new() -> Self {
        Grab {
            holder: None,
            anchor_pos: Vec3::ZERO,
            anchor_yaw: 0.0,
        }
    }

    pub fn holder(&self) -> Option<Hand> {
        self.holder
    }

    /// Advance the grab against this frame's hands. Held: follow the
    /// SAME hand that took the grab (switching to whichever hand is
    /// merely nearer this frame would jump the anchor); its squeeze
    /// dropping below [`GRAB_SQUEEZE_OFF`], or it going untracked,
    /// releases. Not held: either hand squeezing past [`GRAB_SQUEEZE_ON`]
    /// within `reach_m` of `target` takes it — the nearer one, if both
    /// qualify the same frame. Returns the displacement from the
    /// anchor (position, yaw) — `(ZERO, 0.0)` whenever not held.
    pub fn update(
        &mut self,
        left: Option<HandState>,
        right: Option<HandState>,
        target: Vec3,
        reach_m: f32,
    ) -> (Vec3, f32) {
        if let Some(h) = self.holder {
            let current = match h {
                Hand::Left => left,
                Hand::Right => right,
            };
            let Some((pos, squeeze, yaw)) = current else {
                self.holder = None;
                return (Vec3::ZERO, 0.0);
            };
            if squeeze < GRAB_SQUEEZE_OFF {
                self.holder = None;
                return (Vec3::ZERO, 0.0);
            }
            return (pos - self.anchor_pos, wrap_pi(yaw - self.anchor_yaw));
        }
        let candidate = |s: Option<HandState>| {
            s.filter(|(p, squeeze, _)| *squeeze > GRAB_SQUEEZE_ON && p.distance(target) < reach_m)
        };
        match (candidate(left), candidate(right)) {
            (Some(l), Some(r)) => {
                if l.0.distance(target) <= r.0.distance(target) {
                    self.take(Hand::Left, l);
                } else {
                    self.take(Hand::Right, r);
                }
            }
            (Some(l), None) => self.take(Hand::Left, l),
            (None, Some(r)) => self.take(Hand::Right, r),
            (None, None) => {}
        }
        (Vec3::ZERO, 0.0)
    }

    fn take(&mut self, hand: Hand, (pos, _, yaw): HandState) {
        self.holder = Some(hand);
        self.anchor_pos = pos;
        self.anchor_yaw = yaw;
    }
}

fn wrap_pi(a: f32) -> f32 {
    let two_pi = std::f32::consts::TAU;
    let mut a = a % two_pi;
    if a > std::f32::consts::PI {
        a -= two_pi;
    }
    if a < -std::f32::consts::PI {
        a += two_pi;
    }
    a
}

/// A displacement past the dead zone becomes a -1..1 axis, scaled by
/// `sensitivity`; inside the dead zone it is exactly zero, not a small
/// fraction — a resting hand must read as no input at all, not a
/// trickle of one (the same reasoning `InputState`'s own "snap the last
/// sliver" comment gives for the keyboard's smoothing).
pub fn axis_from_displacement(disp: f32, dead_zone: f32, sensitivity: f32) -> f64 {
    let mag = disp.abs();
    if mag <= dead_zone {
        return 0.0;
    }
    (((mag - dead_zone) * disp.signum()) as f64 * sensitivity as f64).clamp(-1.0, 1.0)
}

/// Both virtual controls together, each an independent [`Grab`]: the
/// stick (pitch from forward/back displacement, roll from left/right,
/// yaw from the grip's own twist) and the throttle (thrust from
/// forward/back displacement alone).
#[derive(Debug, Clone, Copy, Default)]
pub struct GrabRig {
    pub stick: Grab,
    pub throttle: Grab,
}

impl GrabRig {
    pub const fn new() -> Self {
        GrabRig {
            stick: Grab::new(),
            throttle: Grab::new(),
        }
    }

    /// This frame's axes, in [`crate::input::InputState::set_vr_stick`]'s
    /// own layout (thrust x/y/z, torque pitch/yaw/roll) — zero wherever
    /// nothing is grabbed.
    pub fn update(&mut self, left: Option<HandState>, right: Option<HandState>) -> [f64; 6] {
        let (stick_disp, stick_yaw) = self.stick.update(left, right, STICK_REST, GRAB_REACH_M);
        let (throttle_disp, _) = self
            .throttle
            .update(left, right, THROTTLE_REST, GRAB_REACH_M);
        let pitch = axis_from_displacement(stick_disp.z, DEAD_ZONE_M, SENSITIVITY_PER_M);
        let roll = axis_from_displacement(stick_disp.x, DEAD_ZONE_M, SENSITIVITY_PER_M);
        let yaw = axis_from_displacement(stick_yaw, DEAD_ZONE_RAD, YAW_SENSITIVITY_PER_RAD);
        let thrust_z = axis_from_displacement(throttle_disp.z, DEAD_ZONE_M, SENSITIVITY_PER_M);
        [0.0, 0.0, thrust_z, pitch, yaw, roll]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_squeeze_within_reach_grabs_and_a_light_squeeze_releases() {
        let mut g = Grab::new();
        let target = Vec3::ZERO;
        // Squeeze too light: no grab.
        let (d, _) = g.update(
            Some((Vec3::new(0.05, 0.0, 0.0), 0.5, 0.0)),
            None,
            target,
            0.2,
        );
        assert!(!g.holder().is_some());
        assert_eq!(d, Vec3::ZERO);
        // Squeeze past ON, within reach: grabbed.
        let (d, _) = g.update(
            Some((Vec3::new(0.05, 0.0, 0.0), 0.8, 0.0)),
            None,
            target,
            0.2,
        );
        assert!(g.holder().is_some());
        assert_eq!(g.holder(), Some(Hand::Left));
        assert_eq!(d, Vec3::ZERO, "no displacement the frame it is taken");
        // Held, hand moves: displacement from the anchor.
        let (d, _) = g.update(
            Some((Vec3::new(0.15, 0.0, 0.0), 0.8, 0.0)),
            None,
            target,
            0.2,
        );
        assert!((d.x - 0.10).abs() < 1e-6, "{d:?}");
        // Squeeze drops below OFF: released.
        let (d, _) = g.update(
            Some((Vec3::new(0.15, 0.0, 0.0), 0.3, 0.0)),
            None,
            target,
            0.2,
        );
        assert!(!g.holder().is_some());
        assert_eq!(d, Vec3::ZERO);
    }

    #[test]
    fn a_squeeze_between_the_thresholds_does_not_flutter() {
        // Hysteresis: once held, a squeeze between OFF and ON stays held.
        let mut g = Grab::new();
        let target = Vec3::ZERO;
        g.update(Some((Vec3::ZERO, 0.9, 0.0)), None, target, 0.2);
        assert!(g.holder().is_some());
        g.update(Some((Vec3::ZERO, 0.5, 0.0)), None, target, 0.2);
        assert!(
            g.holder().is_some(),
            "0.5 is between OFF and ON: still held"
        );
    }

    #[test]
    fn a_squeeze_out_of_reach_does_not_grab() {
        let mut g = Grab::new();
        g.update(
            Some((Vec3::new(1.0, 0.0, 0.0), 0.9, 0.0)),
            None,
            Vec3::ZERO,
            0.2,
        );
        assert!(!g.holder().is_some());
    }

    #[test]
    fn losing_the_hand_entirely_releases_even_mid_squeeze() {
        let mut g = Grab::new();
        g.update(Some((Vec3::ZERO, 0.9, 0.0)), None, Vec3::ZERO, 0.2);
        assert!(g.holder().is_some());
        let (d, _) = g.update(None, None, Vec3::ZERO, 0.2);
        assert!(!g.holder().is_some());
        assert_eq!(d, Vec3::ZERO);
    }

    #[test]
    fn the_nearer_hand_wins_a_fresh_grab_and_the_grab_keeps_that_hand() {
        let mut g = Grab::new();
        let target = Vec3::ZERO;
        let near_left = (Vec3::new(0.05, 0.0, 0.0), 0.9, 0.0);
        let far_right = (Vec3::new(0.15, 0.0, 0.0), 0.9, 0.0);
        g.update(Some(near_left), Some(far_right), target, 0.2);
        assert_eq!(g.holder(), Some(Hand::Left));
        // Next frame the right hand is now nearer, but the grab stays
        // with the left hand — it does not jump.
        let now_near_right = (Vec3::new(0.02, 0.0, 0.0), 0.9, 0.0);
        let now_far_left = (Vec3::new(0.20, 0.0, 0.0), 0.9, 0.0);
        g.update(Some(now_far_left), Some(now_near_right), target, 0.2);
        assert_eq!(g.holder(), Some(Hand::Left));
    }

    #[test]
    fn displacement_inside_the_dead_zone_is_exactly_zero() {
        assert_eq!(axis_from_displacement(0.01, 0.015, 6.0), 0.0);
        assert_eq!(axis_from_displacement(-0.01, 0.015, 6.0), 0.0);
        assert_eq!(
            axis_from_displacement(0.015, 0.015, 6.0),
            0.0,
            "right at the edge"
        );
    }

    #[test]
    fn displacement_past_the_dead_zone_scales_and_keeps_its_sign() {
        let a = axis_from_displacement(0.065, DEAD_ZONE_M, SENSITIVITY_PER_M);
        // (0.065 - 0.015) * 6.0 = 0.3
        assert!((a - 0.3).abs() < 1e-6, "{a}");
        let b = axis_from_displacement(-0.065, DEAD_ZONE_M, SENSITIVITY_PER_M);
        assert!((b + 0.3).abs() < 1e-6, "{b}");
    }

    #[test]
    fn displacement_clamps_to_the_axis_range() {
        let a = axis_from_displacement(5.0, DEAD_ZONE_M, SENSITIVITY_PER_M);
        assert_eq!(a, 1.0);
        let b = axis_from_displacement(-5.0, DEAD_ZONE_M, SENSITIVITY_PER_M);
        assert_eq!(b, -1.0);
    }

    #[test]
    fn the_rig_maps_stick_displacement_to_pitch_and_roll_and_throttle_to_thrust() {
        let mut rig = GrabRig::new();
        // Grab the stick with the left hand.
        rig.update(Some((STICK_REST, 0.9, 0.0)), None);
        // Pull back (+z, toward the pilot) and right (+x): pitch and roll.
        let axes = rig.update(
            Some((STICK_REST + Vec3::new(0.05, 0.0, 0.08), 0.9, 0.0)),
            None,
        );
        assert!(axes[3] > 0.0, "pulling back pitches up: {axes:?}");
        assert!(axes[5] > 0.0, "moving right rolls right: {axes:?}");
        assert_eq!(axes[2], 0.0, "the stick alone does not touch thrust");
    }

    #[test]
    fn the_rig_maps_throttle_displacement_to_forward_thrust() {
        let mut rig = GrabRig::new();
        rig.update(None, Some((THROTTLE_REST, 0.9, 0.0)));
        // Push forward (-z, away from the pilot): more thrust.
        let axes = rig.update(
            None,
            Some((THROTTLE_REST + Vec3::new(0.0, 0.0, -0.08), 0.9, 0.0)),
        );
        assert!(
            axes[2] < 0.0,
            "forward thrust is -Z in body frame: {axes:?}"
        );
        assert_eq!(axes[3], 0.0, "the throttle alone does not touch pitch");
        assert_eq!(axes[5], 0.0, "or roll");
    }

    #[test]
    fn a_stick_grip_twist_drives_yaw_past_its_own_dead_zone() {
        let mut rig = GrabRig::new();
        rig.update(Some((STICK_REST, 0.9, 0.0)), None);
        let axes = rig.update(Some((STICK_REST, 0.9, 0.3)), None);
        assert!(axes[4] > 0.0, "a twist past the dead zone yaws: {axes:?}");
    }
}
