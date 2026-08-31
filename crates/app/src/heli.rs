//! Cold-war helicopters on the planet: pads hash-placed on the surface
//! (deterministic, like the belt's rocks — the same pad is always in the
//! same place), each with a generic utility helicopter parked on it. Land
//! the ship beside one and DISEMBARK's key boards it instead: the sim's
//! ship parameters swap to the helicopter's set (collective lift, cyclic
//! torque, no drives) and the fighter waits where it was left. The rocks'
//! rule holds here too: the helicopters are the app's, not the sim's —
//! the sim flies whatever parameters it is handed, and the golden hash
//! belongs to the fighter's.
//!
//! Nothing here is anyone's assets: the shapes are our own SDFs drawn in
//! `shaders/heli.wgsl`, the parameters our own numbers.

use glam::{DMat3, DQuat, DVec3};

use crate::belt::{hash, unit};
use farfall_sim::{Ground, ShipParams, ShipState, WorldParams, GEAR_HEIGHT_M};

/// How many pads the planet carries.
pub const PADS: usize = 12;
/// Pad 0 sits on the tuning coast — the bench's low-flight spot — so a
/// pilot following the stock spawn's ground track can find one.
pub const COAST_PAD_LAT_LON_DEG: (f64, f64) = (10.0, 320.0);
/// Board from this far away, metres: parked beside it, not on top of it.
pub const BOARD_RANGE_M: f64 = 80.0;
/// Helicopters draw out to here, metres.
pub const DRAW_M: f64 = 60_000.0;

/// A pad's place on the planet, degrees. Pad 0 is the coast; the rest
/// hash across the temperate band the way the belt's rocks hash their
/// cells — no stored state, the same world every run.
pub fn pad_lat_lon_deg(i: usize) -> (f64, f64) {
    if i == 0 {
        return COAST_PAD_LAT_LON_DEG;
    }
    let k = i as i64;
    let lat = -55.0 + 110.0 * unit(hash(k, 71, 9, 1));
    let lon = 360.0 * unit(hash(k, 71, 9, 2));
    (lat, lon)
}

/// The pad's outward normal (up) in the planet frame.
pub fn pad_up(i: usize) -> DVec3 {
    let (lat, lon) = pad_lat_lon_deg(i);
    let (lat, lon) = (lat.to_radians(), lon.to_radians());
    DVec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin())
}

/// Each pad's helicopter faces its own way — hashed, like everything else.
pub fn pad_heading_rad(i: usize) -> f64 {
    core::f64::consts::TAU * unit(hash(i as i64, 71, 9, 3))
}

/// The helicopter parked on pad `i`, as a sim state: on its gear, LANDED,
/// nose along the pad's heading. The same stance height as the fighter
/// (the sim's contact is at [`GEAR_HEIGHT_M`] for every hull).
pub fn parked(params: &WorldParams, i: usize) -> ShipState {
    let up = pad_up(i);
    let pos = up * (params.planet.radius_m + GEAR_HEIGHT_M);
    // A stable east-ish reference frame on the sphere, then the heading.
    let east = DVec3::Y.cross(up).normalize_or_zero();
    let east = if east.length_squared() < 1e-9 {
        DVec3::X
    } else {
        east
    };
    let north = up.cross(east);
    let h = pad_heading_rad(i);
    let nose = east * h.cos() + north * h.sin();
    let right = nose.cross(up).normalize();
    ShipState {
        pos_m: pos,
        vel_mps: DVec3::ZERO,
        orient: DQuat::from_mat3(&DMat3::from_cols(right, up, -nose)),
        ang_vel_radps: DVec3::ZERO,
        ground: Ground::Landed { body: 0, up },
    }
}

