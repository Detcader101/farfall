//! Contract tests for rotational flight assist (TASKS M1.3, SPEC §8).
//!
//! Assist exists to make the ship *flyable* without making it weightless: it
//! damps residual spin using the ship's own torque authority, and does nothing
//! to translation. These tests pin both halves of that promise.

use farfall_sim::{presets, state_hash, step, Controls, WorldParams, WorldState};
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

/// Assist damps rotation only — translation is untouched, so momentum stays
/// the pilot's problem.
#[test]
fn assist_leaves_translation_alone() {
    let params = presets::earth_compact();
    let state = spinning(&params, DVec3::new(0.4, 0.0, 0.0));
    let a = run(
        &params,
        state,
        600,
        Controls {
            assist: true,
            ..Default::default()
        },
    );
    let b = run(&params, state, 600, Controls::default());
    assert_eq!(a.ship.pos_m, b.ship.pos_m, "assist moved the ship");
    assert_eq!(a.ship.vel_mps, b.ship.vel_mps, "assist changed velocity");
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
