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
    /// The other bodies. They pull and they can be hit, like the planet;
    /// only the planet has air.
    pub moon: MoonParams,
    pub sun: SunParams,
    pub uranus: PlanetAfarParams,
}

/// Another planet of the system: fixed in the planet's frame, placed
/// relative to the Sun. (Everything orbits far too slowly to matter over a
/// session; the frame is frozen at one moment of the year.)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetAfarParams {
    pub radius_m: f64,
    pub mu: f64,
    /// Its position relative to the Sun's centre, metres.
    pub from_sun: DVec3,
}

impl PlanetAfarParams {
    pub fn centre(&self, sun: &SunParams) -> DVec3 {
        sun.centre() + self.from_sun
    }
}

/// A moon on a circular orbit about the planet, in the XZ plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoonParams {
    pub radius_m: f64,
    pub mu: f64,
    pub orbit_m: f64,
}

impl MoonParams {
    /// Kepler, from the planet's μ: the period follows from the world.
    pub fn period_s(&self, planet_mu: f64) -> f64 {
        core::f64::consts::TAU * libm::sqrt(self.orbit_m * self.orbit_m * self.orbit_m / planet_mu)
    }

    /// Centre at sim time `t`, planet frame. Starts on +X.
    pub fn centre(&self, planet_mu: f64, t_s: f64) -> DVec3 {
        let phase = core::f64::consts::TAU * t_s / self.period_s(planet_mu);
        DVec3::new(
            self.orbit_m * libm::cos(phase),
            0.0,
            self.orbit_m * libm::sin(phase),
        )
    }
}

/// The sun: fixed in the planet's frame, a direction and a distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunParams {
    pub radius_m: f64,
    pub mu: f64,
    pub distance_m: f64,
    pub dir: DVec3,
}

impl SunParams {
    pub fn centre(&self) -> DVec3 {
        self.dir.normalize() * self.distance_m
    }
}

/// A gravitating, solid sphere, as the integrator sees every body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub centre: DVec3,
    pub radius_m: f64,
    pub mu: f64,
}

impl WorldParams {
    /// Each body's velocity in the planet's frame at `t`, m/s: the planet
    /// and the Sun are still, the Moon goes round. Finite difference over
    /// one step, so it is exactly consistent with where `bodies` puts it
    /// from one step to the next.
    pub fn body_velocities(&self, t_s: f64) -> [DVec3; BODIES] {
        let moon = (self.moon.centre(self.planet.mu, t_s + DT)
            - self.moon.centre(self.planet.mu, t_s))
            / DT;
        [DVec3::ZERO, moon, DVec3::ZERO, DVec3::ZERO]
    }

    /// Every body at sim time `t`: planet, moon, sun, uranus.
    pub fn bodies(&self, t_s: f64) -> [Body; BODIES] {
        [
            Body {
                centre: DVec3::ZERO,
                radius_m: self.planet.radius_m,
                mu: self.planet.mu,
            },
            Body {
                centre: self.moon.centre(self.planet.mu, t_s),
                radius_m: self.moon.radius_m,
                mu: self.moon.mu,
            },
            Body {
                centre: self.sun.centre(),
                radius_m: self.sun.radius_m,
                mu: self.sun.mu,
            },
            Body {
                centre: self.uranus.centre(&self.sun),
                radius_m: self.uranus.radius_m,
                mu: self.uranus.mu,
            },
        ]
    }
}

