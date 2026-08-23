//! Contract tests for rotational flight assist (TASKS M1.3, SPEC §8).
//!
//! Assist exists to make the ship *flyable* without making it weightless: it
//! damps residual spin using the ship's own torque authority, and does nothing
//! to translation. These tests pin both halves of that promise.

use farfall_sim::{presets, state_hash, step, Controls, WorldParams, WorldState, DT};
use glam::DVec3;

fn spinning(params: &WorldParams, ang_vel: DVec3) -> WorldState {
    let mut s = presets::circular_orbit(params, 40_000.0);
    s.ship.ang_vel_radps = ang_vel;
    s
}

fn run(params: &WorldParams, mut state: WorldState, steps: u64, controls: Controls) -> WorldState {
    for _ in 0..steps {
        state = step(params, &state, controls);
    }
    state
}

/// With assist on and hands off the stick, the ship comes to rotational rest.
#[test]
fn assist_damps_rotation_to_rest() {
    let params = presets::earth_compact();
    let state = spinning(&params, DVec3::new(0.8, -0.5, 0.3));
    let hands_off = Controls {
        assist: true,
        ..Default::default()
    };
    // 3 s at 120 Hz.
    let end = run(&params, state, 360, hands_off);
    let residual = end.ship.ang_vel_radps.length();
    assert!(
        residual < 1e-3,
        "assist left {residual:e} rad/s of residual spin after 3 s"
    );
}

/// With assist off, angular momentum is conserved exactly — no hidden damping,
/// no drift. This is the invariant that makes "assist off" mean something.
#[test]
fn no_assist_conserves_rotation() {
    let params = presets::earth_compact();
    let w = DVec3::new(0.8, -0.5, 0.3);
    let end = run(&params, spinning(&params, w), 1_200, Controls::default());
    assert_eq!(
        end.ship.ang_vel_radps, w,
        "assist-off must not touch angular velocity"
    );
}

/// Assist must not fight the pilot: at full stick deflection the ship
/// accelerates exactly as it would with assist disabled.
#[test]
fn assist_does_not_fight_full_input() {
    let params = presets::earth_compact();
    let state = spinning(&params, DVec3::ZERO);
    let stick = DVec3::new(0.0, 1.0, 0.0);
    let with = run(
        &params,
        state,
        240,
        Controls {
            torque_body: stick,
            assist: true,
            ..Default::default()
        },
    );
    let without = run(
        &params,
        state,
        240,
        Controls {
            torque_body: stick,
            assist: false,
            ..Default::default()
        },
    );
    let diff = (with.ship.ang_vel_radps - without.ship.ang_vel_radps).length();
    assert!(
        diff < 1e-12,
        "assist altered the commanded axis by {diff:e} rad/s"
    );
}

/// Damping is torque-limited and must converge, never overshoot: the sign of
/// each spin component may reach zero but must never flip.
#[test]
fn assist_never_overshoots() {
    let params = presets::earth_compact();
    let mut state = spinning(&params, DVec3::new(0.05, -0.02, 0.011));
    let start = state.ship.ang_vel_radps;
    let hands_off = Controls {
        assist: true,
        ..Default::default()
    };
    for i in 0..600 {
        state = step(&params, &state, hands_off);
        let w = state.ship.ang_vel_radps;
        for axis in 0..3 {
            let (s0, s1) = (start[axis], w[axis]);
            assert!(
                s0 * s1 >= 0.0,
                "step {i}: axis {axis} overshot through zero ({s0} -> {s1})"
            );
        }
    }
}

