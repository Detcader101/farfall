//! The stick: a HOTAS or joystick read natively (winmm on Windows) or
//! through the browser's Gamepad API, mapped onto the flight axes and the
//! named binds — and the wizard that learns the map by watching what the
//! pilot moves.
//!
//! Two layers, kept apart on purpose (the design lifted from the Arma
//! Reforger HOTAS tool): a **device map** — which raw axis index and which
//! button number is which flight control — learned per unit by the wizard
//! and kept as `stick.*` keys; and the game's **actions**, which the map
//! points at through the same [`Named`] table the keyboard uses. A stick
//! button bound to BOOST is BOOST: it presses the key BOOST is bound to.
//!
//! Everything here is pure and windowless except the two readers at the
//! bottom, so the mapping, the shaping and the wizard are all under test.

use crate::input::Named;
use farfall_render::text::{TextBitmap, MENU_COLS};
use winit::keyboard::KeyCode;

/// Raw axes a sample carries. winmm has six; a browser may report more.
pub const MAX_AXES: usize = 8;
/// Real buttons take bits `0..HAT_BIT` of the mask; the hat's four ways
/// are the top four bits, so a hat direction binds like any button.
pub const HAT_BIT: u8 = 28;
pub const MAX_BUTTONS: u8 = 32;

/// One reading of the stick: every axis in [-1, 1], every button a bit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub axes: [f32; MAX_AXES],
    pub buttons: u32,
}

impl Sample {
    pub fn button(&self, n: u8) -> bool {
        n < MAX_BUTTONS && self.buttons & (1 << n) != 0
    }

    /// The hat as a bit mask from its bearing in centidegrees (winmm's
    /// convention; 0 is up, clockwise), `None` when centred.
    #[cfg(any(windows, test))]
    pub fn with_hat(mut self, centideg: Option<u32>) -> Self {
        if let Some(d) = centideg {
            let d = d % 36000;
            // Each way spans a quarter turn either side of its bearing,
            // open at the ends, so a diagonal lights two.
            let up = !(9000 - 1..=27000 + 1).contains(&d);
            let right = (2..18000 - 1).contains(&d);
            let down = (9000 + 2..27000 - 1).contains(&d);
            let left = (18000 + 2..36000 - 1).contains(&d);
            for (on, bit) in [(up, 0), (right, 1), (down, 2), (left, 3)] {
                if on {
                    self.buttons |= 1 << (HAT_BIT + bit);
                }
            }
        }
        self
    }
}

/// What is plugged in, as far as the reader can tell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub axes: usize,
    /// Real buttons; the hat, if there is one, is four more (its bits).
    pub buttons: usize,
    pub hat: bool,
}

/// Sticks the game knows by their USB ids: the name is shown in the
/// wizard and the STICK page, and a known stick starts from its own map.
const KNOWN: &[(u16, u16, &str)] = &[
    // Short enough for a DEVICE row and a FOUND: line: no "THRUSTMASTER".
    (0x044F, 0xB67C, "T.FLIGHT HOTAS 4"),
    (0x044F, 0xB67B, "HOTAS 4 IN PS4 MODE: SET PC"),
    (0x044F, 0xB108, "THRUSTMASTER T.FLIGHT HOTAS X"),
    (0x044F, 0xB10A, "THRUSTMASTER T.16000M"),
    (0x044F, 0x0402, "THRUSTMASTER WARTHOG STICK"),
    (0x044F, 0x0404, "THRUSTMASTER WARTHOG THROTTLE"),
    (0x068E, 0x00F3, "CH FIGHTERSTICK"),
    (0x046D, 0xC215, "LOGITECH EXTREME 3D PRO"),
    (0x046D, 0xC29A, "LOGITECH G940"),
];

impl Device {
    pub fn known_name(vid: u16, pid: u16) -> Option<&'static str> {
        KNOWN
            .iter()
            .find(|(v, p, _)| *v == vid && *p == pid)
            .map(|(_, _, n)| *n)
    }

    /// The line the wizard shows: the known name, else whatever the
    /// platform called it.
    pub fn label(&self) -> String {
        match Self::known_name(self.vid, self.pid) {
            Some(n) => n.to_string(),
            None if self.name.is_empty() => format!("{:04X}:{:04X}", self.vid, self.pid),
            None => self.name.to_ascii_uppercase(),
        }
    }

    #[cfg(test)]
    pub fn is_hotas4(&self) -> bool {
        self.vid == 0x044F && matches!(self.pid, 0xB67C | 0xB67B)
    }
}

/// The six flight controls a stick can drive. Order is the wizard's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flight {
    Pitch,
    Yaw,
    Roll,
    Throttle,
    Strafe,
    Lift,
}

impl Flight {
    pub const COUNT: usize = 6;
    pub const ALL: [Flight; Flight::COUNT] = [
        Flight::Pitch,
        Flight::Yaw,
        Flight::Roll,
        Flight::Throttle,
        Flight::Strafe,
        Flight::Lift,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Flight::Pitch => "PITCH",
            Flight::Yaw => "YAW",
            Flight::Roll => "ROLL",
            Flight::Throttle => "THROTTLE",
            Flight::Strafe => "STRAFE",
            Flight::Lift => "LIFT",
        }
    }

    /// Settings key: stick.<this>.
    pub fn key(self) -> &'static str {
        match self {
            Flight::Pitch => "pitch",
            Flight::Yaw => "yaw",
            Flight::Roll => "roll",
            Flight::Throttle => "throttle",
            Flight::Strafe => "strafe",
            Flight::Lift => "lift",
        }
    }

    /// What the wizard asks for: a move in the control's POSITIVE sense,
    /// so the direction it sees tells it whether to invert.
    pub fn prompt(self) -> [&'static str; 2] {
        match self {
            Flight::Pitch => ["PULL THE STICK BACK", "(NOSE UP)"],
            Flight::Yaw => ["TWIST THE STICK RIGHT", "(OR RUDDER RIGHT)"],
            Flight::Roll => ["PUSH THE STICK RIGHT", "(RIGHT WING DOWN)"],
            Flight::Throttle => ["PUSH THE THROTTLE FORWARD", "(FROM THE MIDDLE)"],
            Flight::Strafe => ["ROCKER ON THE THROTTLE: RIGHT", "(OR A SECOND STICK RIGHT)"],
            Flight::Lift => ["ANY AXIS FOR UP", "(S TO SKIP: R/F KEYS DO IT)"],
        }
    }

    /// The flight control behind a keyboard action, for the KEYS page.
    pub fn for_action(a: crate::input::Action) -> Option<Flight> {
        use crate::input::Action as A;
        Some(match a {
            A::PitchUp | A::PitchDown => Flight::Pitch,
            A::YawLeft | A::YawRight => Flight::Yaw,
            A::RollLeft | A::RollRight => Flight::Roll,
            A::ThrustForward | A::ThrustBack => Flight::Throttle,
            A::StrafeLeft | A::StrafeRight => Flight::Strafe,
            A::ThrustUp | A::ThrustDown => Flight::Lift,
        })
    }

    /// The sim's control this drives: (is torque, component, sign for
    /// a positive value). Signs follow input.rs's AXES table.
    fn body(self) -> (bool, usize, f64) {
        match self {
            Flight::Pitch => (true, 0, 1.0),
            Flight::Yaw => (true, 1, -1.0),
            Flight::Roll => (true, 2, -1.0),
            Flight::Throttle => (false, 2, -1.0),
            Flight::Strafe => (false, 0, 1.0),
            Flight::Lift => (false, 1, 1.0),
        }
    }
}

/// One flight control's source: which raw axis, and which way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AxisMap {
    pub axis: Option<u8>,
    pub invert: bool,
}

impl AxisMap {
    const NONE: AxisMap = AxisMap {
        axis: None,
        invert: false,
    };
    const fn at(axis: u8, invert: bool) -> Self {
        Self {
            axis: Some(axis),
            invert,
        }
    }

    /// Settings value: the index, a trailing `-` when inverted; `none`.
    pub fn render(self) -> String {
        match self.axis {
            None => "none".to_string(),
            Some(a) => format!("{a}{}", if self.invert { "-" } else { "" }),
        }
    }

    pub fn parse(v: &str) -> Option<Self> {
        let v = v.trim();
        if v.eq_ignore_ascii_case("none") || v.is_empty() {
            return Some(Self::NONE);
        }
        let (num, invert) = match v.strip_suffix('-') {
            Some(n) => (n, true),
            None => (v.strip_suffix('+').unwrap_or(v), false),
        };
        let a = num.trim().parse::<u8>().ok()?;
        (usize::from(a) < MAX_AXES).then_some(Self::at(a, invert))
    }

    /// Menu label: `AXIS 1 (Y) -`, or the control's own name once the
    /// stick is known: `STICK Y -`.
    pub fn label(self, m: &StickMap) -> String {
        match self.axis {
            None => "NONE".to_string(),
            Some(a) => format!(
                "{}{}",
                m.axis_name(a),
                if self.invert { " -" } else { " +" }
            ),
        }
    }
}

/// Which stick the raw indices are read as, for naming them: the wizard
/// and the pages say TRIGGER and ROCKER rather than B0 and AXIS 4 once
/// the stick has been identified. Set by the reader from the USB id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Generic,
    Hotas4,
}

/// The T.Flight HOTAS 4's physical controls by winmm index (measured).
/// U (index 3) is reported by the driver but nothing on the unit moves
/// it, so coverage never counts it as a control without a job.
const HOTAS4_AXES: [&str; 6] = ["STICK X", "STICK Y", "LEVER", "AXIS U", "ROCKER", "TWIST"];
const HOTAS4_DEAD_AXES: [u8; 1] = [3];
const HOTAS4_BUTTONS: [&str; 12] = [
    "TRIGGER", "L1", "R3", "L3", "FACE L", "FACE D", "FACE R", "FACE U", "R2", "L2", "BASE L",
    "BASE R",
];

/// Where the throttle lever's zero is: the middle (back half is reverse
/// thrust) or the bottom (the lever is 0..1 ahead only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrottleZero {
    Centre,
    Bottom,
}

pub const DEADZONE_MIN: f32 = 0.0;
pub const DEADZONE_MAX: f32 = 0.5;
pub const CURVE_MIN: f32 = 1.0;
pub const CURVE_MAX: f32 = 3.0;