/// How many bodies there are: planet, Moon, Sun, Uranus.
pub const BODIES: usize = 4;

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
    /// Altitude above which there is exactly no air, m. An exponential never
    /// reaches zero, and with the air able to torque the hull a ship in
    /// "space" would otherwise be brushed by 10⁻⁹ kg/m³ forever — enough to
    /// break the contract that vacuum conserves rotation bit for bit.
    pub atmo_top_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShipParams {
    pub mass_kg: f64,
    /// Max thrust acceleration per body axis at |control| = 1, m/s².
    /// x: lateral, y: vertical, z: the main engine. They are deliberately
    /// unequal — a ship with one big motor behind it and small thrusters
    /// everywhere else accelerates far better forwards than sideways, and that
    /// asymmetry is most of what makes it feel like a ship rather than a cursor.
    pub max_thrust_mps2: DVec3,
    /// Multiplier applied to thrust while boosting.
    pub boost_multiplier: f64,
    /// Fraction of thrust the flight computer may spend steering the velocity
    /// vector toward the nose. Zero makes the ship a pure Newtonian body.
    pub align_authority: f64,
    /// Time constant for that steering, seconds. Larger feels heavier.
    pub align_tau_s: f64,
    /// Retro-thrust available to the air brake, m/s². Independent of the main
    /// engine: stopping is its own system, and a ship that can dart needs to
    /// shed speed faster than it built it.
    pub brake_mps2: f64,
    /// Time constant for the brake, seconds.
    pub brake_tau_s: f64,
    /// Time constant of the emergency gyro's despin, seconds.
    pub despin_tau_s: f64,
    /// The hyper drive: the wormhole drive half-engaged — a field that
    /// moves space rather than the ship, so relativity's wall does not
    /// apply to it. Held, the velocity runs exponentially toward this
    /// speed down the nose (the Sun in about a minute), whatever the ship
    /// was doing; the drive's own wall is the whole wormhole.
    pub hyper_max_mps: f64,
    /// The field's time constant: how long the run-up takes to settle.
    pub hyper_tau_s: f64,
    /// Restitution on hitting the ground: 0 lands, 1 bounces perfectly.
    pub ground_restitution: f64,
    /// Fraction of tangential speed kept per second while in contact.
    pub ground_friction: f64,
    /// Max angular acceleration per body axis at |control| = 1, rad/s².
    /// x: pitch, y: yaw, z: roll. Roll is the slowest on purpose: it is the axis
    /// that most easily disorients, and a ship that rolls lazily reads as
    /// heavy while still turning quickly where it matters.
    pub max_torque_radps2: DVec3,
    /// Drag coefficient × reference area, m² (F_drag = ½·ρ·|v|²·CdA, opposing v),
    /// with the air coming straight down the nose. The SHAPE, part one.
    pub cd_area_m2: f64,
    /// The shape, part two: the same product with the air hitting the hull
    /// broadside. A long slender ship is many times draggier sideways than
    /// nose-on; the drag at any attitude blends between the two by sin²α, α
    /// being the angle between the nose and the airflow.
    pub cd_area_side_m2: f64,
    /// Lift slope × area, m²: how much the hull acts as a wing. Lift sits
    /// perpendicular to the airflow, in the plane of nose and airflow, and
    /// scales as sin α·cos α — rising with angle of attack, then stalling
    /// back to nothing broadside, the way a flat plate does.
    pub lift_area_m2: f64,
    /// Where the air pushes: the centre of pressure, metres along the body
    /// axis, aft positive (the nose is −Z, so aft is +Z). Fins and a tail
    /// put it behind the middle.
    pub centre_of_pressure_m: f64,
    /// Where the mass sits: the centre of gravity, metres along the body
    /// axis, aft positive. Engines at the back put it behind the middle.
    ///
    /// This is the variable that decides what the air DOES to the attitude.
    /// The aerodynamic force acts at the centre of pressure and the ship
    /// rotates about its centre of gravity, so the lever arm between them is
    /// the whole story: pressure behind gravity and the ship weathervanes —
    /// the nose follows the airflow, and as gravity bends the trajectory
    /// down, the nose drops with it, like a jet. Gravity ahead of pressure
    /// and the same air flips the ship end for end, like an arrow thrown
    /// tail first.
    pub centre_of_gravity_m: f64,
    /// Moment of inertia per body axis (pitch, yaw, roll), kg·m². Turns the
    /// aerodynamic torque into angular acceleration. The pilot's own
    /// torque is already expressed as acceleration (`max_torque_radps2`),
    /// so this only matters to the air.
    pub inertia_kgm2: DVec3,
    /// Rotational aerodynamic damping, m⁴: torque = −ρ·|v|·k·ω. Air resists
    /// spin as well as motion; without it a weathervaning ship would hunt
    /// about the wind forever.
    pub aero_damping_m4: f64,
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
    /// Air brake: dump velocity, whatever direction it points in.
    pub brake: bool,
    /// The emergency gyro: kill the spin, however fast, on a fixed time
    /// constant — the one control that ignores the torque limits, for the
    /// tumble nothing else gets you out of.
    pub despin: bool,
    /// The hyper drive, held: the ship is hauled down its nose.
    pub hyper: bool,
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
            brake: self.brake,
            despin: self.despin,
            hyper: self.hyper,
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
                // 12.5 scale heights: ρ/ρ₀ ≈ 4·10⁻⁶ at the top, well below
                // anything the hull, the ear or the eye can register.
                atmo_top_m: 25_000.0,
            },
            // The real Moon and Sun at the same 1:100, each keeping its real
            // surface gravity the way the planet keeps 9.81 (μ = g·R²):
            // the Moon's 1.62 m/s², the Sun's 274. Distances at 1:100 too,
            // so each subtends the half degree it really does.
            moon: MoonParams {
                radius_m: 17_374.0,
                mu: 1.62 * 17_374.0 * 17_374.0,
                orbit_m: 3_844_000.0,
            },
            sun: SunParams {
                radius_m: 6_963_400.0,
                mu: 274.0 * 6_963_400.0 * 6_963_400.0,
                distance_m: 1_495_978_700.0,
                dir: DVec3::new(0.62, 0.42, -0.66),
            },
            // Uranus at 1:100 too: 25,362 km radius, 8.69 m/s² at the
            // cloud tops, 19.2 AU from the Sun — placed round the far side
            // of the Sun from us, a little out of our plane, so the line
            // there runs past the Sun on the way.
            uranus: PlanetAfarParams {
                radius_m: 253_620.0,
                mu: 8.69 * 253_620.0 * 253_620.0,
                from_sun: DVec3::new(0.55, 0.10, -0.83).normalize() * 28_706_000_000.0,
            },
            ship: ShipParams {
                mass_kg: 12_000.0,
                // Heavy on the stick, quick off the line: rotation stays
                // deliberate while translation answers immediately.
                // Main engine roughly 3.5x the manoeuvring thrusters.
                max_thrust_mps2: DVec3::new(45.0, 45.0, 165.0),
                boost_multiplier: 3.5,
                // Most of the manoeuvring thrusters are the flight computer's
                // to spend on steering; the main engine stays the pilot's.
                align_authority: 0.8,
                align_tau_s: 1.1,
                brake_mps2: 210.0,
                brake_tau_s: 0.35,
                despin_tau_s: 0.6,
                // 1 AU at 1:100 is 1.5e9 m: at 3e7 m/s that is fifty
                // seconds, plus the run-up.
                hyper_max_mps: 3.0e7,
                hyper_tau_s: 6.0,
                // Landing, not bouncing — and enough friction to come to rest
                // rather than skating around the equator forever.
                ground_restitution: 0.0,
                ground_friction: 0.4,
                max_torque_radps2: DVec3::new(1.7, 1.4, 0.8),
                // Sleek nose-on — a fighter's Cd over a few square metres
                // of frontal area. Sized so gravity alone can do something:
                // a nose-down dive from the 12 km spawn passes mach 1 below
                // 3 km and reaches ~395 m/s; at 8 m² it topped out at 329
                // and never broke the barrier. Terminal velocity nose-down
                // at sea level is ~360 m/s.
                cd_area_m2: 1.5,
                // The same hull broadside: thirty times the nose. Shape is
                // the brake — pitch across the airflow to shed speed.
                cd_area_side_m2: 45.0,
                lift_area_m2: 30.0,
                // Tail and fins behind the middle; engines further back
                // still but balanced by the cockpit and the forward tanks,
                // so gravity sits a metre ahead of pressure: stable, with a
                // lazy weathervane rather than a snap.
                centre_of_pressure_m: 2.5,
                centre_of_gravity_m: 1.5,
                // A 12 t, ~10 m body: m·L²/12 about pitch and yaw; roll is
                // a much tighter radius.
                inertia_kgm2: DVec3::new(100_000.0, 100_000.0, 20_000.0),
                aero_damping_m4: 400.0,
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
    if h >= planet.atmo_top_m {
        return 0.0;
    }
    planet.atmo_rho0 * libm::exp(-h / planet.atmo_scale_height_m)
}

