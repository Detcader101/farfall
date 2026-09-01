//! Contract tests for the planet's wind (SPEC §7.2, §7.3).
//!
//! The wind is a pure, deterministic function of position and sim time —
//! layered zonal bands, a jet stream, travelling cells and slow gusts —
//! and it enters the physics through exactly one door: the air-relative
//! velocity `v − wind` that every aerodynamic term acts on.

use farfall_sim::{
    aero_forces, aero_forces_wind, presets, state_hash, step, wind_mps, Controls, Ground,
    WorldParams, WorldState, DT, GEAR_HEIGHT_M,
};
use glam::{DQuat, DVec3};

fn run(params: &WorldParams, mut state: WorldState, steps: u64, controls: Controls) -> WorldState {
    for _ in 0..steps {
        state = step(params, &state, controls);
    }
    state
}

/// A point `h` metres over the surface at latitude/longitude (radians).
fn at(params: &WorldParams, lat: f64, lon: f64, h: f64) -> DVec3 {
    let up = DVec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin());
    up * (params.planet.radius_m + h)
}

/// A grid of places and moments inside the air, poles excluded.
fn samples(params: &WorldParams, h: f64) -> Vec<(DVec3, f64)> {
    let mut out = Vec::new();
    for i in 0..7 {
        let lat = -1.2 + 0.4 * i as f64;
        for j in 0..8 {
            let lon = -3.0 + 0.8 * j as f64;
            for k in 0..5 {
                let t = 13.0 * k as f64;
                out.push((at(params, lat, lon, h), t));
            }
        }
    }
    out
}

// ------------------------------------------------------------ determinism

/// The wind is a pure function: the same place and moment give the same
/// vector, and the same flight through it gives the same world, twice.
#[test]
fn wind_is_identical_across_two_runs() {
    let p = presets::earth_compact();
    for (pos, t) in samples(&p, 900.0) {
        assert_eq!(wind_mps(&p, pos, t), wind_mps(&p, pos, t));
    }
    let s0 = presets::circular_orbit(&p, 3_000.0);
    let controls = Controls {
        thrust_body: DVec3::new(0.2, -0.1, 0.6),
        torque_body: DVec3::new(0.1, -0.3, 0.2),
        ..Default::default()
    };
    let a = run(&p, s0, 2_000, controls);
    let b = run(&p, s0, 2_000, controls);
    assert_eq!(state_hash(&a), state_hash(&b));
}

// --------------------------------------------------------------- the field

/// No air, no wind: exactly zero at the top of the atmosphere and
/// everywhere above it, already faded to a whisper just under it — and a
/// ship in vacuum feels no aero at all, wind or no wind.
#[test]
fn wind_dies_above_the_atmosphere() {
    let p = presets::earth_compact();
    let top = p.planet.atmo_top_m;
    for (lat, lon, t) in [(0.4, 1.0, 7.0), (-0.9, -2.0, 130.0), (1.1, 0.3, 999.0)] {
        for h in [top, top + 1.0, top + 1.0e6] {
            assert_eq!(wind_mps(&p, at(&p, lat, lon, h), t), DVec3::ZERO);
        }
        let whisper = wind_mps(&p, at(&p, lat, lon, top * 0.95), t);
        assert!(whisper.length() < 2.0, "{whisper} under the top");
    }
    let s = presets::circular_orbit(&p, top + 100.0);
    let aero = aero_forces_wind(&p.ship, 0.0, &s.ship, wind_mps(&p, s.ship.pos_m, s.time_s));
    assert_eq!(aero.accel_world, DVec3::ZERO);
    assert_eq!(aero.ang_accel_body, DVec3::ZERO);
}

/// The wind blows round the planet, not out of it, and it keeps to its
/// lanes: gusts near the surface, the jet stream fast at altitude, and
/// the strength knob scales the whole field linearly (clamped at 2).
#[test]
fn the_wind_keeps_to_its_lanes() {
    let p = presets::earth_compact();
    let mut surface_max: f64 = 0.0;
    let mut surface_sum = 0.0;
    let mut n = 0.0;
    for (pos, t) in samples(&p, 40.0) {
        let w = wind_mps(&p, pos, t);
        let up = pos.normalize();
        assert!(
            w.dot(up).abs() < 1e-9 * w.length().max(1.0),
            "the wind left the ground: {w} at {pos}"
        );
        surface_max = surface_max.max(w.length());
        surface_sum += w.length();
        n += 1.0;
    }
    assert!(
        surface_max > 8.0 && surface_max < 32.0,
        "surface gusts topped out at {surface_max:.1} m/s"
    );
    let mean = surface_sum / n;
    assert!((2.0..20.0).contains(&mean), "surface mean {mean:.1} m/s");

    // The jet band: mid-latitude, a little over half way up the air.
    let mut jet_max: f64 = 0.0;
    for (pos, t) in samples(&p, p.planet.atmo_top_m * 0.55) {
        jet_max = jet_max.max(wind_mps(&p, pos, t).length());
    }
    assert!(
        jet_max > 40.0 && jet_max < 68.0,
        "the jet blows at {jet_max:.1} m/s"
    );

    // The knob: linear, and clamped at 2.
    let mut twice = p;
    twice.wind_strength = 2.0;
    let mut wild = p;
    wild.wind_strength = 5.0;
    let pos = at(&p, 0.7, 1.3, 300.0);
    let w1 = wind_mps(&p, pos, 21.0);
    let w2 = wind_mps(&twice, pos, 21.0);
    assert!((w2 - w1 * 2.0).length() < 1e-9, "{w1} vs {w2}");
    assert_eq!(w2, wind_mps(&wild, pos, 21.0), "past 2 is 2");
}