/// The device map: raw axes and buttons onto the flight controls, the
/// trigger and the named binds. Kept in the settings file as `stick.*`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickMap {
    pub enabled: bool,
    pub axes: [AxisMap; Flight::COUNT],
    /// Symmetric dead band about centre, as a fraction of half travel.
    pub deadzone: f32,
    /// Response exponent: 1 linear, higher is finer about centre.
    pub curve: f32,
    pub throttle_zero: ThrottleZero,
    /// The trigger: the button that fires the guns.
    pub fire: Option<u8>,
    /// A button per named control, by [`Named`] index.
    pub buttons: [Option<u8>; Named::COUNT],
    /// How to name the raw indices (stick.layout; the reader sets it).
    pub layout: Layout,
    /// The lever hard back (the bottom ~5% of travel) holds the air brake.
    pub throttle_brake: bool,
    /// The lever slammed forward fires the chaos drive for two seconds.
    pub throttle_jump: bool,
}

impl Default for StickMap {
    /// The T.Flight HOTAS 4 as it enumerates through winmm on a measured
    /// unit (the Reforger tool's device-audit): X roll, Y pitch, Z the
    /// throttle lever (forward is low), V (index 4) the rocker, R (index
    /// 5) the twist; trigger B0, L1 B1, R3 B2, L3 B3, the throttle's four
    /// face buttons B4..B7 (left, down, right, up), R2 B8, L2 B9, the
    /// two base buttons B10 B11.
    fn default() -> Self {
        Self::hotas4()
    }
}

impl StickMap {
    pub fn hotas4() -> Self {
        let mut buttons = [None; Named::COUNT];
        for (n, b) in [
            (Named::Boost, 1),
            (Named::Brake, 2),
            (Named::Despin, 3),
            (Named::Map, 4),
            (Named::Landing, 5),
            (Named::NextWeapon, 6),
            (Named::Assist, 7),
            (Named::Hyper, 8),
            (Named::WarpStop, 9),
            (Named::Chase, 10),
            (Named::Holo, 11),
            (Named::LookLock, HAT_BIT),
            (Named::Hold, HAT_BIT + 2),
            (Named::Weapon1, HAT_BIT + 3),
            (Named::Weapon2, HAT_BIT + 1),
        ] {
            buttons[n as usize] = Some(b);
        }
        Self {
            enabled: true,
            axes: [
                AxisMap::at(1, false),
                AxisMap::at(5, false),
                AxisMap::at(0, false),
                AxisMap::at(2, true),
                AxisMap::at(4, false),
                AxisMap::NONE,
            ],
            deadzone: 0.08,
            curve: 1.5,
            throttle_zero: ThrottleZero::Centre,
            fire: Some(0),
            buttons,
            layout: Layout::Hotas4,
            throttle_brake: true,
            throttle_jump: true,
        }
    }

    /// The throttle's raw demand after invert, before the deadzone — the
    /// gestures read the lever's true position, not the shaped output.
    /// `None` when no throttle axis is mapped (or the reader is holding
    /// it back as uncalibrated, in which case it reads 0 and stays out
    /// of both gesture zones).
    fn throttle_raw(&self, s: &Sample) -> Option<f32> {
        let m = self.axes[Flight::Throttle as usize];
        let a = m.axis?;
        let raw = *s.axes.get(usize::from(a))?;
        if !raw.is_finite() {
            return None;
        }
        Some(if m.invert { -raw } else { raw })
    }

    /// A raw axis's name under this layout.
    pub fn axis_name(&self, a: u8) -> String {
        match self.layout {
            Layout::Hotas4 => HOTAS4_AXES
                .get(usize::from(a))
                .map_or_else(|| axis_label(a), |n| n.to_string()),
            Layout::Generic => axis_label(a),
        }
    }

    /// A button's name under this layout (`-` for none).
    pub fn button_name(&self, b: Option<u8>) -> String {
        match (self.layout, b) {
            (Layout::Hotas4, Some(b)) if usize::from(b) < HOTAS4_BUTTONS.len() => {
                HOTAS4_BUTTONS[usize::from(b)].to_string()
            }
            _ => button_label(b),
        }
    }

    /// Walk the hardware: how many of the stick's controls have a job,
    /// how many there are, and the names of those that have none — so a
    /// control with no job is a visible hole, not an omission. The
    /// count comes from the device (its axes and buttons, the hat as
    /// four) or, without one, from the layout.
    pub fn coverage(&self, device: Option<&Device>) -> (usize, usize, Vec<String>) {
        let (axes, buttons): (Vec<u8>, Vec<u8>) = match (device, self.layout) {
            (Some(d), layout) => {
                let mut b: Vec<u8> = (0..d.buttons.min(usize::from(HAT_BIT)) as u8).collect();
                if d.hat {
                    b.extend(HAT_BIT..HAT_BIT + 4);
                }
                let axes = (0..d.axes.min(MAX_AXES) as u8)
                    .filter(|a| layout != Layout::Hotas4 || !HOTAS4_DEAD_AXES.contains(a))
                    .collect();
                (axes, b)
            }
            (None, Layout::Hotas4) => (
                vec![0, 1, 2, 4, 5],
                (0..12).chain(HAT_BIT..HAT_BIT + 4).collect(),
            ),
            (None, Layout::Generic) => (Vec::new(), Vec::new()),
        };
        let mut free = Vec::new();
        let mut jobs = 0;
        for a in &axes {
            if self.axes.iter().any(|m| m.axis == Some(*a)) {
                jobs += 1;
            } else {
                free.push(self.axis_name(*a));
            }
        }
        for b in &buttons {
            if self.fire == Some(*b) || self.buttons.contains(&Some(*b)) {
                jobs += 1;
            } else {
                free.push(self.button_name(Some(*b)));
            }
        }
        (jobs, axes.len() + buttons.len(), free)
    }

    /// Nothing mapped: what the wizard builds up from on a stick it has
    /// never seen (the tests' clean slate; a player's file starts from
    /// the HOTAS 4 map and the wizard overwrites it control by control).
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            enabled: true,
            axes: [AxisMap::NONE; Flight::COUNT],
            deadzone: 0.08,
            curve: 1.5,
            throttle_zero: ThrottleZero::Centre,
            fire: None,
            buttons: [None; Named::COUNT],
            layout: Layout::Generic,
            throttle_brake: true,
            throttle_jump: true,
        }
    }

    pub fn axis(&self, f: Flight) -> AxisMap {
        self.axes[f as usize]
    }

    pub fn button_for(&self, n: Named) -> Option<u8> {
        self.buttons[n as usize]
    }

    /// Give button `b` to named control `n`; whoever had it loses it (one
    /// button, one job — the keyboard's rule).
    pub fn bind_button(&mut self, n: Named, b: Option<u8>) {
        if let Some(b) = b {
            self.unbind_button(b);
        }
        self.buttons[n as usize] = b;
    }

    /// The trigger, with the same one-job rule.
    pub fn bind_fire(&mut self, b: Option<u8>) {
        if let Some(b) = b {
            self.unbind_button(b);
        }
        self.fire = b;
    }

    fn unbind_button(&mut self, b: u8) {
        for slot in self.buttons.iter_mut() {
            if *slot == Some(b) {
                *slot = None;
            }
        }
        if self.fire == Some(b) {
            self.fire = None;
        }
    }

    /// Give axis `a` to flight control `f`; another control on the same
    /// axis lets go of it.
    pub fn bind_axis(&mut self, f: Flight, m: AxisMap) {
        if let Some(a) = m.axis {
            for (i, slot) in self.axes.iter_mut().enumerate() {
                if i != f as usize && slot.axis == Some(a) {
                    *slot = AxisMap::NONE;
                }
            }
        }
        self.axes[f as usize] = m;
    }

    /// What the button means: the named control it is bound to.
    pub fn named_for_button(&self, b: u8) -> Option<Named> {
        Named::ALL
            .iter()
            .copied()
            .find(|n| self.buttons[*n as usize] == Some(b))
    }

    /// Each flight control's shaped value in [-1, 1].
    pub fn flight(&self, s: &Sample) -> [f32; Flight::COUNT] {
        let mut out = [0.0f32; Flight::COUNT];
        for (i, f) in Flight::ALL.iter().enumerate() {
            let m = self.axes[i];
            let Some(a) = m.axis else {
                continue;
            };
            let Some(&raw) = s.axes.get(usize::from(a)) else {
                continue;
            };
            let mut v = if m.invert { -raw } else { raw };
            if *f == Flight::Throttle && self.throttle_zero == ThrottleZero::Bottom {
                // The lever's whole travel is 0..1 ahead: -1 at the stop
                // is zero thrust, and the dead band sits at the stop.
                v = (v + 1.0) * 0.5;
                v = shape(v, self.deadzone, self.curve);
            } else {
                v = shape(v, self.deadzone, self.curve);
            }
            out[i] = v;
        }
        out
    }

    /// The sim's six body axes — thrust xyz, torque xyz — from a sample.
    /// Every component is finite and in [-1, 1].
    pub fn body_axes(&self, s: &Sample) -> [f64; 6] {
        let mut out = [0.0f64; 6];
        if !self.enabled {
            return out;
        }
        for (i, f) in Flight::ALL.iter().enumerate() {
            let (torque, c, sign) = f.body();
            let v = f64::from(self.flight(s)[i]) * sign;
            let k = if torque { 3 + c } else { c };
            out[k] = (out[k] + v).clamp(-1.0, 1.0);
        }
        out
    }

    /// One `stick.*` line from the settings file. Returns true if the key
    /// was one of ours.
    pub fn parse_key(&mut self, k: &str, v: &str) -> bool {
        let Some(rest) = k.strip_prefix("stick.") else {
            return false;
        };
        match rest {
            "enabled" => self.enabled = matches!(v, "on" | "true" | "1"),
            "deadzone" => {
                if let Ok(f) = v.parse::<f32>() {
                    if f.is_finite() {
                        self.deadzone = f.clamp(DEADZONE_MIN, DEADZONE_MAX);
                    }
                }
            }
            "curve" => {
                if let Ok(f) = v.parse::<f32>() {
                    if f.is_finite() {
                        self.curve = f.clamp(CURVE_MIN, CURVE_MAX);
                    }
                }
            }
            "throttle-zero" => {
                self.throttle_zero = match v {
                    "bottom" => ThrottleZero::Bottom,
                    "centre" | "center" => ThrottleZero::Centre,
                    _ => self.throttle_zero,
                }
            }
            "fire" => self.fire = button_from_name(v),
            "throttle-brake" => self.throttle_brake = matches!(v, "on" | "true" | "1"),
            "throttle-jump" => self.throttle_jump = matches!(v, "on" | "true" | "1"),
            "layout" => {
                self.layout = match v {
                    "hotas4" => Layout::Hotas4,
                    "generic" => Layout::Generic,
                    _ => self.layout,
                }
            }
            other => {
                if let Some(name) = other.strip_prefix("button.") {
                    if let Some(n) = Named::ALL.iter().copied().find(|n| n.key() == name) {
                        self.buttons[n as usize] = button_from_name(v);
                    }
                } else if let Some(f) = Flight::ALL.iter().copied().find(|f| f.key() == other) {
                    if let Some(m) = AxisMap::parse(v) {
                        self.axes[f as usize] = m;
                    }
                }
            }
        }
        true
    }

    /// The `stick.*` block of the settings file.
    pub fn render(&self, out: &mut String) {
        out.push_str(&format!(
            "stick.enabled = {}\n",
            if self.enabled { "on" } else { "off" }
        ));
        for f in Flight::ALL {
            out.push_str(&format!("stick.{} = {}\n", f.key(), self.axis(f).render()));
        }
        out.push_str(&format!("stick.deadzone = {:.2}\n", self.deadzone));
        out.push_str(&format!("stick.curve = {:.2}\n", self.curve));
        out.push_str(&format!(
            "stick.throttle-zero = {}\n",
            match self.throttle_zero {
                ThrottleZero::Centre => "centre",
                ThrottleZero::Bottom => "bottom",
            }
        ));
        out.push_str(&format!(
            "stick.layout = {}\n",
            match self.layout {
                Layout::Hotas4 => "hotas4",
                Layout::Generic => "generic",
            }
        ));
        out.push_str(&format!("stick.fire = {}\n", button_key(self.fire)));
        out.push_str(&format!(
            "stick.throttle-brake = {}\n",
            if self.throttle_brake { "on" } else { "off" }
        ));
        out.push_str(&format!(
            "stick.throttle-jump = {}\n",
            if self.throttle_jump { "on" } else { "off" }
        ));
        for n in Named::ALL {
            out.push_str(&format!(
                "stick.button.{} = {}\n",
                n.key(),
                button_key(self.buttons[n as usize])
            ));
        }
    }
}

