//! farfall-app — native shell (SPEC §5.1).
//!
//! Owns the window, the fixed-timestep accumulator (SPEC §7.2), and the
//! sim → render translation. The sim is authoritative (SPEC §5.2): this loop
//! feeds it inputs and *reads* state; nothing here mutates the world directly.
//!
//! M1 scope: the ship is hand-flown. Keys map to sim [`Controls`] (see
//! [`input`]), the camera rides the hull looking down the nose, and rotational
//! flight assist is toggleable. Planet, HUD, and sun arrive in M1 tasks 4-6.

mod capture;
mod input;
mod telemetry;

use glam::{DQuat, DVec3};
use std::sync::Arc;
use std::time::{Duration, Instant};

use capture::Capture;
use farfall_audio::Audio;
use farfall_render::{
    bake::BakedMaps,
    blit::BlitPass,
    gauge::{AltitudeFade, GaugeFade, GaugePass, GaugeUniforms, HoloSway, MachAlert},
    hud::HudPass,
    planet::{PlanetAppearance, PlanetPass, PlanetUniforms},
    starfield::StarfieldPass,
    text::TextBitmap,
    thermal::{PlasmaPass, PlasmaUniforms, ThermalInputs, ThermalPass},
    CameraFrame, FrameUniforms, SceneTarget,
};
use farfall_sim as sim;
use input::InputState;
use telemetry::FrameStats;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

const STAR_DENSITY: f64 = 1.0;
/// World-space direction to the sun. Fixed for now: a moving sun is a sim
/// concern (planet rotation, orbit) and does not belong in the renderer.
/// Chosen so the terminator crosses the visible face at spawn.
const SUN_DIR: glam::Vec3 = glam::Vec3::new(0.62, 0.42, -0.66);
/// How often the frame-time window is summarised to the log.
const PERF_LOG_EVERY: Duration = Duration::from_secs(5);
/// Where the velocity gauge sits on the canopy (NDC): first instrument of the
/// lower-right cluster.
const VELOCITY_GAUGE_ANCHOR: [f32; 2] = [0.52, -0.48];
/// The altimeter mirrors it bottom-left: the cluster grows symmetrically.
const ALTITUDE_GAUGE_ANCHOR: [f32; 2] = [-0.52, -0.48];
/// The text readout's top-left corner on the canopy.
const HUD_TEXT_ANCHOR: [f32; 2] = [-0.72, 0.62];
/// Speed of sound for the mach instrument and the sonic boom, m/s. The
/// gauge shader hard-codes the same number for its barrier tick (shaders
/// cannot import Rust consts) — change one, change both.
const MACH1_MPS: f64 = 340.0;

/// Runtime knobs, read from the environment so a perf A/B needs no rebuild:
///   FARFALL_MSAA=1|2|4|8   (default 4)
///   FARFALL_VSYNC=on|off   (default on; off uncaps the frame rate so the
///                           renderer's real headroom is measurable rather
///                           than hidden behind the display's refresh rate)
///   FARFALL_GPU_SYNC=1     (profiling only: block until the GPU finishes each
///                           frame before timing it)
///   FARFALL_WINDOWED=1     (start windowed instead of borderless fullscreen)
///   FARFALL_BENCH=1        (freeze the sim at spawn so every run renders the
///                           identical frame — without this the ship drifts and
///                           two perf runs are not comparable. Forces a window
///                           rather than fullscreen and exits by itself, so a
///                           benchmark can never be left sitting on screen
///                           being mistaken for a game with broken controls.)
///   FARFALL_BENCH_SECONDS  (how long a benchmark runs before quitting; 20)
///   FARFALL_BENCH_ALT      (frozen altitude in metres; low values are the
///                           worst case, a screen filled edge to edge with
///                           ground, which is where this renderer hurts)
///   FARFALL_SCALE=0.25..1  (scene render scale; the HUD stays native)
///   FARFALL_MUTE=1         (no audio stream at all)
///   FARFALL_SKIP=a,b       (profiling only: leave out passes by name —
///                           starfield, planet, plasma, gauge, hud, blit —
///                           so each one's cost can be measured by its absence)
struct Config {
    msaa: u32,
    vsync: bool,
    gpu_sync: bool,
    windowed: bool,
    bench: bool,
    bench_seconds: f64,
    scale: f32,
    skip: Vec<String>,
}

impl Config {
    fn draws(&self, pass: &str) -> bool {
        !self.skip.iter().any(|s| s == pass)
    }

    fn from_env() -> Self {
        let msaa = std::env::var("FARFALL_MSAA")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|n| matches!(n, 1 | 2 | 4 | 8))
            .unwrap_or(4);
        let vsync = !matches!(
            std::env::var("FARFALL_VSYNC").as_deref(),
            Ok("off" | "0" | "false")
        );
        // Without this, CPU-side frame timing measures how fast we can *submit*
        // work, not how long the GPU takes: submission is asynchronous, so the
        // CPU runs ahead and reports sub-millisecond "frames" while the GPU is
        // still busy. Blocking on completion makes the wall clock mean
        // something. It costs pipelining, so it is a profiling mode, not a
        // default.
        let gpu_sync = matches!(
            std::env::var("FARFALL_GPU_SYNC").as_deref(),
            Ok("1" | "on" | "true")
        );
        let mut windowed = matches!(
            std::env::var("FARFALL_WINDOWED").as_deref(),
            Ok("1" | "on" | "true")
        );
        // Screen coverage is what this renderer costs, and coverage depends on
        // altitude — so a moving ship makes every measurement a different
        // scene. Freezing it is the difference between profiling and guessing.
        let bench = matches!(
            std::env::var("FARFALL_BENCH").as_deref(),
            Ok("1" | "on" | "true")
        );
        if bench {
            // Never fullscreen: a benchmark must be visibly not the game.
            windowed = true;
        }
        let bench_seconds = std::env::var("FARFALL_BENCH_SECONDS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(20.0);
        let scale = std::env::var("FARFALL_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.25, 1.0);
        Self {
            msaa,
            vsync,
            gpu_sync,
            windowed,
            bench,
            bench_seconds,
            skip: std::env::var("FARFALL_SKIP")
                .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            scale,
        }
    }
}

/// Presentation-side timing. Separate from the sim clock: the sim advances in
/// fixed steps regardless of how long a frame took, and conflating the two
/// would make the perf numbers a function of the physics rate.
struct Perf {
    stats: FrameStats,
    /// Time spent on the CPU building and encoding the frame, measured
    /// separately from the wall-clock frame time. The difference between the
    /// two is the GPU (plus any vsync wait), and keeping them apart is the only
    /// way to know which half is actually the bottleneck.
    cpu: FrameStats,
    /// Time blocked acquiring a swapchain image — GPU and vsync, not CPU.
    wait: FrameStats,
    last_frame: Instant,
    last_log: Instant,
    last_title: Instant,
}

impl Perf {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            stats: FrameStats::default(),
            cpu: FrameStats::default(),
            wait: FrameStats::default(),
            last_frame: now,
            last_log: now,
            last_title: now,
        }
    }
}

