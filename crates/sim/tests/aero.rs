//! Aerodynamics: the shape and the balance (SPEC §7.2).
//!
//! Two independent variables decide what the air does. The SHAPE — nose-on
//! against broadside drag, and lift — sets the forces. The BALANCE — centre
//! of pressure against centre of gravity — sets the torque. These tests pin
//! each of them separately, and pin that vacuum is still vacuum.

use farfall_sim::{aero_forces, atmo_density, presets, step, Controls, WorldParams, WorldState};
use glam::{DQuat, DVec3};

fn run(params: &WorldParams, mut state: WorldState, steps: u64, controls: Controls) -> WorldState {
    for _ in 0..steps {
        state = step(params, &state, controls);
    }
    state
}

/// A ship deep in the air, flying prograde, with its nose pitched by
/// `pitch_rad` away from the airflow (positive = nose up, above the velocity).
fn in_air_pitched(params: &WorldParams, pitch_rad: f64) -> WorldState {
    let mut s = presets::circular_orbit(params, 2_000.0);
    let nose_axis = s.ship.orient * DVec3::X;
    s.ship.orient = DQuat::from_axis_angle(nose_axis, pitch_rad) * s.ship.orient;
    s
}

/// Angle between the nose and the velocity, radians.
fn alpha(state: &WorldState) -> f64 {
    let nose = state.ship.orient * DVec3::NEG_Z;
    nose.dot(state.ship.vel_mps.normalize())
        .clamp(-1.0, 1.0)
        .acos()
}

/// Rotate the ship's velocity direction without touching its attitude.
fn with_velocity_dir(mut s: WorldState, dir: DVec3) -> WorldState {
    let speed = s.ship.vel_mps.length();
    s.ship.vel_mps = dir.normalize() * speed;
    s
}

// ------------------------------------------------------------------ shape

/// Broadside, the hull is far draggier than nose-on. The shape, directly.
#[test]
fn broadside_drags_more_than_nose_on() {
    let p = presets::earth_compact();
    let nose_on = in_air_pitched(&p, 0.0);
    let broadside = in_air_pitched(&p, std::f64::consts::FRAC_PI_2);
    let rho = atmo_density(&p.planet, nose_on.ship.pos_m.length());
    let a_nose = aero_forces(&p.ship, rho, &nose_on.ship)
        .accel_world
        .length();
    let a_side = aero_forces(&p.ship, rho, &broadside.ship)
        .accel_world
        .length();
    assert!(
        a_side > a_nose * 5.0,
        "broadside {a_side} should dwarf nose-on {a_nose}"
    );
}

/// Grazing the air nose-first bleeds forward speed — the old isotropic
/// contract, kept.
#[test]
fn grazing_the_atmosphere_reduces_forward_speed() {
    let p = presets::earth_compact();
    let mut s = presets::circular_orbit(&p, 6_000.0);
    let v0 = s.ship.vel_mps.length();
    s = run(&p, s, 600, Controls::default());
    assert!(s.ship.vel_mps.length() < v0);
}

/// Nose above the airflow, the hull lifts: an acceleration component away
/// from the airflow on the nose's side, perpendicular to the velocity.
#[test]
fn pitching_the_nose_up_produces_lift() {
    let p = presets::earth_compact();
    let s = in_air_pitched(&p, 0.2);
    let rho = atmo_density(&p.planet, s.ship.pos_m.length());
    let a = aero_forces(&p.ship, rho, &s.ship).accel_world;
    let v_hat = s.ship.vel_mps.normalize();
    let nose = s.ship.orient * DVec3::NEG_Z;
    let lift_dir = (nose - v_hat * nose.dot(v_hat)).normalize();
    assert!(a.dot(lift_dir) > 0.0, "no lift toward the nose side");
    assert!(a.dot(v_hat) < 0.0, "drag still has to oppose the motion");
}

/// Broadside the plate stalls: no lift at 90 degrees, only drag.
#[test]
fn broadside_has_no_lift() {
    let p = presets::earth_compact();
    let s = in_air_pitched(&p, std::f64::consts::FRAC_PI_2);
    let rho = atmo_density(&p.planet, s.ship.pos_m.length());
    let a = aero_forces(&p.ship, rho, &s.ship).accel_world;
    let v_hat = s.ship.vel_mps.normalize();
    let along = a.dot(v_hat);
    let across = (a - v_hat * along).length();
    assert!(
        across < along.abs() * 1e-6,
        "lift survived the stall: {across}"
    );
}

// ---------------------------------------------------------------- balance

/// Pressure behind gravity: the nose is pulled into the airflow. Hands off,
/// the angle of attack shrinks. This is what lets gravity steer the nose —
/// as the trajectory bends down, the nose follows it.
#[test]
fn pressure_aft_of_gravity_weathervanes() {
    let p = presets::earth_compact();
    assert!(p.ship.centre_of_pressure_m > p.ship.centre_of_gravity_m);
    let s0 = in_air_pitched(&p, 0.35);
    let a0 = alpha(&s0);
    let s1 = run(&p, s0, 240, Controls::default()); // 2 s
    let a1 = alpha(&s1);
    assert!(
        a1 < a0 * 0.8,
        "nose did not swing into the wind: {a0} -> {a1}"
    );
}

/// Gravity behind pressure — engines far aft, nothing forward to balance
/// them — and the same air throws the nose OUT of the airflow.
#[test]
fn gravity_aft_of_pressure_is_unstable() {
    let mut p = presets::earth_compact();
    p.ship.centre_of_gravity_m = p.ship.centre_of_pressure_m + 1.0;
    let s0 = in_air_pitched(&p, 0.1);
    let a0 = alpha(&s0);
    let s1 = run(&p, s0, 240, Controls::default());
    let a1 = alpha(&s1);
    assert!(
        a1 > a0 * 1.5,
        "tail-heavy ship should diverge: {a0} -> {a1}"
    );
}

