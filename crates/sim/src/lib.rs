//! farfall-sim — deterministic, headless simulation core.
//!
//! Contract (SPEC §5.1, §7):
//! - Pure data + pure functions. No GPU, window, thread, clock, or filesystem deps.
//! - Fixed timestep [`DT`]; the caller owns the accumulator.
//! - Bit-identical results across platforms and runs: f64 IEEE ops plus `libm`
//!   transcendentals only (never `std` float transcendentals — platform libm differs).
//! - The whole mutable world is [`WorldState`]; it is hashable ([`state_hash`]) for
//!   determinism tests and, later, netcode desync detection.
//!
//! Frame: planet-centered inertial. Units: SI (meters, seconds, kilograms, radians).
//!
//! Body frame is **right-handed**: +X right, +Y up, **−Z forward (the nose)**.
//! This is the glam/OpenGL convention and it is not negotiable — declaring
//! "+Z forward" alongside "+X right, +Y up" describes a *left*-handed frame,
//! which silently mirrors yaw, roll, and strafe against right-handed rotation
//! math. (It did exactly that here until it was caught by flying it.)

#![forbid(unsafe_code)]

use glam::{DQuat, DVec3};

/// Fixed simulation timestep, seconds (SPEC §7.2).
pub const DT: f64 = 1.0 / 120.0;

/// Immutable per-scenario parameters (SPEC §7.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldParams {
    pub planet: PlanetParams,
    pub ship: ShipParams,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetParams {
    /// Planet radius, m.
    pub radius_m: f64,
    /// Standard gravitational parameter μ = GM, m³/s².
    pub mu: f64,
    /// Atmosphere sea-level density, kg/m³.
    pub atmo_rho0: f64,
    /// Atmosphere scale height, m (ρ = ρ₀·e^(−h/H)).
    pub atmo_scale_height_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShipParams {
    pub mass_kg: f64,
    /// Max thrust acceleration along each body axis at |control| = 1, m/s².
    pub max_thrust_mps2: f64,
    /// Multiplier applied to thrust while boosting.
    pub boost_multiplier: f64,
    /// Fraction of thrust the flight computer may spend steering the velocity
    /// vector toward the nose. Zero makes the ship a pure Newtonian body.
    pub align_authority: f64,
    /// Time constant for that steering, seconds. Larger feels heavier.
    pub align_tau_s: f64,
    /// Max angular acceleration about each body axis at |control| = 1, rad/s².
    pub max_torque_radps2: f64,
    /// Drag coefficient × reference area, m² (F_drag = ½·ρ·|v|²·CdA, opposing v).
    pub cd_area_m2: f64,
}

/// The entire mutable world (SPEC §7.1). Plain data by policy — no ECS until
/// entity counts demand it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldState {
    pub time_s: f64,
    pub ship: ShipState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShipState {
    /// Position, planet-centered inertial, m.
    pub pos_m: DVec3,
    /// Velocity, m/s.
    pub vel_mps: DVec3,
    /// Body→world rotation.
    pub orient: DQuat,
    /// Angular velocity, body frame, rad/s.
    pub ang_vel_radps: DVec3,
}

/// Player/agent intent for one tick. Components are clamped to [-1, 1] on use,
/// so out-of-range input cannot break determinism or physics.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Controls {
    /// Thrust demand, body frame (+x right, +y up, −z forward).
    pub thrust_body: DVec3,
    /// Torque demand, body frame (pitch, yaw, roll).
    pub torque_body: DVec3,
    /// The flight computer. It does two things: bleeds off residual spin, and
    /// steers the velocity vector toward the nose so the ship goes where it is
    /// pointed.
    ///
    /// The second half is a deliberate departure from Newton. In a vacuum,
    /// velocity is independent of attitude: you aim somewhere, burn, and still
    /// drift the way you were going. That is correct, and it feels wrong —
    /// an X-wing, a helicopter and a VTOL all go where they point, because air
    /// couples heading to motion. The assist plays the part of that air, using
    /// nothing but the ship's own thrust, capped by `align_authority`.
    ///
    /// Off by default, which keeps the physics contract (and the golden hash)
    /// identical to a world where this feature doesn't exist. With it off the
    /// ship is a pure ballistic body and orbits are conserved exactly.
    pub assist: bool,
    /// Engage the overdrive: same thrust axes, much more of it.
    pub boost: bool,
}