/// What the air does to the ship this instant: a translational
/// acceleration in the world frame and an angular acceleration in the body
/// frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aero {
    pub accel_world: DVec3,
    pub ang_accel_body: DVec3,
    /// Angle of attack, radians: between the nose and the airflow. Zero
    /// nose-first, π/2 broadside, π flying backwards. For instruments.
    pub alpha_rad: f64,
    /// Dynamic pressure ½·ρ·v², Pa. For instruments and sound.
    pub q_pa: f64,
}

/// Aerodynamics of the hull, evaluated in the body frame (SPEC §7.2).
///
/// Two independent things decide it. The SHAPE sets the forces: drag blends
/// from the nose-on value to the broadside one by sin²α, and the hull lifts
/// like a plate, sin α·cos α, perpendicular to the airflow. The BALANCE sets
/// the torque: those forces act at the centre of pressure, the ship turns
/// about its centre of gravity, and the lever between them — a single signed
/// distance along the hull — decides whether the nose is pulled into the
/// wind or thrown out of it. Air also damps spin.
///
/// Exactly zero in vacuum: ρ = 0 zeroes every term, so orbits above the air
/// keep their contract (and the golden hash's vacuum regime) bit for bit.
pub fn aero_forces(ship_p: &ShipParams, rho: f64, ship: &ShipState) -> Aero {
    let speed = ship.vel_mps.length();
    if rho <= 0.0 || speed <= 0.0 {
        return Aero {
            accel_world: DVec3::ZERO,
            ang_accel_body: DVec3::ZERO,
            alpha_rad: 0.0,
            q_pa: 0.0,
        };
    }
    let q = 0.5 * rho * speed * speed;

    // Airflow in the body frame, and the angle it makes with the nose.
    let v_body = ship.orient.conjugate() * ship.vel_mps;
    let v_hat = v_body / speed;
    let nose = DVec3::NEG_Z;
    let cos_a = v_hat.dot(nose).clamp(-1.0, 1.0);
    let sin2_a = (1.0 - cos_a * cos_a).max(0.0);
    let sin_a = libm::sqrt(sin2_a);
    let alpha = libm::acos(cos_a);

    // Drag: the shape seen by the airflow.
    let cd_area = ship_p.cd_area_m2 + (ship_p.cd_area_side_m2 - ship_p.cd_area_m2) * sin2_a;
    let f_drag = v_hat * (-q * cd_area);

    // Lift: perpendicular to the airflow, toward the side the nose is on.
    // Degenerates cleanly to zero nose-on and broadside.
    let perp = nose - v_hat * cos_a;
    let f_lift = if sin_a > 1e-9 {
        (perp / sin_a) * (q * ship_p.lift_area_m2 * sin_a * cos_a)
    } else {
        DVec3::ZERO
    };
    let f_body = f_drag + f_lift;

    // Torque about the centre of gravity, from the force at the centre of
    // pressure. Aft is +Z.
    let arm = DVec3::new(
        0.0,
        0.0,
        ship_p.centre_of_pressure_m - ship_p.centre_of_gravity_m,
    );
    let torque = arm.cross(f_body) - ship.ang_vel_radps * (rho * speed * ship_p.aero_damping_m4);

    Aero {
        accel_world: ship.orient * (f_body / ship_p.mass_kg),
        ang_accel_body: torque / ship_p.inertia_kgm2,
        alpha_rad: alpha,
        q_pa: q,
    }
}