struct Gpu {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    scene: SceneTarget,
    blit: BlitPass,
    starfield: StarfieldPass,
    planet: PlanetPass,
    gauge: GaugePass,
    alt_gauge: GaugePass,
    /// The hull heat field, simulated on the GPU, and the sheath it lights.
    thermal: ThermalPass,
    plasma: PlasmaPass,
    /// Owns the baked textures the planet pass samples.
    _baked: BakedMaps,
    hud: HudPass,
    text: TextBitmap,
    cfg: Config,
    perf: Perf,
    /// Set by a key press; consumed by the next frame.
    capture_requested: bool,
    bench_captured: bool,
}

impl Gpu {
    /// Close out the frame: record its duration, refresh the live readout in
    /// the title bar, and periodically summarise the window to the log.
    fn frame_timing(
        &mut self,
        cpu_seconds: f64,
        wait_seconds: f64,
        altitude_m: f64,
        speed_mps: f64,
        assist: bool,
    ) {
        self.perf.cpu.record(cpu_seconds);
        self.perf.wait.record(wait_seconds);
        let now = Instant::now();
        let dt = now.duration_since(self.perf.last_frame).as_secs_f64();
        self.perf.last_frame = now;
        self.perf.stats.record(dt);

        // 4 Hz is fast enough to feel live and slow enough to stay readable.
        if now.duration_since(self.perf.last_title) >= Duration::from_millis(250) {
            self.perf.last_title = now;
            let fps = self.perf.stats.smoothed_fps();
            let low = self.perf.stats.recent_low_1pct_fps();
            self.text.clear();
            self.text.draw(0, 0, &format!("{fps:.0} FPS"));
            self.text.draw(0, 6, &format!("1% LOW {low:.0}"));

            // CPU against total, side by side, because "the CPU feels busy" is
            // a hypothesis and this is the measurement that settles it.
            let frame_ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };
            let cpu_fps = self.perf.cpu.smoothed_fps();
            let cpu_ms = if cpu_fps > 0.0 { 1000.0 / cpu_fps } else { 0.0 };
            self.text.draw(0, 12, &format!("CPU {cpu_ms:.1}MS"));
            self.text.draw(
                0,
                18,
                &format!("REST {:.1}MS", (frame_ms - cpu_ms).max(0.0)),
            );
            let (sw, sh) = self.scene.size();
            self.text.draw(
                0,
                24,
                &format!(
                    "{}X MSAA  {:.0}%",
                    self.cfg.msaa,
                    self.scene.scale() * 100.0
                ),
            );
            self.text.draw(0, 30, &format!("{sw}X{sh}"));
            self.text.draw(0, 36, &format!("ALT {altitude_m:.0}M"));
            self.text.draw(0, 42, &format!("VEL {speed_mps:.0}M/S"));
            // The flight computer's state lives on the HUD because the log is
            // invisible in fullscreen — X seemed broken when it was merely
            // silent.
            self.text
                .draw(0, 48, if assist { "FC ON" } else { "FC OFF" });
            if self.cfg.bench {
                self.text.draw(0, 54, "BENCH SIM FROZEN");
            }
        }

        if now.duration_since(self.perf.last_log) >= PERF_LOG_EVERY {
            self.perf.last_log = now;
            if let Some(s) = self.perf.stats.take_summary() {
                log::info!(
                    "perf {}x{} {}xMSAA vsync={} gpu_sync={}: {:.1} fps avg \
                     | 1% low {:.1} fps | frame avg {:.2}ms worst {:.2}ms \
                     best {:.2}ms | {} frames",
                    self.config.width,
                    self.config.height,
                    self.cfg.msaa,
                    if self.cfg.vsync { "on" } else { "off" },
                    self.cfg.gpu_sync,
                    s.avg_fps,
                    s.low_1pct_fps,
                    s.avg_ms,
                    s.worst_ms,
                    s.best_ms,
                    s.frames,
                );
                if let (Some(c), Some(w)) =
                    (self.perf.cpu.take_summary(), self.perf.wait.take_summary())
                {
                    log::info!(
                        "perf split: cpu {:.3}ms avg (worst {:.3}) | swapchain wait \
                         {:.2}ms avg (worst {:.2}) — the wait is GPU and vsync, \
                         not CPU",
                        c.avg_ms,
                        c.worst_ms,
                        w.avg_ms,
                        w.worst_ms,
                    );
                }
            }
        }
    }
}

struct Game {
    params: sim::WorldParams,
    state: sim::WorldState,
    input: InputState,
    /// Rotational assist. On by default: the ship is hard enough to fly with
    /// momentum intact, and the pilot can turn it off to feel that.
    assist: bool,
    /// Freeze the simulation, for repeatable measurement.
    frozen: bool,
    accumulator: f64,
    last_frame: Instant,
    started: Instant,
    /// Smoothed thrust effort in [0,1], driving the camera's response.
    /// Render-side only — it must never feed back into the sim.
    effort: f32,
    /// Relevance fade for the velocity hologram. Render-side only.
    gauge_fade: GaugeFade,
    /// Relevance fade for the altimeter: the ground's own instrument.
    alt_fade: AltitudeFade,
    /// Hologram inertia: instruments lag rotation for parallax.
    holo_sway: HoloSway,
    /// The sound-barrier flash, fired by the same edge as the sonic boom.
    mach_alert: MachAlert,
    /// Last wall-clock frame time, clamped, for presentation-side integrators.
    frame_dt: f32,
    /// Which world we are looking at. Cycled with the number keys until there
    /// is a real settings panel.
    appearance: PlanetAppearance,
    appearance_index: usize,
}

