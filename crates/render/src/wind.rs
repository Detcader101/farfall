//! Wind ribbons: the planet's wind made visible. Streaks of moving air
//! hash-placed on a lattice of world cells about the eye (so each holds
//! its place as the ship flies through), each stretched along the local
//! wind vector with a bright packet running downwind — direction readable
//! at a glance, length and brightness riding the strength, denser in a
//! gust, fading with the air itself, nothing at all above the atmosphere.
//! See `shaders/wind.wgsl`.
//!
//! The wind itself is the sim's: the app samples `farfall_sim::wind_mps`
//! at the eye and a gap above it and hands both vectors over; the shader
//! only interpolates between them. One field, one source of truth — the
//! GPU never grows a second opinion about the weather.

use glam::{DVec3, Vec3};

use crate::CameraFrame;

/// The lattice's cell, metres. The volume drawn is `REACH` cells each way.
pub const CELL_M: f64 = 50.0;
/// Cells each way from the eye's own (a 5×5×5 block at 2).
pub const REACH: u32 = 2;
/// Ribbons per cell at full density.
pub const PER_CELL: u32 = 10;
/// Ribbons drawn (the whole block).
pub const RIBBONS: u32 = (2 * REACH + 1) * (2 * REACH + 1) * (2 * REACH + 1) * PER_CELL;
/// How far above the eye the second wind sample sits, metres.
pub const SAMPLE_GAP_M: f64 = 300.0;

/// Where the eye sits on the lattice: its cell (wrapped to i32 — the
/// hash only needs a stable index) and its offset within that cell,
/// metres, 0..CELL_M.
pub fn lattice(eye_m: DVec3) -> ([i32; 3], Vec3) {
    let cell = eye_m.map(|v| (v / CELL_M).floor());
    let frac = (eye_m - cell * CELL_M).as_vec3();
    let wrap = |v: f64| (v as i64).rem_euclid(1 << 31) as i32;
    ([wrap(cell.x), wrap(cell.y), wrap(cell.z)], frac)
}