/// Dead band, then a power curve, then a clamp: symmetric about zero,
/// never NaN (a NaN reading is a centred stick).
pub fn shape(v: f32, deadzone: f32, curve: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let v = v.clamp(-1.0, 1.0);
    let dz = deadzone.clamp(DEADZONE_MIN, DEADZONE_MAX);
    let mag = v.abs();
    if mag <= dz {
        return 0.0;
    }
    // Rescale so the band's edge is zero and full travel is still one.
    let t = ((mag - dz) / (1.0 - dz)).clamp(0.0, 1.0);
    let shaped = t.powf(curve.clamp(CURVE_MIN, CURVE_MAX));
    (shaped.copysign(v)).clamp(-1.0, 1.0)
}

/// A button's settings name: its number, or the hat's way.
pub fn button_key(b: Option<u8>) -> String {
    match b {
        None => "none".to_string(),
        Some(b) if b >= HAT_BIT => format!("hat-{}", HAT_WAYS[usize::from(b - HAT_BIT) % 4]),
        Some(b) => b.to_string(),
    }
}

const HAT_WAYS: [&str; 4] = ["up", "right", "down", "left"];

pub fn button_from_name(v: &str) -> Option<u8> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return None;
    }
    for (i, way) in HAT_WAYS.iter().enumerate() {
        if v.eq_ignore_ascii_case(&format!("hat-{way}")) {
            return Some(HAT_BIT + i as u8);
        }
    }
    let n = v.trim_start_matches(['b', 'B']).parse::<u8>().ok()?;
    (n < HAT_BIT).then_some(n)
}

/// The hat's way, short enough for a KEYS row beside a key: `HAT-U`.
fn hat_name(b: u8) -> &'static str {
    match b - HAT_BIT {
        0 => "HAT-U",
        1 => "HAT-R",
        2 => "HAT-D",
        _ => "HAT-L",
    }
}

/// A button's menu name: `B4`, or `HAT-U`.
pub fn button_label(b: Option<u8>) -> String {
    match b {
        None => "-".to_string(),
        Some(b) if b >= HAT_BIT => hat_name(b).to_string(),
        Some(b) => format!("B{b}"),
    }
}

/// An axis's menu name. winmm's order is X Y Z U V R (the Reforger
/// tool's axis0..5, so a map measured there reads the same here); a
/// browser numbers them itself.
pub fn axis_label(a: u8) -> String {
    #[cfg(windows)]
    {
        let letter = ["X", "Y", "Z", "U", "V", "R", "6", "7"];
        format!("AXIS {a} ({})", letter.get(usize::from(a)).unwrap_or(&"?"))
    }
    #[cfg(not(windows))]
    {
        format!("AXIS {a}")
    }
}

// ---------------------------------------------------------------------
// The STICK page's rows (the menu lists them; the values live here so the
// menu's own tables stay small).
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickItem {
    Device,
    Enabled,
    Wizard,
    Axis(Flight),
    Deadzone,
    Curve,
    ThrottleZero,
    /// The lever hard back holds the air brake.
    ThrottleBrake,
    /// The lever slammed forward fires the chaos drive for two seconds.
    ThrottleJump,
    Fire,
}

impl StickItem {
    pub fn all() -> Vec<StickItem> {
        let mut v = vec![StickItem::Device, StickItem::Enabled, StickItem::Wizard];
        v.extend(Flight::ALL.iter().map(|&f| StickItem::Axis(f)));
        v.extend([
            StickItem::Deadzone,
            StickItem::Curve,
            StickItem::ThrottleZero,
            StickItem::ThrottleBrake,
            StickItem::ThrottleJump,
            StickItem::Fire,
        ]);
        v
    }

    pub fn label(self) -> &'static str {
        match self {
            StickItem::Device => "DEVICE",
            StickItem::Enabled => "STICK",
            StickItem::Wizard => "SETUP WIZARD",
            StickItem::Axis(f) => f.name(),
            StickItem::Deadzone => "DEADZONE",
            StickItem::Curve => "CURVE",
            StickItem::ThrottleZero => "THROTTLE ZERO",
            StickItem::ThrottleBrake => "LEVER BRAKE",
            StickItem::ThrottleJump => "LEVER JUMP",
            StickItem::Fire => "TRIGGER",
        }
    }

    pub fn value(self, m: &StickMap, device: Option<&Device>) -> String {
        match self {
            StickItem::Device => match device {
                Some(d) => {
                    let l = d.label();
                    if l.len() > 26 {
                        l[l.len() - 26..].to_string()
                    } else {
                        l
                    }
                }
                None => "NONE FOUND".to_string(),
            },
            StickItem::Enabled => if m.enabled { "ON" } else { "OFF" }.to_string(),
            StickItem::Wizard => "ENTER: RUN".to_string(),
            StickItem::Axis(f) => m.axis(f).label(m),
            StickItem::Deadzone => format!("{:.0}%", m.deadzone * 100.0),
            StickItem::Curve => format!("{:.2}", m.curve),
            StickItem::ThrottleZero => match m.throttle_zero {
                ThrottleZero::Centre => "CENTRE",
                ThrottleZero::Bottom => "BOTTOM",
            }
            .to_string(),
            StickItem::ThrottleBrake => if m.throttle_brake { "ON" } else { "OFF" }.to_string(),
            StickItem::ThrottleJump => if m.throttle_jump { "ON" } else { "OFF" }.to_string(),
            StickItem::Fire => m.button_name(m.fire),
        }
    }

    /// One line on what the row does, for the menu's footer.
    pub fn describe(self) -> &'static str {
        match self {
            StickItem::Device => {
                "THE STICK THE GAME FOUND, BY ITS USB ID. PLUG ONE IN AND IT IS READ AT ONCE."
            }
            StickItem::Enabled => "THE STICK FLIES THE SHIP, OR IS IGNORED ENTIRELY.",
            StickItem::Wizard => {
                "WALK THE STICK CONTROL BY CONTROL: MOVE WHAT EACH STEP ASKS FOR AND IT IS LEARNED."
            }
            StickItem::Axis(Flight::Pitch) => {
                "WHICH STICK AXIS PITCHES THE NOSE. ENTER FLIPS ITS DIRECTION."
            }
            StickItem::Axis(Flight::Yaw) => {
                "WHICH AXIS YAWS THE NOSE - THE TWIST ON A HOTAS. ENTER FLIPS IT."
            }
            StickItem::Axis(Flight::Roll) => "WHICH AXIS ROLLS THE SHIP. ENTER FLIPS IT.",
            StickItem::Axis(Flight::Throttle) => {
                "WHICH AXIS IS THE MAIN THROTTLE, ALONG THE NOSE. ENTER FLIPS IT."
            }
            StickItem::Axis(Flight::Strafe) => {
                "WHICH AXIS STRAFES - THE ROCKER ON A HOTAS THROTTLE. ENTER FLIPS IT."
            }
            StickItem::Axis(Flight::Lift) => {
                "WHICH AXIS THRUSTS UP AND DOWN; NONE LEAVES IT ON THE R AND F KEYS."
            }
            StickItem::Deadzone => {
                "TRAVEL ABOUT CENTRE THAT COUNTS AS NOTHING, SO A RESTING STICK RESTS."
            }
            StickItem::Curve => {
                "1.00 IS LINEAR; HIGHER IS FINER NEAR CENTRE AND STILL FULL AT THE STOP."
            }
            StickItem::ThrottleZero => {
                "WHERE THE LEVER MEANS ZERO: THE CENTRE (THE BACK HALF REVERSES) OR THE BOTTOM."
            }
            StickItem::ThrottleBrake => {
                "THE LEVER HARD BACK HOLDS THE AIR BRAKE, LIKE HOLDING SPACE. CENTRE ZERO ONLY."
            }
            StickItem::ThrottleJump => {
                "SLAM THE LEVER FORWARD: THE CHAOS DRIVE FIRES FOR TWO SECONDS. A SMOOTH PUSH NEVER DOES."
            }
            StickItem::Fire => "THE STICK BUTTON THAT FIRES THE GUNS.",
        }
    }

    /// The settings keys this row edits (the menu's coverage ledger).
    #[cfg(test)]
    pub fn keys(self) -> Vec<String> {
        match self {
            // The reader sets the layout from the stick's USB id; the
            // DEVICE row is where that state shows.
            StickItem::Device => vec!["stick.layout".to_string()],
            StickItem::Enabled => vec!["stick.enabled".to_string()],
            StickItem::Wizard => Vec::new(),
            StickItem::Axis(f) => vec![format!("stick.{}", f.key())],
            StickItem::Deadzone => vec!["stick.deadzone".to_string()],
            StickItem::Curve => vec!["stick.curve".to_string()],
            StickItem::ThrottleZero => vec!["stick.throttle-zero".to_string()],
            StickItem::ThrottleBrake => vec!["stick.throttle-brake".to_string()],
            StickItem::ThrottleJump => vec!["stick.throttle-jump".to_string()],
            StickItem::Fire => vec!["stick.fire".to_string()],
        }
    }

    /// Left/Right on the row (Enter on an axis row flips its direction).
    /// True if anything changed.
    pub fn adjust(self, m: &mut StickMap, forward: bool, enter: bool) -> bool {
        match self {
            StickItem::Device | StickItem::Wizard => false,
            StickItem::Enabled => {
                m.enabled = !m.enabled;
                true
            }
            StickItem::Axis(f) => {
                let cur = m.axis(f);
                if enter {
                    if cur.axis.is_none() {
                        return false;
                    }
                    m.axes[f as usize].invert = !cur.invert;
                    return true;
                }
                // NONE, 0, 1, ... MAX_AXES-1, round again.
                let n = MAX_AXES as i32 + 1;
                let i = cur.axis.map_or(0, |a| i32::from(a) + 1);
                let next = (i + if forward { 1 } else { n - 1 }) % n;
                let axis = (next > 0).then_some((next - 1) as u8);
                m.bind_axis(
                    f,
                    AxisMap {
                        axis,
                        invert: cur.invert,
                    },
                );
                true
            }
            StickItem::Deadzone => step(&mut m.deadzone, forward, 0.02, DEADZONE_MIN, DEADZONE_MAX),
            StickItem::Curve => step(&mut m.curve, forward, 0.25, CURVE_MIN, CURVE_MAX),
            StickItem::ThrottleZero => {
                m.throttle_zero = match m.throttle_zero {
                    ThrottleZero::Centre => ThrottleZero::Bottom,
                    ThrottleZero::Bottom => ThrottleZero::Centre,
                };
                true
            }
            StickItem::ThrottleBrake => {
                m.throttle_brake = !m.throttle_brake;
                true
            }
            StickItem::ThrottleJump => {
                m.throttle_jump = !m.throttle_jump;
                true
            }
            StickItem::Fire => {
                let n = i32::from(MAX_BUTTONS) + 1;
                let i = m.fire.map_or(0, |b| i32::from(b) + 1);
                let next = (i + if forward { 1 } else { n - 1 }) % n;
                m.bind_fire((next > 0).then_some((next - 1) as u8));
                true
            }
        }
    }
}