/// The helicopter's own numbers, our own: a ~4 t utility hull. The main
/// rotor is thrust along +Y alone — hover at Earth gravity sits at ~61%
/// collective — the cyclic is pitch/roll torque, the tail rotor yaw. No
/// main engine, no boost, no drives; the air damps rotation hard.
pub fn heli_params() -> ShipParams {
    ShipParams {
        mass_kg: 4_300.0,
        // Rotor thrust is vertical only: no lateral or main-engine axes.
        max_thrust_mps2: DVec3::new(0.0, 16.0, 0.0),
        boost_multiplier: 1.0,
        // No thruster cluster for the flight computer to steer with.
        align_authority: 0.0,
        align_tau_s: 1.0,
        // A gentle speed bleed stands in for the rotor disc's own drag.
        brake_mps2: 6.0,
        brake_tau_s: 1.2,
        despin_tau_s: 0.8,
        // The drives do not exist in this hull; the demand is stripped
        // before the sim ever sees it (route_controls), this is belt and
        // braces.
        hyper_max_mps: 0.0,
        hyper_tau_s: 4.0,
        ground_restitution: 0.0,
        // Skids grip: almost all tangential speed gone within a second.
        ground_friction: 0.02,
        // Cyclic authority: pitch and roll answer, the pedals are slower.
        max_torque_radps2: DVec3::new(1.2, 0.9, 1.5),
        // A draggy blob, near the same from every side — a helicopter
        // does not weathervane like a dart, it just slows down.
        cd_area_m2: 12.0,
        cd_area_side_m2: 16.0,
        lift_area_m2: 0.0,
        // Pressure on gravity: the air exerts no lever on the attitude.
        centre_of_pressure_m: 0.0,
        centre_of_gravity_m: 0.0,
        inertia_kgm2: DVec3::new(14_000.0, 16_000.0, 9_000.0),
        // Strong rotational damping once there is any airspeed at all.
        aero_damping_m4: 9_000.0,
    }
}

/// Helicopter inputs from the ship's controls: the forward-thrust axis
/// (the throttle lever, W/S on the keys) becomes collective — thrust up
/// the mast, never down — the vertical axis adds to it, cyclic and pedals
/// ride the torque demand unchanged, and the drives do not come: no
/// boost, no hyper. The brake stays — a speed bleed, not a drive.
pub fn route_controls(c: farfall_sim::Controls) -> farfall_sim::Controls {
    // Body frame: -Z is the nose, so a forward demand is negative z.
    let collective = ((-c.thrust_body.z) + c.thrust_body.y).clamp(0.0, 1.0);
    farfall_sim::Controls {
        thrust_body: DVec3::new(0.0, collective, 0.0),
        torque_body: c.torque_body,
        assist: c.assist,
        boost: false,
        brake: c.brake,
        despin: c.despin,
        hyper: false,
        hyper_level: 0.0,
    }
}

/// Where every helicopter is and who is flying: the app's book-keeping,
/// none of it the sim's.
#[derive(Debug, Clone, Default)]
pub struct Helis {
    /// The pilot is in a helicopter (the sim's ship IS the helicopter).
    pub in_heli: bool,
    /// Which pad's helicopter is being flown.
    pub flying_pad: Option<usize>,
    /// The fighter, waiting where it was left.
    pub fighter: Option<ShipState>,
    /// Helicopters set down off their pads: (pad, where it rests now).
    pub displaced: Vec<(usize, ShipState)>,
}

impl Helis {
    /// Pad `i`'s helicopter as it stands: where it was last set down, or
    /// parked on its pad if it never moved. None while it is being flown.
    pub fn heli_state(&self, params: &WorldParams, i: usize) -> Option<ShipState> {
        if self.in_heli && self.flying_pad == Some(i) {
            return None;
        }
        Some(
            self.displaced
                .iter()
                .find(|(p, _)| *p == i)
                .map(|(_, s)| *s)
                .unwrap_or_else(|| parked(params, i)),
        )
    }