/// The speed of light, m/s. A universal constant, not a planet parameter.
pub const LIGHT_SPEED_MPS: f64 = 299_792_458.0;

/// Below this speed the relativistic correction is under a part in 10⁹
/// and is skipped entirely, so that everything a ship does today — orbit,
/// entry, a dive, a boost — is integrated bit for bit as before (the
/// golden hash is the proof). Above it, special relativity: no engine, no
/// amount of time, gets the ship to c. That is the wall a wormhole drive
/// will one day have to go around rather than through.
pub const RELATIVITY_FROM_MPS: f64 = 9_500.0;

/// Relativistic velocity kick. `newton` is where a Newtonian step would
/// have put the velocity; the change is scaled by γ⁻³ — the rate at which
/// proper acceleration along the motion turns into coordinate velocity —
/// which goes to zero as v → c. Exact identity below RELATIVITY_FROM_MPS.
pub fn light_limit(before: DVec3, newton: DVec3) -> DVec3 {
    let speed = before.length();
    if speed < RELATIVITY_FROM_MPS {
        return newton;
    }
    let beta2 = (speed * speed) / (LIGHT_SPEED_MPS * LIGHT_SPEED_MPS);
    let factor = (1.0 - beta2).max(0.0);
    let scale = factor * libm::sqrt(factor);
    let vel = before + (newton - before) * scale;
    // Never past c, whatever the arithmetic: the ceiling is the ceiling.
    let v = vel.length();
    if v >= LIGHT_SPEED_MPS {
        vel * ((LIGHT_SPEED_MPS * (1.0 - 1e-12)) / v)
    } else {
        vel
    }
}