/// How many of the ribbons show, 0..1: nothing without air or with the
/// WIND setting off, more of them the thicker the air, the harder it
/// blows, and the higher the setting (0..2, 1 stock).
pub fn density(air: f32, setting: f32, speed_mps: f32) -> f32 {
    let air = air.clamp(0.0, 1.0);
    let gust = (speed_mps.max(0.0) / 18.0).clamp(0.0, 1.5);
    (air.sqrt() * 0.5 * setting.clamp(0.0, 2.0) * (0.5 + 0.5 * gust)).clamp(0.0, 1.0)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WindUniforms {
    /// The camera's basis in the WORLD frame; w: aspect, tan(fov/2), time.
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    /// The eye's cell on the lattice (xyz), w unused.
    cell: [i32; 4],
    /// xyz: the eye's offset within its cell (m); w: the cell size (m).
    frac: [f32; 4],
    /// xyz: the sim's wind at the eye (m/s, world); w: the air 0..1.
    wlow: [f32; 4],
    /// xyz: the sim's wind a gap above the eye; w: the gap (m).
    whigh: [f32; 4],
    /// xyz: the planet's up at the eye (world); w: the WIND setting.
    upw: [f32; 4],
    /// x: ribbon density 0..1, y: brightness, z: wind speed / 60,
    /// w: target height (px).
    look: [f32; 4],
}

/// The scene the ribbons are drawn in, world frame.
#[derive(Debug, Clone, Copy)]
pub struct WindScene {
    pub eye_m: DVec3,
    /// The sim's wind at the eye, m/s.
    pub wind_low: Vec3,
    /// The sim's wind `SAMPLE_GAP_M` above the eye, m/s.
    pub wind_high: Vec3,
    /// The planet's up at the eye.
    pub up: Vec3,
    /// How much air surrounds the hull, 0 (vacuum) .. 1 (sea level).
    pub air: f32,
    /// The WIND setting, 0..2.
    pub setting: f32,
    pub target_height_px: f32,
}

impl WindUniforms {
    pub fn new(cam: &CameraFrame, scene: &WindScene) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let sane = |v: Vec3| if v.is_finite() { v } else { Vec3::ZERO };
        let (right, up, forward) = cam.basis();
        let (cell, frac) = lattice(scene.eye_m);
        let low = sane(scene.wind_low);
        let high = sane(scene.wind_high);
        let air = if scene.air.is_finite() {
            scene.air.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let setting = scene.setting.clamp(0.0, 2.0);
        Self {
            right: v4(right, cam.aspect),
            up: v4(up, (cam.fov_y * 0.5).tan()),
            fwd: v4(forward, cam.time_s.rem_euclid(1000.0)),
            cell: [cell[0], cell[1], cell[2], 0],
            frac: v4(frac, CELL_M as f32),
            wlow: v4(low, air),
            whigh: v4(high, SAMPLE_GAP_M as f32),
            upw: v4(scene.up.normalize_or_zero(), setting),
            look: [
                density(air, setting, low.length()),
                0.7 + 0.3 * setting,
                (low.length() / 60.0).clamp(0.0, 2.0),
                scene.target_height_px.max(1.0),
            ],
        }
    }

    pub fn density(&self) -> f32 {
        self.look[0]
    }
}

pub struct WindPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl WindPass {
    pub fn new(
        device: &wgpu::Device,
        world_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wind"),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(crate::shaders::WIND).into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wind uniforms"),
            size: std::mem::size_of::<WindUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wind bgl"),
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
            label: Some("wind bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wind layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        // Additive: the ribbons are light, black costs nothing.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wind"),
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
                    format: world_format,
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
        });
        Self {
            pipeline,
            uniforms,
            bind_group,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, u: &WindUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(u));
    }

    /// Nothing at all without air, wind, or the setting.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, u: &WindUniforms) {
        if u.density() <= 0.0 || u.wlow[3] <= 0.0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..6, 0..RIBBONS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    /// A ribbon is a point on a world lattice: move the eye one cell over
    /// and the cell index steps by one with the same offset inside it —
    /// which is what keeps each ribbon in its place as the ship flies by.
    #[test]
    fn ribbons_hold_their_places_on_the_lattice() {
        let eye = DVec3::new(63_712.4, 812.6, -3.5e4);
        let (c0, f0) = lattice(eye);
        let (c1, f1) = lattice(eye + DVec3::new(CELL_M, -3.0 * CELL_M, 0.0));
        assert_eq!(c1[0], c0[0] + 1);
        assert_eq!(c1[1], c0[1] - 3);
        assert_eq!(c1[2], c0[2]);
        assert!((f1 - f0).length() < 1e-3, "{f0} vs {f1}");
        assert!(f0.min_element() >= 0.0 && f0.max_element() < CELL_M as f32);
    }

    /// More air, more wind, or more setting: more ribbons. No air, or the
    /// setting off: none at all, whatever blows.
    #[test]
    fn ribbons_thicken_with_air_and_wind_and_vanish_when_off() {
        assert_eq!(density(0.0, 1.0, 25.0), 0.0, "vacuum shows nothing");
        assert_eq!(density(1.0, 0.0, 25.0), 0.0, "OFF is off");
        let calm = density(1.0, 1.0, 4.0);
        let gale = density(1.0, 1.0, 30.0);
        assert!(gale > calm, "{calm} vs {gale}");
        assert!(density(1.0, 2.0, 12.0) > density(1.0, 1.0, 12.0), "200%");
        assert!(density(0.2, 1.0, 12.0) < density(1.0, 1.0, 12.0));
        for d in [calm, gale, density(1.0, 2.0, 60.0)] {
            assert!((0.0..=1.0).contains(&d), "{d}");
        }
    }

    /// The uniforms carry the sim's own samples into their lanes — the
    /// shader interpolates these two vectors and never re-derives the
    /// weather.
    #[test]
    fn the_uniforms_carry_the_sim_wind_into_their_lanes() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.6,
            time_s: 3.0,
            exposure: 1.0,
        };
        let u = WindUniforms::new(
            &cam,
            &WindScene {
                eye_m: DVec3::new(150.0, 0.0, 0.0),
                wind_low: Vec3::new(12.0, 0.0, -5.0),
                wind_high: Vec3::new(30.0, 0.0, 0.0),
                up: Vec3::new(2.0, 0.0, 0.0),
                air: 0.8,
                setting: 1.0,
                target_height_px: 900.0,
            },
        );
        assert_eq!(std::mem::size_of::<WindUniforms>(), 9 * 16);
        assert_eq!(u.cell[0], 3);
        assert_eq!(u.frac, [0.0, 0.0, 0.0, CELL_M as f32]);
        assert_eq!(u.wlow, [12.0, 0.0, -5.0, 0.8], "wind then air");
        assert_eq!(u.whigh, [30.0, 0.0, 0.0, SAMPLE_GAP_M as f32]);
        assert_eq!(u.upw, [1.0, 0.0, 0.0, 1.0], "up normalised, the setting");
        assert_eq!(u.look[0], density(0.8, 1.0, 13.0));
        assert_eq!(u.look[3], 900.0);
        // A vacuum scene draws nothing, however hard the numbers blow.
        let vac = WindUniforms::new(
            &cam,
            &WindScene {
                air: 0.0,
                ..WindScene {
                    eye_m: DVec3::ZERO,
                    wind_low: Vec3::X * 50.0,
                    wind_high: Vec3::X * 50.0,
                    up: Vec3::Y,
                    air: 0.0,
                    setting: 2.0,
                    target_height_px: 1.0,
                }
            },
        );
        assert_eq!(vac.density(), 0.0);
    }
}