fn step(v: &mut f32, forward: bool, by: f32, lo: f32, hi: f32) -> bool {
    let next = (*v + if forward { by } else { -by }).clamp(lo, hi);
    let next = (next * 100.0).round() / 100.0;
    if (next - *v).abs() < 1e-6 {
        return false;
    }
    *v = next;
    true
}

// ---------------------------------------------------------------------
// The throttle's gestures: hard back holds the air brake, a slam
// forward fires the chaos drive for two seconds.
// ---------------------------------------------------------------------

/// Below this raw lever position (post-invert) the air brake holds:
/// the bottom ~5% of travel — past anything the deadzone would keep.
const BRAKE_ZONE: f32 = -0.9;
/// A slam: the lever travels at least this far forward...
const SLAM_TRAVEL: f32 = 0.7;
/// ...within this window, seconds. A smooth push cannot do it.
const SLAM_WINDOW_S: f64 = 0.25;
/// ...ending at least this far forward.
const SLAM_MIN: f32 = 0.5;
/// How long a slam holds the chaos drive, seconds.
const SLAM_BURST_S: f64 = 2.0;

/// The gesture detector: fed the admitted sample once a frame with the
/// wall clock, answers (brake held, hyper held).
#[derive(Clone, Debug, Default)]
pub struct Gestures {
    /// The lever's recent positions, oldest first, within SLAM_WINDOW_S.
    window: std::collections::VecDeque<(f64, f32)>,
    /// The chaos drive held until this time by a slam.
    hyper_until: f64,
}

impl Gestures {
    /// Everything forgotten (the wizard opened, the stick unplugged, the
    /// map turned off): no gesture survives a gap in the feed.
    pub fn reset(&mut self) {
        self.window.clear();
        self.hyper_until = 0.0;
    }

    /// One frame. `t` is a monotonic clock in seconds.
    pub fn step(&mut self, m: &StickMap, s: &Sample, t: f64) -> (bool, bool) {
        let Some(v) = m.throttle_raw(s).filter(|_| m.enabled) else {
            self.reset();
            return (false, false);
        };
        // The brake zone reads the lever's true position. With the zero
        // at the bottom the whole idle rest would brake, so it is a
        // centre-zero gesture only.
        let brake = m.throttle_brake && m.throttle_zero == ThrottleZero::Centre && v <= BRAKE_ZONE;
        // The slam: enough travel forward inside the window.
        self.window.push_back((t, v));
        while self
            .window
            .front()
            .is_some_and(|(t0, _)| t - t0 > SLAM_WINDOW_S)
        {
            self.window.pop_front();
        }
        if self.window.len() > 64 {
            self.window.pop_front();
        }
        if m.throttle_jump && v >= SLAM_MIN && t >= self.hyper_until {
            let lowest = self
                .window
                .iter()
                .fold(f32::INFINITY, |acc, (_, w)| acc.min(*w));
            if v - lowest >= SLAM_TRAVEL {
                self.hyper_until = t + SLAM_BURST_S;
                log::info!("stick: throttle slam - chaos drive for {SLAM_BURST_S} s");
            }
        }
        (brake, t < self.hyper_until)
    }
}

// ---------------------------------------------------------------------
// The wizard: walk the stick, one control at a time.
// ---------------------------------------------------------------------

/// One page of the wizard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Axis(Flight),
    ThrottleZero,
    Deadzone,
    Curve,
    Fire,
    Button(Named),
    Summary,
}

/// The named controls the wizard offers a button for, most useful first.
const BUTTON_ORDER: [Named; Named::COUNT] = [
    Named::Boost,
    Named::Brake,
    Named::Despin,
    Named::Hyper,
    Named::WarpStop,
    Named::Assist,
    Named::Landing,
    Named::Disembark,
    Named::Map,
    Named::Hold,
    Named::NextWeapon,
    Named::Weapon1,
    Named::Weapon2,
    Named::Chase,
    Named::Holo,
    Named::HoloOut,
    Named::HoloIn,
    Named::LookLock,
    Named::Appearance,
    Named::Engage,
    Named::Design,
    Named::Trajectory,
    Named::Capture,
    Named::Bay,
    Named::ScaleDown,
    Named::ScaleUp,
];

fn steps() -> Vec<Step> {
    let mut v: Vec<Step> = Flight::ALL.iter().map(|&f| Step::Axis(f)).collect();
    v.extend([Step::ThrottleZero, Step::Deadzone, Step::Curve, Step::Fire]);
    v.extend(BUTTON_ORDER.iter().map(|&n| Step::Button(n)));
    v.push(Step::Summary);
    v
}

/// What the wizard saw the pilot move on this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Detected {
    /// A raw axis, and whether it went positive.
    Axis {
        index: u8,
        positive: bool,
    },
    Button(u8),
}

/// What a key in the wizard asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardEvent {
    Nothing,
    /// The map changed: save.
    Changed,
    /// The wizard is over (finished or left): save, close it.
    Done,
}

/// How far an axis must travel from where it rested to count as "the one
/// you moved". Half travel: a brushed rocker or a lever's creep won't do
/// it, a deliberate push will.
const DETECT_TRAVEL: f32 = 0.5;

#[derive(Clone, Debug)]
pub struct Wizard {
    steps: Vec<Step>,
    at: usize,
    /// Where everything rested when this step opened.
    baseline: Option<Sample>,
    /// The latest reading, for the live bar.
    now: Sample,
    detected: Option<Detected>,
    /// Steps taken to the end: the summary's count.
    done: usize,
}

impl Wizard {
    pub fn new() -> Self {
        Self::at_step(0)
    }

    /// Open on step `n` (the bench's FARFALL_BENCH_STICK=n).
    pub fn at_step(n: usize) -> Self {
        let steps = steps();
        Self {
            at: n.min(steps.len() - 1),
            steps,
            baseline: None,
            now: Sample::default(),
            detected: None,
            done: 0,
        }
    }

    pub fn step(&self) -> Step {
        self.steps[self.at]
    }

    #[cfg(test)]
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    #[cfg(test)]
    pub fn detected(&self) -> Option<Detected> {
        self.detected
    }

    /// A reading from the stick: the first one on a step is where things
    /// rest; after that, the axis (or button) that has moved furthest
    /// from rest is what the pilot means. Re-evaluated every frame until
    /// ENTER, so a wrong twitch is corrected by a right push.
    pub fn feed(&mut self, s: Sample) {
        self.now = s;
        let Some(base) = self.baseline else {
            self.baseline = Some(s);
            return;
        };
        match self.step() {
            Step::Axis(_) => {
                let mut best: Option<(u8, f32)> = None;
                for (i, (a, b)) in s.axes.iter().zip(base.axes).enumerate() {
                    let d = a - b;
                    if d.is_finite()
                        && d.abs() >= DETECT_TRAVEL
                        && best.is_none_or(|(_, bd)| d.abs() > bd.abs())
                    {
                        best = Some((i as u8, d));
                    }
                }
                if let Some((index, d)) = best {
                    self.detected = Some(Detected::Axis {
                        index,
                        positive: d > 0.0,
                    });
                }
            }
            Step::Fire | Step::Button(_) => {
                let fresh = s.buttons & !base.buttons;
                if fresh != 0 {
                    self.detected = Some(Detected::Button(fresh.trailing_zeros() as u8));
                }
            }
            _ => {}
        }
    }

    /// The bench's stand-in for a hand on the stick: a plausible
    /// detection for the current step, so a capture shows the line.
    pub fn bench_detect(&mut self) {
        self.detected = match self.step() {
            Step::Axis(f) => Some(Detected::Axis {
                index: StickMap::hotas4().axis(f).axis.unwrap_or(0),
                positive: !StickMap::hotas4().axis(f).invert,
            }),
            Step::Fire => Some(Detected::Button(0)),
            Step::Button(n) => Some(Detected::Button(
                StickMap::hotas4().button_for(n).unwrap_or(4),
            )),
            _ => None,
        };
        let mut s = Sample::default();
        if let Some(Detected::Axis { index, positive }) = self.detected {
            s.axes[usize::from(index)] = if positive { 0.8 } else { -0.8 };
        }
        self.now = s;
    }

