//! Space dust: fine motes and ice crystals in a volume about the eye,
//! hash-placed in world cells so they hold still as the ship moves through
//! them, Sun-lit with a glint, drawn into streaks by the ship's velocity
//! relative to the local orbit — sparse in deep space, dense in the belt
//! and in a planet's air; and a few motes drifting in the cabin's own
//! light. See `shaders/dust.wgsl`.
//!
//! One instanced draw: each instance is a mote, placed by the vertex
//! shader from the cell lattice around the eye; the fragment shader draws
//! a soft capsule (a dot, or a streak). Additive: light, not paint.

use glam::{DVec3, Quat, Vec3};

use crate::CameraFrame;

/// The lattice's cell, metres. The volume drawn is `REACH` cells each way.
pub const CELL_M: f64 = 40.0;
/// Cells each way from the eye's own (a 7×7×7 block at 3).
pub const REACH: u32 = 3;
/// Motes per cell at full density.
pub const PER_CELL: u32 = 12;
/// Space motes drawn (the whole block).
pub const SPACE_MOTES: u32 = (2 * REACH + 1) * (2 * REACH + 1) * (2 * REACH + 1) * PER_CELL;
/// Motes drifting in the cabin.
pub const CABIN_MOTES: u32 = 40;
/// A frame's worth of motion: how long a streak is, seconds.
pub const STREAK_S: f32 = 1.0 / 60.0;

/// Where the eye sits on the lattice: its cell (wrapped to i32 — the
/// hash only needs a stable index) and its offset within that cell,
/// metres, 0..CELL_M.
pub fn lattice(eye_m: DVec3) -> ([i32; 3], Vec3) {
    let cell = eye_m.map(|v| (v / CELL_M).floor());
    let frac = (eye_m - cell * CELL_M).as_vec3();
    let wrap = |v: f64| (v as i64).rem_euclid(1 << 31) as i32;
    ([wrap(cell.x), wrap(cell.y), wrap(cell.z)], frac)
}

/// How thick the dust is here, 0..1: a floor of deep-space motes, the
/// belt's grit, a planet's air (by its density over sea level), thinned
/// to nothing under the hyper field; scaled by the DUST setting (1 stock).
pub fn density(belt: f32, air: f32, hyper: f32, setting: f32) -> f32 {
    let base = 0.10;
    let raw = base + belt.clamp(0.0, 1.0) * 0.9 + air.clamp(0.0, 1.0).sqrt() * 0.9;
    (raw.min(1.0) * (1.0 - hyper.clamp(0.0, 1.0)) * setting.clamp(0.0, 2.0)).clamp(0.0, 1.0)
}