/// The flight computer steers the ship where it points: velocity that is not
/// along the nose is bled away. This is the whole reason it exists — without it
/// the ship aims one way and drifts another, which is correct Newtonian physics
/// and feels nothing like flying.
#[test]
fn assist_steers_velocity_toward_the_nose() {
    let params = presets::earth_compact();
    let mut state = spinning(&params, DVec3::ZERO);
    // Nose along -Z, but moving sideways along +X.
    //
    // The figure matters: steering is capped by the *lateral* thrusters, so the
    // ship can only bend drift as fast as those can push. Asserting a rate the
    // hull cannot physically deliver would be testing the wishes, not the ship.
    state.ship.orient = glam::DQuat::IDENTITY;
    state.ship.vel_mps = DVec3::new(120.0, 0.0, 0.0);

    let nose = DVec3::NEG_Z;
    let lateral_before = {
        let v = state.ship.vel_mps;
        (v - nose * v.dot(nose)).length()
    };

    let end = run(
        &params,
        state,
        600, // 5 s
        Controls {
            assist: true,
            ..Default::default()
        },
    );
    let v = end.ship.vel_mps;
    let lateral_after = (v - nose * v.dot(nose)).length();
    assert!(
        lateral_after < lateral_before * 0.25,
        "sideways drift barely changed: {lateral_before:.1} -> {lateral_after:.1} m/s"
    );
}

/// With the computer off the ship is a pure ballistic body: nothing steers the
/// velocity, so a sideways drift stays exactly as sideways as gravity leaves it.
#[test]
fn assist_off_does_not_steer_velocity() {
    let params = presets::earth_compact();
    let mut state = spinning(&params, DVec3::ZERO);
    state.ship.orient = glam::DQuat::IDENTITY;
    state.ship.vel_mps = DVec3::new(120.0, 0.0, 0.0);

    let with = run(
        &params,
        state,
        600,
        Controls {
            assist: true,
            ..Default::default()
        },
    );
    let without = run(&params, state, 600, Controls::default());
    let nose = DVec3::NEG_Z;
    let lat = |v: DVec3| (v - nose * v.dot(nose)).length();
    assert!(
        lat(without.ship.vel_mps) > lat(with.ship.vel_mps) * 3.0,
        "assist-off must leave the drift alone"
    );
}

/// The steering is thrust-limited, not magic: it can never apply more than its
/// share of the engine, however fast the ship is drifting.
#[test]
fn alignment_respects_its_thrust_budget() {
    let params = presets::earth_compact();
    let cap = params.ship.max_thrust_mps2.min_element() * params.ship.align_authority;

    let mut state = spinning(&params, DVec3::ZERO);
    state.ship.orient = glam::DQuat::IDENTITY;
    // Absurdly fast sideways: the demand would be enormous if uncapped.
    state.ship.vel_mps = DVec3::new(50_000.0, 0.0, 0.0);

    let before = state.ship.vel_mps;
    let after = step(
        &params,
        &state,
        Controls {
            assist: true,
            ..Default::default()
        },
    )
    .ship
    .vel_mps;

    // Subtract the coasting step so only the assist's contribution remains.
    let coast = step(&params, &state, Controls::default()).ship.vel_mps;
    let applied = (after - coast).length() / DT;
    assert!(
        applied <= cap * 1.001,
        "alignment applied {applied:.1} m/s^2, budget is {cap:.1}"
    );
    assert!(applied > cap * 0.9, "alignment should be saturating here");
    let _ = before;
}

/// Assist is inside the deterministic core, so it must hash identically across
/// runs like everything else (SPEC §7.3).
#[test]
fn assist_is_deterministic() {
    let params = presets::earth_compact();
    let state = spinning(&params, DVec3::new(0.7, 0.2, -0.9));
    let controls = Controls {
        thrust_body: DVec3::new(0.3, 0.0, 0.8),
        torque_body: DVec3::new(0.2, -0.4, 0.0),
        assist: true,
        ..Default::default()
    };
    let a = run(&params, state, 2_000, controls);
    let b = run(&params, state, 2_000, controls);
    assert_eq!(state_hash(&a), state_hash(&b));
}

/// Sanity: a single step of assist can't produce a NaN from the 1/DT term.
#[test]
fn assist_survives_zero_spin() {
    let params = presets::earth_compact();
    let state = spinning(&params, DVec3::ZERO);
    let end = step(
        &params,
        &state,
        Controls {
            assist: true,
            ..Default::default()
        },
    );
    assert!(end.ship.ang_vel_radps.is_finite());
    assert_eq!(end.ship.ang_vel_radps, DVec3::ZERO);
}