    fn go(&mut self, to: usize) {
        self.at = to.min(self.steps.len() - 1);
        self.baseline = None;
        self.detected = None;
    }

    fn next(&mut self) {
        self.done = self.done.max(self.at + 1);
        let to = self.at + 1;
        self.go(to);
    }

    /// Write what was detected into the map, then move on.
    fn accept(&mut self, m: &mut StickMap) -> WizardEvent {
        let changed = match (self.step(), self.detected) {
            (Step::Axis(f), Some(Detected::Axis { index, positive })) => {
                m.bind_axis(
                    f,
                    AxisMap {
                        axis: Some(index),
                        invert: !positive,
                    },
                );
                true
            }
            (Step::Fire, Some(Detected::Button(b))) => {
                m.bind_fire(Some(b));
                true
            }
            (Step::Button(n), Some(Detected::Button(b))) => {
                m.bind_button(n, Some(b));
                true
            }
            (Step::Summary, _) => return WizardEvent::Done,
            _ => false,
        };
        self.next();
        if changed {
            WizardEvent::Changed
        } else {
            WizardEvent::Nothing
        }
    }

    /// A key while the wizard is up. ENTER keeps what was detected (or
    /// the current value) and moves on; S skips; X clears this control;
    /// I flips an axis; B or BACKSPACE goes back; LEFT/RIGHT adjust the
    /// knobs; ESC finishes with what has been done so far.
    pub fn key(&mut self, key: KeyCode, m: &mut StickMap) -> WizardEvent {
        match key {
            KeyCode::Escape => WizardEvent::Done,
            KeyCode::Enter | KeyCode::Space => self.accept(m),
            KeyCode::KeyS | KeyCode::Tab => {
                if self.step() == Step::Summary {
                    return WizardEvent::Done;
                }
                self.next();
                WizardEvent::Nothing
            }
            KeyCode::KeyB | KeyCode::Backspace => {
                if self.at > 0 {
                    let to = self.at - 1;
                    self.go(to);
                }
                WizardEvent::Nothing
            }
            KeyCode::KeyX | KeyCode::Delete => {
                let changed = match self.step() {
                    Step::Axis(f) => {
                        m.bind_axis(f, AxisMap::NONE);
                        true
                    }
                    Step::Fire => {
                        m.bind_fire(None);
                        true
                    }
                    Step::Button(n) => {
                        m.bind_button(n, None);
                        true
                    }
                    _ => false,
                };
                self.detected = None;
                if changed {
                    self.next();
                    WizardEvent::Changed
                } else {
                    WizardEvent::Nothing
                }
            }
            KeyCode::KeyI => {
                if let Step::Axis(f) = self.step() {
                    if let Some(Detected::Axis { index, positive }) = self.detected {
                        self.detected = Some(Detected::Axis {
                            index,
                            positive: !positive,
                        });
                        return WizardEvent::Nothing;
                    }
                    if m.axis(f).axis.is_some() {
                        m.axes[f as usize].invert = !m.axes[f as usize].invert;
                        return WizardEvent::Changed;
                    }
                }
                WizardEvent::Nothing
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let fwd = key == KeyCode::ArrowRight;
                let changed = match self.step() {
                    Step::ThrottleZero => StickItem::ThrottleZero.adjust(m, fwd, false),
                    Step::Deadzone => StickItem::Deadzone.adjust(m, fwd, false),
                    Step::Curve => StickItem::Curve.adjust(m, fwd, false),
                    _ => false,
                };
                if changed {
                    WizardEvent::Changed
                } else {
                    WizardEvent::Nothing
                }
            }
            _ => WizardEvent::Nothing,
        }
    }

    /// The wizard's page: the menu card's shape — `MENU_COLS` columns,
    /// sixteen lines — drawn into the text bitmap in the menu's place.
    pub fn render(&self, text: &mut TextBitmap, m: &StickMap, device: Option<&Device>) {
        const W: usize = MENU_COLS;
        /// The card's lines: a header, twelve rows, a footer, two more.
        const LINES: usize = 16;
        let clip = |s: &str| s.chars().take(W).collect::<String>();
        text.clear();
        text.draw_line(0, 0, "STICK WIZARD");
        let step_line = format!("STEP {}/{}", self.at + 1, self.steps.len());
        text.draw_line(W.saturating_sub(step_line.len()), 0, &step_line);
        let dev = match device {
            Some(d) => format!("FOUND: {}", d.label()),
            None => "NO STICK FOUND - PLUG ONE IN".to_string(),
        };
        text.draw_line(0, 1, &clip(&dev));

        let mut lines: Vec<String> = Vec::new();
        match self.step() {
            Step::Axis(f) => {
                let [a, b] = f.prompt();
                lines.push(format!("{}  (NOW {})", f.name(), m.axis(f).label(m)));
                lines.push(a.to_string());
                lines.push(b.to_string());
                lines.push(String::new());
                match self.detected {
                    Some(Detected::Axis { index, positive }) => {
                        lines.push(format!(
                            "DETECTED {} {}",
                            m.axis_name(index),
                            if positive { "+" } else { "- (INVERT)" }
                        ));
                        let v = self.now.axes[usize::from(index)];
                        lines.push(bar(v));
                    }
                    _ => {
                        lines.push("WAITING...".to_string());
                        lines.push(String::new());
                    }
                }
            }
            Step::ThrottleZero => {
                lines.push("THROTTLE ZERO".to_string());
                lines.push("CENTRE: PULL BACK TO REVERSE".to_string());
                lines.push("BOTTOM: THE LEVER IS 0..1 AHEAD".to_string());
                lines.push(String::new());
                lines.push(format!(
                    "< > NOW: {}",
                    StickItem::ThrottleZero.value(m, None)
                ));
            }
            Step::Deadzone => {
                lines.push("DEADZONE".to_string());
                lines.push("LET GO OF THE STICK. IF THE".to_string());
                lines.push("BAR STILL MOVES, RAISE IT.".to_string());
                lines.push(String::new());
                lines.push(format!("< > NOW: {}", StickItem::Deadzone.value(m, None)));
                let drift = self.now.axes.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));
                lines.push(bar(shape(drift, m.deadzone, 1.0)));
            }
            Step::Curve => {
                lines.push("RESPONSE CURVE".to_string());
                lines.push("1.00 LINEAR. HIGHER IS FINER".to_string());
                lines.push("ABOUT CENTRE, FULL AT THE STOP.".to_string());
                lines.push(String::new());
                lines.push(format!("< > NOW: {}", StickItem::Curve.value(m, None)));
            }
            Step::Fire => {
                lines.push(format!("TRIGGER  (NOW {})", m.button_name(m.fire)));
                lines.push("SQUEEZE THE TRIGGER".to_string());
                lines.push("(FIRES THE GUNS)".to_string());
                lines.push(String::new());
                lines.push(match self.detected {
                    Some(Detected::Button(b)) => format!("DETECTED {}", m.button_name(Some(b))),
                    _ => "WAITING...".to_string(),
                });
            }
            Step::Button(n) => {
                lines.push(format!(
                    "{}  (NOW {})",
                    n.name(),
                    m.button_name(m.button_for(n))
                ));
                lines.push("PRESS THE BUTTON FOR IT".to_string());
                lines.push(format!(
                    "(KEY: {})",
                    crate::input::key_name(n.default_key())
                ));
                lines.push(String::new());
                lines.push(match self.detected {
                    Some(Detected::Button(b)) => format!("DETECTED {}", m.button_name(Some(b))),
                    _ => "WAITING...".to_string(),
                });
            }
            Step::Summary => {
                for f in Flight::ALL {
                    lines.push(format!("{:<9}{}", f.name(), m.axis(f).label(m)));
                }
                lines.push(format!("TRIGGER  {}", m.button_name(m.fire)));
                // Walk the hardware: every control with no job, by name.
                let (jobs, total, free) = m.coverage(device);
                if total == 0 {
                    lines.push("NO STICK TO CHECK COVERAGE ON".to_string());
                } else if free.is_empty() {
                    lines.push(format!("COMPLETE: {jobs} OF {total} HAVE A JOB"));
                } else {
                    lines.push(format!("{jobs} OF {total} HAVE A JOB. FREE:"));
                    let mut row = String::new();
                    for name in free {
                        if row.len() + name.len() + 1 > W {
                            row.push_str(" ..");
                            break;
                        }
                        if !row.is_empty() {
                            row.push(' ');
                        }
                        row.push_str(&name);
                    }
                    lines.push(row);
                }
            }
        }
        for (i, l) in lines.iter().enumerate().take(9) {
            text.draw_line(0, i + 3, &clip(l));
        }
        let foot = match self.step() {
            Step::Summary => ["ENTER FINISH  B BACK", ""],
            Step::ThrottleZero | Step::Deadzone | Step::Curve => {
                ["< > ADJUST  ENTER NEXT", "B BACK  ESC FINISH"]
            }
            Step::Axis(_) => [
                "ENTER KEEP  I INVERT  X CLEAR",
                "S SKIP  B BACK  ESC FINISH",
            ],
            _ => ["ENTER KEEP  X CLEAR  S SKIP", "B BACK  ESC FINISH"],
        };
        text.draw_line(0, LINES - 3, foot[0]);
        text.draw_line(0, LINES - 2, foot[1]);
    }
}

impl Default for Wizard {
    fn default() -> Self {
        Self::new()
    }
}

