//! The cockpit layout: which instrument sits in which slot on the glass.
//!
//! Instruments are drawn by shaders at an anchor on the canopy; the anchor
//! is all that "where" means. A slot is a named anchor, and the layout maps
//! every instrument to a slot — or to none, which hides it entirely. The
//! menu edits this; the settings file keeps it.

/// Every instrument the cockpit can show. Order is the settings-file and
/// menu order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instrument {
    Speed,
    Altitude,
    Gyro,
    Horizon,
    Trajectory,
    Readout,
}

impl Instrument {
    pub const ALL: [Instrument; 6] = [
        Instrument::Speed,
        Instrument::Altitude,
        Instrument::Gyro,
        Instrument::Horizon,
        Instrument::Trajectory,
        Instrument::Readout,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Instrument::Speed => "SPEED",
            Instrument::Altitude => "ALTITUDE",
            Instrument::Gyro => "GYRO",
            Instrument::Horizon => "HORIZON",
            Instrument::Trajectory => "PATH",
            Instrument::Readout => "READOUT",
        }
    }

    /// Settings-file key.
    pub fn key(self) -> &'static str {
        match self {
            Instrument::Speed => "speed",
            Instrument::Altitude => "altitude",
            Instrument::Gyro => "gyro",
            Instrument::Horizon => "horizon",
            Instrument::Trajectory => "path",
            Instrument::Readout => "readout",
        }
    }

    /// Dials live in slots; overlays (the horizon line, the path) are on
    /// or off — they are wherever the world puts them.
    pub fn slotted(self) -> bool {
        matches!(
            self,
            Instrument::Speed | Instrument::Altitude | Instrument::Gyro
        )
    }
}

/// A place on the glass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    BottomLeft,
    BottomCentre,
    BottomRight,
    MidLeft,
    MidRight,
    TopCentre,
    /// For overlays: shown.
    On,
    /// Hidden entirely.
    Off,
}

impl Slot {
    /// The choices a slotted instrument can cycle through.
    pub const DIALS: [Slot; 7] = [
        Slot::BottomLeft,
        Slot::BottomCentre,
        Slot::BottomRight,
        Slot::MidLeft,
        Slot::MidRight,
        Slot::TopCentre,
        Slot::Off,
    ];
    /// The choices an overlay can cycle through.
    pub const OVERLAYS: [Slot; 2] = [Slot::On, Slot::Off];

    pub fn name(self) -> &'static str {
        match self {
            Slot::BottomLeft => "LOW LEFT",
            Slot::BottomCentre => "LOW MID",
            Slot::BottomRight => "LOW RIGHT",
            Slot::MidLeft => "MID LEFT",
            Slot::MidRight => "MID RIGHT",
            Slot::TopCentre => "TOP MID",
            Slot::On => "ON",
            Slot::Off => "OFF",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Slot::BottomLeft => "low-left",
            Slot::BottomCentre => "low-mid",
            Slot::BottomRight => "low-right",
            Slot::MidLeft => "mid-left",
            Slot::MidRight => "mid-right",
            Slot::TopCentre => "top-mid",
            Slot::On => "on",
            Slot::Off => "off",
        }
    }

    pub fn from_key(key: &str) -> Option<Slot> {
        Slot::DIALS
            .iter()
            .chain(Slot::OVERLAYS.iter())
            .copied()
            .find(|s| s.key() == key)
    }

    /// Canopy anchor, NDC. `None` for the overlay states.
    pub fn anchor(self) -> Option<[f32; 2]> {
        match self {
            Slot::BottomLeft => Some([-0.52, -0.48]),
            Slot::BottomCentre => Some([0.0, -0.58]),
            Slot::BottomRight => Some([0.52, -0.48]),
            Slot::MidLeft => Some([-0.80, 0.0]),
            Slot::MidRight => Some([0.80, 0.0]),
            Slot::TopCentre => Some([0.0, 0.62]),
            Slot::On | Slot::Off => None,
        }
    }

    pub fn visible(self) -> bool {
        self != Slot::Off
    }
}

/// Instrument → slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    slots: [Slot; Instrument::ALL.len()],
}

impl Default for Layout {
    fn default() -> Self {
        let mut l = Self {
            slots: [Slot::Off; Instrument::ALL.len()],
        };
        l.set(Instrument::Speed, Slot::BottomRight);
        l.set(Instrument::Altitude, Slot::BottomLeft);
        l.set(Instrument::Gyro, Slot::BottomCentre);
        l.set(Instrument::Horizon, Slot::On);
        l.set(Instrument::Trajectory, Slot::On);
        l.set(Instrument::Readout, Slot::On);
        l
    }
}

impl Layout {
    pub fn get(&self, i: Instrument) -> Slot {
        self.slots[i as usize]
    }

    pub fn set(&mut self, i: Instrument, slot: Slot) {
        self.slots[i as usize] = slot;
    }

    /// Step an instrument to the next (or previous) choice in its cycle.
    pub fn cycle(&mut self, i: Instrument, forward: bool) {
        let choices: &[Slot] = if i.slotted() {
            &Slot::DIALS
        } else {
            &Slot::OVERLAYS
        };
        let cur = choices.iter().position(|&s| s == self.get(i)).unwrap_or(0);
        let n = choices.len();
        let next = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        self.set(i, choices[next]);
    }

    /// Anchor for a dial, if it is shown.
    pub fn anchor(&self, i: Instrument) -> Option<[f32; 2]> {
        self.get(i).anchor()
    }

    pub fn shown(&self, i: Instrument) -> bool {
        self.get(i).visible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_shows_the_cluster() {
        let l = Layout::default();
        assert!(l.anchor(Instrument::Speed).is_some());
        assert!(l.anchor(Instrument::Altitude).is_some());
        assert!(l.shown(Instrument::Horizon));
        assert!(l.shown(Instrument::Trajectory));
    }

    #[test]
    fn cycling_visits_off_and_returns() {
        let mut l = Layout::default();
        let start = l.get(Instrument::Speed);
        for _ in 0..Slot::DIALS.len() {
            l.cycle(Instrument::Speed, true);
        }
        assert_eq!(l.get(Instrument::Speed), start);
        let mut seen_off = false;
        for _ in 0..Slot::DIALS.len() {
            l.cycle(Instrument::Speed, true);
            seen_off |= !l.shown(Instrument::Speed);
        }
        assert!(seen_off);
        // Overlays only know on and off.
        l.cycle(Instrument::Horizon, true);
        assert!(!l.shown(Instrument::Horizon));
        l.cycle(Instrument::Horizon, true);
        assert!(l.shown(Instrument::Horizon));
    }

    #[test]
    fn slot_keys_round_trip() {
        for s in Slot::DIALS.iter().chain(Slot::OVERLAYS.iter()) {
            assert_eq!(Slot::from_key(s.key()), Some(*s));
        }
    }
}
