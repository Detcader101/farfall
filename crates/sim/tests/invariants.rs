//! Contract tests for the simulation core (SPEC §8).
//!
//! These define what "the physics works" means. If you change the integrator,
//! the models, or the state layout, these tell you exactly what you changed.

use farfall_sim::{
    presets, specific_energy, state_hash, step, Controls, WorldParams, WorldState, DT,
};
use glam::DVec3;

fn run(params: &WorldParams, mut state: WorldState, steps: u64, controls: Controls) -> WorldState {
    for _ in 0..steps {
        state = step(params, &state, controls);
    }
    state
}

/// One orbital period, in whole steps.
fn period_steps(params: &WorldParams, radius_m: f64) -> u64 {
    let t = 2.0 * std::f64::consts::PI * (radius_m.powi(3) / params.planet.mu).sqrt();
    (t / DT) as u64
}

// ---------------------------------------------------------------- invariants

/// A drag-free circular orbit stays circular: radius, speed, and energy bounded
/// over three full orbits (symplectic Euler ⇒ bounded, not growing, error).
#[test]
fn circular_orbit_is_stable() {
    let params = presets::earth_compact();
    // 40 km altitude on the compact planet: far above the (exaggerated) atmosphere.
    let alt = 40_000.0;
    let state0 = presets::circular_orbit(&params, alt);
    let r0 = state0.ship.pos_m.length();
    let v0 = state0.ship.vel_mps.length();
    let e0 = specific_energy(&params.planet, &state0.ship);

    let mut state = state0;
    let steps = period_steps(&params, r0);
    for orbit in 0..3 {
        state = run(&params, state, steps, Controls::default());
        let r = state.ship.pos_m.length();
        let v = state.ship.vel_mps.length();
        let e = specific_energy(&params.planet, &state.ship);
        assert!(
            (r - r0).abs() / r0 < 0.01,
            "orbit {orbit}: radius drifted {:.4}%",
            100.0 * (r - r0).abs() / r0
        );
        assert!(
            (v - v0).abs() / v0 < 0.01,
            "orbit {orbit}: speed drifted {:.4}%",
            100.0 * (v - v0).abs() / v0
        );
        assert!(
            (e - e0).abs() / e0.abs() < 0.001,
            "orbit {orbit}: energy drifted {:.6}%",
            100.0 * (e - e0).abs() / e0.abs()
        );
    }
}

/// In atmosphere with no thrust, drag strictly bleeds speed.
#[test]
fn drag_decays_speed() {
    let params = presets::earth_compact();
    // 1 km altitude: deep in the exaggerated atmosphere, moving fast.
    let mut state = presets::circular_orbit(&params, 1_000.0);
    let mut last_speed = state.ship.vel_mps.length();
    for _ in 0..10 {
        state = run(&params, state, 120, Controls::default()); // 1 s chunks
        let speed = state.ship.vel_mps.length();
        assert!(speed < last_speed, "drag failed to reduce speed");
        last_speed = speed;
    }
}

/// Out-of-range control inputs are clamped, not obeyed: a demand of 1000× max
/// thrust produces exactly the same trajectory as a demand of 1×.
#[test]
fn controls_clamp() {
    let params = presets::earth_compact();
    let state0 = presets::circular_orbit(&params, 40_000.0);
    let sane = Controls {
        thrust_body: DVec3::new(0.0, 0.0, 1.0),
        torque_body: DVec3::ZERO,
    };
    let insane = Controls {
        thrust_body: DVec3::new(0.0, 0.0, 1000.0),
        torque_body: DVec3::ZERO,
    };
    let a = run(&params, state0, 600, sane);
    let b = run(&params, state0, 600, insane);
    assert_eq!(state_hash(&a), state_hash(&b));
}

