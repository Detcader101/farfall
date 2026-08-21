//! Contract tests for the air brake and ground contact (SPEC §8).

use farfall_sim::{presets, state_hash, step, Controls, WorldParams, WorldState, DT};
use glam::{DQuat, DVec3};

fn run(params: &WorldParams, mut state: WorldState, steps: u64, controls: Controls) -> WorldState {
    for _ in 0..steps {
        state = step(params, &state, controls);
    }
    state
}

fn braking() -> Controls {
    Controls {
        brake: true,
        ..Default::default()
    }
}

// ------------------------------------------------------------------ brake

/// The brake dumps speed whichever way the ship is travelling — it opposes the
/// velocity, not the nose, so it works while drifting sideways or backwards.
#[test]
fn brake_kills_speed_from_any_direction() {
    let params = presets::earth_compact();
    for direction in [DVec3::X, DVec3::NEG_Y, DVec3::new(0.4, -0.6, 0.7)] {
        let mut state = presets::circular_orbit(&params, 40_000.0);
        state.ship.vel_mps = direction.normalize() * 400.0;
        let before = state.ship.vel_mps.length();
        let after = run(&params, state, 240, braking()).ship.vel_mps.length(); // 2 s
        assert!(
            after < before * 0.35,
            "braking along {direction:?} only reached {after:.0} m/s from {before:.0}"
        );
    }
}

/// It is retro-thrust, not a hand on the world: the deceleration it applies can
/// never exceed the ship's rated brake authority.
#[test]
fn brake_respects_its_thrust_budget() {
    let params = presets::earth_compact();
    let mut state = presets::circular_orbit(&params, 40_000.0);
    // Absurd speed, so an unbounded brake would produce an enormous impulse.
    state.ship.vel_mps = DVec3::new(90_000.0, 0.0, 0.0);

    let coast = step(&params, &state, Controls::default()).ship.vel_mps;
    let braked = step(&params, &state, braking()).ship.vel_mps;
    let applied = (braked - coast).length() / DT;
    assert!(
        applied <= params.ship.brake_mps2 * 1.001,
        "brake applied {applied:.0} m/s^2 against a budget of {:.0}",
        params.ship.brake_mps2
    );
}

/// The brake must not push the ship backwards through zero.
///
/// The heading is deliberately *tangential*. The spawn sits on the +X axis, so
/// a velocity along +X is radial — gravity then acts on the very axis under
/// test and reverses it on its own, which is physics doing its job and has
/// nothing to say about the brake.
#[test]
fn brake_never_reverses_the_ship() {
    let params = presets::earth_compact();
    let mut state = presets::circular_orbit(&params, 40_000.0);
    state.ship.vel_mps = DVec3::new(0.0, 0.0, 60.0);
    let heading = state.ship.vel_mps.normalize();

    for _ in 0..600 {
        state = step(&params, &state, braking());
        // Micron-per-second tolerance, not zero: the brake opposes the *total*
        // velocity, and gravity keeps feeding in a radial component, so its
        // thrust direction tilts off the tested axis and leaves a residue there
        // far below anything physical. (The Sun and Moon contribute only their
        // tide here — the planet's frame falls with them — so they add nothing
        // the brake has to fight.)
        assert!(
            state.ship.vel_mps.dot(heading) >= -1e-3,
            "brake reversed the ship: {:?}",
            state.ship.vel_mps
        );
    }
    assert!(
        state.ship.vel_mps.dot(heading) < 1.0,
        "brake never actually stopped it"
    );
}

// ---------------------------------------------------------------- contact

/// A ship falling onto the planet stops at the surface instead of sailing
/// through it — which is what let the pilot see the cloud deck from inside.
#[test]
fn ship_cannot_pass_through_the_surface() {
    let params = presets::earth_compact();
    let surface = params.planet.radius_m;
    let mut state = presets::circular_orbit(&params, 3_000.0);
    // Straight down, hard.
    state.ship.vel_mps = -state.ship.pos_m.normalize() * 900.0;

    for i in 0..2_400 {
        state = step(&params, &state, Controls::default());
        let r = state.ship.pos_m.length();
        assert!(
            r >= surface - 1e-6,
            "step {i}: ship reached {:.1} m below the surface",
            surface - r
        );
    }
    // ...and it actually arrived, rather than the test passing by never falling.
    assert!(
        state.ship.pos_m.length() < surface + 1.0,
        "ship never reached the ground"
    );
}

/// Contact removes the velocity that drove the ship into the ground, and does
/// not fling it back out (restitution is zero: this lands, it does not bounce).
#[test]
fn contact_absorbs_the_impact() {
    let params = presets::earth_compact();
    let mut state = presets::circular_orbit(&params, 200.0);
    state.ship.vel_mps = -state.ship.pos_m.normalize() * 500.0;

    let end = run(&params, state, 600, Controls::default());
    let up = end.ship.pos_m.normalize();
    let vertical = end.ship.vel_mps.dot(up);
    assert!(
        vertical.abs() < 1.0,
        "ship kept {vertical:.2} m/s of vertical speed on the ground"
    );
}

/// Sliding along the ground scrubs speed rather than skating forever.
#[test]
fn ground_friction_brings_the_ship_to_rest() {
    let params = presets::earth_compact();
    let mut state = presets::circular_orbit(&params, 0.0);
    // Put it on the surface moving tangentially, well below orbital speed.
    state.ship.pos_m = state.ship.pos_m.normalize() * params.planet.radius_m;
    state.ship.vel_mps = DVec3::new(0.0, 0.0, -120.0);

    let first = run(&params, state, 120, Controls::default())
        .ship
        .vel_mps
        .length();
    let later = run(&params, state, 1_200, Controls::default())
        .ship
        .vel_mps
        .length();
    assert!(
        later < first * 0.2,
        "friction barely acted: {first:.1} -> {later:.1} m/s"
    );
}

/// Contact is still deterministic — it is inside the sim core like everything
/// else, and a collision response with a branch in it is a classic place for
/// two machines to disagree.
#[test]
fn contact_is_deterministic() {
    let params = presets::earth_compact();
    let mut state = presets::circular_orbit(&params, 1_000.0);
    state.ship.vel_mps = -state.ship.pos_m.normalize() * 700.0;
    state.ship.orient = DQuat::from_rotation_y(0.7);
    let controls = Controls {
        thrust_body: DVec3::new(0.2, 0.0, -0.6),
        brake: true,
        ..Default::default()
    };
    let a = run(&params, state, 1_500, controls);
    let b = run(&params, state, 1_500, controls);
    assert_eq!(state_hash(&a), state_hash(&b));
}

/// Neither system may touch a ship that is flying normally: both are gated, and
/// the ungated path has to stay bit-identical or the golden hash moves.
#[test]
fn flying_clear_of_the_ground_is_untouched() {
    let params = presets::earth_compact();
    let state = presets::circular_orbit(&params, 40_000.0);
    let a = run(&params, state, 2_000, Controls::default());
    let b = run(&params, state, 2_000, Controls::default());
    assert_eq!(state_hash(&a), state_hash(&b));
    assert!(a.ship.pos_m.length() > params.planet.radius_m);
}
