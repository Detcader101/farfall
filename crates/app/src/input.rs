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

    pub const ALL: [Action; Action::COUNT] = [
        Action::ThrustForward,
        Action::ThrustBack,
        Action::StrafeLeft,
        Action::StrafeRight,
        Action::ThrustUp,
        Action::ThrustDown,
        Action::PitchUp,
        Action::PitchDown,
        Action::YawLeft,
        Action::YawRight,
        Action::RollLeft,
        Action::RollRight,
    ];

    /// Menu label.
    pub fn name(self) -> &'static str {
        match self {
            Action::ThrustForward => "THRUST FWD",
            Action::ThrustBack => "THRUST BACK",
            Action::StrafeLeft => "STRAFE LEFT",
            Action::StrafeRight => "STRAFE RIGHT",
            Action::ThrustUp => "THRUST UP",
            Action::ThrustDown => "THRUST DOWN",
            Action::PitchUp => "PITCH UP",
            Action::PitchDown => "PITCH DOWN",
            Action::YawLeft => "YAW LEFT",
            Action::YawRight => "YAW RIGHT",
            Action::RollLeft => "ROLL LEFT",
            Action::RollRight => "ROLL RIGHT",
        }
    }

    /// Settings-file key.
    pub fn key(self) -> &'static str {
        match self {
            Action::ThrustForward => "thrust-forward",
            Action::ThrustBack => "thrust-back",
            Action::StrafeLeft => "strafe-left",
            Action::StrafeRight => "strafe-right",
            Action::ThrustUp => "thrust-up",
            Action::ThrustDown => "thrust-down",
            Action::PitchUp => "pitch-up",
            Action::PitchDown => "pitch-down",
            Action::YawLeft => "yaw-left",
            Action::YawRight => "yaw-right",
            Action::RollLeft => "roll-left",
            Action::RollRight => "roll-right",
        }
    }
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

/// Every named, non-axis control the game has: held states (boost, brake,
/// despin, the chaos drive) and edge toggles (map, modes, cameras). One
/// key each, every one rebindable, every one a row on the KEYS page — a
/// bind that exists in the game but not in the menu is a bug.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Named {
    Boost,
    Brake,
    Despin,
    Hyper,
    WarpStop,
    Map,
    Appearance,
    Engage,
    Landing,
    Design,
    LookLock,
    Trajectory,
    Assist,
    Chase,
    Holo,
    Capture,
    Bay,
    NextWeapon,
    Weapon1,
    Weapon2,
    ScaleDown,
    ScaleUp,
    Hold,
    /// The hologram shows more of the space round the ship, or less.
    HoloOut,
    HoloIn,
}

impl Named {
    pub const COUNT: usize = 25;
    pub const ALL: [Named; Named::COUNT] = [
        Named::Boost,
        Named::Brake,
        Named::Despin,
        Named::Hyper,
        Named::WarpStop,
        Named::Map,
        Named::Appearance,
        Named::Engage,
        Named::Landing,
        Named::Design,
        Named::LookLock,
        Named::Trajectory,
        Named::Assist,
        Named::Chase,
        Named::Holo,
        Named::Capture,
        Named::Bay,
        Named::NextWeapon,
        Named::Weapon1,
        Named::Weapon2,
        Named::ScaleDown,
        Named::ScaleUp,
        Named::Hold,
        Named::HoloOut,
        Named::HoloIn,
    ];