impl Game {
    fn new() -> Self {
        let params = sim::presets::earth_compact();
        let altitude = std::env::var("FARFALL_BENCH_ALT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(SPAWN_ALTITUDE_M);
        let mut state = sim::presets::circular_orbit(&params, altitude);
        state.ship.orient = spawn_attitude();
        let now = Instant::now();
        Self {
            params,
            state,
            input: InputState::default(),
            assist: true,
            frozen: Config::from_env().bench,
            accumulator: 0.0,
            last_frame: now,
            started: now,
            effort: 0.0,
            gauge_fade: GaugeFade::new(),
            alt_fade: AltitudeFade::new(),
            holo_sway: HoloSway::new(),
            mach_alert: MachAlert::new(),
            frame_dt: 0.0,
            appearance: PlanetAppearance::EARTHLIKE,
            appearance_index: 0,
        }
    }

    /// Advance the sim by wall time, in whole fixed steps (SPEC §7.2).
    fn tick(&mut self) {
        let now = Instant::now();
        let mut frame_dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        self.frame_dt = frame_dt.min(0.25) as f32;

        // Instruments are presentation, not physics: the velocity hologram
        // keeps living even when the sim is frozen for a benchmark —
        // otherwise every perf capture shows an empty cockpit.
        self.gauge_fade.update(
            frame_dt.min(0.25) as f32,
            self.state.ship.vel_mps.length() as f32,
        );
        let (altitude, vspeed) = self.altitude_vspeed();
        self.alt_fade
            .update(frame_dt.min(0.25) as f32, altitude as f32, vspeed as f32);
        // Hologram inertia from body rotation rates, and the barrier flash
        // from the same supersonic edge that fires the audio boom.
        let w_body = self.state.ship.orient.conjugate() * self.state.ship.ang_vel_radps;
        self.holo_sway
            .update(frame_dt.min(0.25) as f32, w_body.x as f32, w_body.y as f32);
        self.mach_alert
            .update(frame_dt.min(0.25) as f32, self.is_supersonic());

        if self.frozen {
            return;
        }
        // Death-spiral guard: never simulate more than 0.25 s per frame.
        frame_dt = frame_dt.min(0.25);
        self.accumulator += frame_dt;
        // Controls are sampled once per frame, not per step: every fixed step in
        // this frame sees the same input, which is what a networked client would
        // send upstream (SPEC §5.2).
        // Advance the input ramp on wall time, before sampling it.
        self.input.update(frame_dt);
        let controls = self.input.controls(self.assist);
        while self.accumulator >= sim::DT {
            self.state = sim::step(&self.params, &self.state, controls);
            self.accumulator -= sim::DT;
        }

        // Camera response to the ship's own physics. Exponential smoothing,
        // framerate-independent: at 30 fps and at 240 fps the view opens up over
        // the same wall-clock time, so the ship's weight is a property of the
        // ship and not of the machine.
        let target = self.input.thrust_effort(self.params.ship.boost_multiplier) as f32;
        let alpha = 1.0 - (-(frame_dt as f32) / FOV_RESPONSE_S).exp();
        self.effort += (target - self.effort) * alpha;
    }

    /// How much atmosphere surrounds the hull, 0 (vacuum) to 1 (thick air).
    /// The single definition shared by the audio levels, the mach gate and
    /// the instruments — one border, agreed on by ear and eye.
    fn air_ratio(&self) -> f64 {
        let r = self.state.ship.pos_m.length();
        let rho = sim::atmo_density(&self.params.planet, r);
        (rho / self.params.planet.atmo_rho0 * 12.0).clamp(0.0, 1.0)
    }

    /// Supersonic IN the atmosphere — mach in vacuum is meaningless. The
    /// rising edge of this one expression fires both the sonic boom and the
    /// HUD's barrier flash, so they cannot drift apart.
    fn is_supersonic(&self) -> bool {
        self.air_ratio() > 0.65 && self.state.ship.vel_mps.length() > MACH1_MPS
    }

    /// Mach number for the instrument, or a negative number outside the
    /// atmosphere: the gauge hides a meaningless reading entirely.
    fn mach(&self) -> f32 {
        if self.air_ratio() > 0.10 {
            (self.state.ship.vel_mps.length() / MACH1_MPS) as f32
        } else {
            -1.0
        }
    }

    /// Altitude above the sphere and radial (climb) velocity, m and m/s.
    fn altitude_vspeed(&self) -> (f64, f64) {
        let r = self.state.ship.pos_m.length();
        let up = self.state.ship.pos_m / r.max(1.0);
        (
            r - self.params.planet.radius_m,
            self.state.ship.vel_mps.dot(up),
        )
    }

    /// Planet as the camera sees it. The world-space subtraction happens here,
    /// in f64, and only the *relative* offset is narrowed to f32 — which is the
    /// whole floating-origin discipline in one line (SPEC P3).
    fn planet_uniforms(&self, cam: &CameraFrame) -> PlanetUniforms {
        let centre_rel = (DVec3::ZERO - self.state.ship.pos_m).as_vec3();
        PlanetUniforms::new(
            cam,
            centre_rel,
            self.params.planet.radius_m as f32,
            SUN_DIR,
            &self.appearance,
            // Weather advances on sim time, so the sky is a function of the
            // world's clock rather than of how long the window has been open.
            self.state.time_s as f32 * 0.05,
        )
    }

    /// What the hull feels this frame: the wind in its own frame and the air
    /// it is cutting through. The heating itself is computed on the GPU
    /// (render::thermal); the CPU hands over physics and never a temperature.
    fn thermal_inputs(&self, dt: f32) -> ThermalInputs {
        let ship = &self.state.ship;
        let r = ship.pos_m.length();
        ThermalInputs {
            vel_ship_mps: farfall_render::thermal::ship_frame_velocity(
                ship.orient.as_quat(),
                ship.vel_mps.as_vec3(),
            ),
            rho: sim::atmo_density(&self.params.planet, r) as f32,
            rho0: self.params.planet.atmo_rho0 as f32,
            dt,
            reset: false,
        }
    }

    /// Step through the atmosphere presets. A stand-in for the settings panel:
    /// an alien world is not new code, it is different numbers.
    fn cycle_appearance(&mut self) {
        self.appearance_index = (self.appearance_index + 1) % PlanetAppearance::PRESETS.len();
        self.appearance = PlanetAppearance::PRESETS[self.appearance_index];
        log::info!(
            "atmosphere: {} (density {:.2}, cloud cover {:.0}%, deck {:.0} m)",
            self.appearance.name,
            self.appearance.atmosphere_density,
            self.appearance.cloud_coverage * 100.0,
            self.appearance.cloud_altitude_m,
        );
    }

    /// The ship's voice, from the sim's own physics (SPEC P2, in sound):
    /// wind is the actual dynamic pressure ½ρv², vacuum is the actual air
    /// density at altitude, load is the actual felt acceleration. Nothing
    /// here is a sound "event" — the audio is a continuous function of the
    /// world's state, so it cannot desync from what the pilot sees.
    fn audio_levels(&self) -> farfall_audio::Levels {
        let ship = &self.state.ship;
        let planet = &self.params.planet;
        let r = ship.pos_m.length();
        let rho = sim::atmo_density(planet, r);
        let rho_ratio = rho / planet.atmo_rho0;
        let speed = ship.vel_mps.length();

        // Dynamic pressure against a "full roar" reference: sea level at
        // 300 m/s pins the top of the wind's range.
        let q = 0.5 * rho * speed * speed;
        let q_ref = 0.5 * planet.atmo_rho0 * 300.0 * 300.0;

        let controls = self.input.controls(self.assist);

        // Entry intensity: the BUILD-UP of arrival. It needs three things at
        // once — thin air starting to bite (a wide, early-onset density ramp,
        // so the crackle grows through the descent the way the wind does
        // later), not yet full atmosphere (once the ship is properly inside,
        // the wind takes over and this collapses — which is the falling edge
        // the boom fires on), and actually diving at speed: sitting in orbit
        // over the same altitude is silent, and mach matters — a gentle sink
        // whispers, a fast plunge crackles like torn air.
        let air = self.air_ratio();
        let up = ship.pos_m / r.max(1.0);
        let dive = (-ship.vel_mps.dot(up) / 120.0).clamp(0.0, 1.0);
        let air_wide = (rho_ratio * 90.0).clamp(0.0, 1.0);
        let mach_bite = ((speed - 120.0) / 300.0).clamp(0.0, 1.0);
        let entry = (air_wide * (1.0 - air) * dive * mach_bite).clamp(0.0, 1.0);

        farfall_audio::Levels {
            effort: self.input.thrust_effort(self.params.ship.boost_multiplier) as f32,
            wind_q: ((q / q_ref) as f32).clamp(0.0, 1.0),
            vacuum: 1.0 - ((rho_ratio * 12.0) as f32).clamp(0.0, 1.0),
            brake: if controls.brake { 1.0 } else { 0.0 },
            // Attitude thrusters: the largest torque demand. Rolling is
            // flying, and a silent manoeuvre reads as a broken game.
            rcs: controls.torque_body.abs().max_element() as f32,
            entry: entry as f32,
            supersonic: if self.is_supersonic() { 1.0 } else { 0.0 },
            master: 0.8,
        }
    }

    /// Log why and where the session ended. Every exit path goes through this:
    /// a silent exit and a deliberate quit are indistinguishable in a log, and
    /// telling them apart is the difference between "the pilot stopped" and
    /// "the event loop died".
    fn log_exit(&self, reason: &str) {
        let ship = &self.state.ship;
        log::info!(
            "exit ({reason}): sim t={:.1}s alt={:.0}m speed={:.0}m/s spin={:.3}rad/s \
             assist={} hash={:#018x}",
            self.state.time_s,
            ship.pos_m.length() - self.params.planet.radius_m,
            ship.vel_mps.length(),
            ship.ang_vel_radps.length(),
            if self.assist { "on" } else { "off" },
            sim::state_hash(&self.state),
        );
    }

    /// Camera pose for this frame: ride the hull, look down the nose. The view
    /// is the ship's orientation, so steering turns the world rather than
    /// sliding a detached camera around it.
    fn camera(&self, aspect: f32) -> CameraFrame {
        // The camera *is* the ship's orientation: both use the same
        // right-handed frame with the nose at -Z, so no fix-up rotation is
        // needed. Any conversion here would be a sign bug waiting to happen.
        let orient = self.state.ship.orient.as_quat();
        CameraFrame {
            orient,
            fov_y: (BASE_FOV + FOV_THRUST_GAIN * self.effort).to_radians(),
            aspect,
            time_s: self.started.elapsed().as_secs_f32(),
            exposure: 1.6,
        }
    }
}

/// How far the nose is pitched down from prograde at spawn, degrees.
///
/// Near level, and it has to be: with the flight computer steering velocity
/// toward the nose, attitude *is* trajectory. Hold the horizon and the orbit
/// holds; pitch down and you descend. A steeply nose-down spawn would fly the
/// ship into the ground before the pilot touched anything.
const SPAWN_PITCH_DEG: f64 = 12.0;
/// Low enough that the horizon cuts across the frame while looking where you
/// are going — above roughly 14 km the planet drops entirely out of a
/// forward-facing view, because the surface is always 90 degrees off prograde.
/// Also close enough to make an approach, and eventually a collision, short.
const SPAWN_ALTITUDE_M: f64 = 12_000.0;

/// Base vertical field of view, degrees.
const BASE_FOV: f32 = 70.0;
/// How much the view opens up under full boost, degrees. The camera reads the
/// ship's own thrust demand, so acceleration is *seen*, not just measured — the
/// cheapest honest speed cue there is, and it costs nothing but a lerp.
const FOV_THRUST_GAIN: f32 = 14.0;
/// Seconds for the view to catch up to a change in effort. Long enough that the
/// ship feels like it has mass, short enough that it still feels answerable.
const FOV_RESPONSE_S: f32 = 0.28;

/// Orbital attitude at spawn: nose prograde, belly toward the planet, pitched
/// down far enough that the disc is in frame immediately.
///
/// The preset leaves orientation at identity so the sim's golden hash stays a
/// property of the *orbit*, not of where the camera happens to be pointing —
/// choosing an attitude is the app's business, not the physics'.
fn spawn_attitude() -> DQuat {
    // The orbit starts at +X with velocity along -Z, so rolling -90 degrees
    // about the body Z axis puts the body's up (+Y) along world +X: radially
    // out, planet underfoot.
    let belly_down = DQuat::from_rotation_z(-std::f64::consts::FRAC_PI_2);
    // Then pitch the nose down toward the planet. In orbit the planet sits a
    // full 90 degrees off prograde — straight down — so a modest pitch does not
    // come close to reaching it: the disc has to be brought most of the way to
    // nadir before it enters the frame at all. `planet_is_in_view_at_spawn`
    // pins this down, because guessing at it got the framing wrong twice.
    belly_down * DQuat::from_rotation_x(-SPAWN_PITCH_DEG.to_radians())
}

#[derive(Default)]
struct App {
    gpu: Option<Gpu>,
    game: Option<Game>,
    audio: Option<Audio>,
}

impl App {
    fn init_gpu(&mut self, event_loop: &ActiveEventLoop) {
        let cfg = Config::from_env();
        let title = if cfg.bench {
            "FARFALL — BENCHMARK (simulation frozen, controls inert)"
        } else {
            "FARFALL"
        };
        let mut attrs = Window::default_attributes().with_title(title);
        if !cfg.windowed {
            // Borderless fullscreen on the current monitor: no mode switch, so
            // alt-tab stays instant and the resolution is the desktop's.
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let display_handle = event_loop.owned_display_handle();
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display_handle)),
        );
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");

