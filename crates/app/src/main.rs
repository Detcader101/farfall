//! farfall-app — native shell (SPEC §5.1).
//!
//! Owns the window, the fixed-timestep accumulator (SPEC §7.2), and the
//! sim → render translation. The sim is authoritative (SPEC §5.2): this loop
//! feeds it inputs and *reads* state; nothing here mutates the world directly.
//!
//! M0 scope: no input mapping yet — the ship coasts in orbit, the camera rides
//! it looking prograde with a slow survey roll, and the starfield renders at
//! MSAA 4x. Input arrives in M1 (TASKS M1.2).

use std::sync::Arc;
use std::time::Instant;

use farfall_render::{starfield::StarfieldPass, CameraFrame, FrameUniforms, MsaaTarget};
use farfall_sim as sim;
use glam::DQuat;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
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
        while self.accumulator >= sim::DT {
            self.state = sim::step(&self.params, &self.state, sim::Controls::default());
            self.accumulator -= sim::DT;
        }
    }

    /// Camera pose for this frame: ride the ship, look prograde, slow survey roll.
    fn camera(&self, aspect: f32) -> CameraFrame {
        let ship = &self.state.ship;
        let prograde = ship.vel_mps.normalize_or_zero();
        let up = ship.pos_m.normalize_or_zero(); // radial out
        let look = look_along(prograde, up);
        let roll = DQuat::from_axis_angle(prograde, 0.03 * self.state.time_s);
        let orient = (roll * look).as_quat();
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
                log::info!(
                    "exit: sim t={:.1}s hash={:#018x}",
                    game.state.time_s,
                    sim::state_hash(&game.state)
                );
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                }
            }
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
