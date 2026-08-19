//! Analytic planet pass (SPEC §6.5).
//!
//! Alpha-blended over the starfield: the shader reports per-pixel coverage from
//! the analytic limb, so the planet's edge is antialiased without MSAA (which
//! cannot see a shader edge — see the pass header in `shaders/planet.wgsl`).

use crate::CameraFrame;
use glam::Vec3;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PlanetUniforms {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    /// x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: [f32; 4],
    /// xyz: planet centre relative to the camera (m), w: radius (m)
    centre_radius: [f32; 4],
    /// xyz: unit vector toward the sun
    sun_dir: [f32; 4],
}

impl PlanetUniforms {
    /// `centre_rel` must already be camera-relative: the world-space
    /// subtraction happens in f64 on the caller's side, so this f32 only ever
    /// carries a local offset (SPEC P3).
    pub fn new(cam: &CameraFrame, centre_rel: Vec3, radius_m: f32, sun_dir: Vec3) -> Self {
        let (right, up, forward) = cam.basis();
        let sun = sun_dir.normalize_or_zero();
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
            centre_radius: [centre_rel.x, centre_rel.y, centre_rel.z, radius_m],
            sun_dir: [sun.x, sun.y, sun.z, 0.0],
        }
    }

    /// Half-angle subtended by the planet, radians. Zero when the camera is at
    /// or inside the surface. Used by the app to decide framing and, later, to
    /// drive band promotion (SPEC §6.7).
    pub fn angular_radius(&self) -> f32 {
        let c = Vec3::from_slice(&self.centre_radius[..3]);
        let d = c.length();
        let r = self.centre_radius[3];
        if d <= r {
            0.0
        } else {
            (r / d).clamp(0.0, 1.0).asin()
        }
    }
}

pub struct PlanetPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl PlanetPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("planet"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::PLANET).into(),
            ),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("planet uniforms"),
            size: std::mem::size_of::<PlanetUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("planet bgl"),
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
            label: Some("planet bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("planet layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("planet"),
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
                    // Premultiplied: the shader composites its own surface and rim
                    // layers, so the blend must add rather than re-weight them.
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

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &PlanetUniforms) {
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
    use glam::Quat;

    fn cam() -> CameraFrame {
        CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 70f32.to_radians(),
            aspect: 1.6,
            time_s: 0.0,
            exposure: 1.6,
        }
    }

    #[test]
    fn angular_radius_matches_geometry() {
        // 20 km above a 63.71 km planet: centre is 83.71 km away.
        let u = PlanetUniforms::new(&cam(), Vec3::new(0.0, -83_710.0, 0.0), 63_710.0, Vec3::X);
        let expected = (63_710.0f32 / 83_710.0).asin();
        assert!((u.angular_radius() - expected).abs() < 1e-6);
        // Sanity: that is a very large planet in the sky, ~50 degrees.
        assert!(u.angular_radius().to_degrees() > 45.0);
    }

    #[test]
    fn angular_radius_shrinks_with_distance() {
        let near = PlanetUniforms::new(&cam(), Vec3::new(0.0, 0.0, -1.0e5), 63_710.0, Vec3::X);
        let far = PlanetUniforms::new(&cam(), Vec3::new(0.0, 0.0, -1.0e7), 63_710.0, Vec3::X);
        assert!(near.angular_radius() > far.angular_radius());
        assert!(far.angular_radius() > 0.0);
    }

    #[test]
    fn inside_the_planet_reports_zero_rather_than_nan() {
        let u = PlanetUniforms::new(&cam(), Vec3::new(0.0, 0.0, -100.0), 63_710.0, Vec3::X);
        assert_eq!(u.angular_radius(), 0.0);
    }

    #[test]
    fn sun_direction_is_normalised() {
        let u = PlanetUniforms::new(&cam(), Vec3::NEG_Y * 1.0e5, 6.0e4, Vec3::new(3.0, 4.0, 0.0));
        let len = (u.sun_dir[0] * u.sun_dir[0] + u.sun_dir[1] * u.sun_dir[1]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "sun dir not unit: {len}");
    }
}
