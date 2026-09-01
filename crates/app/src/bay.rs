//! The SHIP bay: what the ship is fitted with, and the hologram the pilot
//! fits it by. The hardpoints and their mounts are the ship's real
//! loadout — [`crate::arms`] fires from whatever is mounted — and the
//! rest is the picture: a small ship over the dash, turned by hand.

use crate::arms::Weapon;

/// The airframe the pilot's own ship wears (SPEC §6.5b): a parameter and
/// a silhouette, never sim state — the golden hash does not know it
/// exists. The pads' cold-war hulls are not this; they stay their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Craft {
    #[default]
    Fighter,
    Helicopter,
}

impl Craft {
    pub const ALL: [Craft; 2] = [Craft::Fighter, Craft::Helicopter];

    pub fn name(self) -> &'static str {
        match self {
            Craft::Fighter => "FIGHTER",
            Craft::Helicopter => "HELICOPTER",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Craft::Fighter => "fighter",
            Craft::Helicopter => "helicopter",
        }
    }

    pub fn from_key(k: &str) -> Option<Craft> {
        Craft::ALL.iter().copied().find(|c| c.key() == k)
    }

    /// The craft flag every drawing lane's uniforms carry: 0 the fighter,
    /// 1 the helicopter (common.wgsl sd_craft_exterior).
    pub fn kind(self) -> f32 {
        match self {
            Craft::Fighter => 0.0,
            Craft::Helicopter => 1.0,
        }
    }

    /// The next craft round, for the menu.
    pub fn next(self, forward: bool) -> Craft {
        let i = Craft::ALL.iter().position(|&c| c == self).unwrap_or(0);
        let n = Craft::ALL.len();
        Craft::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }
}

/// Where things can be mounted, ship frame (x right, y up, -z the nose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hardpoint {
    Nose,
    WingL,
    WingR,
    Belly,
}

impl Hardpoint {
    pub const ALL: [Hardpoint; 4] = [
        Hardpoint::Nose,
        Hardpoint::WingL,
        Hardpoint::WingR,
        Hardpoint::Belly,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Hardpoint::Nose => "NOSE",
            Hardpoint::WingL => "WING L",
            Hardpoint::WingR => "WING R",
            Hardpoint::Belly => "BELLY",
        }
    }

    /// The muzzle's place, ship frame, metres. The wing points sit at
    /// the nose of the outrigger booms under the wings
    /// (common.wgsl sd_fighter_exterior draws the boom), so a wing gun
    /// is carried on the airframe instead of floating at its muzzle.
    pub fn pos(self) -> glam::DVec3 {
        match self {
            Hardpoint::Nose => glam::DVec3::new(0.0, -0.45, -4.2),
            Hardpoint::WingL => glam::DVec3::new(-2.6, -1.0, 0.9),
            Hardpoint::WingR => glam::DVec3::new(2.6, -1.0, 0.9),
            Hardpoint::Belly => glam::DVec3::new(0.0, -1.95, 1.4),
        }
    }
}

/// What a hardpoint carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mount {
    #[default]
    Empty,
    Cannon,
    Rail,
}

impl Mount {
    pub const ALL: [Mount; 3] = [Mount::Empty, Mount::Cannon, Mount::Rail];

    pub fn name(self) -> &'static str {
        match self {
            Mount::Empty => "EMPTY",
            Mount::Cannon => "CANNON",
            Mount::Rail => "RAIL",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mount::Empty => "empty",
            Mount::Cannon => "cannon",
            Mount::Rail => "rail",
        }
    }

    pub fn from_key(k: &str) -> Option<Mount> {
        Mount::ALL.iter().copied().find(|m| m.key() == k)
    }

    pub fn weapon(self) -> Option<Weapon> {
        match self {
            Mount::Empty => None,
            Mount::Cannon => Some(Weapon::Cannon),
            Mount::Rail => Some(Weapon::Rail),
        }
    }

    /// The pass's kind: 0 empty, 1 cannon, 2 rail.
    pub fn kind(self) -> u8 {
        match self {
            Mount::Empty => 0,
            Mount::Cannon => 1,
            Mount::Rail => 2,
        }
    }

    /// The next mount round, for the menu.
    pub fn next(self, forward: bool) -> Mount {
        let i = Mount::ALL.iter().position(|&m| m == self).unwrap_or(0);
        let n = Mount::ALL.len();
        Mount::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }
}

/// The stock fit: a rail on the nose, a cannon on each wing.
pub const STOCK: [Mount; 4] = [Mount::Rail, Mount::Cannon, Mount::Cannon, Mount::Empty];

/// The fit as every pass draws it: each hardpoint's place from the one
/// table ([`Hardpoint::pos`]) with what the bay mounted there. The SHIP
/// bay's hologram, the cockpit's own airframe and the chase view all
/// read this — one source, so the glass and the bay cannot disagree.
pub fn fit_views(mounts: &[Mount; 4]) -> [farfall_render::hologram::MountView; 4] {
    let mut v = [farfall_render::hologram::MountView {
        at: glam::Vec3::ZERO,
        kind: 0,
    }; 4];
    for ((slot, h), m) in v.iter_mut().zip(Hardpoint::ALL.iter()).zip(mounts.iter()) {
        *slot = farfall_render::hologram::MountView {
            at: h.pos().as_vec3(),
            kind: m.kind(),
        };
    }
    v
}

