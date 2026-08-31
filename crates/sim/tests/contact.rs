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
    // ...and it actually arrived (on its gear), rather than the test passing
    // by never falling.
    assert!(
        state.ship.pos_m.length() < surface + GEAR_HEIGHT_M + 1.0,
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

// ---------------------------------------------------------------- landed

use farfall_sim::{Ground, GEAR_HEIGHT_M, TOUCHDOWN_INTO_MPS};

/// A ship at rest relative to `body` `alt` metres over its surface at the
/// body's +Y pole, level (body-up along the surface normal), coming down
/// at `descent` m/s.
fn over(params: &WorldParams, body: usize, alt: f64, descent: f64) -> WorldState {
    let mut state = presets::circular_orbit(params, 40_000.0);
    let b = params.bodies(0.0)[body];
    let up = DVec3::Y;
    state.ship.pos_m = b.centre + up * (b.radius_m + GEAR_HEIGHT_M + alt);
    state.ship.vel_mps = params.body_velocities(0.0)[body] - up * descent;
    state.ship.orient = DQuat::IDENTITY;
    state.ship.ang_vel_radps = DVec3::ZERO;
    state
}

fn landed_on(state: &WorldState) -> Option<usize> {
    match state.ship.ground {
        Ground::Landed { body, .. } => Some(body),
        _ => None,
    }
}

/// Set down gently and upright, the ship LANDS: it sits on its gear at a
/// fixed height, the velocity is the ground's own, the spin is gone, and a
/// thousand ticks later it has not moved a bit — the settled state is a
/// fixed point of the integrator, not a slow slide.
#[test]
fn a_slow_upright_touchdown_settles_and_stays_put_through_a_thousand_ticks() {
    let params = presets::earth_compact();
    let state = over(&params, 0, 1.0, 2.0);
    let down = run(&params, state, 120, Controls::default());
    assert_eq!(landed_on(&down), Some(0), "{:?}", down.ship.ground);
    let stance = params.planet.radius_m + GEAR_HEIGHT_M;
    assert_eq!(down.ship.pos_m.length(), stance);
    assert_eq!(down.ship.vel_mps, DVec3::ZERO);
    assert_eq!(down.ship.ang_vel_radps, DVec3::ZERO);
    let later = run(&params, down, 1_000, Controls::default());
    assert_eq!(later.ship.pos_m, down.ship.pos_m, "the ship drifted");
    assert_eq!(later.ship.vel_mps, DVec3::ZERO);
    assert_eq!(later.ship.orient, down.ship.orient);
    assert_eq!(landed_on(&later), Some(0));
    // Level: the gear took what tilt there was.
    let up = later.ship.pos_m.normalize();
    assert!((later.ship.orient * DVec3::Y).dot(up) > 0.9999);
}

/// The Moon goes round the planet at a hundred metres a second. A ship
/// landed on it is carried with it, exactly: every tick it sits at the same
/// spot over the Moon's centre, so the ground never slides away underneath.
#[test]
fn a_landed_ship_rides_the_ground_exactly_as_the_planet_turns() {
    // (The planet itself is still in this frame; the Moon is the body that
    // moves, so the Moon is where the contract is tested.)
    let params = presets::earth_compact();
    let state = over(&params, 1, 0.5, 1.0);
    let mut s = run(&params, state, 240, Controls::default());
    assert_eq!(landed_on(&s), Some(1), "{:?}", s.ship.ground);
    let stance = params.moon.radius_m + GEAR_HEIGHT_M;
    let Ground::Landed { up: anchor, .. } = s.ship.ground else {
        unreachable!()
    };
    for _ in 0..2_000 {
        s = step(&params, &s, Controls::default());
        let centre = params.moon.centre(params.planet.mu, s.time_s);
        // Bit for bit: the spot is recomputed from the Moon's centre and
        // the stored normal, never integrated toward.
        assert_eq!(
            s.ship.pos_m,
            centre + anchor * stance,
            "the ship slid off its spot at t={}",
            s.time_s
        );
        assert_eq!(
            s.ship.ground,
            Ground::Landed {
                body: 1,
                up: anchor
            }
        );
    }
    // And it is moving with the Moon, not sitting still in the planet's frame.
    assert!(s.ship.vel_mps.length() > 50.0, "{:?}", s.ship.vel_mps);
}

/// Throttle up and the ship is a ship again: lift on the thrusters takes it
/// off the ground, and the main engine rolls it along and off its spot.
#[test]
fn throttle_releases_a_landed_ship() {
    let params = presets::earth_compact();
    let state = over(&params, 0, 1.0, 2.0);
    let down = run(&params, state, 120, Controls::default());
    assert_eq!(landed_on(&down), Some(0));
    // Lift: +Y thrusters, 45 m/s² against 9.81.
    let lift = Controls {
        thrust_body: DVec3::new(0.0, 1.0, 0.0),
        ..Default::default()
    };
    let up = run(&params, down, 120, lift);
    assert_eq!(up.ship.ground, Ground::Flight, "{:?}", up.ship.ground);
    assert!(
        up.ship.pos_m.length() > down.ship.pos_m.length() + 5.0,
        "no lift: {} vs {}",
        up.ship.pos_m.length(),
        down.ship.pos_m.length()
    );
    // The main engine, level on the ground: rolling, and no longer parked.
    let go = Controls {
        thrust_body: DVec3::new(0.0, 0.0, -1.0),
        ..Default::default()
    };
    let rolling = run(&params, down, 120, go);
    assert_ne!(landed_on(&rolling), Some(0));
    assert!(rolling.ship.vel_mps.length() > 10.0);
    // Boost alone counts as a throttle too.
    let boosted = step(
        &params,
        &down,
        Controls {
            boost: true,
            ..Default::default()
        },
    );
    assert_ne!(landed_on(&boosted), Some(0));
    // The stick does not: torque on the ground is just the gear taking it.
    let stick = run(
        &params,
        down,
        60,
        Controls {
            torque_body: DVec3::new(1.0, 1.0, 1.0),
            ..Default::default()
        },
    );
    assert_eq!(landed_on(&stick), Some(0));
    assert_eq!(stick.ship.orient, down.ship.orient);
}

/// A landing is judged at the moment of touchdown. Coming in too fast, or
/// too far off level, the ship is DOWN but not landed: it skids as it
/// always did and never settles into the parked state — the crash that will
/// one day be modelled is not quietly turned into a clean landing.
#[test]
fn a_hard_or_tilted_touchdown_does_not_settle() {
    let params = presets::earth_compact();
    // Too fast.
    let hard = over(&params, 0, 1.0, TOUCHDOWN_INTO_MPS * 2.0);
    let s = run(&params, hard, 1_200, Controls::default());
    assert!(
        matches!(s.ship.ground, Ground::Down { clean: false, .. }),
        "{:?}",
        s.ship.ground
    );
    assert!(s.ship.pos_m.length() <= params.planet.radius_m + GEAR_HEIGHT_M + 1e-6);
    // Too tilted: 30° of roll on a gentle descent.
    let mut tilted = over(&params, 0, 1.0, 2.0);
    tilted.ship.orient = DQuat::from_rotation_z(30.0_f64.to_radians());
    let s = run(&params, tilted, 1_200, Controls::default());
    assert!(
        matches!(s.ship.ground, Ground::Down { clean: false, .. }),
        "{:?}",
        s.ship.ground
    );
    // Both are still deterministic, and neither has moved off the ground.
    let a = run(&params, hard, 600, Controls::default());
    let b = run(&params, hard, 600, Controls::default());
    assert_eq!(state_hash(&a), state_hash(&b));
}

/// The parked state is part of the world: two ships that differ only in
/// whether they are landed do not hash the same, and a ship that never
/// touches the ground hashes exactly as it did before there was a ground
/// state at all (the golden hash is the proof of that half).
#[test]
fn the_ground_state_is_in_the_hash() {
    let params = presets::earth_compact();
    let flying = over(&params, 0, 1.0, 2.0);
    let mut landed = flying;
    landed.ship.ground = Ground::Landed {
        body: 0,
        up: DVec3::Y,
    };
    assert_ne!(state_hash(&flying), state_hash(&landed));
}