/// The emergency gyro kills any spin on its time constant, with or without
/// the flight computer, far faster than the torque limits could.
#[test]
fn despin_kills_a_tumble_nothing_else_would() {
    let params = presets::earth_compact();
    let wild = DVec3::new(9.0, -7.0, 11.0);
    let spun = spinning(&params, wild);
    let despin = Controls {
        despin: true,
        ..Default::default()
    };
    // Two seconds: exp(-2 / 0.6) of it left, under 4 percent.
    let after = run(&params, spun, 240, despin);
    assert!(
        after.ship.ang_vel_radps.length() < wild.length() * 0.04,
        "still spinning at {:?}",
        after.ship.ang_vel_radps
    );
    // Without it the flight computer alone is torque-limited: at 1.7 rad/s²
    // it has barely dented 9 rad/s in a quarter of a second.
    let fc = Controls {
        assist: true,
        ..Default::default()
    };
    let slow = run(&params, spun, 30, fc);
    assert!(slow.ship.ang_vel_radps.length() > wild.length() * 0.5);
}

/// The hyper drive moves space: held for a minute from orbit the ship has
/// crossed an astronomical unit (the Sun's distance) down its nose —
/// relativity's wall is for things moving through space, not with it —
/// and the velocity is on the nose whatever the orbit was doing.
#[test]
fn the_charged_drive_crosses_an_astronomical_unit_in_seconds() {
    let params = presets::earth_compact();
    let start = spinning(&params, DVec3::ZERO);
    let nose = start.ship.orient * DVec3::NEG_Z;
    let from = start.ship.pos_m;
    let held = Controls {
        hyper: true,
        hyper_level: 1.0,
        ..Default::default()
    };
    let end = run(&params, start, 20 * 120, held);
    let gone = (end.ship.pos_m - from).length();
    assert!(
        gone > params.sun.distance_m * 0.9,
        "twenty seconds of a charged drive cover the Sun's distance: {gone:.3e} of {:.3e}",
        params.sun.distance_m
    );
    // A fresh field is a fraction of that: the charge is the speed.
    let fresh = run(
        &params,
        spinning(&params, DVec3::ZERO),
        20 * 120,
        Controls {
            hyper: true,
            hyper_level: 0.0,
            ..Default::default()
        },
    );
    let gone_fresh = (fresh.ship.pos_m - from).length();
    assert!(gone_fresh < gone * 0.15, "{gone_fresh:.3e} vs {gone:.3e}");
    let speed = end.ship.vel_mps.length();
    assert!(speed > farfall_sim::LIGHT_SPEED_MPS * 0.05, "{speed}");
    assert!(speed <= params.ship.hyper_max_mps * 1.001, "{speed}");
    let along = end.ship.vel_mps.normalize().dot(nose);
    assert!(along > 0.999, "the velocity is on the nose: cos {along}");
    // Let go: the next step is plain Newton — the field is the app's to
    // collapse, the sim does not snap anything back.
    let after = step(&params, &end, Controls::default());
    let coast = step(
        &params,
        &end,
        Controls {
            assist: false,
            ..Default::default()
        },
    );
    assert_eq!(after.ship.vel_mps, coast.ship.vel_mps);
}

/// The air brake brakes everything: a tumble dies under it faster than
/// under the gyro alone.
#[test]
fn the_brake_kills_spin_harder_than_the_gyro() {
    let params = presets::earth_compact();
    let w = DVec3::new(1.5, -1.0, 0.7);
    let gyro = run(
        &params,
        spinning(&params, w),
        60,
        Controls {
            despin: true,
            ..Default::default()
        },
    );
    let brake = run(
        &params,
        spinning(&params, w),
        60,
        Controls {
            brake: true,
            ..Default::default()
        },
    );
    let g = gyro.ship.ang_vel_radps.length();
    let b = brake.ship.ang_vel_radps.length();
    assert!(b < g * 0.3, "brake {b} vs gyro {g}");
    assert!(
        b < 0.4,
        "half a second of brake and the tumble is gone: {b}"
    );
}