/// No jumps: from one tick to the next, and from one metre to the next,
/// the field moves smoothly — gusting is slow, low-order noise, never
/// per-tick randomness.
#[test]
fn the_wind_field_is_smooth() {
    let p = presets::earth_compact();
    for h in [30.0, 2_000.0, p.planet.atmo_top_m * 0.55] {
        for (pos, t) in samples(&p, h) {
            let w = wind_mps(&p, pos, t);
            let tick = wind_mps(&p, pos, t + DT);
            assert!(
                (tick - w).length() < 0.1,
                "a {:.3} m/s jump in one tick at h={h}",
                (tick - w).length()
            );
            let metre = wind_mps(&p, pos + pos.normalize().cross(DVec3::Y), t);
            assert!(
                (metre - w).length() < 0.05,
                "a {:.3} m/s jump in one metre at h={h}",
                (metre - w).length()
            );
        }
    }
}

// ------------------------------------------------------------- the physics

/// wind_strength 0 is the pre-wind world, bit for bit: the golden-hash
/// scenario flown in still air reproduces the hash the suite pinned
/// before wind existed (0xe8f76101b8054115 — see invariants.rs history).
#[test]
fn still_air_matches_the_old_drag_exactly() {
    let mut params = presets::earth_compact();
    params.wind_strength = 0.0;
    let state0 = presets::circular_orbit(&params, 20_000.0);
    let controls = Controls {
        thrust_body: DVec3::new(0.5, 0.1, 1.0),
        torque_body: DVec3::new(-0.2, 0.3, 0.7),
        ..Default::default()
    };
    let end = run(&params, state0, 1_000, controls);
    assert_eq!(
        state_hash(&end),
        0xe8f7_6101_b805_4115,
        "still air must be the exact pre-wind physics"
    );
    // And the still-air aero is the windy aero with a zero wind, bitwise.
    let s = presets::circular_orbit(&params, 2_000.0);
    let rho = farfall_sim::atmo_density(&params.planet, s.ship.pos_m.length());
    assert_eq!(
        aero_forces(&params.ship, rho, &s.ship),
        aero_forces_wind(&params.ship, rho, &s.ship, DVec3::ZERO),
    );
}

/// A landed ship sits on its gear, parked: the wind blows over it (it is
/// real at that spot) but the ground holds the ship — a thousand ticks
/// later it has not moved a bit.
#[test]
fn a_landed_ship_feels_the_ground_not_the_wind() {
    let p = presets::earth_compact();
    // A windy mid-latitude spot, not the calm pole.
    let up = DVec3::new(0.5, 0.7, 0.5).normalize();
    let mut s = presets::circular_orbit(&p, 40_000.0);
    s.ship.pos_m = up * (p.planet.radius_m + GEAR_HEIGHT_M + 1.0);
    s.ship.vel_mps = -up * 2.0;
    s.ship.orient = DQuat::from_rotation_arc(DVec3::Y, up);
    s.ship.ang_vel_radps = DVec3::ZERO;
    let down = run(&p, s, 240, Controls::default());
    assert!(
        matches!(down.ship.ground, Ground::Landed { body: 0, .. }),
        "{:?}",
        down.ship.ground
    );
    // The wind is really blowing here — the parked state is not a becalmed
    // corner of the field.
    let blowing = wind_mps(&p, down.ship.pos_m, down.time_s).length();
    assert!(blowing > 1.0, "no wind at the pad: {blowing:.2} m/s");
    let later = run(&p, down, 1_000, Controls::default());
    assert_eq!(
        later.ship.pos_m, down.ship.pos_m,
        "the wind moved a parked ship"
    );
    assert_eq!(later.ship.vel_mps, DVec3::ZERO);
    assert_eq!(later.ship.orient, down.ship.orient);
}

/// A crosswind weathervanes the nose toward the AIR's motion, not the
/// ground track: hands off, the angle between the nose and the airflow
/// shrinks while the wind holds it off the velocity vector.
#[test]
fn the_nose_swings_into_the_moving_air() {
    let p = presets::earth_compact();
    let s = presets::circular_orbit(&p, 1_500.0);
    let steady = run(&p, s, 1_200, Controls::default());
    let wind = wind_mps(&p, steady.ship.pos_m, steady.time_s);
    // Airflow the hull actually sees.
    let v_air = steady.ship.vel_mps - wind;
    let nose = steady.ship.orient * DVec3::NEG_Z;
    let alpha_air = nose.dot(v_air.normalize()).clamp(-1.0, 1.0).acos();
    assert!(
        alpha_air < 0.05,
        "after ten hands-off seconds the nose sits {alpha_air:.3} rad off the airflow"
    );
}