/// The models are scale-free (SPEC P3, §7.5): scaling lengths by s and μ by s³
/// is an exact similarity transform — positions scale by s, the trajectory shape
/// and timing are identical. Run the same scenario at two scales and compare.
#[test]
fn scale_invariance() {
    let base = presets::earth_compact();
    let s = 7.0; // arbitrary non-round factor
    let scaled = WorldParams {
        planet: farfall_sim::PlanetParams {
            radius_m: base.planet.radius_m * s,
            mu: base.planet.mu * s * s * s,
            // Similarity requires the drag term to scale consistently:
            // a_drag ∝ ρ·v²·CdA/m; v² scales by s² and we need a ∝ s, so scale
            // CdA/m by 1/s (keep ρ profile shape via H·s).
            atmo_rho0: base.planet.atmo_rho0,
            atmo_scale_height_m: base.planet.atmo_scale_height_m * s,
        },
        ship: farfall_sim::ShipParams {
            cd_area_m2: base.ship.cd_area_m2 / s,
            // Thrust acceleration must also scale by s to keep the similarity.
            max_thrust_mps2: base.ship.max_thrust_mps2 * s,
            ..base.ship
        },
    };

    let alt = 5_000.0;
    let a0 = presets::circular_orbit(&base, alt);
    let b0 = presets::circular_orbit(&scaled, alt * s);

    let controls = Controls {
        thrust_body: DVec3::new(0.1, 0.0, 0.3),
        torque_body: DVec3::ZERO,
    };
    let a = run(&base, a0, 2_400, controls); // 20 s
    let b = run(&scaled, b0, 2_400, controls);

    let rel = (b.ship.pos_m / s - a.ship.pos_m).length() / a.ship.pos_m.length();
    assert!(rel < 1e-9, "similarity broken: relative error {rel:e}");
}

// ------------------------------------------------------------- determinism

/// Same scenario, run twice → identical hash (SPEC §7.3).
#[test]
fn determinism_run_twice() {
    let params = presets::earth_compact();
    let state0 = presets::circular_orbit(&params, 12_345.0);
    let controls = Controls {
        thrust_body: DVec3::new(0.3, -0.2, 0.9),
        torque_body: DVec3::new(0.1, 0.4, -0.5),
    };
    let a = run(&params, state0, 5_000, controls);
    let b = run(&params, state0, 5_000, controls);
    assert_eq!(state_hash(&a), state_hash(&b));
}

/// Cross-platform golden hash (SPEC §8): this exact constant must hold on
/// macOS-arm64 AND linux-x86_64 (CI runs both). If this test fails after an
/// intentional physics change, update the constant IN ITS OWN COMMIT and say
/// why in the message. If it fails across platforms with unchanged code, we
/// have a determinism leak — fix the leak, never fudge the constant.
#[test]
fn golden_hash() {
    let params = presets::earth_compact();
    let state0 = presets::circular_orbit(&params, 20_000.0);
    let controls = Controls {
        thrust_body: DVec3::new(0.5, 0.1, 1.0),
        torque_body: DVec3::new(-0.2, 0.3, 0.7),
    };
    let end = run(&params, state0, 1_000, controls);
    let hash = state_hash(&end);
    assert_eq!(
        hash, GOLDEN,
        "golden hash mismatch: got {hash:#018x} — see this test's doc comment"
    );
}

// Generated on macOS-arm64, rustc pinned in rust-toolchain.toml. Placeholder is
// replaced by the value printed by `print_golden` on first setup.
const GOLDEN: u64 = 0xa73194421bbda8a7;

/// Not an assertion — prints the current golden value for setup/updates:
/// `cargo test -p farfall-sim print_golden -- --ignored --nocapture`
#[test]
#[ignore]
fn print_golden() {
    let params = presets::earth_compact();
    let state0 = presets::circular_orbit(&params, 20_000.0);
    let controls = Controls {
        thrust_body: DVec3::new(0.5, 0.1, 1.0),
        torque_body: DVec3::new(-0.2, 0.3, 0.7),
    };
    let end = run(&params, state0, 1_000, controls);
    println!("GOLDEN = {:#018x}", state_hash(&end));
}