impl Controls {
    fn clamped(self) -> Self {
        Self {
            thrust_body: self
                .thrust_body
                .clamp(DVec3::splat(-1.0), DVec3::splat(1.0)),
            torque_body: self
                .torque_body
                .clamp(DVec3::splat(-1.0), DVec3::splat(1.0)),
            assist: self.assist,
            boost: self.boost,
        }
    }
}

/// Scenario presets (SPEC §7.5). The models are scale-free; these are just numbers.
pub mod presets {
    use super::*;

    /// Compact Earth, 1:100 linear scale, surface gravity kept ≈ 9.81 m/s²,
    /// atmosphere deliberately thickened for visual depth (SPEC §7.5).
    /// Low circular orbit ≈ 790 m/s, period ≈ 8.5 min.
    pub fn earth_compact() -> WorldParams {
        let radius_m = 63_710.0;
        WorldParams {
            planet: PlanetParams {
                radius_m,
                // g = μ/R² ⇒ μ = g·R²
                mu: 9.81 * radius_m * radius_m,
                atmo_rho0: 1.225,
                atmo_scale_height_m: 2_000.0,
            },
            ship: ShipParams {
                mass_kg: 12_000.0,
                // Heavy on the stick, quick off the line: rotation stays
                // deliberate while translation answers immediately.
                max_thrust_mps2: 85.0,
                boost_multiplier: 3.5,
                // Half the engine is available to bend the velocity vector
                // toward the nose; the rest is the pilot's.
                align_authority: 0.5,
                align_tau_s: 1.4,
                max_torque_radps2: 1.5,
                cd_area_m2: 8.0,
            },
        }
    }

    /// A ship in a circular orbit at `altitude_m`, coasting nose-first: with an
    /// identity orientation the nose (−Z) points along the velocity, so the
    /// pilot starts looking where they are going.
    pub fn circular_orbit(params: &WorldParams, altitude_m: f64) -> WorldState {
        let r = params.planet.radius_m + altitude_m;
        let speed = libm::sqrt(params.planet.mu / r);
        WorldState {
            time_s: 0.0,
            ship: ShipState {
                pos_m: DVec3::new(r, 0.0, 0.0),
                vel_mps: DVec3::new(0.0, 0.0, -speed),
                orient: DQuat::IDENTITY,
                ang_vel_radps: DVec3::ZERO,
            },
        }
    }
}

/// Atmospheric density at radius `r_m` from planet center, kg/m³.
/// Exponential profile; zero below the surface never happens in practice but the
/// formula is total (no branches that could differ across platforms).
pub fn atmo_density(planet: &PlanetParams, r_m: f64) -> f64 {
    let h = r_m - planet.radius_m;
    planet.atmo_rho0 * libm::exp(-h / planet.atmo_scale_height_m)
}

