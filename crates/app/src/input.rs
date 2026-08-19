//! Keyboard → [`Controls`] mapping (TASKS M1.2).
//!
//! Pure and windowless by design: the whole mapping is a function of held keys,
//! so it is unit-testable without a GPU, a window, or an event loop.
//!
//! Determinism note (SPEC §7.3): controls are assembled by walking [`AXES`] in
//! a fixed order, never by iterating a set. Float addition is not associative,
//! so summing the same contributions in a different order can differ in the
//! last bit — which, once controls travel over a wire to an authoritative
//! server, is exactly how two machines silently desync.
//!
//! Body frame is right-handed: +X right, +Y up, **−Z forward (the nose)**.

use farfall_sim::Controls;
use glam::DVec3;
use winit::keyboard::KeyCode;

/// One physical control input. Order is part of the contract (see module docs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    ThrustForward,
    ThrustBack,
    StrafeLeft,
    StrafeRight,
    ThrustUp,
    ThrustDown,
    PitchUp,
    PitchDown,
    YawLeft,
    YawRight,
    RollLeft,
    RollRight,
}

impl Action {
    pub const COUNT: usize = 12;
}

/// Which axis each action drives: (action, is_torque, component, sign).
///
/// Rotation signs, derived in the right-handed body frame with the nose at −Z.
/// These are counter-intuitive enough that `sim_directions` in `main.rs` asserts
/// every one of them against the actual integrator rather than trusting comments:
/// - pitch up: +X torque swings the nose (−Z) toward +Y.
/// - yaw right: −Y torque swings the nose toward +X.
/// - roll right: −Z torque tips the up vector (+Y) toward +X (right wing down).
const AXES: [(Action, bool, usize, f64); Action::COUNT] = [
    (Action::ThrustForward, false, 2, -1.0),
    (Action::ThrustBack, false, 2, 1.0),
    (Action::StrafeLeft, false, 0, -1.0),
    (Action::StrafeRight, false, 0, 1.0),
    (Action::ThrustUp, false, 1, 1.0),
    (Action::ThrustDown, false, 1, -1.0),
    (Action::PitchUp, true, 0, 1.0),
    (Action::PitchDown, true, 0, -1.0),
    (Action::YawLeft, true, 1, 1.0),
    (Action::YawRight, true, 1, -1.0),
    (Action::RollLeft, true, 2, 1.0),
    (Action::RollRight, true, 2, -1.0),
];

/// Boost is a modifier rather than an axis, so it sits outside [`AXES`].
const BOOST_KEY: KeyCode = KeyCode::ShiftLeft;

/// Physical-key bindings. Physical (not logical) keys so the layout is the same
/// shape on QWERTY, AZERTY, and Dvorak.
const BINDINGS: [(KeyCode, Action); Action::COUNT] = [
    (KeyCode::KeyW, Action::ThrustForward),
    (KeyCode::KeyS, Action::ThrustBack),
    (KeyCode::KeyA, Action::StrafeLeft),
    (KeyCode::KeyD, Action::StrafeRight),
    (KeyCode::KeyR, Action::ThrustUp),
    (KeyCode::KeyF, Action::ThrustDown),
    (KeyCode::ArrowUp, Action::PitchUp),
    (KeyCode::ArrowDown, Action::PitchDown),
    (KeyCode::ArrowLeft, Action::YawLeft),
    (KeyCode::ArrowRight, Action::YawRight),
    (KeyCode::KeyQ, Action::RollLeft),
    (KeyCode::KeyE, Action::RollRight),
];

pub fn action_for(key: KeyCode) -> Option<Action> {
    let mut i = 0;
    while i < BINDINGS.len() {
        if matches!(BINDINGS[i].0, k if k == key) {
            return Some(BINDINGS[i].1);
        }
        i += 1;
    }
    None
}

/// Seconds for a control axis to reach ~63% of a newly pressed key's demand.
/// Short enough to feel immediate, long enough that the ship eases into a turn.
const ATTACK_S: f64 = 0.13;
/// Seconds to fall back to neutral on release. Slower than the attack, so
/// letting go coasts out of the manoeuvre rather than snapping out of it.
const RELEASE_S: f64 = 0.22;

