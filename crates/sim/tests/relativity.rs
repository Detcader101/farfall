//! Nothing outruns light (SPEC §7.2, eventually §9: the warp drive).

use farfall_sim::{light_limit, presets, step, Controls, LIGHT_SPEED_MPS, RELATIVITY_FROM_MPS};
use glam::DVec3;

/// Below the threshold the kick is the Newtonian kick, bit for bit: the
/// whole game as it is played today is untouched.
#[test]
fn slow_ships_are_newtonian_to_the_bit() {
    let before = DVec3::new(700.0, -30.0, 12.5);
    let newton = DVec3::new(703.3, -31.1, 12.9);
    assert_eq!(light_limit(before, newton), newton);
    let fast = DVec3::new(RELATIVITY_FROM_MPS * 0.999, 0.0, 0.0);
    assert_eq!(light_limit(fast, fast * 1.01), fast * 1.01);
}

/// An absurd engine burning for an absurd time still never reaches c.
#[test]
fn nothing_outruns_light() {
    let mut p = presets::earth_compact();
    // Far from the planet and the air; an engine no one will ever build.
    p.ship.max_thrust_mps2 = DVec3::new(0.0, 0.0, 1.0e8);
    let mut s = presets::circular_orbit(&p, 1.0e9);
    let c = Controls {
        thrust_body: DVec3::new(0.0, 0.0, -1.0),
        ..Default::default()
    };
    let mut last = 0.0;
    for _ in 0..2_400 {
        s = step(&p, &s, c);
        let v = s.ship.vel_mps.length();
        assert!(v < LIGHT_SPEED_MPS, "passed c: {v}");
        assert!(v >= last, "speed fell under constant thrust: {last} -> {v}");
        last = v;
    }
    // Newton would have been at 2·10⁹ m/s by now; we are close to c and
    // still climbing, ever more slowly.
    assert!(last > 0.9 * LIGHT_SPEED_MPS, "{last}");
}

/// The closer to c, the less each burn buys: the same kick gives less
/// speed at 0.9c than at 0.5c.
#[test]
fn acceleration_fades_toward_c() {
    let kick = DVec3::new(1.0e6, 0.0, 0.0);
    let at = |beta: f64| {
        let v = DVec3::new(beta * LIGHT_SPEED_MPS, 0.0, 0.0);
        (light_limit(v, v + kick) - v).length()
    };
    assert!(at(0.5) > at(0.9));
    assert!(at(0.9) > at(0.99));
    assert!(at(0.99) > 0.0);
}