    /// Menu label.
    pub fn name(self) -> &'static str {
        match self {
            Named::Boost => "BOOST",
            Named::Brake => "AIR BRAKE",
            Named::Despin => "DESPIN",
            Named::Hyper => "CHAOS DRIVE",
            Named::WarpStop => "WARP STOP",
            Named::Map => "MAP",
            Named::Appearance => "ATMOSPHERE",
            Named::Engage => "ENGAGE DRIVE",
            Named::Landing => "LANDING MODE",
            Named::Design => "DESIGN MODE",
            Named::LookLock => "LOOK LOCK",
            Named::Trajectory => "TRAJECTORY",
            Named::Assist => "FLIGHT ASSIST",
            Named::Chase => "CHASE CAM",
            Named::Holo => "HOLO3PP",
            Named::Capture => "CAPTURE",
            Named::Bay => "SHIP BAY",
            Named::NextWeapon => "NEXT WEAPON",
            Named::Weapon1 => "CANNON",
            Named::Weapon2 => "RAIL",
            Named::ScaleDown => "RENDER SCALE -",
            Named::ScaleUp => "RENDER SCALE +",
            Named::Hold => "HOLD",
            Named::HoloOut => "HOLO WIDER",
            Named::HoloIn => "HOLO CLOSER",
        }
    }

    /// Settings-file key (control.<this>).
    pub fn key(self) -> &'static str {
        match self {
            Named::Boost => "boost",
            Named::Brake => "brake",
            Named::Despin => "despin",
            Named::Hyper => "hyper",
            Named::WarpStop => "warp-stop",
            Named::Map => "map",
            Named::Appearance => "atmosphere",
            Named::Engage => "engage",
            Named::Landing => "landing",
            Named::Design => "design",
            Named::LookLock => "look-lock",
            Named::Trajectory => "trajectory",
            Named::Assist => "assist",
            Named::Chase => "chase",
            Named::Holo => "holo",
            Named::Capture => "capture",
            Named::Bay => "bay",
            Named::NextWeapon => "next-weapon",
            Named::Weapon1 => "weapon-1",
            Named::Weapon2 => "weapon-2",
            Named::ScaleDown => "scale-down",
            Named::ScaleUp => "scale-up",
            Named::Hold => "hold",
            Named::HoloOut => "holo-out",
            Named::HoloIn => "holo-in",
        }
    }

    /// The stock key. H is the chaos drive and stays the chaos drive: the
    /// holo3PP lives on N, clear of everything the dash already answers to.
    pub fn default_key(self) -> KeyCode {
        match self {
            Named::Boost => KeyCode::ShiftLeft,
            Named::Brake => KeyCode::Space,
            Named::Despin => KeyCode::KeyZ,
            Named::Hyper => KeyCode::KeyH,
            Named::WarpStop => KeyCode::KeyV,
            Named::Map => KeyCode::KeyM,
            Named::Appearance => KeyCode::KeyC,
            Named::Engage => KeyCode::KeyJ,
            Named::Landing => KeyCode::KeyG,
            Named::Design => KeyCode::KeyK,
            Named::LookLock => KeyCode::KeyL,
            Named::Trajectory => KeyCode::KeyT,
            Named::Assist => KeyCode::KeyX,
            Named::Chase => KeyCode::KeyY,
            Named::Holo => KeyCode::KeyN,
            Named::Capture => KeyCode::KeyP,
            Named::Bay => KeyCode::KeyB,
            // N is the holo3PP; the weapons cycle on U.
            Named::NextWeapon => KeyCode::KeyU,
            Named::Weapon1 => KeyCode::Digit1,
            Named::Weapon2 => KeyCode::Digit2,
            Named::ScaleDown => KeyCode::BracketLeft,
            Named::ScaleUp => KeyCode::BracketRight,
            Named::Hold => KeyCode::KeyO,
            // Comma and period: DESIGN mode borrows them for tilt only while
            // it is on.
            Named::HoloOut => KeyCode::Comma,
            Named::HoloIn => KeyCode::Period,
        }
    }
}

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

/// The pilot's key map: one physical key per action, plus every named
/// control. Rebindable from the menu, kept in the settings file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bindings {
    keys: [KeyCode; Action::COUNT],
    named: [KeyCode; Named::COUNT],
}