/// Which control keys are held, plus the smoothed axis values they drive.
///
/// A key is binary and a stick is not. Feeding raw 0/1 into the sim makes every
/// input a step change — instant full deflection, instant full stop — which
/// reads as twitchy however heavy the ship's physics are. Ramping each axis
/// toward its target gives keyboard control the shape of an analog input, and
/// costs one lerp per axis per frame.
#[derive(Clone, Copy, Default, Debug)]
pub struct InputState {
    held: [bool; Action::COUNT],
    boost: bool,
    /// Smoothed axis values: [thrust xyz, torque xyz].
    axes: [f64; 6],
}

impl InputState {
    pub fn set(&mut self, key: KeyCode, pressed: bool) {
        if key == BOOST_KEY {
            self.boost = pressed;
        }
        if let Some(action) = action_for(key) {
            self.held[action as usize] = pressed;
        }
    }

    /// Fraction of maximum available thrust currently demanded, in [0, 1].
    /// Render-side only: it drives the camera's response to acceleration, and
    /// must never feed back into the sim.
    pub fn thrust_effort(&self, boost_multiplier: f64) -> f64 {
        let c = self.controls(false);
        let axes = c.thrust_body.abs().max_element();
        let scale = if self.boost { boost_multiplier } else { 1.0 };
        (axes * scale / boost_multiplier).clamp(0.0, 1.0)
    }

    /// Drop all held keys. Called on focus loss — otherwise a key held while
    /// alt-tabbing away never receives its release and the ship flies off alone.
    /// The smoothed axes are zeroed too: easing out of a burn the pilot can no
    /// longer see would be worse than stopping it.
    pub fn release_all(&mut self) {
        self.held = [false; Action::COUNT];
        self.boost = false;
        self.axes = [0.0; 6];
    }

    /// Advance the smoothing by `dt` seconds.
    ///
    /// Exponential, and framerate-independent by construction: the same key
    /// press takes the same wall-clock time to reach full deflection at 30 fps
    /// and at 240. A per-frame lerp constant would make the ship handle
    /// differently on every machine.
    pub fn update(&mut self, dt: f64) {
        let target = self.raw_axes();
        for (axis, want) in self.axes.iter_mut().zip(target) {
            let tau = if want.abs() > axis.abs() {
                ATTACK_S
            } else {
                RELEASE_S
            };
            let alpha = 1.0 - (-dt / tau).exp();
            *axis += (want - *axis) * alpha;
            // Snap the last sliver. An exponential only approaches its target,
            // and "99.99% thrust forever" is both a lie about what the pilot
            // asked for and a trickle of input that never lets the ship settle.
            if (want - *axis).abs() < 1e-4 {
                *axis = want;
            }
        }
    }

    /// The instantaneous demand from held keys, before smoothing.
    fn raw_axes(&self) -> [f64; 6] {
        let mut out = [0.0f64; 6];
        for (action, is_torque, component, sign) in AXES {
            if self.held[action as usize] {
                out[if is_torque { 3 + component } else { component }] += sign;
            }
        }
        out
    }

    /// Assemble sim controls from the smoothed axes. Opposing keys cancel; every
    /// component lands in [-1, 1] by construction, so the sim's clamp is a
    /// backstop, not a crutch.
    pub fn controls(&self, assist: bool) -> Controls {
        Controls {
            thrust_body: DVec3::new(self.axes[0], self.axes[1], self.axes[2]),
            torque_body: DVec3::new(self.axes[3], self.axes[4], self.axes[5]),
            assist,
            boost: self.boost,
        }
    }

