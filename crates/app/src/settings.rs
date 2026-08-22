//! The settings file: graphics, controls, cockpit layout.
//!
//! Plain `key = value` lines at `~/.farfall/settings.cfg`, written whole on
//! every change from the menu and read once at start. No format crate: a
//! file a pilot can fix with any editor, and nothing to get wrong in it
//! that the defaults can't cover — unknown keys are ignored, bad values
//! fall back, missing lines mean default.

use crate::cockpit::{Instrument, Layout, Slot};
use crate::input::{key_from_name, key_name, Action, Bindings};
use crate::warp::{Destination, Plan};
use std::path::PathBuf;

/// "x,y" → a pair of finite numbers.
fn parse_pair(v: &str) -> Option<[f32; 2]> {
    let (a, b) = v.split_once(',')?;
    let x = a.trim().parse::<f32>().ok()?;
    let y = b.trim().parse::<f32>().ok()?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

/// Where the settings menu's text starts and the map pane is centred,
/// until the pilot drags them.
pub const MENU_ANCHOR_DEFAULT: [f32; 2] = [-0.72, 0.62];
pub const MAP_ANCHOR_DEFAULT: [f32; 2] = [0.42, 0.12];

fn clamp_anchor(a: [f32; 2]) -> [f32; 2] {
    [a[0].clamp(-0.95, 0.95), a[1].clamp(-0.95, 0.95)]
}

/// The landing hoops' spacings on offer, metres.
pub const LANDING_SPACINGS: [f32; 4] = [100.0, 250.0, 500.0, 1000.0];

/// Hoop size range, as a multiple of the stock diameter.
pub const HOOP_SIZE_MIN: f32 = 0.25;
pub const HOOP_SIZE_MAX: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub msaa: u32,
    pub scale: f32,
    pub vsync: bool,
    pub bindings: Bindings,
    pub layout: Layout,
    /// Freelook: radians per mouse count, relative to the default.
    pub look_sensitivity: f32,
    /// Hoop diameter, as a multiple of the stock size.
    pub hoop_size: f32,
    /// Where the settings menu's text block sits (top-left, canopy NDC).
    pub menu_anchor: [f32; 2],
    /// Where the map pane's centre sits (canopy NDC); the DRIVE panel
    /// hangs off its left edge.
    pub map_anchor: [f32; 2],
    /// The wireframe cabin: drawn at all, how bright its lines, how opaque
    /// its hull.
    pub cockpit_frame: bool,
    pub cockpit_glow: f32,
    pub cockpit_hull: f32,
    /// Spacing of the landing hoops, metres.
    pub landing_spacing_m: f32,
    /// Rings drawn around each body on the map, 0..=6.
    pub map_rings: u32,
    /// The map's reference grid.
    pub map_grid: bool,
    /// The wormhole drive's destination and safe distance.
    pub plan: Plan,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            msaa: 4,
            scale: 1.0,
            vsync: true,
            bindings: Bindings::default(),
            layout: Layout::default(),
            look_sensitivity: 1.0,
            hoop_size: 1.0,
            menu_anchor: MENU_ANCHOR_DEFAULT,
            map_anchor: MAP_ANCHOR_DEFAULT,
            cockpit_frame: true,
            cockpit_glow: 1.0,
            cockpit_hull: 0.92,
            landing_spacing_m: 250.0,
            map_rings: 4,
            map_grid: true,
            plan: Plan::default(),
        }
    }
}

pub const MSAA_CHOICES: [u32; 4] = [1, 2, 4, 8];

