//! farfall-app — native shell (SPEC §5.1).
//!
//! Owns the window, the fixed-timestep accumulator (SPEC §7.2), and the
//! sim → render translation. The sim is authoritative (SPEC §5.2): this loop
//! feeds it inputs and *reads* state; nothing here mutates the world directly.
//!
//! M1 scope: the ship is hand-flown. Keys map to sim [`Controls`] (see
//! [`input`]), the camera rides the hull looking down the nose, and rotational
//! flight assist is toggleable. Planet, HUD, and sun arrive in M1 tasks 4-6.

mod input;

use std::sync::Arc;
use std::time::Instant;

use farfall_render::{starfield::StarfieldPass, CameraFrame, FrameUniforms, MsaaTarget};
use farfall_sim as sim;
use input::InputState;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

const MSAA_SAMPLES: u32 = 4;
const STAR_DENSITY: f64 = 1.0;

struct Gpu {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    msaa: MsaaTarget,
    starfield: StarfieldPass,
}

struct Game {
    params: sim::WorldParams,
    state: sim::WorldState,
    input: InputState,
    /// Rotational assist. On by default: the ship is hard enough to fly with
    /// momentum intact, and the pilot can turn it off to feel that.
    assist: bool,
    accumulator: f64,
    last_frame: Instant,
    started: Instant,
}

impl Game {
    fn new() -> Self {
        let params = sim::presets::earth_compact();
        // 20 km up: black sky, planet below (drawn from M1).
        let state = sim::presets::circular_orbit(&params, 20_000.0);
        let now = Instant::now();
        Self {
            params,
            state,
            input: InputState::default(),
            assist: true,
            accumulator: 0.0,
            last_frame: now,
            started: now,
        }
    }

    /// Advance the sim by wall time, in whole fixed steps (SPEC §7.2).
    fn tick(&mut self) {
        let now = Instant::now();
        let mut frame_dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        // Death-spiral guard: never simulate more than 0.25 s per frame.
        frame_dt = frame_dt.min(0.25);
        self.accumulator += frame_dt;
        // Controls are sampled once per frame, not per step: every fixed step in
        // this frame sees the same input, which is what a networked client would
        // send upstream (SPEC §5.2).
        let controls = self.input.controls(self.assist);
        while self.accumulator >= sim::DT {
            self.state = sim::step(&self.params, &self.state, controls);
            self.accumulator -= sim::DT;
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
            fov_y: 70f32.to_radians(),
            aspect,
            time_s: self.started.elapsed().as_secs_f32(),
            exposure: 1.6,
        }
    }
}

#[derive(Default)]
struct App {
    gpu: Option<Gpu>,
    game: Option<Game>,
}

impl App {
    fn init_gpu(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("FARFALL — M0 bedrock"))
                .expect("create window"),
        );

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
            let config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .expect("surface unsupported by adapter");
            surface.configure(&device, &config);
            (device, queue, config)
        });

        let msaa = MsaaTarget::new(MSAA_SAMPLES, config.format);
        let starfield = StarfieldPass::new(&device, config.format, MSAA_SAMPLES, STAR_DENSITY);

        window.request_redraw();
        self.gpu = Some(Gpu {
            window,
            device,
            queue,
            surface,
            config,
            msaa,
            starfield,
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
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                game.tick();

                let frame = match gpu.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) => frame,
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => {
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
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                gpu.msaa
                    .ensure(&gpu.device, gpu.config.width, gpu.config.height);
                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                let cam = game.camera(aspect);
                gpu.starfield
                    .update(&gpu.queue, &FrameUniforms::from_camera(&cam));

                let mut encoder = gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("sky"),
                        color_attachments: &[Some(gpu.msaa.color_attachment(&view))],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    gpu.starfield.draw(&mut pass);
                }
                gpu.queue.submit([encoder.finish()]);
                gpu.queue.present(frame);
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
        let mut state = sim::presets::circular_orbit(&params, 20_000.0);
        state.ship.orient = DQuat::IDENTITY;
        state.ship.vel_mps = DVec3::ZERO; // isolate control response from orbit
        let mut input = InputState::default();
        for k in keys {
            input.set(*k, true);
        }
        let controls = input.controls(false);
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

    #[test]
    fn action_count_matches_bindings() {
        assert_eq!(Action::COUNT, 12);
    }
}