/// What the dust rests in: the local circular orbit about the nearest
/// body — so a ship coasting in orbit sees its motes hang still, and one
/// under thrust or falling sees them stream. `rel`: the body's centre
/// from the ship (m); `mu`: its gravitational parameter; `body_vel`: the
/// body's own velocity; `ship_vel`: the ship's. Returns the ship's
/// velocity relative to that rest, m/s.
pub fn drift(rel: DVec3, mu: f64, body_vel: DVec3, ship_vel: DVec3) -> DVec3 {
    let r = rel.length();
    if r.is_nan() || r <= 1.0 || mu.is_nan() || mu <= 0.0 {
        return ship_vel - body_vel;
    }
    let radial = rel / r;
    let v_rel = ship_vel - body_vel;
    let tangent = (v_rel - radial * v_rel.dot(radial)).normalize_or_zero();
    let circular = (mu / r).sqrt();
    v_rel - tangent * circular
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DustUniforms {
    /// The camera's basis in the WORLD frame; w: aspect, tan(fov/2), time.
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    /// The eye's cell on the lattice (xyz), w unused.
    cell: [i32; 4],
    /// xyz: the eye's offset within its cell (m); w: the cell size (m).
    frac: [f32; 4],
    /// xyz: the eye's velocity through the dust, world frame (m/s);
    /// w: the streak's exposure (s).
    vel: [f32; 4],
    /// xyz: the Sun's direction, world frame; w: its strength.
    sun: [f32; 4],
    /// x: density 0..1, y: brightness, z: exposure, w: target height (px).
    look: [f32; 4],
    /// xyz: an opaque body's centre from the eye (world frame, m); w: its
    /// radius (0: none) — motes behind it are hidden.
    occluder: [f32; 4],
    /// The view's basis in the SHIP frame (the cabin motes' room); w of
    /// fwd: the cabin light 0..2, w of right: cabin motes on (1) or off.
    cright: [f32; 4],
    cup: [f32; 4],
    cfwd: [f32; 4],
}

/// The scene the dust is drawn in, world frame unless said.
#[derive(Debug, Clone, Copy)]
pub struct DustScene {
    pub eye_m: DVec3,
    /// The eye's velocity relative to the dust's rest (see [`drift`]).
    pub drift_mps: DVec3,
    pub sun_dir: Vec3,
    pub density: f32,
    /// The DUST setting, 0..2: brightness rides it a little too.
    pub setting: f32,
    /// The nearest body from the eye, and its radius (m).
    pub occluder_rel: Vec3,
    pub occluder_radius_m: f32,
    /// The head in the ship's frame, for the cabin's motes; None when the
    /// eye is not in the cabin (the chase view).
    pub cabin: Option<(Quat, f32)>,
    pub target_height_px: f32,
}

impl DustUniforms {
    pub fn new(cam: &CameraFrame, scene: &DustScene) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let (right, up, forward) = cam.basis();
        let (cell, frac) = lattice(scene.eye_m);
        let (head, cabin_light, cabin_on) = match scene.cabin {
            Some((h, l)) => (h, l.clamp(0.0, 2.0), 1.0),
            None => (Quat::IDENTITY, 0.0, 0.0),
        };
        let drift = if scene.drift_mps.is_finite() {
            scene.drift_mps.as_vec3()
        } else {
            Vec3::ZERO
        };
        Self {
            right: v4(right, cam.aspect),
            up: v4(up, (cam.fov_y * 0.5).tan()),
            fwd: v4(forward, cam.time_s.rem_euclid(1000.0)),
            cell: [cell[0], cell[1], cell[2], 0],
            frac: v4(frac, CELL_M as f32),
            vel: v4(drift, STREAK_S),
            sun: v4(scene.sun_dir.normalize_or_zero(), 1.0),
            look: [
                scene.density.clamp(0.0, 1.0),
                0.7 + 0.3 * scene.setting.clamp(0.0, 2.0),
                cam.exposure,
                scene.target_height_px.max(1.0),
            ],
            occluder: v4(scene.occluder_rel, scene.occluder_radius_m.max(0.0)),
            cright: v4(head * Vec3::X, cabin_on),
            cup: v4(head * Vec3::Y, 0.0),
            cfwd: v4(head * Vec3::NEG_Z, cabin_light),
        }
    }

    pub fn density(&self) -> f32 {
        self.look[0]
    }
}

pub struct DustPass {
    /// The motes in space: drawn into the world (HDR) target.
    pipeline: wgpu::RenderPipeline,
    /// The motes in the cabin: drawn in the ship pass, whose target is the
    /// swapchain format — one pipeline per target format, or wgpu refuses.
    cabin_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl DustPass {
    pub fn new(
        device: &wgpu::Device,
        world_format: wgpu::TextureFormat,
        ship_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dust"),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(crate::shaders::DUST).into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dust uniforms"),
            size: std::mem::size_of::<DustUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dust bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dust bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("dust layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        // Additive: motes are light, black costs nothing.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let make = |target_format: wgpu::TextureFormat, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(additive),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make(world_format, "dust");
        let cabin_pipeline = make(ship_format, "dust cabin");
        Self {
            pipeline,
            cabin_pipeline,
            uniforms,
            bind_group,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, u: &DustUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(u));
    }

    /// The motes in space: before the ship and the cabin, so both hide
    /// what is behind them. Nothing at all at zero density.
    pub fn draw_space(&self, pass: &mut wgpu::RenderPass<'_>, u: &DustUniforms) {
        if u.density() <= 0.0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..6, 0..SPACE_MOTES);
    }