impl Settings {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".farfall").join("settings.cfg"))
    }

    /// Read the file, or defaults if there is none.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let s = Self::parse(&text);
                log::info!("settings: loaded {}", path.display());
                s
            }
            Err(_) => Self::default(),
        }
    }

    /// Write the file. Failure is logged, never fatal: a read-only home
    /// must not stop the game.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, self.render()) {
            log::warn!("settings: could not write {}: {e}", path.display());
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "graphics.msaa" => {
                    if let Ok(n) = v.parse::<u32>() {
                        if MSAA_CHOICES.contains(&n) {
                            s.msaa = n;
                        }
                    }
                }
                "graphics.scale" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.scale = f.clamp(0.25, 1.0);
                        }
                    }
                }
                "graphics.vsync" => s.vsync = matches!(v, "on" | "true" | "1"),
                "ui.safe-edge" => {
                    if let Ok(f) = v.trim_end_matches('%').parse::<f32>() {
                        s.layout.set_safe_edge(f / 100.0);
                    }
                }
                "warp.destination" => {
                    if let Some(d) = Destination::from_key(v) {
                        s.plan.dest = d;
                    }
                }
                "warp.safe-radii" => {
                    if let Ok(f) = v.parse::<f64>() {
                        s.plan.set_safe(f);
                    }
                }
                "control.look-sens" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.look_sensitivity = f.clamp(0.1, 5.0);
                        }
                    }
                }
                "ui.panel-menu" => {
                    if let Some(a) = parse_pair(v) {
                        s.menu_anchor = clamp_anchor(a);
                    }
                }
                "ui.panel-map" => {
                    if let Some(a) = parse_pair(v) {
                        s.map_anchor = clamp_anchor(a);
                    }
                }
                "cockpit.frame" => match v {
                    "on" => s.cockpit_frame = true,
                    "off" => s.cockpit_frame = false,
                    _ => {}
                },
                "cockpit.glow" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.cockpit_glow = f.clamp(0.25, 2.0);
                        }
                    }
                }
                "cockpit.hull" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.cockpit_hull = f.clamp(0.0, 1.0);
                        }
                    }
                }
                "ui.landing-hoops" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if LANDING_SPACINGS.contains(&f) {
                            s.landing_spacing_m = f;
                        }
                    }
                }
                "map.rings" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.map_rings = n.min(crate::map::RINGS_MAX);
                    }
                }
                "map.grid" => match v {
                    "on" => s.map_grid = true,
                    "off" => s.map_grid = false,
                    _ => {}
                },
                "ui.hoop-size" => {
                    if let Ok(f) = v.parse::<f32>() {
                        if f.is_finite() {
                            s.hoop_size = f.clamp(HOOP_SIZE_MIN, HOOP_SIZE_MAX);
                        }
                    }
                }
                "control.boost" => {
                    if let Some(key) = key_from_name(v) {
                        s.bindings.bind_boost(key);
                    }
                }
                "control.brake" => {
                    if let Some(key) = key_from_name(v) {
                        s.bindings.bind_brake(key);
                    }
                }
                _ => {
                    if let Some(name) = k.strip_prefix("control.") {
                        if let (Some(action), Some(key)) = (
                            Action::ALL.iter().copied().find(|a| a.key() == name),
                            key_from_name(v),
                        ) {
                            s.bindings.bind(action, key);
                        }
                    } else if let Some(name) = k.strip_prefix("ui.") {
                        let inst = Instrument::ALL.iter().copied().find(|i| i.key() == name);
                        // "slot at x,y": the slot, then the dragged anchor.
                        let (slot_key, free) = match v.split_once(" at ") {
                            Some((sk, at)) => (sk.trim(), parse_pair(at)),
                            None => (v, None),
                        };
                        if let (Some(inst), Some(slot)) = (inst, Slot::from_key(slot_key)) {
                            // A dial cannot be "on" and an overlay has no
                            // slot: keep each to its own choices.
                            let valid = if inst.slotted() {
                                Slot::DIALS.contains(&slot)
                            } else {
                                Slot::OVERLAYS.contains(&slot)
                            };
                            if valid {
                                s.layout.set(inst, slot);
                                if let Some(at) = free {
                                    s.layout.set_free(inst, at);
                                }
                            }
                        }
                    }
                }
            }
        }
        s
    }

    pub fn render(&self) -> String {
        let mut out = String::from("# FARFALL settings — edited by the in-game menu (Esc)\n");
        out.push_str(&format!("graphics.msaa = {}\n", self.msaa));
        out.push_str(&format!("graphics.scale = {:.2}\n", self.scale));
        out.push_str(&format!(
            "graphics.vsync = {}\n",
            if self.vsync { "on" } else { "off" }
        ));
        for a in Action::ALL {
            out.push_str(&format!(
                "control.{} = {}\n",
                a.key(),
                key_name(self.bindings.key_for(a))
            ));
        }
        out.push_str(&format!(
            "control.boost = {}\n",
            key_name(self.bindings.boost)
        ));
        out.push_str(&format!(
            "control.brake = {}\n",
            key_name(self.bindings.brake)
        ));
        out.push_str(&format!(
            "control.look-sens = {:.2}\n",
            self.look_sensitivity
        ));
        out.push_str(&format!("ui.hoop-size = {:.2}\n", self.hoop_size));
        out.push_str(&format!(
            "ui.panel-menu = {:.3},{:.3}\n",
            self.menu_anchor[0], self.menu_anchor[1]
        ));
        out.push_str(&format!(
            "ui.panel-map = {:.3},{:.3}\n",
            self.map_anchor[0], self.map_anchor[1]
        ));
        out.push_str(&format!(
            "cockpit.frame = {}\n",
            if self.cockpit_frame { "on" } else { "off" }
        ));
        out.push_str(&format!("cockpit.glow = {:.2}\n", self.cockpit_glow));
        out.push_str(&format!("cockpit.hull = {:.2}\n", self.cockpit_hull));
        out.push_str(&format!(
            "ui.landing-hoops = {:.0}\n",
            self.landing_spacing_m
        ));
        out.push_str(&format!("map.rings = {}\n", self.map_rings));
        out.push_str(&format!(
            "map.grid = {}\n",
            if self.map_grid { "on" } else { "off" }
        ));
        out.push_str(&format!("warp.destination = {}\n", self.plan.dest.key()));
        out.push_str(&format!("warp.safe-radii = {:.3}\n", self.plan.safe_radii));
        for i in Instrument::ALL {
            match self.layout.free(i) {
                Some([x, y]) => out.push_str(&format!(
                    "ui.{} = {} at {:.3},{:.3}\n",
                    i.key(),
                    self.layout.get(i).key(),
                    x,
                    y
                )),
                None => out.push_str(&format!("ui.{} = {}\n", i.key(), self.layout.get(i).key())),
            }
        }
        out.push_str(&format!(
            "ui.safe-edge = {:.0}%\n",
            self.layout.safe_edge * 100.0
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn defaults_round_trip() {
        let s = Settings::default();
        assert_eq!(Settings::parse(&s.render()), s);
    }

    #[test]
    fn edits_round_trip() {
        let mut s = Settings {
            msaa: 2,
            scale: 0.75,
            vsync: false,
            ..Default::default()
        };
        s.bindings.bind(Action::PitchUp, KeyCode::KeyI);
        s.bindings.bind_boost(KeyCode::ControlLeft);
        s.layout.set(Instrument::Gyro, Slot::TopCentre);
        s.layout.set(Instrument::Horizon, Slot::Off);
        s.layout.set_safe_edge(0.07);
        s.layout.set_free(Instrument::Speed, [0.125, -0.5]);
        s.look_sensitivity = 1.75;
        s.hoop_size = 2.5;
        s.map_rings = 2;
        s.landing_spacing_m = 500.0;
        s.cockpit_frame = false;
        s.cockpit_glow = 1.5;
        s.cockpit_hull = 0.25;
        s.menu_anchor = [-0.25, 0.5];
        s.map_anchor = [0.125, -0.125];
        s.map_grid = false;
        s.plan.dest = Destination::Moon;
        s.plan.set_safe(3.5);
        assert_eq!(Settings::parse(&s.render()), s);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        let s = Settings::parse(
            "graphics.msaa = 3\ngraphics.scale = nope\nui.gyro = on\nnonsense\ncontrol.pitch-up = ESCAPE\n",
        );
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn a_key_means_one_thing() {
        let mut b = Bindings::default();
        // I takes pitch-up; pitch-up's old key (Up) goes to whatever I had (nothing).
        assert!(b.bind(Action::PitchUp, KeyCode::KeyI));
        assert_eq!(b.key_for(Action::PitchUp), KeyCode::KeyI);
        // W (thrust forward) rebound to pitch-up: thrust-forward takes I.
        assert!(b.bind(Action::PitchUp, KeyCode::KeyW));
        assert_eq!(b.key_for(Action::ThrustForward), KeyCode::KeyI);
        assert_eq!(b.action_for(KeyCode::KeyW), Some(Action::PitchUp));
        // Reserved keys are refused.
        assert!(!b.bind(Action::YawLeft, KeyCode::Escape));
    }
}