impl Default for Bindings {
    fn default() -> Self {
        let mut keys = [KeyCode::KeyW; Action::COUNT];
        for (key, action) in BINDINGS {
            keys[action as usize] = key;
        }
        let mut named = [KeyCode::KeyW; Named::COUNT];
        for n in Named::ALL {
            named[n as usize] = n.default_key();
        }
        Self { keys, named }
    }
}

impl Bindings {
    pub fn key_for(&self, action: Action) -> KeyCode {
        self.keys[action as usize]
    }

    pub fn action_for(&self, key: KeyCode) -> Option<Action> {
        Action::ALL
            .iter()
            .copied()
            .find(|&a| self.keys[a as usize] == key)
    }

    /// Bind `key` to `action`. A key can mean one thing: whatever it meant
    /// before is unbound by swap — the displaced control takes this
    /// action's old key, so no key is ever lost and no key ever means two
    /// things. Returns false if the key is reserved by the game itself.
    pub fn bind(&mut self, action: Action, key: KeyCode) -> bool {
        if is_reserved(key) {
            return false;
        }
        let old = self.keys[action as usize];
        if let Some(displaced) = self.action_for(key) {
            if displaced != action {
                self.keys[displaced as usize] = old;
            }
        }
        for n in Named::ALL {
            if self.named[n as usize] == key {
                self.named[n as usize] = old;
            }
        }
        self.keys[action as usize] = key;
        true
    }

    pub fn named(&self, n: Named) -> KeyCode {
        self.named[n as usize]
    }

    /// Bind a named control, with the same swap rule as [`Self::bind`]:
    /// whatever held `key` — an axis action or another named control —
    /// takes this control's old key. One uniform rule for all sixteen,
    /// instead of a bespoke (and forgetful) function per control.
    pub fn bind_named(&mut self, n: Named, key: KeyCode) -> bool {
        if is_reserved(key) {
            return false;
        }
        let old = self.named[n as usize];
        if let Some(displaced) = self.action_for(key) {
            self.keys[displaced as usize] = old;
        }
        for m in Named::ALL {
            if m != n && self.named[m as usize] == key {
                self.named[m as usize] = old;
            }
        }
        self.named[n as usize] = key;
        true
    }
}

/// Keys the game keeps for itself: the menu and its navigation, and F1
/// for the CONTROLS card.
pub fn is_reserved(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::Escape | KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::F1
    )
}

