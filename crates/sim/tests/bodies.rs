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
    let [_, moon, _, _] = params.bodies(0.0);
    let up = DVec3::Y;
    let g = gravity_all(&params, 0.0, moon.centre + up * moon.radius_m);
    let down = -g.dot(up);
    assert!((down - 1.62).abs() < 0.02, "lunar surface gravity {down}");
}

#[test]
fn sun_surface_gravity_is_solar() {
    let params = presets::earth_compact();
    let [_, _, sun, _] = params.bodies(0.0);
    let up = -params.sun.dir;
    let g = gravity_all(&params, 0.0, sun.centre + up * sun.radius_m);
    let down = -g.dot(up);
    assert!((down - 274.0).abs() < 1.0, "solar surface gravity {down}");
}

#[test]
fn uranus_is_far_large_and_pulls_like_uranus() {
    let params = presets::earth_compact();
    let [_, _, sun, uranus] = params.bodies(0.0);
    let from_sun = (uranus.centre - sun.centre).length();
    // 19.2 AU at 1:100, against the Sun at 1 AU.
    assert!(
        (from_sun / params.sun.distance_m - 19.19).abs() < 0.05,
        "{from_sun}"
    );
    assert!(uranus.radius_m > 3.9 * params.planet.radius_m);
    let up = DVec3::Y;
    let g = gravity_all(&params, 0.0, uranus.centre + up * uranus.radius_m);
    let down = -g.dot(up);
    assert!((down - 8.69).abs() < 0.05, "uranus surface gravity {down}");
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
    let [_, moon, _, _] = params.bodies(0.0);
    let up = DVec3::Y;
    let mut state = at(
        &params,
        moon.centre + up * (moon.radius_m + 50.0),
        up * -200.0,
    );
    for _ in 0..600 {
        state = step(&params, &state, Controls::default());
        let [_, moon, _, _] = params.bodies(state.time_s);
        let r = (state.ship.pos_m - moon.centre).length();
        assert!(r >= moon.radius_m - 1e-6, "fell through the moon: {r}");
    }
}

/// Set down on the Moon, the ship stays set down: it rides along with the
/// Moon rather than being left behind by it.
#[test]
fn a_ship_landed_on_the_moon_goes_with_it() {
    let params = presets::earth_compact();
    let [_, moon, _, _] = params.bodies(0.0);
    let up = DVec3::Y;
    let mut state = at(
        &params,
        moon.centre + up * (moon.radius_m + 2.0),
        params.body_velocities(0.0)[1] - up * 1.0,
    );
    for _ in 0..2400 {
        state = step(&params, &state, Controls::default());
    }
    let [_, moon, _, _] = params.bodies(state.time_s);
    let rel = state.ship.pos_m - moon.centre;
    assert!(
        (rel.length() - moon.radius_m).abs() < 0.5,
        "not on the surface: {}",
        rel.length() - moon.radius_m
    );
    let v_rel = state.ship.vel_mps - params.body_velocities(state.time_s)[1];
    assert!(
        v_rel.length() < 0.5,
        "still moving over the ground at {} m/s",
        v_rel.length()
    );
    // And it has not drifted round the Moon: still under the +Y point.
    assert!(rel.normalize().dot(up) > 0.999, "{rel:?}");
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
