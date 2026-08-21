//! The settings file: graphics, controls, cockpit layout.
//!
//! Plain `key = value` lines at `~/.farfall/settings.cfg`, written whole on
//! every change from the menu and read once at start. No format crate: a
//! file a pilot can fix with any editor, and nothing to get wrong in it
//! that the defaults can't cover — unknown keys are ignored, bad values
//! fall back, missing lines mean default.

use crate::cockpit::{Instrument, Layout, Slot};
use crate::input::{key_from_name, key_name, Action, Bindings};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub msaa: u32,
    pub scale: f32,
    pub vsync: bool,
    pub bindings: Bindings,
    pub layout: Layout,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            msaa: 4,
            scale: 1.0,
            vsync: true,
            bindings: Bindings::default(),
            layout: Layout::default(),
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
                        if let (Some(inst), Some(slot)) = (
                            Instrument::ALL.iter().copied().find(|i| i.key() == name),
                            Slot::from_key(v),
                        ) {
                            // A dial cannot be "on" and an overlay has no
                            // slot: keep each to its own choices.
                            let valid = if inst.slotted() {
                                Slot::DIALS.contains(&slot)
                            } else {
                                Slot::OVERLAYS.contains(&slot)
                            };
                            if valid {
                                s.layout.set(inst, slot);
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
        for i in Instrument::ALL {
            out.push_str(&format!("ui.{} = {}\n", i.key(), self.layout.get(i).key()));
        }
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
