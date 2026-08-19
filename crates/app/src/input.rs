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

/// Which control keys are currently held.
#[derive(Clone, Copy, Default, Debug)]
pub struct InputState {
    held: [bool; Action::COUNT],
}

impl InputState {
    pub fn set(&mut self, key: KeyCode, pressed: bool) {
        if let Some(action) = action_for(key) {
            self.held[action as usize] = pressed;
        }
    }

    /// Drop all held keys. Called on focus loss — otherwise a key held while
    /// alt-tabbing away never receives its release and the ship flies off alone.
    pub fn release_all(&mut self) {
        self.held = [false; Action::COUNT];
    }

    /// Assemble sim controls. Opposing keys cancel; every component lands in
    /// [-1, 1] by construction, so the sim's clamp is a backstop, not a crutch.
    pub fn controls(&self, assist: bool) -> Controls {
        let mut thrust = [0.0f64; 3];
        let mut torque = [0.0f64; 3];
        for (action, is_torque, component, sign) in AXES {
            if self.held[action as usize] {
                let axis = if is_torque { &mut torque } else { &mut thrust };
                axis[component] += sign;
            }
        }
        Controls {
            thrust_body: DVec3::from_array(thrust),
            torque_body: DVec3::from_array(torque),
            assist,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(keys: &[KeyCode]) -> InputState {
        let mut s = InputState::default();
        for k in keys {
            s.set(*k, true);
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
        assert_eq!(s.controls(false).thrust_body, DVec3::ZERO);
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