    /// The nearest boardable helicopter within [`BOARD_RANGE_M`] of `pos`.
    pub fn nearest_heli(&self, params: &WorldParams, pos: DVec3) -> Option<(usize, ShipState)> {
        let mut best: Option<(f64, usize, ShipState)> = None;
        for i in 0..PADS {
            let Some(s) = self.heli_state(params, i) else {
                continue;
            };
            let d = (s.pos_m - pos).length();
            if d <= BOARD_RANGE_M && best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, i, s));
            }
        }
        best.map(|(_, i, s)| (i, s))
    }

    /// Board pad `i`'s helicopter: the fighter parks where it stands, the
    /// helicopter's state comes back to become the sim's.
    pub fn board(&mut self, params: &WorldParams, i: usize, fighter: ShipState) -> ShipState {
        let s = self
            .heli_state(params, i)
            .unwrap_or_else(|| parked(params, i));
        self.displaced.retain(|(p, _)| *p != i);
        self.fighter = Some(fighter);
        self.in_heli = true;
        self.flying_pad = Some(i);
        s
    }

    /// Leave the helicopter where it is and take the fighter back.
    pub fn disembark(&mut self, heli: ShipState) -> Option<ShipState> {
        let fighter = self.fighter.take()?;
        if let Some(pad) = self.flying_pad.take() {
            self.displaced.retain(|(p, _)| *p != pad);
            self.displaced.push((pad, heli));
        }
        self.in_heli = false;
        Some(fighter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farfall_sim::{presets, Controls, DT};

    #[test]
    fn the_pads_are_where_the_hash_says_every_time() {
        for i in 0..PADS {
            assert_eq!(pad_lat_lon_deg(i), pad_lat_lon_deg(i));
            let (lat, _) = pad_lat_lon_deg(i);
            assert!((-60.0..=60.0).contains(&lat), "pad {i} off the band");
        }
        assert_eq!(pad_lat_lon_deg(0), COAST_PAD_LAT_LON_DEG);
        let params = presets::earth_compact();
        let s = parked(&params, 0);
        assert!(
            (s.pos_m.length() - (params.planet.radius_m + GEAR_HEIGHT_M)).abs() < 1e-6,
            "parked on its gear at the surface"
        );
        assert!(matches!(s.ground, Ground::Landed { body: 0, .. }));
        // Its own up really is the pad's outward normal.
        let up = s.orient * DVec3::Y;
        assert!(up.dot(pad_up(0)) > 0.999_999);
    }

    #[test]
    fn a_heli_hovers_when_collective_balances_gravity() {
        let mut params = presets::earth_compact();
        params.ship = heli_params();
        // Hovering 30 m over the coast pad, level.
        let mut state = farfall_sim::WorldState {
            time_s: 0.0,
            ship: parked(&params, 0),
        };
        state.ship.ground = Ground::Flight;
        state.ship.pos_m += pad_up(0) * 30.0;
        let g = 9.81;
        let hover = g / params.ship.max_thrust_mps2.y;
        // The pilot's demand: full forward-axis would be a climb; the
        // routed collective at the hover fraction holds the sky still.
        let c = route_controls(Controls {
            thrust_body: DVec3::new(0.0, 0.0, -hover),
            assist: false,
            ..Default::default()
        });
        let start = state.ship.pos_m;
        for _ in 0..240 {
            state = farfall_sim::step(&params, &state, c);
        }
        // Two seconds of hover: height held to well under a metre (drag
        // and the sphere's curvature are the only leaks).
        let risen = (state.ship.pos_m.length() - start.length()).abs();
        assert!(risen < 0.5, "hover drifted {risen} m in 2 s");
        assert!(state.ship.vel_mps.length() < 0.5);
    }

    #[test]
    fn boarding_swaps_params_and_disembark_swaps_back() {
        let params = presets::earth_compact();
        let fighter_ship = params.ship;
        let mut helis = Helis::default();
        // The fighter lands beside pad 3's helicopter.
        let heli_at = parked(&params, 3);
        let mut fighter = heli_at;
        fighter.pos_m += fighter.orient * DVec3::new(30.0, 0.0, 0.0);
        assert_eq!(
            helis.nearest_heli(&params, fighter.pos_m).map(|(i, _)| i),
            Some(3)
        );
        let heli = helis.board(&params, 3, fighter);
        assert!(helis.in_heli);
        assert_eq!(heli.pos_m, heli_at.pos_m, "the sim takes the heli's seat");
        // The app swaps the ship parameters at the same moment: the heli's
        // set is not the fighter's.
        assert_ne!(heli_params().max_thrust_mps2, fighter_ship.max_thrust_mps2);
        // Set the helicopter down somewhere else and walk back.
        let mut heli_after = heli;
        heli_after.pos_m += heli_after.orient * DVec3::new(0.0, 0.0, -500.0);
        let back = helis.disembark(heli_after).expect("the fighter waited");
        assert_eq!(
            back.pos_m, fighter.pos_m,
            "the fighter is where it was left"
        );
        assert!(!helis.in_heli);
        // The helicopter now rests where it was set down, not on its pad.
        let rest = helis.heli_state(&params, 3).unwrap();
        assert_eq!(rest.pos_m, heli_after.pos_m);
    }

    #[test]
    fn the_drives_are_inert_in_the_helicopter() {
        let c = route_controls(Controls {
            thrust_body: DVec3::new(1.0, 0.0, -1.0),
            boost: true,
            hyper: true,
            hyper_level: 1.0,
            ..Default::default()
        });
        assert!(!c.boost && !c.hyper, "boost and the hyper field stripped");
        assert_eq!(c.hyper_level, 0.0);
        assert_eq!(
            c.thrust_body,
            DVec3::new(0.0, 1.0, 0.0),
            "all thrust goes up the mast"
        );
        // And the collective never pushes down.
        let down = route_controls(Controls {
            thrust_body: DVec3::new(0.0, 0.0, 1.0),
            ..Default::default()
        });
        assert_eq!(down.thrust_body, DVec3::ZERO);
        let _ = DT;
    }
}