    /// Controls as if smoothing had already settled — the pilot's intent rather
    /// than the ship's current response to it. Used by the tests that check how
    /// the sim maps a control to motion, where waiting out the ramp would only
    /// obscure what is being tested.
    #[cfg(test)]
    pub fn controls_immediate(&self, assist: bool) -> Controls {
        let a = self.raw_axes();
        Controls {
            thrust_body: DVec3::new(a[0], a[1], a[2]),
            torque_body: DVec3::new(a[3], a[4], a[5]),
            assist,
            boost: self.boost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hold keys and let the smoothing settle, so tests see steady-state
    /// deflection rather than the first frame of the ramp.
    fn held(keys: &[KeyCode]) -> InputState {
        let mut s = InputState::default();
        for k in keys {
            s.set(*k, true);
        }
        for _ in 0..600 {
            s.update(1.0 / 120.0);
        }
        s
    }

    #[test]
    fn idle_input_is_neutral() {
        let c = InputState::default().controls(false);
        assert_eq!(c.thrust_body, DVec3::ZERO);
        assert_eq!(c.torque_body, DVec3::ZERO);
    }

    #[test]
    fn forward_key_thrusts_along_nose() {
        let c = held(&[KeyCode::KeyW]).controls(false);
        assert_eq!(
            c.thrust_body,
            DVec3::NEG_Z,
            "nose is -Z in a right-handed frame"
        );
        assert_eq!(c.torque_body, DVec3::ZERO);
    }

    #[test]
    fn opposing_keys_cancel() {
        for (a, b) in [
            (KeyCode::KeyW, KeyCode::KeyS),
            (KeyCode::KeyA, KeyCode::KeyD),
            (KeyCode::KeyR, KeyCode::KeyF),
            (KeyCode::ArrowUp, KeyCode::ArrowDown),
            (KeyCode::ArrowLeft, KeyCode::ArrowRight),
            (KeyCode::KeyQ, KeyCode::KeyE),
        ] {
            let c = held(&[a, b]).controls(false);
            assert_eq!(c.thrust_body, DVec3::ZERO, "{a:?}+{b:?} thrust");
            assert_eq!(c.torque_body, DVec3::ZERO, "{a:?}+{b:?} torque");
        }
    }

    /// Even with every key mashed at once, nothing escapes [-1, 1] or goes NaN.
    #[test]
    fn every_key_at_once_stays_in_range() {
        let all: Vec<KeyCode> = BINDINGS.iter().map(|(k, _)| *k).collect();
        let c = held(&all).controls(true);
        for v in [c.thrust_body, c.torque_body] {
            assert!(v.is_finite(), "non-finite control: {v:?}");
            for axis in 0..3 {
                assert!((-1.0..=1.0).contains(&v[axis]), "out of range: {v:?}");
            }
        }
    }

    #[test]
    fn release_clears_the_axis() {
        let mut s = held(&[KeyCode::KeyW]);
        s.set(KeyCode::KeyW, false);
        for _ in 0..600 {
            s.update(1.0 / 120.0);
        }
        assert_eq!(
            s.controls(false).thrust_body,
            DVec3::ZERO,
            "a released key must reach exactly neutral, not merely approach it"
        );
    }

    /// The ramp is monotonic, never overshoots, and actually gets there.
    #[test]
    fn smoothing_ramps_without_overshoot() {
        let mut s = InputState::default();
        s.set(KeyCode::KeyW, true);
        let mut last = 0.0;
        for _ in 0..240 {
            s.update(1.0 / 120.0);
            let v = -s.controls(false).thrust_body.z; // nose is -Z
            assert!(v >= last - 1e-12, "ramp went backwards");
            assert!(v <= 1.0 + 1e-12, "ramp overshot to {v}");
            last = v;
        }
        assert!(last > 0.99, "ramp never arrived: {last}");
    }

    /// Ramp time is wall-clock, not frame count: the ship must handle
    /// identically on a 30 fps machine and a 240 fps one.
    #[test]
    fn smoothing_is_framerate_independent() {
        let settle = |steps: u32, dt: f64| {
            let mut s = InputState::default();
            s.set(KeyCode::KeyW, true);
            for _ in 0..steps {
                s.update(dt);
            }
            -s.controls(false).thrust_body.z
        };
        // Half a second of holding, at 30 fps and at 240 fps.
        let slow = settle(15, 1.0 / 30.0);
        let fast = settle(120, 1.0 / 240.0);
        assert!(
            (slow - fast).abs() < 0.01,
            "frame rate changed the handling: {slow:.4} vs {fast:.4}"
        );
    }

    /// Smoothing must not turn a released key into a slow drift in the opposite
    /// direction, nor stall a reversal.
    #[test]
    fn reversing_passes_through_neutral() {
        let mut s = held(&[KeyCode::KeyW]);
        s.set(KeyCode::KeyW, false);
        s.set(KeyCode::KeyS, true);
        let mut crossed = false;
        for _ in 0..240 {
            s.update(1.0 / 120.0);
            if s.controls(false).thrust_body.z.abs() < 0.02 {
                crossed = true;
            }
        }
        assert!(crossed, "reversal never passed through neutral");
        assert!(
            s.controls(false).thrust_body.z > 0.9,
            "reversal did not complete"
        );
    }

    #[test]
    fn focus_loss_releases_everything() {
        let mut s = held(&[KeyCode::KeyW, KeyCode::ArrowLeft, KeyCode::KeyQ]);
        s.release_all();
        let c = s.controls(false);
        assert_eq!(c.thrust_body, DVec3::ZERO);
        assert_eq!(c.torque_body, DVec3::ZERO);
    }

    #[test]
    fn unbound_keys_are_ignored() {
        let mut s = InputState::default();
        s.set(KeyCode::KeyZ, true);
        s.set(KeyCode::Space, true);
        assert_eq!(s.controls(false).thrust_body, DVec3::ZERO);
    }

    #[test]
    fn boost_is_a_modifier_not_an_axis() {
        let s = held(&[KeyCode::KeyW, KeyCode::ShiftLeft]);
        let c = s.controls(false);
        assert!(c.boost, "shift must engage boost");
        // Boost must not disturb the axes it multiplies.
        assert_eq!(c.thrust_body, DVec3::NEG_Z);
        assert_eq!(c.torque_body, DVec3::ZERO);
    }

    #[test]
    fn thrust_effort_reports_the_camera_signal() {
        let mult = 3.5;
        assert_eq!(InputState::default().thrust_effort(mult), 0.0);
        let cruise = held(&[KeyCode::KeyW]).thrust_effort(mult);
        let boosting = held(&[KeyCode::KeyW, KeyCode::ShiftLeft]).thrust_effort(mult);
        assert!(boosting > cruise, "boost must read as more effort");
        assert!((boosting - 1.0).abs() < 1e-9, "full boost is full effort");
        assert!(cruise > 0.0 && cruise < 1.0);
    }

    #[test]
    fn focus_loss_releases_boost_too() {
        let mut s = held(&[KeyCode::KeyW, KeyCode::ShiftLeft]);
        s.release_all();
        assert!(!s.controls(false).boost);
    }

    /// Assist is passed through untouched and never leaks into the axes.
    #[test]
    fn assist_flag_is_orthogonal() {
        let s = held(&[KeyCode::KeyW, KeyCode::ArrowUp]);
        let off = s.controls(false);
        let on = s.controls(true);
        assert!(!off.assist && on.assist);
        assert_eq!(off.thrust_body, on.thrust_body);
        assert_eq!(off.torque_body, on.torque_body);
    }

    /// Every action must be reachable, and no key may drive two actions.
    #[test]
    fn bindings_are_complete_and_unique() {
        assert_eq!(BINDINGS.len(), Action::COUNT);
        assert_eq!(AXES.len(), Action::COUNT);
        for (i, (key, _)) in BINDINGS.iter().enumerate() {
            for (other, _) in BINDINGS.iter().skip(i + 1) {
                assert_ne!(key, other, "key {key:?} bound twice");
            }
        }
        // Each action appears exactly once in AXES, at its own enum index.
        for (i, (action, ..)) in AXES.iter().enumerate() {
            assert_eq!(*action as usize, i, "AXES order must match enum order");
        }
    }
}
