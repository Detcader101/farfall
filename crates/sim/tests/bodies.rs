//! The Moon and the Sun are bodies, not scenery: they pull and they are solid.

use farfall_sim::{gravity_all, presets, step, Controls, ShipState, WorldState};
use glam::{DQuat, DVec3};

fn at(params: &farfall_sim::WorldParams, pos: DVec3, vel: DVec3) -> WorldState {
    let mut state = presets::circular_orbit(params, 40_000.0);
    state.ship = ShipState {
        pos_m: pos,
        vel_mps: vel,
        orient: DQuat::IDENTITY,
        ang_vel_radps: DVec3::ZERO,
    };
    state
}

#[test]
fn moon_surface_gravity_is_lunar() {
    let params = presets::earth_compact();
    let [_, moon, _] = params.bodies(0.0);
    let up = DVec3::Y;
    let g = gravity_all(&params, 0.0, moon.centre + up * moon.radius_m);
    let down = -g.dot(up);
    assert!((down - 1.62).abs() < 0.02, "lunar surface gravity {down}");
}

#[test]
fn sun_surface_gravity_is_solar() {
    let params = presets::earth_compact();
    let [_, _, sun] = params.bodies(0.0);
    let up = -params.sun.dir;
    let g = gravity_all(&params, 0.0, sun.centre + up * sun.radius_m);
    let down = -g.dot(up);
    assert!((down - 274.0).abs() < 1.0, "solar surface gravity {down}");
}

/// The planet's frame falls with the Sun and the Moon, so at the planet they
/// exert only a tide — nothing a ship in low orbit would notice.
#[test]
fn other_bodies_are_only_a_tide_near_the_planet() {
    let params = presets::earth_compact();
    let pos = DVec3::new(params.planet.radius_m + 40_000.0, 0.0, 0.0);
    let all = gravity_all(&params, 0.0, pos);
    let planet = farfall_sim::gravity(&params.planet, pos);
    assert!((all - planet).length() < 1e-4, "tide {:?}", all - planet);
}

#[test]
fn the_moon_is_solid() {
    let params = presets::earth_compact();
    let [_, moon, _] = params.bodies(0.0);
    let up = DVec3::Y;
    let mut state = at(
        &params,
        moon.centre + up * (moon.radius_m + 50.0),
        up * -200.0,
    );
    for _ in 0..600 {
        state = step(&params, &state, Controls::default());
        let [_, moon, _] = params.bodies(state.time_s);
        let r = (state.ship.pos_m - moon.centre).length();
        assert!(r >= moon.radius_m - 1e-6, "fell through the moon: {r}");
    }
}

#[test]
fn the_moon_orbits_the_planet() {
    let params = presets::earth_compact();
    let period = params.moon.period_s(params.planet.mu);
    let a = params.moon.centre(params.planet.mu, 0.0);
    let b = params.moon.centre(params.planet.mu, period / 2.0);
    assert!(
        (a + b).length() < 1.0,
        "half a period is not opposite: {a:?} {b:?}"
    );
    assert!((a.length() - params.moon.orbit_m).abs() < 1e-3);
}
