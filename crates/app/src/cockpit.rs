//! The cockpit layout: which instrument sits in which slot on the glass.
//!
//! Instruments are drawn by shaders at an anchor on the canopy; the anchor
//! is all that "where" means. A slot is a named anchor, and the layout maps
//! every instrument to a slot — or to none, which hides it entirely — or,
//! once the pilot has dragged it, to a free anchor of their own anywhere on
//! the glass. The menu edits the slots; a drag sets the free anchor (and
//! cycling the slot again lets go of it); the settings file keeps both.

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
    GForce,
    /// The hoops on the path (the ribbon stays).
    Hoops,
    /// The womp a passing hoop makes.
    HoopSound,
    /// The horizon's pitch ladder (the level line stays).
    Ladder,
    /// Finder rings around the Moon and the Sun.
    BodyTags,
}

impl Instrument {
    pub const ALL: [Instrument; 11] = [
        Instrument::Speed,
        Instrument::Altitude,
        Instrument::Gyro,
        Instrument::GForce,
        Instrument::Horizon,
        Instrument::Ladder,
        Instrument::Trajectory,
        Instrument::Hoops,
        Instrument::HoopSound,
        Instrument::BodyTags,
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
            Instrument::GForce => "G METER",
            Instrument::Hoops => "PATH HOOPS",
            Instrument::HoopSound => "HOOP SOUND",
            Instrument::Ladder => "PITCH LADDER",
            Instrument::BodyTags => "BODY TAGS",
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
            Instrument::GForce => "g-meter",
            Instrument::Hoops => "hoops",
            Instrument::HoopSound => "hoop-sound",
            Instrument::Ladder => "ladder",
            Instrument::BodyTags => "body-tags",
        }
    }

    /// Dials live in slots; overlays (the horizon line, the path) are on
    /// or off — they are wherever the world puts them.
    pub fn slotted(self) -> bool {
        matches!(
            self,
            Instrument::Speed | Instrument::Altitude | Instrument::Gyro | Instrument::GForce
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

    /// Canopy anchor, NDC, before the safe-edge inset. Out near the rim of
    /// the glass: the middle of the view is for the world. `None` for the
    /// overlay states.
    pub fn anchor(self) -> Option<[f32; 2]> {
        match self {
            Slot::BottomLeft => Some([-0.78, -0.64]),
            Slot::BottomCentre => Some([0.0, -0.74]),
            Slot::BottomRight => Some([0.78, -0.64]),
            Slot::MidLeft => Some([-0.88, 0.02]),
            Slot::MidRight => Some([0.88, 0.02]),
            Slot::TopCentre => Some([0.0, 0.74]),
            Slot::On | Slot::Off => None,
        }
    }

    pub fn visible(self) -> bool {
        self != Slot::Off
    }
}

/// Instrument → slot, and the safe edge: a fraction of the screen kept
/// clear at the rim, for a display whose edges are hidden or bent. Every
/// anchor is pulled toward the centre by it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    slots: [Slot; Instrument::ALL.len()],
    /// A dragged dial's own anchor (canopy NDC, before the safe edge),
    /// overriding its slot's while set.
    free: [Option<[f32; 2]>; Instrument::ALL.len()],
    pub safe_edge: f32,
}

/// How far out a free anchor may go, NDC: just short of the rim.
pub const FREE_LIMIT: f32 = 0.95;

pub const SAFE_EDGE_MAX: f32 = 0.30;

impl Default for Layout {
    fn default() -> Self {
        let mut l = Self {
            slots: [Slot::Off; Instrument::ALL.len()],
            free: [None; Instrument::ALL.len()],
            safe_edge: 0.0,
        };
        l.set(Instrument::Speed, Slot::BottomRight);
        l.set(Instrument::Altitude, Slot::BottomLeft);
        l.set(Instrument::Gyro, Slot::BottomCentre);
        l.set(Instrument::GForce, Slot::MidRight);
        l.set(Instrument::Horizon, Slot::On);
        l.set(Instrument::Trajectory, Slot::On);
        l.set(Instrument::Readout, Slot::On);
        l.set(Instrument::Hoops, Slot::On);
        l.set(Instrument::HoopSound, Slot::On);
        l.set(Instrument::Ladder, Slot::On);
        l.set(Instrument::BodyTags, Slot::On);
        l
    }
}

impl Layout {
    pub fn get(&self, i: Instrument) -> Slot {
        self.slots[i as usize]
    }

    /// Assign a slot. Lets go of any free anchor: the slot is the pilot's
    /// new word on where it goes.
    pub fn set(&mut self, i: Instrument, slot: Slot) {
        self.slots[i as usize] = slot;
        self.free[i as usize] = None;
    }