/// Key names for the settings file and the menu. Physical keys, so the
/// names are positions, not letters on the cap.
pub const KEY_NAMES: &[(&str, KeyCode)] = &[
    ("A", KeyCode::KeyA),
    ("B", KeyCode::KeyB),
    ("C", KeyCode::KeyC),
    ("D", KeyCode::KeyD),
    ("E", KeyCode::KeyE),
    ("F", KeyCode::KeyF),
    ("G", KeyCode::KeyG),
    ("H", KeyCode::KeyH),
    ("I", KeyCode::KeyI),
    ("J", KeyCode::KeyJ),
    ("K", KeyCode::KeyK),
    ("L", KeyCode::KeyL),
    ("M", KeyCode::KeyM),
    ("N", KeyCode::KeyN),
    ("O", KeyCode::KeyO),
    ("P", KeyCode::KeyP),
    ("Q", KeyCode::KeyQ),
    ("R", KeyCode::KeyR),
    ("S", KeyCode::KeyS),
    ("T", KeyCode::KeyT),
    ("U", KeyCode::KeyU),
    ("V", KeyCode::KeyV),
    ("W", KeyCode::KeyW),
    ("X", KeyCode::KeyX),
    ("Y", KeyCode::KeyY),
    ("Z", KeyCode::KeyZ),
    ("0", KeyCode::Digit0),
    ("1", KeyCode::Digit1),
    ("2", KeyCode::Digit2),
    ("3", KeyCode::Digit3),
    ("4", KeyCode::Digit4),
    ("5", KeyCode::Digit5),
    ("6", KeyCode::Digit6),
    ("7", KeyCode::Digit7),
    ("8", KeyCode::Digit8),
    ("9", KeyCode::Digit9),
    ("UP", KeyCode::ArrowUp),
    ("DOWN", KeyCode::ArrowDown),
    ("LEFT", KeyCode::ArrowLeft),
    ("RIGHT", KeyCode::ArrowRight),
    ("SPACE", KeyCode::Space),
    ("LSHIFT", KeyCode::ShiftLeft),
    ("RSHIFT", KeyCode::ShiftRight),
    ("LCTRL", KeyCode::ControlLeft),
    ("RCTRL", KeyCode::ControlRight),
    ("LALT", KeyCode::AltLeft),
    ("RALT", KeyCode::AltRight),
    ("COMMA", KeyCode::Comma),
    ("PERIOD", KeyCode::Period),
    ("SLASH", KeyCode::Slash),
    ("SEMI", KeyCode::Semicolon),
    ("QUOTE", KeyCode::Quote),
    ("LBRACKET", KeyCode::BracketLeft),
    ("RBRACKET", KeyCode::BracketRight),
    ("MINUS", KeyCode::Minus),
    ("EQUAL", KeyCode::Equal),
    ("BACKQUOTE", KeyCode::Backquote),
    ("HOME", KeyCode::Home),
    ("END", KeyCode::End),
    ("PGUP", KeyCode::PageUp),
    ("PGDN", KeyCode::PageDown),
    ("INSERT", KeyCode::Insert),
    ("DELETE", KeyCode::Delete),
    ("NUM0", KeyCode::Numpad0),
    ("NUM1", KeyCode::Numpad1),
    ("NUM2", KeyCode::Numpad2),
    ("NUM3", KeyCode::Numpad3),
    ("NUM4", KeyCode::Numpad4),
    ("NUM5", KeyCode::Numpad5),
    ("NUM6", KeyCode::Numpad6),
    ("NUM7", KeyCode::Numpad7),
    ("NUM8", KeyCode::Numpad8),
    ("NUM9", KeyCode::Numpad9),
    ("NUM+", KeyCode::NumpadAdd),
    ("NUM-", KeyCode::NumpadSubtract),
    ("NUM*", KeyCode::NumpadMultiply),
    ("NUM/", KeyCode::NumpadDivide),
    ("NUM.", KeyCode::NumpadDecimal),
    ("NUMENTER", KeyCode::NumpadEnter),
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("F13", KeyCode::F13),
    ("F14", KeyCode::F14),
    ("F15", KeyCode::F15),
    ("BACKSLASH", KeyCode::Backslash),
    ("CAPS", KeyCode::CapsLock),
    ("LSUPER", KeyCode::SuperLeft),
    ("RSUPER", KeyCode::SuperRight),
    ("CTXMENU", KeyCode::ContextMenu),
    ("INTL\\", KeyCode::IntlBackslash),
];

pub fn key_name(key: KeyCode) -> &'static str {
    KEY_NAMES
        .iter()
        .find(|(_, k)| *k == key)
        .map(|(n, _)| *n)
        .unwrap_or("?")
}

pub fn key_from_name(name: &str) -> Option<KeyCode> {
    KEY_NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, k)| *k)
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
    brake: bool,
    despin: bool,
    hyper: bool,
    /// Smoothed axis values: [thrust xyz, torque xyz].
    axes: [f64; 6],
    bindings: Option<Bindings>,
}

impl InputState {
    pub fn bindings(&self) -> Bindings {
        self.bindings.unwrap_or_default()
    }

    /// Swap the key map. Everything held is released: a key that meant one
    /// thing a moment ago must not keep doing it under a new name.
    pub fn set_bindings(&mut self, bindings: Bindings) {
        self.bindings = Some(bindings);
        self.release_all();
    }

