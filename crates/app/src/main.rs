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
use glam::{DQuat, DVec3};
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
        let ship = &self.state.ship;
        let nose = ship.orient * DVec3::Z;
        let up = ship.orient * DVec3::Y;
        let orient = look_along(nose, up).as_quat();
        CameraFrame {
            orient,
            fov_y: 70f32.to_radians(),
            aspect,
            time_s: self.started.elapsed().as_secs_f32(),
            exposure: 1.6,
        }
    }
}

/// Orientation whose -Z is `forward` and whose +Y approximates `up`.
fn look_along(forward: glam::DVec3, up: glam::DVec3) -> DQuat {
    let f = forward.normalize_or_zero();
    let r = f.cross(up).normalize_or_zero();
    let u = r.cross(f);
    // Column-major basis: camera space +X=r, +Y=u, -Z=f (right-handed view).
    DQuat::from_mat3(&glam::DMat3::from_cols(r, u, -f))
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
    use farfall_sim::Controls;

    fn basis_of(q: glam::Quat) -> (glam::Vec3, glam::Vec3, glam::Vec3) {
        CameraFrame {
            orient: q,
            fov_y: 1.0,
            aspect: 1.0,
            time_s: 0.0,
            exposure: 1.0,
        }
        .basis()
    }

    /// `look_along` must produce an orthonormal, right-handed basis whose
    /// forward is the requested direction — a mirrored basis would flip the
    /// whole world and is exactly the kind of bug that hides until you read
    /// text in-world.
    #[test]
    fn look_along_is_orthonormal_and_right_handed() {
        for (f, up) in [
            (DVec3::Z, DVec3::Y),
            (DVec3::X, DVec3::Y),
            (DVec3::new(1.0, 2.0, -3.0), DVec3::Y),
            (DVec3::NEG_Z, DVec3::new(0.1, 1.0, 0.0)),
        ] {
            let q = look_along(f, up).as_quat();
            let (r, u, fwd) = basis_of(q);
            assert!(
                (fwd - f.normalize().as_vec3()).length() < 1e-5,
                "forward mismatch for {f:?}"
            );
            assert!((r.length() - 1.0).abs() < 1e-5, "right not unit");
            assert!((u.length() - 1.0).abs() < 1e-5, "up not unit");
            assert!(r.dot(u).abs() < 1e-5, "basis not orthogonal");
            // Right-handed: right x up == -forward for a -Z-forward camera.
            assert!(
                (r.cross(u) + fwd).length() < 1e-5,
                "basis is mirrored for {f:?}"
            );
        }
    }

    /// The camera rides the hull: yawing the ship must swing the view by the
    /// same angle. This is what makes steering turn the world instead of
    /// sliding a detached camera around it.
    #[test]
    fn camera_follows_ship_orientation() {
        let mut game = Game::new();
        game.state.ship.orient = DQuat::IDENTITY;
        let before = basis_of(game.camera(1.0).orient).2;

        // Yaw 90 degrees about the body up axis.
        game.state.ship.orient = DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2);
        let after = basis_of(game.camera(1.0).orient).2;

        let angle = before.dot(after).clamp(-1.0, 1.0).acos();
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "view swung {angle} rad, expected pi/2"
        );
    }

    /// Yaw input must actually rotate the ship, in the documented direction:
    /// right yaw swings the nose toward body +X.
    #[test]
    fn yaw_input_turns_the_nose() {
        let params = sim::presets::earth_compact();
        let mut state = sim::presets::circular_orbit(&params, 20_000.0);
        state.ship.orient = DQuat::IDENTITY;
        let controls = Controls {
            torque_body: DVec3::new(0.0, 1.0, 0.0),
            assist: true,
            ..Default::default()
        };
        for _ in 0..120 {
            state = sim::step(&params, &state, controls);
        }
        let nose = state.ship.orient * DVec3::Z;
        assert!(
            nose.x > 0.05,
            "yaw-right did not swing the nose right: {nose:?}"
        );
        assert!(nose.z > 0.0, "nose flipped past 90 degrees in 1 s");
    }
}