    /// Put a dial at an anchor of the pilot's own (canopy NDC, before the
    /// safe edge), clamped to the glass. Only a shown dial can be placed;
    /// its slot is kept underneath for the menu to cycle from.
    pub fn set_free(&mut self, i: Instrument, anchor: [f32; 2]) {
        if !i.slotted() || !self.shown(i) || !anchor[0].is_finite() || !anchor[1].is_finite() {
            return;
        }
        self.free[i as usize] = Some([
            anchor[0].clamp(-FREE_LIMIT, FREE_LIMIT),
            anchor[1].clamp(-FREE_LIMIT, FREE_LIMIT),
        ]);
    }

    /// The free anchor, if the dial has been dragged.
    pub fn free(&self, i: Instrument) -> Option<[f32; 2]> {
        self.free[i as usize]
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

    /// Anchor for a dial, if it is shown, inset by the safe edge: the free
    /// anchor if it has one, else its slot's.
    pub fn anchor(&self, i: Instrument) -> Option<[f32; 2]> {
        let slot = self.get(i);
        if !slot.visible() {
            return None;
        }
        self.free[i as usize]
            .or_else(|| slot.anchor())
            .map(|a| self.inset(a))
    }

    /// The inverse of [`Layout::inset`]: a point on the glass back to the
    /// anchor that lands there.
    pub fn uninset(&self, a: [f32; 2]) -> [f32; 2] {
        let k = 1.0 - self.safe_edge.clamp(0.0, SAFE_EDGE_MAX);
        [a[0] / k, a[1] / k]
    }

    /// Pull a canopy point toward the centre by the safe edge.
    pub fn inset(&self, a: [f32; 2]) -> [f32; 2] {
        let k = 1.0 - self.safe_edge.clamp(0.0, SAFE_EDGE_MAX);
        [a[0] * k, a[1] * k]
    }

    pub fn set_safe_edge(&mut self, f: f32) {
        self.safe_edge = if f.is_finite() {
            f.clamp(0.0, SAFE_EDGE_MAX)
        } else {
            0.0
        };
    }

    pub fn shown(&self, i: Instrument) -> bool {
        self.get(i).visible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dragged_dial_keeps_its_own_anchor_until_the_slot_is_cycled() {
        let mut l = Layout::default();
        l.set_safe_edge(0.1);
        let slot_anchor = l.anchor(Instrument::Speed).unwrap();
        l.set_free(Instrument::Speed, [0.2, -0.3]);
        let a = l.anchor(Instrument::Speed).unwrap();
        assert!(
            (a[0] - 0.18).abs() < 1e-6 && (a[1] + 0.27).abs() < 1e-6,
            "{a:?}"
        );
        assert_ne!(a, slot_anchor);
        let back = l.uninset(a);
        assert!((back[0] - 0.2).abs() < 1e-6 && (back[1] + 0.3).abs() < 1e-6);
        // Clamped to the glass; hidden dials, overlays and garbage refused.
        l.set_free(Instrument::Speed, [5.0, -5.0]);
        assert_eq!(l.free(Instrument::Speed), Some([FREE_LIMIT, -FREE_LIMIT]));
        l.set_free(Instrument::Speed, [f32::NAN, 0.0]);
        assert_eq!(l.free(Instrument::Speed), Some([FREE_LIMIT, -FREE_LIMIT]));
        l.set_free(Instrument::Horizon, [0.1, 0.1]);
        assert_eq!(l.free(Instrument::Horizon), None);
        l.set(Instrument::Gyro, Slot::Off);
        l.set_free(Instrument::Gyro, [0.1, 0.1]);
        assert_eq!(l.free(Instrument::Gyro), None);
        // Cycling the slot lets go of the free anchor.
        l.cycle(Instrument::Speed, true);
        assert_eq!(l.free(Instrument::Speed), None);
        assert_eq!(
            l.anchor(Instrument::Speed).is_some(),
            l.shown(Instrument::Speed)
        );
    }

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
    fn safe_edge_pulls_anchors_inward_and_is_bounded() {
        let mut l = Layout::default();
        let a = l.anchor(Instrument::Speed).unwrap();
        l.set_safe_edge(0.1);
        let b = l.anchor(Instrument::Speed).unwrap();
        assert!(b[0].abs() < a[0].abs() && b[1].abs() < a[1].abs());
        assert!((b[0] - a[0] * 0.9).abs() < 1e-6);
        l.set_safe_edge(9.0);
        assert_eq!(l.safe_edge, SAFE_EDGE_MAX);
        l.set_safe_edge(f32::NAN);
        assert_eq!(l.safe_edge, 0.0);
    }

    #[test]
    fn dials_sit_near_the_rim() {
        for s in Slot::DIALS {
            if let Some(a) = s.anchor() {
                assert!(
                    a[0].abs().max(a[1].abs()) >= 0.74,
                    "{s:?} is too central: {a:?}"
                );
            }
        }
    }

    #[test]
    fn slot_keys_round_trip() {
        for s in Slot::DIALS.iter().chain(Slot::OVERLAYS.iter()) {
            assert_eq!(Slot::from_key(s.key()), Some(*s));
        }
    }
}