        let (device, queue, config) = pollster::block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await
                .expect("no suitable GPU adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("farfall device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                })
                .await
                .expect("request device");
            let size = window.inner_size();
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .expect("surface unsupported by adapter");
            config.present_mode = if Config::from_env().vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            };
            surface.configure(&device, &config);
            (device, queue, config)
        });

        log::info!(
            "renderer: {}x MSAA, vsync {}, gpu_sync {}, {:?}",
            cfg.msaa,
            if cfg.vsync { "on" } else { "off" },
            cfg.gpu_sync,
            config.format
        );
        let scene = SceneTarget::new(cfg.msaa, config.format, cfg.scale);
        let blit = BlitPass::new(&device, config.format);
        // Bake the static world fields before the first frame. Everything the
        // planet pass reads per pixel is generated here, by shader, once.
        let baked = BakedMaps::bake(&device, &queue);
        let starfield = StarfieldPass::new(&device, config.format, cfg.msaa, STAR_DENSITY, &baked);
        let planet = PlanetPass::new(&device, config.format, cfg.msaa, &baked);
        let gauge = GaugePass::new(&device, config.format, cfg.msaa);
        let alt_gauge = GaugePass::new(&device, config.format, cfg.msaa);
        let thermal = ThermalPass::new(&device);
        let plasma = PlasmaPass::new(&device, config.format, cfg.msaa, &thermal, &baked);
        // The HUD draws straight onto the swapchain, after the upscale, so it
        // is always native resolution and single-sampled however low the scene
        // scale goes (P1: the readout must never soften).
        let hud = HudPass::new(&device, config.format, 1);

        window.request_redraw();
        // Audio: live synthesis, muted for benchmarks (a frozen sim droning
        // at full volume helps nobody) and by FARFALL_MUTE.
        let mute = cfg.bench
            || matches!(
                std::env::var("FARFALL_MUTE").as_deref(),
                Ok("1" | "on" | "true")
            );
        if !mute {
            self.audio = Audio::start();
            if self.audio.is_none() {
                log::warn!("audio: no output device, running silent");
            }
        }

        self.gpu = Some(Gpu {
            window,
            device,
            queue,
            surface,
            config,
            scene,
            blit,
            starfield,
            planet,
            gauge,
            alt_gauge,
            thermal,
            plasma,
            _baked: baked,
            hud,
            text: TextBitmap::new(),
            cfg,
            perf: Perf::new(),
            capture_requested: false,
            bench_captured: false,
        });
        self.game = Some(Game::new());
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            self.init_gpu(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let (Some(gpu), Some(game)) = (self.gpu.as_mut(), self.game.as_mut()) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                game.log_exit("window closed");
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                match code {
                    KeyCode::Escape if pressed => {
                        game.log_exit("escape");
                        event_loop.exit();
                    }
                    // Edge-triggered, and `repeat` is filtered: holding the key
                    // must not strobe the toggle.
                    KeyCode::KeyC if pressed && !event.repeat => game.cycle_appearance(),
                    KeyCode::F12 | KeyCode::KeyP if pressed && !event.repeat => {
                        gpu.capture_requested = true;
                    }
                    KeyCode::BracketLeft | KeyCode::BracketRight if pressed && !event.repeat => {
                        let step = if code == KeyCode::BracketRight {
                            0.1
                        } else {
                            -0.1
                        };
                        let next = gpu.scene.scale() + step;
                        gpu.scene.set_scale(next);
                        log::info!("render scale {:.0}%", gpu.scene.scale() * 100.0);
                    }
                    KeyCode::KeyX if pressed && !event.repeat => {
                        game.assist = !game.assist;
                        log::info!("flight assist {}", if game.assist { "ON" } else { "OFF" });
                    }
                    _ => game.input.set(code, pressed),
                }
            }
            // A key held while the window loses focus never sees its release
            // event; without this the ship keeps thrusting unattended.
            WindowEvent::Focused(false) => game.input.release_all(),
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                // Reconfiguring the swapchain stalls; that frame is not the
                // renderer's fault and must not pollute the worst-frame stat.
                gpu.perf.stats.skip_next_frame();
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let tick_start = Instant::now();
                game.tick();
                if let Some(audio) = &self.audio {
                    audio.set(&game.audio_levels());
                }

                // Acquiring a swapchain image BLOCKS until one is free, which
                // when the GPU is the bottleneck means blocking for roughly a
                // GPU frame. Timing it as CPU work made the readout claim 37 ms
                // of CPU against a real figure of half a millisecond — the
                // renderer's own instrument accusing the wrong half of the
                // machine. It is measured separately and reported as WAIT.
                let sim_seconds = tick_start.elapsed().as_secs_f64();
                let acquire_start = Instant::now();
                let frame = match gpu.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {
                        // Benchmarks must neither hang nor lie when occluded:
                        // the window being buried is no reason to skip the
                        // capture, because the scene target is offscreen and
                        // needs no swapchain. Render it headless and save.
                        // (This is the seed of the golden-image harness: a
                        // frame produced with no visible window at all.)
                        if gpu.cfg.bench {
                            let t = game.started.elapsed().as_secs_f64();
                            if t > gpu.cfg.bench_seconds * 0.5 && !gpu.bench_captured {
                                gpu.bench_captured = true;
                                if gpu.scene.ensure(
                                    &gpu.device,
                                    gpu.config.width,
                                    gpu.config.height,
                                ) {
                                    if let Some(view) = gpu.scene.colour_view() {
                                        gpu.blit.rebind(&gpu.device, view);
                                    }
                                }
                                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                                let cam = game.camera(aspect);
                                gpu.starfield.update(
                                    &gpu.queue,
                                    &FrameUniforms::from_camera(&cam).with_occluder(
                                        (DVec3::ZERO - game.state.ship.pos_m).as_vec3(),
                                        game.params.planet.radius_m as f32,
                                    ),
                                );
                                gpu.planet.update(&gpu.queue, &game.planet_uniforms(&cam));
                                let (altitude_m, _) = game.altitude_vspeed();
                                gpu.gauge.update(
                                    &gpu.queue,
                                    &GaugeUniforms::speed(
                                        game.state.ship.vel_mps.length() as f32,
                                        game.gauge_fade.level(),
                                        cam.time_s,
                                        aspect,
                                        gpu.scene.size().1 as f32,
                                        VELOCITY_GAUGE_ANCHOR,
                                        game.holo_sway.sway(),
                                        game.mach(),
                                        game.mach_alert.level(),
                                    ),
                                );
                                gpu.alt_gauge.update(
                                    &gpu.queue,
                                    &GaugeUniforms::altitude(
                                        altitude_m as f32,
                                        game.alt_fade.level(),
                                        cam.time_s,
                                        aspect,
                                        gpu.scene.size().1 as f32,
                                        ALTITUDE_GAUGE_ANCHOR,
                                        game.holo_sway.sway(),
                                    ),
                                );
                                // The capture should show what the pilot
                                // sees, text included. The HUD pipeline is
                                // single-sample (it draws in the present
                                // pass), so it can only join a 1x scene.
                                let capture_text = gpu.cfg.msaa == 1;
                                if capture_text {
                                    gpu.text.clear();
                                    gpu.text.draw(0, 0, "HEADLESS CAPTURE");
                                    gpu.text.draw(0, 6, &format!("ALT {altitude_m:.0}M"));
                                    gpu.text.draw(
                                        0,
                                        12,
                                        &format!("VEL {:.0}M/S", game.state.ship.vel_mps.length()),
                                    );
                                    let (_, sh) = gpu.scene.size();
                                    let hud_scale = (sh as f32 / 260.0).clamp(2.0, 8.0).floor();
                                    gpu.hud.update(
                                        &gpu.queue,
                                        &gpu.text,
                                        HUD_TEXT_ANCHOR,
                                        hud_scale * 2.0 / sh as f32,
                                        aspect,
                                        sh as f32,
                                        game.holo_sway.sway(),
                                    );
                                }
                                let mut encoder = gpu.device.create_command_encoder(
                                    &wgpu::CommandEncoderDescriptor {
                                        label: Some("headless"),
                                    },
                                );
                                let thermal_in = game.thermal_inputs(game.frame_dt);
                                gpu.plasma.update(
                                    &gpu.queue,
                                    &PlasmaUniforms::new(&cam, thermal_in.vel_ship_mps),
                                );
                                gpu.thermal.step(&gpu.queue, &mut encoder, &thermal_in);
                                {
                                    let mut pass =
                                        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                            label: Some("scene headless"),
                                            color_attachments: &[Some(
                                                gpu.scene.colour_attachment(),
                                            )],
                                            depth_stencil_attachment: None,
                                            timestamp_writes: None,
                                            occlusion_query_set: None,
                                            multiview_mask: None,
                                        });
                                    gpu.starfield.draw(&mut pass);
                                    gpu.planet.draw(&mut pass);
                                    gpu.plasma.draw(&mut pass, &gpu.thermal);
                                    gpu.gauge.draw(&mut pass);
                                    gpu.alt_gauge.draw(&mut pass);
                                    if capture_text {
                                        gpu.hud.draw(&mut pass);
                                    }
                                }
                                let capture = gpu.scene.colour_texture().map(|tex| {
                                    let path = std::env::temp_dir()
                                        .join(format!("farfall-{:.0}.png", t * 1000.0));
                                    Capture::record(&gpu.device, &mut encoder, tex, path)
                                });
                                gpu.queue.submit([encoder.finish()]);
                                if let Some(capture) = capture {
                                    let bgra = matches!(
                                        gpu.scene.format(),
                                        wgpu::TextureFormat::Bgra8Unorm
                                            | wgpu::TextureFormat::Bgra8UnormSrgb
                                    );
                                    match capture.save(&gpu.device, bgra) {
                                        Ok(path) => {
                                            log::info!("screenshot: {}", path.display())
                                        }
                                        Err(e) => log::warn!("headless capture failed: {e}"),
                                    }
                                }
                            }
                            if t > gpu.cfg.bench_seconds {
                                event_loop.exit();
                            }
                        }
                        gpu.window.request_redraw();
                        return;
                    }
                    other => {
                        log::warn!("surface: {other:?}, reconfiguring");
                        gpu.surface.configure(&gpu.device, &gpu.config);
                        gpu.window.request_redraw();
                        return;
                    }
                };
                let wait_seconds = acquire_start.elapsed().as_secs_f64();
                let encode_start = Instant::now();
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                if gpu
                    .scene
                    .ensure(&gpu.device, gpu.config.width, gpu.config.height)
                {
                    // The scene textures were recreated; a bind group still
                    // pointing at the old view would sample a destroyed
                    // resource.
                    let view = gpu.scene.colour_view().expect("scene target");
                    gpu.blit.rebind(&gpu.device, view);
                    gpu.perf.stats.skip_next_frame();
                }

                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                let cam = game.camera(aspect);
                gpu.starfield.update(
                    &gpu.queue,
                    &FrameUniforms::from_camera(&cam).with_occluder(
                        (DVec3::ZERO - game.state.ship.pos_m).as_vec3(),
                        game.params.planet.radius_m as f32,
                    ),
                );
                gpu.planet.update(&gpu.queue, &game.planet_uniforms(&cam));
                let thermal_in = game.thermal_inputs(game.frame_dt);
                gpu.plasma.update(
                    &gpu.queue,
                    &PlasmaUniforms::new(&cam, thermal_in.vel_ship_mps),
                );
                let (altitude_m, _) = game.altitude_vspeed();
                gpu.gauge.update(
                    &gpu.queue,
                    &GaugeUniforms::speed(
                        game.state.ship.vel_mps.length() as f32,
                        game.gauge_fade.level(),
                        cam.time_s,
                        aspect,
                        gpu.scene.size().1 as f32,
                        VELOCITY_GAUGE_ANCHOR,
                        game.holo_sway.sway(),
                        game.mach(),
                        game.mach_alert.level(),
                    ),
                );
                gpu.alt_gauge.update(
                    &gpu.queue,
                    &GaugeUniforms::altitude(
                        altitude_m as f32,
                        game.alt_fade.level(),
                        cam.time_s,
                        aspect,
                        gpu.scene.size().1 as f32,
                        ALTITUDE_GAUGE_ANCHOR,
                        game.holo_sway.sway(),
                    ),
                );

                // Scale the readout with the surface so it keeps the same
                // apparent size on a retina fullscreen and a small window;
                // the size is chosen in pixels and expressed in canopy units.
                let hud_scale = (gpu.config.height as f32 / 260.0).clamp(2.0, 8.0).floor();
                let px_canopy = hud_scale * 2.0 / gpu.config.height as f32;
                gpu.hud.update(
                    &gpu.queue,
                    &gpu.text,
                    HUD_TEXT_ANCHOR,
                    px_canopy,
                    aspect,
                    gpu.config.height as f32,
                    game.holo_sway.sway(),
                );

                let mut encoder = gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                // Pass 0: advance the hull heat field (64x64, on the GPU).
                gpu.thermal.step(&gpu.queue, &mut encoder, &thermal_in);
                {
                    // Pass 1: the expensive world, at whatever scale is set.
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("scene"),
                        color_attachments: &[Some(gpu.scene.colour_attachment())],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if gpu.cfg.draws("starfield") {
                        gpu.starfield.draw(&mut pass);
                    }
                    if gpu.cfg.draws("planet") {
                        gpu.planet.draw(&mut pass);
                    }
                    if gpu.cfg.draws("plasma") {
                        gpu.plasma.draw(&mut pass, &gpu.thermal);
                    }
                    if gpu.cfg.draws("gauge") {
                        gpu.gauge.draw(&mut pass);
                        gpu.alt_gauge.draw(&mut pass);
                    }
                }
                {
                    // Pass 2: upscale, then the HUD at native resolution.
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("present"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    if gpu.cfg.draws("blit") {
                        gpu.blit.draw(&mut pass);
                    }
                    if gpu.cfg.draws("hud") {
                        gpu.hud.draw(&mut pass);
                    }
                }
                // Screenshot: recorded into the same command buffer, so it
                // captures exactly the frame that was just drawn.
                let pending = if gpu.capture_requested {
                    gpu.capture_requested = false;
                    if gpu.scene.colour_texture().is_none() {
                        log::warn!("capture skipped: scene target has no colour texture");
                    }
                    gpu.scene.colour_texture().map(|tex| {
                        let path = std::env::temp_dir().join(format!(
                            "farfall-{:.0}.png",
                            game.started.elapsed().as_secs_f64() * 1000.0
                        ));
                        Capture::record(&gpu.device, &mut encoder, tex, path)
                    })
                } else {
                    None
                };

                gpu.queue.submit([encoder.finish()]);
                // Genuine CPU work: simulation, uniform packing, encoding.
                let cpu_seconds = sim_seconds + encode_start.elapsed().as_secs_f64();

                if let Some(capture) = pending {
                    let bgra = matches!(
                        gpu.scene.format(),
                        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                    );
                    match capture.save(&gpu.device, bgra) {
                        Ok(path) => log::info!("screenshot: {}", path.display()),
                        Err(e) => log::warn!("screenshot failed: {e}"),
                    }
                    // The readback blocks on the GPU; that frame's timing says
                    // nothing about the renderer.
                    gpu.perf.stats.skip_next_frame();
                }

                gpu.queue.present(frame);
                if gpu.cfg.gpu_sync {
                    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
                }
                // A benchmark stops itself. Left running, it is a frozen window
                // that looks exactly like the game and answers no controls.
                if gpu.cfg.bench {
                    let t = game.started.elapsed().as_secs_f64();
                    // One automatic capture partway through, so a headless
                    // benchmark leaves a picture of what it measured.
                    if t > gpu.cfg.bench_seconds * 0.5 && !gpu.bench_captured {
                        gpu.bench_captured = true;
                        gpu.capture_requested = true;
                        log::info!("benchmark capture requested at t={t:.1}s");
                    }
                    if t > gpu.cfg.bench_seconds {
                        log::info!("benchmark complete, exiting");
                        event_loop.exit();
                    }
                }
                gpu.frame_timing(
                    cpu_seconds,
                    wait_seconds,
                    game.state.ship.pos_m.length() - game.params.planet.radius_m,
                    game.state.ship.vel_mps.length(),
                    game.assist,
                );
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut App::default()).expect("run");
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{DQuat, DVec3};
    use input::{Action, InputState};
    use winit::keyboard::KeyCode;

    /// The camera basis must agree with the ship's own axes exactly. If these
    /// ever diverge, the world is mirrored or rolled relative to the hull.
    #[test]
    fn camera_basis_matches_ship_axes() {
        let mut game = Game::new();
        game.state.ship.orient = DQuat::from_euler(glam::EulerRot::YXZ, 0.7, -0.3, 0.2).normalize();
        let (right, up, forward) = game.camera(1.0).basis();
        let ship = game.state.ship.orient;
        for (got, want, name) in [
            (right, ship * DVec3::X, "right"),
            (up, ship * DVec3::Y, "up"),
            (forward, ship * DVec3::NEG_Z, "forward"),
        ] {
            assert!(
                (got - want.as_vec3()).length() < 1e-5,
                "camera {name} {got:?} != ship {name} {want:?}"
            );
        }
    }

    /// Steering the ship must swing the view by the same angle.
    #[test]
    fn camera_follows_ship_orientation() {
        let mut game = Game::new();
        game.state.ship.orient = DQuat::IDENTITY;
        let before = game.camera(1.0).basis().2;
        game.state.ship.orient = DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2);
        let after = game.camera(1.0).basis().2;
        let angle = before.dot(after).clamp(-1.0, 1.0).acos();
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "view swung {angle} rad, expected pi/2"
        );
    }

    fn fly(keys: &[KeyCode], secs: u64) -> sim::ShipState {
        let params = sim::presets::earth_compact();
        let altitude = std::env::var("FARFALL_BENCH_ALT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(SPAWN_ALTITUDE_M);
        // In vacuum: these tests are about the control mapping, and a hull
        // falling through air picks up real aerodynamic torques of its own
        // (a broadside wind on a tail-heavy-of-pressure ship yaws it). The
        // air's behaviour has its own tests in the sim crate.
        let altitude = altitude.max(params.planet.atmo_top_m + 1_000.0);
        let mut state = sim::presets::circular_orbit(&params, altitude);
        state.ship.orient = DQuat::IDENTITY;
        state.ship.vel_mps = DVec3::ZERO; // isolate control response from orbit
        let mut input = InputState::default();
        for k in keys {
            input.set(*k, true);
        }
        let controls = input.controls_immediate(false);
        for _ in 0..(secs * 120) {
            state = sim::step(&params, &state, controls);
        }
        state.ship
    }

    /// Every control direction, asserted against the real integrator from the
    /// pilot's point of view. Comments lie; this is what caught the frame being
    /// declared left-handed while the rotation math was right-handed, which
    /// silently mirrored yaw, roll, and strafe.
    #[test]
    fn sim_directions() {
        // Translation: compare against an unthrusted control run so gravity —
        // which pulls toward the planet regardless of input — cancels out and
        // only the thrust contribution remains.
        let coast = fly(&[], 1).vel_mps;
        let cases: [(KeyCode, DVec3, &str); 6] = [
            (KeyCode::KeyW, DVec3::NEG_Z, "W thrusts along the nose"),
            (KeyCode::KeyS, DVec3::Z, "S thrusts backward"),
            (KeyCode::KeyD, DVec3::X, "D strafes right"),
            (KeyCode::KeyA, DVec3::NEG_X, "A strafes left"),
            (KeyCode::KeyR, DVec3::Y, "R thrusts up"),
            (KeyCode::KeyF, DVec3::NEG_Y, "F thrusts down"),
        ];
        for (key, want, what) in cases {
            let v = (fly(&[key], 1).vel_mps - coast).normalize();
            assert!(
                (v - want).length() < 1e-4,
                "{what}: got {v:?}, want {want:?}"
            );
        }

        // Rotation: after a short burn the named axis must have moved the right way.
        let nose_up = fly(&[KeyCode::ArrowUp], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_up.y > 0.05,
            "Up arrow must pitch the nose up: {nose_up:?}"
        );

        let nose_down = fly(&[KeyCode::ArrowDown], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_down.y < -0.05,
            "Down arrow must pitch the nose down: {nose_down:?}"
        );

        let nose_right = fly(&[KeyCode::ArrowRight], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_right.x > 0.05,
            "Right arrow must yaw the nose right: {nose_right:?}"
        );

        let nose_left = fly(&[KeyCode::ArrowLeft], 1).orient * DVec3::NEG_Z;
        assert!(
            nose_left.x < -0.05,
            "Left arrow must yaw the nose left: {nose_left:?}"
        );

        // Roll right = right wing down = the up vector tips toward +X.
        let up_right = fly(&[KeyCode::KeyE], 1).orient * DVec3::Y;
        assert!(
            up_right.x > 0.05,
            "E must roll right: up tipped to {up_right:?}"
        );

        let up_left = fly(&[KeyCode::KeyQ], 1).orient * DVec3::Y;
        assert!(
            up_left.x < -0.05,
            "Q must roll left: up tipped to {up_left:?}"
        );
    }

    /// A rotation must not bleed into the other two axes.
    #[test]
    fn rotation_axes_do_not_cross_couple() {
        for (key, axis) in [
            (KeyCode::ArrowUp, 0usize),
            (KeyCode::ArrowRight, 1),
            (KeyCode::KeyE, 2),
        ] {
            let w = fly(&[key], 1).ang_vel_radps;
            for other in 0..3 {
                if other != axis {
                    assert!(
                        w[other].abs() < 1e-12,
                        "{key:?} leaked {} rad/s into axis {other}",
                        w[other]
                    );
                }
            }
        }
    }

    /// The nose starts pointing along the velocity: the pilot begins looking
    /// where they are going, not backwards.
    #[test]
    fn orbit_preset_starts_nose_prograde() {
        let params = sim::presets::earth_compact();
        let s = sim::presets::circular_orbit(&params, 20_000.0).ship;
        let nose = s.orient * DVec3::NEG_Z;
        let prograde = s.vel_mps.normalize();
        assert!(
            (nose - prograde).length() < 1e-9,
            "nose {nose:?} is not prograde {prograde:?}"
        );
    }

    /// The planet must be in frame on the very first frame. Getting the spawn
    /// attitude right is pure geometry and I got it wrong twice by reasoning
    /// about it in prose, so it is asserted instead: the angle from the camera's
    /// forward axis to the planet's centre, minus the disc's angular radius,
    /// has to land inside the vertical half-FOV.
    #[test]
    fn planet_is_in_view_at_spawn() {
        let game = Game::new();
        let cam = game.camera(16.0 / 9.0);
        let forward = cam.basis().2.as_dvec3();

        let to_planet = (-game.state.ship.pos_m).normalize();
        let angle = forward.dot(to_planet).clamp(-1.0, 1.0).acos();

        let r = game.params.planet.radius_m;
        let d = game.state.ship.pos_m.length();
        let angular_radius = (r / d).asin();

        let half_fov = (cam.fov_y as f64) * 0.5;
        assert!(
            angle - angular_radius < half_fov,
            "planet off screen at spawn: centre {:.1} deg from forward, disc radius \
             {:.1} deg, half-FOV {:.1} deg",
            angle.to_degrees(),
            angular_radius.to_degrees(),
            half_fov.to_degrees(),
        );
        // ...and the *limb* must be on screen, not just some surface: a frame
        // filled edge to edge with ground reads as terrain, not as a world. The
        // horizon crossing the view is what sells the curve.
        let limb_offset = (angle - angular_radius).abs();
        assert!(
            limb_offset < half_fov,
            "horizon is off screen: limb sits {:.1} deg from forward, half-FOV {:.1}",
            (angle - angular_radius).to_degrees(),
            half_fov.to_degrees(),
        );
    }

    /// The ship starts flying where it is looking. With the flight computer
    /// steering velocity toward the nose, any large angle between the two is a
    /// trajectory change the pilot never asked for — so the spawn attitude must
    /// be close to prograde or the ship immediately departs its own orbit.
    #[test]
    fn spawn_flies_where_it_looks() {
        let game = Game::new();
        let nose = game.state.ship.orient * DVec3::NEG_Z;
        let prograde = game.state.ship.vel_mps.normalize();
        let off = nose.dot(prograde).clamp(-1.0, 1.0).acos().to_degrees();
        assert!(off < 20.0, "nose is {off:.1} deg off prograde at spawn");
    }

    /// The planet is underfoot, not overhead: the ship is the right way up.
    #[test]
    fn spawn_attitude_puts_the_planet_below() {
        let game = Game::new();
        let down = (game.state.ship.orient * DVec3::NEG_Y).normalize();
        let to_planet = (-game.state.ship.pos_m).normalize();
        assert!(
            down.dot(to_planet) > 0.8,
            "planet is not below the ship: {:.2}",
            down.dot(to_planet)
        );
    }

    #[test]
    fn thrust_is_ship_relative_at_any_attitude() {
        let params = sim::presets::earth_compact();
        let attitudes = [
            DQuat::IDENTITY,
            DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
            DQuat::from_rotation_x(-1.1),
            DQuat::from_euler(glam::EulerRot::YXZ, 2.3, -0.9, 1.7).normalize(),
        ];
        for (key, body_axis) in [
            (KeyCode::KeyW, DVec3::NEG_Z),
            (KeyCode::KeyD, DVec3::X),
            (KeyCode::KeyR, DVec3::Y),
        ] {
            for orient in attitudes {
                let mut state = sim::presets::circular_orbit(&params, 60_000.0);
                state.ship.orient = orient;
                state.ship.vel_mps = DVec3::ZERO;
                let mut input = InputState::default();
                input.set(key, true);

                let coast = {
                    let mut s = state;
                    for _ in 0..120 {
                        s = sim::step(&params, &s, InputState::default().controls(false));
                    }
                    s.ship.vel_mps
                };
                let mut s = state;
                for _ in 0..120 {
                    s = sim::step(&params, &s, input.controls_immediate(false));
                }

                // Gravity cancels; what remains must lie along the ship's axis.
                let delta = (s.ship.vel_mps - coast).normalize();
                let expected = (orient * body_axis).normalize();
                assert!(
                    (delta - expected).length() < 1e-4,
                    "{key:?} at attitude {orient:?}: thrust went {delta:?}, expected {expected:?}"
                );
            }
        }
    }

    /// Boost multiplies thrust without steering it somewhere new.
    #[test]
    fn boost_adds_thrust_along_the_same_axis() {
        let params = sim::presets::earth_compact();
        let mut base = sim::presets::circular_orbit(&params, 60_000.0);
        base.ship.orient = DQuat::IDENTITY;
        base.ship.vel_mps = DVec3::ZERO;

        let run = |boost: bool| {
            let mut input = InputState::default();
            input.set(KeyCode::KeyW, true);
            if boost {
                input.set(KeyCode::ShiftLeft, true);
            }
            let controls = input.controls_immediate(false);
            let mut s = base;
            for _ in 0..120 {
                s = sim::step(&params, &s, controls);
            }
            s.ship.vel_mps
        };
        let coast = {
            let mut s = base;
            for _ in 0..120 {
                s = sim::step(&params, &s, InputState::default().controls(false));
            }
            s.ship.vel_mps
        };

        let plain = run(false) - coast;
        let boosted = run(true) - coast;
        let ratio = boosted.length() / plain.length();
        assert!(
            (ratio - params.ship.boost_multiplier).abs() < 0.01,
            "boost gave {ratio:.2}x, expected {:.2}x",
            params.ship.boost_multiplier
        );
        // Same direction, only more of it.
        assert!((boosted.normalize() - plain.normalize()).length() < 1e-6);
    }

    #[test]
    fn action_count_matches_bindings() {
        assert_eq!(Action::COUNT, 12);
    }
}