/// Gravity at a point: a = −μ·r̂/|r|². The one expression the integrator
/// uses, exposed so an instrument can subtract exactly what the sim added —
/// felt acceleration is what is left of Δv/Δt once this is gone.
pub fn gravity(planet: &PlanetParams, pos_m: DVec3) -> DVec3 {
    let r = pos_m.length();
    pos_m * (-planet.mu / (r * r * r))
}

/// Gravity from every body at sim time `t`, in the planet's frame.
///
/// The planet sits at the origin and does not move, which is only honest
/// because it is in free fall about the Sun (and towards the Moon): that
/// frame is accelerating with the planet, so what a ship in it feels from
/// any other body is the *difference* between that body's pull here and its
/// pull on the planet — the tide. Near the planet that is tiny; close to the
/// Moon or the Sun it is, to all intents, that body's own gravity.
pub fn gravity_all(params: &WorldParams, t_s: f64, pos_m: DVec3) -> DVec3 {
    let bodies = params.bodies(t_s);
    let mut a = point_gravity(bodies[0].mu, pos_m - bodies[0].centre);
    for b in &bodies[1..] {
        a += point_gravity(b.mu, pos_m - b.centre) - point_gravity(b.mu, -b.centre);
    }
    a
}

fn point_gravity(mu: f64, rel: DVec3) -> DVec3 {
    let r = rel.length();
    if r > 0.0 {
        rel * (-mu / (r * r * r))
    } else {
        DVec3::ZERO
    }
}