    /// The motes in the cabin: after the cabin, in its light.
    pub fn draw_cabin(&self, pass: &mut wgpu::RenderPass<'_>, u: &DustUniforms) {
        if u.cright[3] < 0.5 || u.look[1] <= 0.0 {
            return;
        }
        pass.set_pipeline(&self.cabin_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..6, SPACE_MOTES..SPACE_MOTES + CABIN_MOTES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mote is a point on a world lattice: move the eye one cell over
    /// and the cell index steps by one with the same offset inside it —
    /// which is what keeps the motes still as the ship flies through.
    #[test]
    fn motes_stay_put_on_the_lattice_as_the_eye_moves() {
        let eye = DVec3::new(12345.6, -7.25, 2.5e10);
        let (c0, f0) = lattice(eye);
        let (c1, f1) = lattice(eye + DVec3::new(CELL_M, 0.0, -2.0 * CELL_M));
        assert_eq!(c1[0], c0[0] + 1);
        assert_eq!(c1[1], c0[1]);
        assert_eq!(c1[2], c0[2] - 2);
        assert!((f1 - f0).length() < 1e-3, "{f0} vs {f1}");
        assert!(f0.min_element() >= 0.0 && f0.max_element() < CELL_M as f32);
        let (cn, fnn) = lattice(DVec3::new(-1.0, -41.0, 0.0));
        assert_eq!(
            cn[0],
            (-1i64).rem_euclid(1 << 31) as i32,
            "negative cells wrap, never panic"
        );
        assert_eq!(cn[1], (-2i64).rem_euclid(1 << 31) as i32);
        assert!((fnn.x - 39.0).abs() < 1e-3 && (fnn.y - 39.0).abs() < 1e-3);
    }

    #[test]
    fn dust_is_sparse_in_deep_space_thick_in_the_belt_and_gone_under_the_drive() {
        let deep = density(0.0, 0.0, 0.0, 1.0);
        assert!(deep > 0.05 && deep < 0.2, "{deep}");
        assert!(density(1.0, 0.0, 0.0, 1.0) > 0.9);
        assert!(density(0.0, 0.5, 0.0, 1.0) > 0.6);
        assert_eq!(density(1.0, 1.0, 1.0, 1.0), 0.0);
        assert_eq!(density(1.0, 1.0, 0.0, 0.0), 0.0, "DUST off is off");
        assert!(density(0.0, 0.0, 0.0, 2.0) > deep, "200% is more");
    }

    /// Coasting in a circular orbit the dust hangs still; thrusting
    /// prograde it streams past at the surplus.
    #[test]
    fn dust_rests_in_the_local_orbit() {
        let mu = 3.986e14;
        let rel = DVec3::new(0.0, -7.0e6, 0.0);
        let circ = (mu / 7.0e6f64).sqrt();
        let orbit = DVec3::new(circ, 0.0, 0.0);
        assert!(drift(rel, mu, DVec3::ZERO, orbit).length() < 1e-6);
        let d = drift(rel, mu, DVec3::ZERO, orbit + DVec3::new(300.0, 0.0, 0.0));
        assert!((d.x - 300.0).abs() < 1e-6, "{d}");
        // Riding with a moving body counts from the body's own velocity.
        let body_v = DVec3::new(0.0, 0.0, 5000.0);
        assert!(drift(rel, mu, body_v, orbit + body_v).length() < 1e-6);
        // No body to speak of: the ship's velocity is the drift.
        assert_eq!(drift(DVec3::ZERO, 0.0, DVec3::ZERO, DVec3::X), DVec3::X);
    }

    #[test]
    fn dust_lanes_hold_their_places() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 2.0,
            exposure: 1.6,
        };
        let u = DustUniforms::new(
            &cam,
            &DustScene {
                eye_m: DVec3::new(80.0, 0.0, 0.0),
                drift_mps: DVec3::new(0.0, 0.0, -3000.0),
                sun_dir: Vec3::new(0.0, 2.0, 0.0),
                density: 0.5,
                setting: 1.0,
                occluder_rel: Vec3::new(0.0, -12_000.0, 0.0),
                occluder_radius_m: 6.0e6,
                cabin: Some((Quat::IDENTITY, 1.5)),
                target_height_px: 600.0,
            },
        );
        assert_eq!(std::mem::size_of::<DustUniforms>(), 12 * 16);
        assert_eq!(u.cell[0], 2);
        assert_eq!(u.frac, [0.0, 0.0, 0.0, 40.0]);
        assert_eq!(u.vel, [0.0, 0.0, -3000.0, STREAK_S]);
        assert_eq!(u.sun[1], 1.0);
        assert_eq!(u.density(), 0.5);
        assert_eq!(u.look[3], 600.0);
        assert_eq!(u.occluder[3], 6.0e6);
        assert_eq!(u.cright[3], 1.0, "cabin motes on");
        assert_eq!(u.cfwd[3], 1.5, "the cabin's light");
        assert_eq!(u.cfwd[2], -1.0, "the head looks down -Z");
        let chase = DustUniforms::new(
            &cam,
            &DustScene {
                cabin: None,
                ..DustScene {
                    eye_m: DVec3::ZERO,
                    drift_mps: DVec3::ZERO,
                    sun_dir: Vec3::Y,
                    density: 0.1,
                    setting: 1.0,
                    occluder_rel: Vec3::ZERO,
                    occluder_radius_m: 0.0,
                    cabin: None,
                    target_height_px: 1.0,
                }
            },
        );
        assert_eq!(chase.cright[3], 0.0, "no cabin motes from the chase rig");
    }
}