/// The orbit camera's reach, ship metres from the hull's middle.
pub const DIST_MIN: f32 = 22.0;
pub const DIST_MAX: f32 = 110.0;
pub const DIST_STOCK: f32 = 48.0;
/// The bay's slow yaw when nobody is turning it, rad/s.
pub const SPIN_RADPS: f32 = 0.25;
/// How long a drag holds the spin off, seconds.
const SPIN_REST_S: f32 = 2.5;

/// The orbit: yaw round the hull's up, pitch above the deck, distance
/// out — the map camera's pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BayView {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    /// Seconds until the slow yaw resumes after a drag.
    rest_s: f32,
}

impl Default for BayView {
    fn default() -> Self {
        // Three-quarter from the front left, seen a little from above.
        Self {
            yaw: 0.75,
            pitch: 0.30,
            dist: DIST_STOCK,
            rest_s: 0.0,
        }
    }
}

impl BayView {
    const DRAG_RATE: f32 = 0.008;

    pub fn drag(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * Self::DRAG_RATE;
        self.pitch = (self.pitch - dy * Self::DRAG_RATE).clamp(-1.2, 1.2);
        self.rest_s = SPIN_REST_S;
    }

    pub fn zoom_by(&mut self, notches: f32) {
        self.dist = (self.dist / 1.15f32.powf(notches)).clamp(DIST_MIN, DIST_MAX);
    }

    /// The slow yaw, when it is on and the hand is off.
    pub fn tick(&mut self, dt: f32, spin: bool) {
        if self.rest_s > 0.0 {
            self.rest_s = (self.rest_s - dt).max(0.0);
        } else if spin {
            self.yaw = (self.yaw + SPIN_RADPS * dt).rem_euclid(std::f32::consts::TAU);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pass_reads_the_same_fit_table() {
        // The glass, the bay and the chase view all draw from this one
        // mapping: each hardpoint's true place with its mounted kind.
        let fit = fit_views(&[Mount::Rail, Mount::Cannon, Mount::Empty, Mount::Cannon]);
        for (i, h) in Hardpoint::ALL.iter().enumerate() {
            assert_eq!(fit[i].at, h.pos().as_vec3(), "{}", h.name());
        }
        assert_eq!(
            [fit[0].kind, fit[1].kind, fit[2].kind, fit[3].kind],
            [2, 1, 0, 1],
            "kinds ride with their hardpoints"
        );
    }

    #[test]
    fn the_craft_cycles_and_names_its_keys() {
        assert_eq!(Craft::default(), Craft::Fighter, "the fighter is stock");
        assert_eq!(Craft::Fighter.next(true), Craft::Helicopter);
        assert_eq!(Craft::Helicopter.next(true), Craft::Fighter);
        assert_eq!(Craft::Fighter.next(false), Craft::Helicopter);
        for c in Craft::ALL {
            assert_eq!(Craft::from_key(c.key()), Some(c));
        }
        assert_eq!(Craft::from_key("huey"), None);
        assert_eq!(Craft::Fighter.kind(), 0.0);
        assert_eq!(Craft::Helicopter.kind(), 1.0);
    }

    #[test]
    fn mounts_cycle_and_name_their_keys() {
        assert_eq!(Mount::Empty.next(true), Mount::Cannon);
        assert_eq!(Mount::Rail.next(true), Mount::Empty);
        assert_eq!(Mount::Empty.next(false), Mount::Rail);
        for m in Mount::ALL {
            assert_eq!(Mount::from_key(m.key()), Some(m));
        }
        assert_eq!(Mount::from_key("laser"), None);
        assert_eq!(Mount::Cannon.weapon(), Some(Weapon::Cannon));
        assert_eq!(STOCK[0], Mount::Rail);
        for (i, h) in Hardpoint::ALL.iter().enumerate() {
            assert_eq!(Hardpoint::ALL[i], *h);
            assert!(h.pos().z < 2.0, "mounts sit forward of the engines");
        }
    }

    #[test]
    fn the_orbit_turns_tilts_zooms_and_spins_when_left_alone() {
        let mut v = BayView::default();
        let y0 = v.yaw;
        v.drag(100.0, 0.0);
        assert!(v.yaw != y0);
        v.drag(0.0, -10_000.0);
        assert_eq!(v.pitch, 1.2);
        v.zoom_by(100.0);
        assert_eq!(v.dist, DIST_MIN);
        v.zoom_by(-200.0);
        assert_eq!(v.dist, DIST_MAX);
        // A drag holds the spin off for a moment; then it resumes, and
        // only when it is on.
        let y = v.yaw;
        v.tick(0.5, true);
        assert_eq!(v.yaw, y, "resting after the drag");
        v.tick(10.0, true);
        v.tick(1.0, true);
        assert!(
            (v.yaw - (y + SPIN_RADPS)).abs() < 1e-5,
            "spinning: {}",
            v.yaw
        );
        let y = v.yaw;
        v.tick(1.0, false);
        assert_eq!(v.yaw, y, "spin off");
    }
}