/// What the pilot feels across one step from `before` to `after`: the
/// change in velocity per unit time, less gravity at the start of the
/// step (a body in free fall feels nothing). Engine, air, brake, ground
/// — all of it, in world frame. Exact for the sim's own steps, because
/// the integrator kicks with gravity at the start position. `t_s` is the
/// sim time at `before`.
pub fn felt_acceleration(
    params: &WorldParams,
    t_s: f64,
    before: &ShipState,
    after: &ShipState,
) -> DVec3 {
    (after.vel_mps - before.vel_mps) / DT - gravity_all(params, t_s, before.pos_m)
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

    let a_gravity = gravity_all(params, state.time_s, ship.pos_m);

    // The air: drag and lift from the hull's shape, and the torque they
    // exert about the centre of gravity. `a_drag` keeps its name — it is the
    // translational half, drag and lift together.
    let aero = aero_forces(&params.ship, atmo_density(planet, r), ship);
    let a_drag = aero.accel_world;

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
        // The weakest axis sets the budget: steering can point any direction,
        // so it may only promise what the ship can deliver in every direction.
        let cap = params.ship.max_thrust_mps2.min_element() * params.ship.align_authority;
        demand.clamp_length_max(cap)
    } else {
        DVec3::ZERO
    };

    // Air brake: rate-limited retro-thrust opposing the velocity, whichever way
    // it points. Deliberately not tied to the main engine — being able to stop
    // faster than you can start is what makes a ship dart rather than drift.
    let a_brake = if c.brake {
        let demand = -ship.vel_mps / params.ship.brake_tau_s;
        demand.clamp_length_max(params.ship.brake_mps2)
    } else {
        DVec3::ZERO
    };

    // Kick, then drift. The branch with neither aid is kept textually
    // identical, so a system the pilot never engages cannot perturb the
    // physics contract (or the golden hash).
    let vel = if c.assist || c.brake {
        ship.vel_mps + (a_gravity + a_drag + a_thrust + a_align + a_brake) * DT
    } else {
        ship.vel_mps + (a_gravity + a_drag + a_thrust) * DT
    };
    let vel = light_limit(ship.vel_mps, vel);
    // The hyper drive: space moves, not the ship, so it comes AFTER the
    // light limit — the velocity runs toward hyper_max down the nose on
    // the field's time constant, lateral motion folded away with it.
    let vel = if c.hyper {
        let nose = ship.orient * DVec3::NEG_Z;
        let k = 1.0 - libm::exp(-DT / params.ship.hyper_tau_s);
        vel + (nose * params.ship.hyper_max_mps - vel) * k
    } else {
        vel
    };
    let pos = ship.pos_m + vel * DT;

    // Ground contact. The planet is a sphere, so this is exact rather than a
    // mesh query: if the new position is inside it, put the ship back on the
    // surface, remove the velocity that drove it in, and scrub the rest.
    //
    // No tunnelling is possible at any speed the ship can reach — a step moves
    // metres against a radius of tens of kilometres, and a straight line into a
    // sphere cannot cross the far side without crossing the near one.
    // Every body is solid: the same contact, about each one's centre, in
    // that body's own frame — the Moon is moving, and a ship set down on it
    // must come to rest on *it*, carried along, not scrubbed to a stop in
    // the planet's frame while the ground slides away underneath.
    let mut pos = pos;
    let mut vel = vel;
    let body_vel = params.body_velocities(state.time_s);
    for (i, b) in params.bodies(state.time_s + DT).iter().enumerate() {
        let rel = pos - b.centre;
        let r = rel.length();
        if r < b.radius_m && r > 0.0 {
            let up = rel / r;
            let v_rel = vel - body_vel[i];
            let into = v_rel.dot(up);
            let tangential = v_rel - up * into;
            let bounced = if into < 0.0 {
                up * (-into * params.ship.ground_restitution)
            } else {
                up * into
            };
            let friction = libm::pow(params.ship.ground_friction, DT);
            pos = b.centre + up * b.radius_m;
            vel = body_vel[i] + tangential * friction + bounced;
        }
    }

    // Rotation: identity inertia tensor for now (SPEC: revisit with ship variety).
    //
    // The assist-off branch reproduces the original expression *exactly*, down
    // to the parenthesisation: float multiplication isn't associative, so
    // `t * (m * DT)` and `(t * m) * DT` can differ in the last bit — enough to
    // move the golden hash. A feature that is off by default must not perturb
    // the physics contract, so the two paths stay textually separate.
    // The air's torque applies in every branch: it is not an aid the pilot
    // can switch off, it is the atmosphere.
    let ang_vel = if c.assist {
        // Torque-limited damping toward zero spin, blended per axis by how much
        // the pilot is *not* commanding that axis: full input means no fighting
        // the pilot, no input means full damping, and partial input blends.
        // The damping term is the acceleration that would exactly cancel the
        // current spin in one step, clipped to the ship's real authority — so
        // it converges to precisely zero and can never overshoot into a wobble.
        let max = params.ship.max_torque_radps2;
        let cancel = (-ship.ang_vel_radps / DT).clamp(-max, max);
        let gain = DVec3::ONE - c.torque_body.abs();
        ship.ang_vel_radps + (c.torque_body * max + gain * cancel + aero.ang_accel_body) * DT
    } else {
        ship.ang_vel_radps
            + (c.torque_body * params.ship.max_torque_radps2 + aero.ang_accel_body) * DT
    };
    // The emergency gyro: an exponential bleed of the spin, whatever the
    // torque limits say — deterministic, and it cannot overshoot.
    let ang_vel = if c.despin {
        ang_vel * libm::exp(-DT / params.ship.despin_tau_s)
    } else {
        ang_vel
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