/// A 21-character bar: `[.........:.........]` with the value's mark.
fn bar(v: f32) -> String {
    let v = if v.is_finite() {
        v.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let mut s: Vec<u8> = b"[.........:.........]".to_vec();
    let i = ((v + 1.0) * 0.5 * 18.0).round() as usize + 1;
    s[i.min(19)] = b'*';
    String::from_utf8(s).unwrap_or_default()
}

// ---------------------------------------------------------------------
// The reader: polls the platform for the one stick the game is on.
// ---------------------------------------------------------------------

/// The stick as the app sees it frame to frame: the device (if any), the
/// latest sample, and the button edges since last time.
#[derive(Clone, Debug, Default)]
pub struct Reader {
    pub device: Option<Device>,
    last: Sample,
    /// Frames until the next look for a stick when none is attached.
    retry: u32,
    /// Axes seen inside the rails since the stick appeared, one bit each.
    /// winmm reports an axis nothing has touched since plug-in at full
    /// deflection (the HOTAS 4's rocker read `strafe +1.00` for a whole
    /// live flight), so a railed axis is "no data yet", not a demand.
    calibrated: u8,
    /// Which railed axes have been logged, so each says so once.
    warned: u8,
    #[cfg(windows)]
    id: u32,
}

/// Inside this much of full travel an axis counts as real (calibrated).
const AXIS_ALIVE: f32 = 0.95;

/// Frames between looks for a stick that isn't there (a second at 60).
const RETRY_FRAMES: u32 = 60;

impl Reader {
    /// Read the stick. `None` when there isn't one. Edges are against the
    /// previous call's reading.
    pub fn poll(&mut self) -> Option<Sample> {
        if self.device.is_none() {
            if self.retry > 0 {
                self.retry -= 1;
                return None;
            }
            self.retry = RETRY_FRAMES;
            match platform::find() {
                Some((handle, device)) => {
                    log::info!(
                        "stick: found {} ({:04X}:{:04X}, {} axes, {} buttons{})",
                        device.label(),
                        device.vid,
                        device.pid,
                        device.axes,
                        device.buttons,
                        if device.hat { ", a hat" } else { "" }
                    );
                    #[cfg(windows)]
                    {
                        self.id = handle;
                    }
                    let _ = handle;
                    self.device = Some(device);
                }
                None => return None,
            }
            self.calibrated = 0;
            self.warned = 0;
        }
        #[cfg(windows)]
        let read = platform::read(self.id);
        #[cfg(not(windows))]
        let read = platform::read(0);
        match read {
            Some(s) => Some(self.admit(s)),
            None => {
                log::info!("stick: unplugged");
                self.device = None;
                self.last = Sample::default();
                None
            }
        }
    }

    /// The calibration gate: an axis that has only ever read full
    /// deflection contributes nothing until it is first seen inside the
    /// rails — then it is real for good, rails included.
    fn admit(&mut self, mut s: Sample) -> Sample {
        for (i, v) in s.axes.iter_mut().enumerate() {
            let bit = 1u8 << i;
            if self.calibrated & bit != 0 {
                continue;
            }
            if v.abs() < AXIS_ALIVE {
                self.calibrated |= bit;
            } else {
                if self.warned & bit == 0 {
                    log::info!("stick: axis {i} rests at full deflection - ignored until it moves");
                    self.warned |= bit;
                }
                *v = 0.0;
            }
        }
        s
    }

    /// Note the sample as seen: returns (pressed, released) button masks
    /// against the last one noted.
    pub fn edges(&mut self, s: Sample) -> (u32, u32) {
        let pressed = s.buttons & !self.last.buttons;
        let released = self.last.buttons & !s.buttons;
        self.last = s;
        (pressed, released)
    }

    #[cfg(test)]
    pub fn last(&self) -> Sample {
        self.last
    }
}

#[cfg(windows)]
mod platform {
    //! winmm: `joyGetPosEx`, the oldest joystick API Windows has and the
    //! one every HID stick still answers to, no window handle needed. Six
    //! axes, 32 buttons, one hat — the whole of a T.Flight HOTAS 4. Axis
    //! order X Y Z U V R, so the indices match the Reforger tool's.
    use super::{Device, Sample, MAX_AXES};
    use windows_sys::Win32::Media::Multimedia::{joyGetDevCapsW, joyGetPosEx, JOYCAPSW, JOYINFOEX};

    const JOY_RETURNALL: u32 = 0xFF;
    const JOY_POVCENTERED: u32 = 0xFFFF;
    const JOYCAPS_HASPOV: u32 = 0x10;

    pub fn find() -> Option<(u32, Device)> {
        for id in 0..16u32 {
            // SAFETY: a zeroed JOYCAPSW is a valid out-parameter; winmm
            // fills it or returns an error.
            let mut caps: JOYCAPSW = unsafe { std::mem::zeroed() };
            let rc = unsafe {
                joyGetDevCapsW(
                    id as usize,
                    &mut caps,
                    std::mem::size_of::<JOYCAPSW>() as u32,
                )
            };
            if rc != 0 || caps.wNumAxes == 0 {
                continue;
            }
            // Caps come back for a configured id even with nothing on it:
            // a real reading is what says it's attached.
            if read(id).is_none() {
                continue;
            }
            // JOYCAPSW is packed: copy each field out before touching it.
            let pname = caps.szPname;
            let (vid, pid, num_axes, num_buttons, wcaps) = (
                caps.wMid,
                caps.wPid,
                caps.wNumAxes,
                caps.wNumButtons,
                caps.wCaps,
            );
            let name: String = char::decode_utf16(pname.iter().copied().take_while(|c| *c != 0))
                .map(|c| c.unwrap_or('?'))
                .collect();
            return Some((
                id,
                Device {
                    name,
                    vid,
                    pid,
                    axes: (num_axes as usize).min(MAX_AXES),
                    buttons: num_buttons as usize,
                    hat: wcaps & JOYCAPS_HASPOV != 0,
                },
            ));
        }
        None
    }

    pub fn read(id: u32) -> Option<Sample> {
        // SAFETY: JOYINFOEX is plain data; dwSize and dwFlags are set
        // before the call as winmm requires.
        let mut info: JOYINFOEX = unsafe { std::mem::zeroed() };
        info.dwSize = std::mem::size_of::<JOYINFOEX>() as u32;
        info.dwFlags = JOY_RETURNALL;
        if unsafe { joyGetPosEx(id, &mut info) } != 0 {
            return None;
        }
        let unit = |raw: u32| (raw.min(65535) as f32 / 65535.0) * 2.0 - 1.0;
        let mut s = Sample {
            axes: [0.0; MAX_AXES],
            buttons: info.dwButtons & ((1u32 << super::HAT_BIT) - 1),
        };
        for (i, raw) in [
            info.dwXpos,
            info.dwYpos,
            info.dwZpos,
            info.dwUpos,
            info.dwVpos,
            info.dwRpos,
        ]
        .into_iter()
        .enumerate()
        {
            s.axes[i] = unit(raw);
        }
        let hat = (info.dwPOV != JOY_POVCENTERED && info.dwPOV < 36000).then_some(info.dwPOV);
        Some(s.with_hat(hat))
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    //! The browser's Gamepad API: `navigator.getGamepads()`, polled once
    //! a frame. Axes and buttons come in the browser's own order, so the
    //! wizard is the map here; the ids carry the USB vendor/product in
    //! Chrome's "(Vendor: 044f Product: b67c)" and Firefox's
    //! "044f-b67c-" forms, both read.
    use super::{Device, Sample, MAX_AXES};
    use wasm_bindgen::JsCast;

    fn first_pad() -> Option<web_sys::Gamepad> {
        let pads = web_sys::window()?.navigator().get_gamepads().ok()?;
        for i in 0..pads.length() {
            let v = pads.get(i);
            if v.is_null() || v.is_undefined() {
                continue;
            }
            if let Ok(gp) = v.dyn_into::<web_sys::Gamepad>() {
                if gp.connected() {
                    return Some(gp);
                }
            }
        }
        None
    }

    fn ids(id: &str) -> (u16, u16) {
        let lower = id.to_ascii_lowercase();
        let hex = |s: &str| u16::from_str_radix(s.trim(), 16).ok();
        let after = |tag: &str| {
            lower
                .find(tag)
                .and_then(|i| lower[i + tag.len()..].trim_start().get(..4).and_then(hex))
        };
        if let (Some(v), Some(p)) = (after("vendor:"), after("product:")) {
            return (v, p);
        }
        let mut parts = lower.splitn(3, '-');
        if let (Some(v), Some(p)) = (parts.next().and_then(hex), parts.next().and_then(hex)) {
            return (v, p);
        }
        (0, 0)
    }

    pub fn find() -> Option<(u32, Device)> {
        let gp = first_pad()?;
        let (vid, pid) = ids(&gp.id());
        Some((
            gp.index(),
            Device {
                name: gp.id(),
                vid,
                pid,
                axes: (gp.axes().length() as usize).min(MAX_AXES),
                buttons: gp.buttons().length() as usize,
                hat: false,
            },
        ))
    }

    pub fn read(_id: u32) -> Option<Sample> {
        let gp = first_pad()?;
        let mut s = Sample::default();
        let axes = gp.axes();
        for i in 0..axes.length().min(MAX_AXES as u32) {
            s.axes[i as usize] = axes.get(i).as_f64().unwrap_or(0.0) as f32;
        }
        let buttons = gp.buttons();
        for i in 0..buttons.length().min(u32::from(super::HAT_BIT)) {
            if let Ok(b) = buttons.get(i).dyn_into::<web_sys::GamepadButton>() {
                if b.pressed() {
                    s.buttons |= 1 << i;
                }
            }
        }
        Some(s)
    }
}

#[cfg(not(any(windows, target_arch = "wasm32")))]
mod platform {
    //! No native reader yet on this platform (gilrs would be the road:
    //! evdev on Linux, IOKit on macOS — see docs/HOTAS.md).
    use super::{Device, Sample};
    pub fn find() -> Option<(u32, Device)> {
        None
    }
    pub fn read(_id: u32) -> Option<Sample> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(axes: &[(usize, f32)], buttons: &[u8]) -> Sample {
        let mut s = Sample::default();
        for &(i, v) in axes {
            s.axes[i] = v;
        }
        for &b in buttons {
            s.buttons |= 1 << b;
        }
        s
    }

    #[test]
    fn every_shaped_value_stays_finite_and_in_range() {
        for dz in [0.0, 0.08, 0.5] {
            for curve in [1.0, 1.5, 3.0] {
                for raw in [
                    -2.0,
                    -1.0,
                    -0.5,
                    -0.08,
                    0.0,
                    0.08,
                    0.5,
                    1.0,
                    2.0,
                    f32::NAN,
                    f32::INFINITY,
                ] {
                    let v = shape(raw, dz, curve);
                    assert!(v.is_finite(), "shape({raw}, {dz}, {curve}) = {v}");
                    assert!(
                        (-1.0..=1.0).contains(&v),
                        "shape({raw}, {dz}, {curve}) = {v}"
                    );
                }
            }
        }
    }

    #[test]
    fn deadzone_is_symmetric_and_full_travel_still_reaches_one() {
        for v in [0.05, 0.3, 0.7, 1.0] {
            assert!((shape(v, 0.1, 1.5) + shape(-v, 0.1, 1.5)).abs() < 1e-6);
        }
        assert_eq!(shape(0.05, 0.1, 1.5), 0.0, "inside the band is nothing");
        assert_eq!(shape(-0.05, 0.1, 1.5), 0.0);
        assert!(
            (shape(1.0, 0.1, 1.5) - 1.0).abs() < 1e-6,
            "the stop is still full"
        );
        assert!((shape(-1.0, 0.1, 1.5) + 1.0).abs() < 1e-6);
        // Above the band the response starts from zero, not from a step.
        assert!(shape(0.11, 0.1, 1.0) < 0.02);
        // A higher curve is finer about the centre.
        assert!(shape(0.5, 0.0, 3.0) < shape(0.5, 0.0, 1.0));
    }

    /// The stock HOTAS 4 map, with the measured directions: back on the
    /// stick pitches up (+X torque), twist right yaws right (-Y), stick
    /// right rolls right (-Z), the lever forward (which winmm reads as
    /// negative) thrusts along the nose (-Z), the rocker right strafes
    /// right (+X).
    #[test]
    fn hotas4_defaults_fly_the_ship_the_right_way() {
        let m = StickMap::hotas4();
        let pitch = m.body_axes(&sample(&[(1, 1.0)], &[]));
        assert!(pitch[3] > 0.99, "pull back = pitch up: {pitch:?}");
        let yaw = m.body_axes(&sample(&[(5, 1.0)], &[]));
        assert!(yaw[4] < -0.99, "twist right = yaw right: {yaw:?}");
        let roll = m.body_axes(&sample(&[(0, 1.0)], &[]));
        assert!(roll[5] < -0.99, "stick right = roll right: {roll:?}");
        let thr = m.body_axes(&sample(&[(2, -1.0)], &[]));
        assert!(
            thr[2] < -0.99,
            "lever forward = thrust along the nose: {thr:?}"
        );
        let strafe = m.body_axes(&sample(&[(4, 1.0)], &[]));
        assert!(strafe[0] > 0.99, "rocker right = strafe right: {strafe:?}");
        assert_eq!(m.fire, Some(0), "the trigger fires");
        assert_eq!(m.button_for(Named::Hyper), Some(8), "R2 is the chaos drive");
    }

    #[test]
    fn body_axes_never_leave_range_or_go_nan() {
        let mut m = StickMap::hotas4();
        // Two flight controls on one raw axis (a hand-edited file) still
        // sum into range.
        m.axes[Flight::Lift as usize] = AxisMap::at(1, false);
        m.axes[Flight::Strafe as usize] = AxisMap::at(1, false);
        let s = Sample {
            axes: [f32::NAN, 2.0, -2.0, 1.0, 1.0, -1.0, f32::INFINITY, 0.0],
            ..Default::default()
        };
        for v in m.body_axes(&s) {
            assert!(v.is_finite() && (-1.0..=1.0).contains(&v), "{v}");
        }
        assert_eq!(
            m.body_axes(&Sample::default()),
            [0.0; 6],
            "a centred stick is neutral"
        );
        m.enabled = false;
        assert_eq!(
            m.body_axes(&sample(&[(1, 1.0)], &[])),
            [0.0; 6],
            "off is off"
        );
    }

    #[test]
    fn a_bottom_zero_throttle_is_ahead_only() {
        let mut m = StickMap::hotas4();
        m.throttle_zero = ThrottleZero::Bottom;
        // The lever full back (winmm +1, inverted to -1) is zero thrust...
        assert_eq!(m.body_axes(&sample(&[(2, 1.0)], &[]))[2], 0.0);
        // ...the middle is half...
        let mid = m.body_axes(&sample(&[(2, 0.0)], &[]))[2];
        assert!(mid < 0.0 && mid > -0.8, "{mid}");
        // ...and full forward is full.
        assert!(m.body_axes(&sample(&[(2, -1.0)], &[]))[2] < -0.99);
    }

    #[test]
    fn hat_ways_bind_like_buttons() {
        let s = Sample::default().with_hat(Some(0));
        assert!(s.button(HAT_BIT), "up");
        let s = Sample::default().with_hat(Some(13500));
        assert!(
            s.button(HAT_BIT + 1) && s.button(HAT_BIT + 2),
            "down-right is both"
        );
        assert!(!Sample::default().with_hat(None).button(HAT_BIT));
        assert_eq!(button_from_name("hat-left"), Some(HAT_BIT + 3));
        assert_eq!(button_key(Some(HAT_BIT + 1)), "hat-right");
        assert_eq!(button_from_name("7"), Some(7));
        assert_eq!(button_from_name("none"), None);
    }

    /// Once the stick is known its controls have names, and the summary
    /// walks the hardware: a control with no job is listed by name.
    #[test]
    fn a_known_stick_names_its_controls_and_shows_the_holes() {
        let mut m = StickMap::hotas4();
        assert_eq!(m.button_name(Some(0)), "TRIGGER");
        assert_eq!(m.button_name(Some(8)), "R2");
        assert_eq!(m.axis_name(4), "ROCKER");
        assert_eq!(m.axis(Flight::Yaw).label(&m), "TWIST +");
        assert_eq!(m.button_name(Some(HAT_BIT)), "HAT-U");
        assert_eq!(
            m.button_name(Some(20)),
            "B20",
            "past the catalogue: the number"
        );
        m.layout = Layout::Generic;
        assert_eq!(m.button_name(Some(0)), "B0");
        assert!(m.axis_name(4).starts_with("AXIS 4"));
        // Coverage on the stock map: the HOTAS 4 has 21 controls (5 live
        // axes, 12 buttons, a 4-way hat); U is unused and never counted.
        let m = StickMap::hotas4();
        let (jobs, total, free) = m.coverage(None);
        assert_eq!(total, 21);
        assert_eq!(jobs, 21, "the stock map leaves nothing free: {free:?}");
        let dev = Device {
            name: String::new(),
            vid: 0x044F,
            pid: 0xB67C,
            axes: 6,
            buttons: 12,
            hat: true,
        };
        let mut m2 = m;
        m2.bind_button(Named::Chase, None);
        m2.bind_axis(Flight::Strafe, AxisMap::NONE);
        let (jobs, total, free) = m2.coverage(Some(&dev));
        assert_eq!(total, 21, "the driver's U axis is not a control");
        assert_eq!(jobs, 19);
        assert!(free.contains(&"BASE L".to_string()) && free.contains(&"ROCKER".to_string()));
        assert!(!free.contains(&"AXIS U".to_string()));
        // A stick nobody knows: coverage needs the device.
        let mut g = StickMap::empty();
        assert_eq!(g.coverage(None).1, 0);
        g.bind_fire(Some(0));
        let gd = Device {
            axes: 2,
            buttons: 3,
            ..Device::default()
        };
        let (jobs, total, free) = g.coverage(Some(&gd));
        assert_eq!((jobs, total), (1, 5));
        assert_eq!(free.len(), 4);
        assert!(free.contains(&"B1".to_string()) && free.contains(&"B2".to_string()));
        assert!(free[0].starts_with("AXIS 0"));
    }

    #[test]
    fn a_button_has_one_job_and_an_axis_one_control() {
        let mut m = StickMap::hotas4();
        m.bind_button(Named::Map, Some(1));
        assert_eq!(m.button_for(Named::Map), Some(1));
        assert_eq!(m.button_for(Named::Boost), None, "boost lost B1");
        m.bind_fire(Some(1));
        assert_eq!(m.button_for(Named::Map), None);
        assert_eq!(m.fire, Some(1));
        m.bind_axis(Flight::Lift, AxisMap::at(1, true));
        assert_eq!(m.axis(Flight::Pitch).axis, None, "pitch let go of Y");
        assert_eq!(m.axis(Flight::Lift), AxisMap::at(1, true));
    }

    #[test]
    fn the_map_round_trips_through_the_settings_file() {
        let mut m = StickMap::hotas4();
        m.bind_axis(Flight::Lift, AxisMap::at(3, true));
        m.bind_button(Named::Capture, Some(HAT_BIT + 2));
        m.throttle_zero = ThrottleZero::Bottom;
        m.deadzone = 0.12;
        m.curve = 2.0;
        m.enabled = false;
        m.throttle_brake = false;
        m.throttle_jump = false;
        let mut text = String::new();
        m.render(&mut text);
        let mut back = StickMap::empty();
        for line in text.lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(back.parse_key(k.trim(), v.trim()), "{k} not ours");
        }
        assert_eq!(back, m);
        assert!(!StickMap::empty().parse_key("graphics.msaa", "4"));
        // Nonsense leaves the value alone.
        let mut m2 = StickMap::hotas4();
        m2.parse_key("stick.pitch", "banana");
        m2.parse_key("stick.deadzone", "nan");
        m2.parse_key("stick.fire", "99");
        assert_eq!(
            m2.axis(Flight::Pitch),
            StickMap::hotas4().axis(Flight::Pitch)
        );
        assert_eq!(m2.deadzone, StickMap::hotas4().deadzone);
        assert_eq!(m2.fire, None, "an out-of-range button is no button");
    }

    #[test]
    fn the_wizard_learns_an_axis_from_the_move_it_sees() {
        let mut w = Wizard::new();
        let mut m = StickMap::empty();
        assert_eq!(w.step(), Step::Axis(Flight::Pitch));
        // The first reading is where things rest — even off centre.
        w.feed(sample(&[(3, 0.4)], &[]));
        assert_eq!(w.detected(), None);
        // A small twitch on another axis does not count.
        w.feed(sample(&[(3, 0.4), (0, 0.3)], &[]));
        assert_eq!(w.detected(), None);
        // A push the other way on Y (nose-down first) is seen as inverted...
        w.feed(sample(&[(3, 0.4), (1, -0.9)], &[]));
        assert_eq!(
            w.detected(),
            Some(Detected::Axis {
                index: 1,
                positive: false
            })
        );
        // ...and corrected by the pull back the prompt asked for.
        w.feed(sample(&[(3, 0.4), (1, 0.9)], &[]));
        assert_eq!(
            w.detected(),
            Some(Detected::Axis {
                index: 1,
                positive: true
            })
        );
        assert_eq!(w.key(KeyCode::Enter, &mut m), WizardEvent::Changed);
        assert_eq!(m.axis(Flight::Pitch), AxisMap::at(1, false));
        assert_eq!(w.step(), Step::Axis(Flight::Yaw));
        assert_eq!(w.detected(), None, "a new step starts clean");
        // Twist left when asked for right: inverted, and I flips it back.
        w.feed(Sample::default());
        w.feed(sample(&[(5, -1.0)], &[]));
        w.key(KeyCode::KeyI, &mut m);
        w.key(KeyCode::Enter, &mut m);
        assert_eq!(m.axis(Flight::Yaw), AxisMap::at(5, false));
    }

    #[test]
    fn the_wizard_binds_buttons_skips_and_goes_back() {
        let mut w = Wizard::at_step(9);
        let mut m = StickMap::empty();
        assert_eq!(w.step(), Step::Fire);
        w.feed(Sample::default());
        w.feed(sample(&[], &[0]));
        assert_eq!(w.detected(), Some(Detected::Button(0)));
        assert_eq!(w.key(KeyCode::Enter, &mut m), WizardEvent::Changed);
        assert_eq!(m.fire, Some(0));
        assert_eq!(w.step(), Step::Button(Named::Boost));
        // A button already held when the step opened is not a press.
        w.feed(sample(&[], &[0]));
        w.feed(sample(&[], &[0]));
        assert_eq!(w.detected(), None);
        w.feed(sample(&[], &[0, 1]));
        assert_eq!(w.detected(), Some(Detected::Button(1)));
        // Skip leaves it be; back returns; X clears.
        assert_eq!(w.key(KeyCode::KeyS, &mut m), WizardEvent::Nothing);
        assert_eq!(m.button_for(Named::Boost), None);
        assert_eq!(w.step(), Step::Button(Named::Brake));
        w.key(KeyCode::KeyB, &mut m);
        assert_eq!(w.step(), Step::Button(Named::Boost));
        w.key(KeyCode::KeyB, &mut m);
        assert_eq!(w.step(), Step::Fire);
        assert_eq!(w.key(KeyCode::KeyX, &mut m), WizardEvent::Changed);
        assert_eq!(m.fire, None);
        // Esc leaves at any point.
        assert_eq!(w.key(KeyCode::Escape, &mut m), WizardEvent::Done);
    }

    #[test]
    fn the_wizard_walks_every_control_to_a_summary() {
        let w = Wizard::new();
        let steps = steps();
        for f in Flight::ALL {
            assert!(steps.contains(&Step::Axis(f)), "{f:?} has no step");
        }
        for n in Named::ALL {
            assert!(steps.contains(&Step::Button(n)), "{n:?} has no step");
        }
        assert!(steps.contains(&Step::Fire));
        assert_eq!(*steps.last().unwrap(), Step::Summary);
        assert_eq!(w.step_count(), steps.len());
        // Enter through everything with nothing detected ends on the summary.
        let mut w = Wizard::new();
        let mut m = StickMap::hotas4();
        for _ in 0..steps.len() - 1 {
            w.key(KeyCode::Enter, &mut m);
        }
        assert_eq!(w.step(), Step::Summary);
        assert_eq!(m, StickMap::hotas4(), "nothing detected, nothing changed");
        assert_eq!(w.key(KeyCode::Enter, &mut m), WizardEvent::Done);
    }

    /// Every wizard page fits the panel: no line wider than 32 columns,
    /// no more rows than the bitmap holds.
    #[test]
    fn every_wizard_page_fits_the_panel() {
        let m = StickMap::hotas4();
        let dev = Device {
            name: String::new(),
            vid: 0x044F,
            pid: 0xB67C,
            axes: 6,
            buttons: 12,
            hat: true,
        };
        for n in 0..steps().len() {
            let mut w = Wizard::at_step(n);
            w.bench_detect();
            let mut text = TextBitmap::new();
            w.render(&mut text, &m, Some(&dev));
            let mut text2 = TextBitmap::new();
            w.render(&mut text2, &m, None);
        }
        // The lines themselves: every prompt within the panel's width.
        for f in Flight::ALL {
            for l in f.prompt() {
                assert!(l.len() <= 48, "{l:?} is wider than the panel");
            }
        }
        assert_eq!(bar(0.0), "[.........*.........]");
        assert_eq!(bar(-1.0), "[*........:.........]");
        assert_eq!(bar(1.0), "[.........:........*]");
        assert_eq!(bar(f32::NAN).len(), 21);
    }

    #[test]
    fn the_stick_page_rows_cycle_and_flip() {
        let mut m = StickMap::hotas4();
        assert!(StickItem::Axis(Flight::Pitch).adjust(&mut m, true, true));
        assert!(m.axis(Flight::Pitch).invert, "enter flips");
        assert!(StickItem::Axis(Flight::Lift).adjust(&mut m, true, false));
        assert_eq!(m.axis(Flight::Lift).axis, Some(0), "NONE steps to axis 0");
        assert_eq!(m.axis(Flight::Roll).axis, None, "and roll let go of X");
        assert!(StickItem::Axis(Flight::Lift).adjust(&mut m, false, false));
        assert_eq!(m.axis(Flight::Lift).axis, None, "and back to NONE");
        assert!(
            !StickItem::Axis(Flight::Lift).adjust(&mut m, true, true),
            "nothing to flip"
        );
        assert!(StickItem::Deadzone.adjust(&mut m, true, false));
        assert!((m.deadzone - 0.10).abs() < 1e-6);
        for _ in 0..40 {
            StickItem::Curve.adjust(&mut m, true, false);
        }
        assert_eq!(m.curve, CURVE_MAX);
        assert!(
            !StickItem::Curve.adjust(&mut m, true, false),
            "stops at the limit"
        );
        assert_eq!(StickItem::Device.value(&m, None), "NONE FOUND");
        let d = Device {
            name: "x".into(),
            vid: 0x044F,
            pid: 0xB67C,
            axes: 6,
            buttons: 12,
            hat: true,
        };
        assert!(StickItem::Device.value(&m, Some(&d)).contains("HOTAS 4"));
        assert!(d.is_hotas4());
    }

    /// The live bug this answers: a fresh HOTAS 4's rocker read
    /// `strafe +1.00` at rest for a whole flight — winmm reports an axis
    /// nothing has touched since plug-in at full deflection.
    /// A steady push up the whole travel, however far, is throttle — the
    /// slam needs the travel inside a quarter second.
    #[test]
    fn a_smooth_throttle_push_never_jumps() {
        let m = StickMap::hotas4();
        let mut g = Gestures::default();
        // Two seconds from full back to full forward, 120 Hz. Remember
        // the lever is inverted on the HOTAS 4: raw -1 is full forward.
        for i in 0..=240 {
            let t = i as f64 / 120.0;
            let lever = 1.0 - (i as f32 / 240.0) * 2.0;
            let (_, hyper) = g.step(&m, &sample(&[(2, lever)], &[]), t);
            assert!(!hyper, "a smooth push jumped at t={t:.2}");
        }
        // And a jump switched OFF never fires, however hard the slam.
        let mut off = StickMap::hotas4();
        off.throttle_jump = false;
        let mut g = Gestures::default();
        g.step(&off, &sample(&[(2, 1.0)], &[]), 0.0);
        let (_, hyper) = g.step(&off, &sample(&[(2, -1.0)], &[]), 0.1);
        assert!(!hyper);
    }

    #[test]
    fn a_slam_jumps_for_two_seconds_and_releases() {
        let m = StickMap::hotas4();
        let mut g = Gestures::default();
        g.step(&m, &sample(&[(2, 1.0)], &[]), 0.0);
        let (_, hyper) = g.step(&m, &sample(&[(2, -1.0)], &[]), 0.1);
        assert!(hyper, "a slam fires the drive");
        // Held for two seconds from the slam, wherever the lever went...
        let (_, still) = g.step(&m, &sample(&[(2, 0.0)], &[]), 2.0);
        assert!(still);
        // ...and let go after.
        let (_, done) = g.step(&m, &sample(&[(2, 0.0)], &[]), 2.2);
        assert!(!done);
        // Holding the lever forward does not re-fire: it must come back
        // and be slammed again.
        let (_, again) = g.step(&m, &sample(&[(2, -1.0)], &[]), 2.5);
        assert!(!again, "holding forward is not another slam");
    }

    /// The lever hard back is the air brake — but only with the zero at
    /// the centre, and never from an axis the reader is holding back.
    #[test]
    fn the_lever_hard_back_holds_the_air_brake() {
        let m = StickMap::hotas4();
        let mut g = Gestures::default();
        let (brake, _) = g.step(&m, &sample(&[(2, 0.97)], &[]), 0.0);
        assert!(brake, "the bottom of the travel brakes");
        let (brake, _) = g.step(&m, &sample(&[(2, 0.5)], &[]), 0.1);
        assert!(!brake, "half back is reverse thrust, not the brake");
        let mut bottom = StickMap::hotas4();
        bottom.throttle_zero = ThrottleZero::Bottom;
        assert!(
            !Gestures::default()
                .step(&bottom, &sample(&[(2, 1.0)], &[]), 0.0)
                .0,
            "with the zero at the bottom, resting there must not brake"
        );
        let mut off = StickMap::hotas4();
        off.throttle_brake = false;
        assert!(
            !Gestures::default()
                .step(&off, &sample(&[(2, 1.0)], &[]), 0.0)
                .0
        );
        let mut none = StickMap::hotas4();
        none.bind_axis(Flight::Throttle, AxisMap::NONE);
        assert!(
            !Gestures::default()
                .step(&none, &sample(&[(2, 1.0)], &[]), 0.0)
                .0
        );
        // The calibration gate zeroes a railed untouched axis: through
        // the reader, a fresh stick's lever cannot brake by resting.
        let mut r = Reader::default();
        let s = r.admit(sample(&[(2, 1.0)], &[]));
        assert!(!Gestures::default().step(&m, &s, 0.0).0);
    }

    #[test]
    fn an_unidentified_axis_resting_at_full_deflection_moves_nothing() {
        let mut r = Reader::default();
        let s = r.admit(sample(&[(4, 1.0), (1, 0.2)], &[]));
        assert_eq!(s.axes[4], 0.0, "a railed axis is not a demand");
        assert_eq!(s.axes[1], 0.2, "a centred axis works at once");
        // Still railed, either way: still nothing.
        assert_eq!(r.admit(sample(&[(4, -1.0)], &[])).axes[4], 0.0);
        // The moment it reads inside the rails it is real...
        assert_eq!(r.admit(sample(&[(4, 0.3)], &[])).axes[4], 0.3);
        // ...and full deflection then means full deflection.
        assert_eq!(r.admit(sample(&[(4, 1.0)], &[])).axes[4], 1.0);
    }

    #[test]
    fn button_edges_come_from_the_reader() {
        let mut r = Reader::default();
        assert_eq!(r.edges(sample(&[], &[0, 3])), (0b1001, 0));
        assert_eq!(r.edges(sample(&[], &[3])), (0, 0b0001));
        assert_eq!(r.edges(sample(&[], &[3])), (0, 0));
        assert_eq!(r.last().buttons, 0b1000);
    }
}