/// Advance the world by exactly one fixed step [`DT`].
///
/// Symplectic (semi-implicit) Euler: kick velocity with acceleration at the current
/// position, then drift position with the new velocity. Bounded energy error on
/// orbits (SPEC §7.2).
pub fn step(params: &WorldParams, state: &WorldState, controls: Controls) -> WorldState {
    let c = controls.clamped();
    let ship = &state.ship;
    let planet = &params.planet;

    let r = ship.pos_m.length();

    // Gravity: a = -μ·r̂/|r|²
    let a_gravity = ship.pos_m * (-planet.mu / (r * r * r));

    // Drag: F = ½·ρ·|v|²·CdA opposing velocity ⇒ a = F/m.
    let speed = ship.vel_mps.length();
    let a_drag = if speed > 0.0 {
        let rho = atmo_density(planet, r);
        let f = 0.5 * rho * speed * speed * params.ship.cd_area_m2;
        ship.vel_mps * (-f / (params.ship.mass_kg * speed))
    } else {
        DVec3::ZERO
    };

    // Thrust: body-frame demand rotated into world frame. Rotating the demand
    // by the ship's own orientation is what makes the controls ship-relative —
    // "forward" is always where the nose points, at any attitude, with no
    // reference to the planet, the orbit, or the camera.
    //
    // The unboosted expression is kept textually intact so that a boost the
    // pilot never engages cannot perturb the physics contract.
    let a_thrust = if c.boost {
        ship.orient * (c.thrust_body * (params.ship.max_thrust_mps2 * params.ship.boost_multiplier))
    } else {
        ship.orient * (c.thrust_body * params.ship.max_thrust_mps2)
    };

    // The flight computer's translational half: cancel the velocity that is not
    // along the nose, within a fixed slice of the engine. Rate-limited rather
    // than instant, so heading changes feel like a ship swinging its momentum
    // around rather than a cursor being dragged.
    let a_align = if c.assist {
        let nose = ship.orient * DVec3::NEG_Z;
        let lateral = ship.vel_mps - nose * ship.vel_mps.dot(nose);
        let demand = -lateral / params.ship.align_tau_s;
        let cap = params.ship.max_thrust_mps2 * params.ship.align_authority;
        demand.clamp_length_max(cap)
    } else {
        DVec3::ZERO
    };

    // Kick, then drift. The assist-off branch is kept textually identical, so a
    // flight computer the pilot never switches on cannot perturb the contract.
    let vel = if c.assist {
        ship.vel_mps + (a_gravity + a_drag + a_thrust + a_align) * DT
    } else {
        ship.vel_mps + (a_gravity + a_drag + a_thrust) * DT
    };
    let pos = ship.pos_m + vel * DT;

    // Rotation: identity inertia tensor for now (SPEC: revisit with ship variety).
    //
    // The assist-off branch reproduces the original expression *exactly*, down
    // to the parenthesisation: float multiplication isn't associative, so
    // `t * (m * DT)` and `(t * m) * DT` can differ in the last bit — enough to
    // move the golden hash. A feature that is off by default must not perturb
    // the physics contract, so the two paths stay textually separate.
    let ang_vel = if c.assist {
        // Torque-limited damping toward zero spin, blended per axis by how much
        // the pilot is *not* commanding that axis: full input means no fighting
        // the pilot, no input means full damping, and partial input blends.
        // The damping term is the acceleration that would exactly cancel the
        // current spin in one step, clipped to the ship's real authority — so
        // it converges to precisely zero and can never overshoot into a wobble.
        let max = params.ship.max_torque_radps2;
        let cancel = (-ship.ang_vel_radps / DT).clamp(DVec3::splat(-max), DVec3::splat(max));
        let gain = DVec3::ONE - c.torque_body.abs();
        ship.ang_vel_radps + (c.torque_body * max + gain * cancel) * DT
    } else {
        ship.ang_vel_radps + c.torque_body * (params.ship.max_torque_radps2 * DT)
    };
    // dq/dt = ½·ω_world·q, ω in world frame = orient · ω_body
    let w_world = ship.orient * ang_vel;
    let dq = DQuat::from_xyzw(w_world.x, w_world.y, w_world.z, 0.0) * ship.orient;
    let orient = DQuat::from_xyzw(
        ship.orient.x + 0.5 * dq.x * DT,
        ship.orient.y + 0.5 * dq.y * DT,
        ship.orient.z + 0.5 * dq.z * DT,
        ship.orient.w + 0.5 * dq.w * DT,
    )
    .normalize();

    WorldState {
        time_s: state.time_s + DT,
        ship: ShipState {
            pos_m: pos,
            vel_mps: vel,
            orient,
            ang_vel_radps: ang_vel,
        },
    }
}

/// Specific orbital energy (per unit mass): v²/2 − μ/r. Conserved on drag-free
/// coasts; the invariant tests lean on it.
pub fn specific_energy(planet: &PlanetParams, ship: &ShipState) -> f64 {
    0.5 * ship.vel_mps.length_squared() - planet.mu / ship.pos_m.length()
}

/// FNV-1a 64 over the bit patterns of every state field, in defined order
/// (SPEC §7.4). Hand-rolled: zero deps, stable forever by construction.
pub fn state_hash(state: &WorldState) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut eat = |v: f64| {
        for b in v.to_bits().to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(PRIME);
        }
    };
    let s = &state.ship;
    eat(state.time_s);
    for v in [s.pos_m, s.vel_mps, s.ang_vel_radps] {
        eat(v.x);
        eat(v.y);
        eat(v.z);
    }
    eat(s.orient.x);
    eat(s.orient.y);
    eat(s.orient.z);
    eat(s.orient.w);
    h
}