    pub fn set(&mut self, key: KeyCode, pressed: bool) {
        let b = self.bindings();
        if key == b.named(Named::Boost) {
            self.boost = pressed;
        }
        if key == b.named(Named::Brake) {
            self.brake = pressed;
        }
        if key == b.named(Named::Despin) {
            self.despin = pressed;
        }
        if key == b.named(Named::Hyper) {
            self.hyper = pressed;
        }
        if let Some(action) = b.action_for(key) {
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
        self.brake = false;
        self.despin = false;
        self.hyper = false;
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
            brake: self.brake,
            despin: self.despin,
            hyper: self.hyper,
            hyper_level: 0.0,
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
            brake: self.brake,
            despin: self.despin,
            hyper: self.hyper,
            hyper_level: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Out of the box no two controls share a key — in particular the
    /// holo3PP must not sit on H, which is the chaos drive. (It did, for
    /// one commit. The chase view opened on a drive key.)
    #[test]
    fn no_two_default_binds_share_a_key() {
        let b = Bindings::default();
        let mut seen: Vec<(String, KeyCode)> = Vec::new();
        for a in Action::ALL {
            seen.push((format!("{a:?}"), b.key_for(a)));
        }
        for n in Named::ALL {
            seen.push((format!("{n:?}"), b.named(n)));
        }
        for (i, (na, ka)) in seen.iter().enumerate() {
            for (nb, kb) in &seen[i + 1..] {
                assert_ne!(ka, kb, "{na} and {nb} share a default key");
            }
        }
        assert_ne!(
            b.named(Named::Holo),
            b.named(Named::Hyper),
            "the holo3PP stays off the chaos drive's key"
        );
    }

    /// One swap rule for every control: binding a named control to a key
    /// another control holds hands that control the old key back — axis
    /// or named, nothing is forgotten and no key means two things.
    #[test]
    fn rebinding_swaps_uniformly_across_actions_and_named_controls() {
        let mut b = Bindings::default();
        // Chase takes the chaos drive's H: hyper gets chase's old Y.
        let old_chase = b.named(Named::Chase);
        assert!(b.bind_named(Named::Chase, KeyCode::KeyH));
        assert_eq!(b.named(Named::Chase), KeyCode::KeyH);
        assert_eq!(b.named(Named::Hyper), old_chase, "hyper took the old key");
        // A named control takes an axis key: the axis gets the old key.
        let old_map = b.named(Named::Map);
        assert!(b.bind_named(Named::Map, KeyCode::KeyW));
        assert_eq!(b.named(Named::Map), KeyCode::KeyW);
        assert_eq!(b.key_for(Action::ThrustForward), old_map);
        // An axis takes a named key: the named control gets the old key.
        let old_thrust = b.key_for(Action::ThrustBack);
        assert!(b.bind(Action::ThrustBack, b.named(Named::Holo)));
        assert_eq!(b.named(Named::Holo), old_thrust);
        // Reserved keys stay the game's.
        assert!(!b.bind_named(Named::Holo, KeyCode::Escape));
        // After all that shuffling, still no key means two things.
        let mut all: Vec<KeyCode> = Action::ALL.iter().map(|&a| b.key_for(a)).collect();
        all.extend(Named::ALL.iter().map(|&n| b.named(n)));
        for (i, ka) in all.iter().enumerate() {
            for kb in &all[i + 1..] {
                assert_ne!(ka, kb, "a key means two things after swaps");
            }
        }
    }

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
        let mut s = held(&[KeyCode::KeyW, KeyCode::ShiftLeft, KeyCode::Space]);
        s.release_all();
        let c = s.controls(false);
        assert!(!c.boost && !c.brake);
    }

    #[test]
    fn brake_is_a_modifier_not_an_axis() {
        let s = held(&[KeyCode::KeyW, KeyCode::Space]);
        let c = s.controls(false);
        assert!(c.brake, "space must engage the air brake");
        // The brake decides how fast the ship stops, not which way it points.
        assert_eq!(c.thrust_body, DVec3::NEG_Z);
        assert_eq!(c.torque_body, DVec3::ZERO);
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
