//! The Sun and the Moon (`shaders/bodies.wgsl`), at the world's 1:100 scale.

use crate::CameraFrame;
use glam::Vec3;

/// Real Sun and Moon, divided by the world's linear scale. The planet is a
/// 1:100 Earth (63.71 km), so these are 1:100 too — and each subtends the
/// half degree it really does from anywhere near the planet.
pub const WORLD_SCALE: f64 = 1.0 / 100.0;
pub const MOON_RADIUS_M: f64 = 1_737_400.0 * WORLD_SCALE;
pub const MOON_ORBIT_M: f64 = 384_400_000.0 * WORLD_SCALE;
pub const SUN_RADIUS_M: f64 = 696_340_000.0 * WORLD_SCALE;
pub const SUN_DISTANCE_M: f64 = 149_597_870_000.0 * WORLD_SCALE;

/// Angular radius of the Sun's disc, radians: ~0.27°, as seen from Earth.
pub fn sun_angular_radius() -> f32 {
    (SUN_RADIUS_M / SUN_DISTANCE_M) as f32
}

/// Orbital period of the Moon around this planet, seconds: Kepler, from
/// the planet's own μ, so the scaled world stays self-consistent (it is
/// about 2.75 days here, against the real 27.3).
pub fn moon_period_s(mu: f64) -> f64 {
    std::f64::consts::TAU * (MOON_ORBIT_M.powi(3) / mu).sqrt()
}

/// The Moon's position in the planet's frame at sim time `t`, metres. A
/// circular orbit in the XZ plane, starting on +X.
pub fn moon_position(mu: f64, t_s: f64) -> glam::DVec3 {
    let phase = std::f64::consts::TAU * t_s / moon_period_s(mu);
    glam::DVec3::new(MOON_ORBIT_M * phase.cos(), 0.0, MOON_ORBIT_M * phase.sin())
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BodiesUniforms {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    params: [f32; 4],
    moon: [f32; 4],
    sun: [f32; 4],
}

impl BodiesUniforms {
    /// `moon_rel`: the Moon's centre relative to the camera (subtracted in
    /// f64 by the caller — SPEC P3).
    pub fn new(cam: &CameraFrame, moon_rel: Vec3, sun_dir: Vec3) -> Self {
        let (right, up, forward) = cam.basis();
        let s = sun_dir.normalize_or_zero();
        Self {
            right: [right.x, right.y, right.z, 0.0],
            up: [up.x, up.y, up.z, 0.0],
            forward: [forward.x, forward.y, forward.z, 0.0],
            params: [
                (cam.fov_y * 0.5).tan(),
                cam.aspect,
                cam.time_s,
                cam.exposure,
            ],
            moon: [moon_rel.x, moon_rel.y, moon_rel.z, MOON_RADIUS_M as f32],
            sun: [s.x, s.y, s.z, sun_angular_radius()],
        }
    }
}

pub struct BodiesPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl BodiesPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bodies"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::BODIES).into(),
            ),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bodies uniforms"),
            size: std::mem::size_of::<BodiesUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bodies bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bodies bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bodies layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bodies"),
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
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
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

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &BodiesUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both bodies subtend their real half degree: 1:100 on radius and on
    /// distance cancels in the angle.
    #[test]
    fn sun_and_moon_subtend_half_a_degree() {
        let sun_deg = 2.0 * sun_angular_radius().to_degrees();
        assert!((sun_deg - 0.533).abs() < 0.01, "{sun_deg}");
        let moon_deg = 2.0 * (MOON_RADIUS_M / MOON_ORBIT_M).atan().to_degrees();
        assert!((moon_deg - 0.518).abs() < 0.01, "{moon_deg}");
    }

    /// The Moon keeps its distance and goes round: Kepler from the compact
    /// planet's μ gives about 2.75 days.
    #[test]
    fn moon_orbit_is_keplerian_at_this_scale() {
        let radius_m = 63_710.0f64;
        let mu = 9.81 * radius_m * radius_m;
        let period = moon_period_s(mu);
        assert!(
            (period / 86_400.0 - 2.75).abs() < 0.1,
            "{}",
            period / 86_400.0
        );
        for t in [0.0, 1000.0, period * 0.37] {
            let p = moon_position(mu, t);
            assert!((p.length() - MOON_ORBIT_M).abs() < 1.0);
            assert_eq!(p.y, 0.0);
        }
        let half = moon_position(mu, period * 0.5);
        assert!(half.x < -MOON_ORBIT_M * 0.999);
    }
}