/// Pressure ON gravity: no lever, no torque, whatever the attitude.
#[test]
fn coincident_centres_give_no_torque() {
    let mut p = presets::earth_compact();
    p.ship.centre_of_gravity_m = p.ship.centre_of_pressure_m;
    let s = in_air_pitched(&p, 0.7);
    let rho = atmo_density(&p.planet, s.ship.pos_m.length());
    let aero = aero_forces(&p.ship, rho, &s.ship);
    assert_eq!(aero.ang_accel_body, DVec3::ZERO);
}

/// Air damps spin: a rolling ship in the atmosphere slows down on its own.
#[test]
fn air_damps_rotation() {
    let p = presets::earth_compact();
    let mut s = in_air_pitched(&p, 0.0);
    s.ship.ang_vel_radps = DVec3::new(0.0, 0.0, 1.0);
    let s1 = run(&p, s, 240, Controls::default());
    assert!(s1.ship.ang_vel_radps.z < 0.9);
    assert!(
        s1.ship.ang_vel_radps.z > 0.0,
        "damping must not reverse the spin"
    );
}

/// The weathervane is about the airflow, not the planet: flying sideways
/// the nose is pulled toward the velocity, whichever way that is.
#[test]
fn weathervane_follows_the_airflow_not_the_horizon() {
    let p = presets::earth_compact();
    let s = in_air_pitched(&p, 0.0);
    // Same attitude, velocity yawed 0.3 rad to the right of the nose.
    let nose = s.ship.orient * DVec3::NEG_Z;
    let up = s.ship.orient * DVec3::Y;
    let dir = DQuat::from_axis_angle(up, -0.3) * nose;
    let s0 = with_velocity_dir(s, dir);
    let a0 = alpha(&s0);
    let s1 = run(&p, s0, 240, Controls::default());
    assert!(alpha(&s1) < a0 * 0.8);
}

// ----------------------------------------------------------------- vacuum

/// Above the top of the atmosphere there is no air at all: every aero term
/// is exactly zero, so a ship in orbit keeps its attitude bit for bit.
#[test]
fn vacuum_is_exactly_vacuum() {
    let p = presets::earth_compact();
    let mut s = presets::circular_orbit(&p, p.planet.atmo_top_m + 100.0);
    s.ship.ang_vel_radps = DVec3::new(0.3, -0.2, 0.1);
    let rho = atmo_density(&p.planet, s.ship.pos_m.length());
    assert_eq!(rho, 0.0);
    let aero = aero_forces(&p.ship, rho, &s.ship);
    assert_eq!(aero.accel_world, DVec3::ZERO);
    assert_eq!(aero.ang_accel_body, DVec3::ZERO);
    let s1 = run(&p, s, 1_200, Controls::default());
    assert_eq!(s1.ship.ang_vel_radps, s.ship.ang_vel_radps);
}

/// Aero is deterministic like everything else.
#[test]
fn aero_is_deterministic() {
    let p = presets::earth_compact();
    let s0 = in_air_pitched(&p, 0.4);
    let a = run(&p, s0, 600, Controls::default());
    let b = run(&p, s0, 600, Controls::default());
    assert_eq!(a, b);
}

// ------------------------------------------------------------- felt g

/// Coasting in vacuum is free fall: nothing felt, to the last bit of the
/// integrator's own arithmetic.
#[test]
fn free_fall_feels_nothing() {
    let p = presets::earth_compact();
    let s0 = presets::circular_orbit(&p, p.planet.atmo_top_m + 5_000.0);
    let s1 = step(&p, &s0, Controls::default());
    let felt = farfall_sim::felt_acceleration(&p.planet, &s0.ship, &s1.ship);
    assert!(felt.length() < 1e-6, "{felt}");
}

/// Full main engine in vacuum is felt as exactly the engine: the thrust
/// acceleration, along the nose.
#[test]
fn thrust_is_felt_as_thrust() {
    let p = presets::earth_compact();
    let s0 = presets::circular_orbit(&p, p.planet.atmo_top_m + 5_000.0);
    let c = Controls {
        thrust_body: DVec3::new(0.0, 0.0, -1.0),
        ..Default::default()
    };
    let s1 = step(&p, &s0, c);
    let felt = farfall_sim::felt_acceleration(&p.planet, &s0.ship, &s1.ship);
    let nose = s0.ship.orient * DVec3::NEG_Z;
    assert!(
        (felt.length() - p.ship.max_thrust_mps2.z).abs() < 1e-6,
        "{}",
        felt.length()
    );
    assert!(felt.normalize().dot(nose) > 0.9999);
}

/// Hands off in thick air, the pilot feels the drag — and only the drag.
#[test]
fn drag_is_felt_in_air() {
    let p = presets::earth_compact();
    let s0 = presets::circular_orbit(&p, 2_000.0);
    let s1 = step(&p, &s0, Controls::default());
    let felt = farfall_sim::felt_acceleration(&p.planet, &s0.ship, &s1.ship);
    let rho = atmo_density(&p.planet, s0.ship.pos_m.length());
    let aero = aero_forces(&p.ship, rho, &s0.ship).accel_world;
    assert!((felt - aero).length() < 1e-6 * aero.length().max(1.0));
    assert!(felt.length() > 0.5);
}
