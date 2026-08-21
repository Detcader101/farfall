//! Predicted-path pass: the ship's ballistic future, integrated and drawn
//! on the GPU (`shaders/trajectory.wgsl`). The CPU supplies the state and
//! the laws; no list of points crosses over.

use crate::CameraFrame;
use glam::Vec3;

/// Segments in the ribbon. Vertex work grows as the square of this (each
/// vertex integrates its own prefix), so it is a real knob: 160 is ~300k
/// cheap steps a frame.
pub const SEGMENTS: u32 = 160;
/// Distance rings along the path; must match RING_COUNT in the shader.
pub const RINGS: u32 = 8;

/// The world's laws, as the prediction needs them. Plain numbers copied
/// from the sim's parameters by the app: the render crate never imports
/// the sim (SPEC §5.1).
#[derive(Debug, Clone, Copy)]
pub struct TrajectoryWorld {
    /// Planet centre relative to the ship, metres (f64 subtraction done by
    /// the caller — SPEC P3).
    pub centre_rel: Vec3,
    pub radius_m: f32,
    pub mu: f32,
    pub rho0: f32,
    pub scale_height_m: f32,
    pub atmo_top_m: f32,
    /// Ship velocity, world frame.
    pub vel_world: Vec3,
    /// Nose-on drag area over mass, m²/kg: the prediction assumes a
    /// prograde hull.
    pub cda_over_m: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TrajectoryUniforms {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    params: [f32; 4],
    centre_radius: [f32; 4],
    phys: [f32; 4],
    vel: [f32; 4],
    look: [f32; 4],
}

impl TrajectoryUniforms {
    pub fn new(
        cam: &CameraFrame,
        world: &TrajectoryWorld,
        horizon_s: f32,
        visibility: f32,
        height_px: f32,
    ) -> Self {
        let (right, up, forward) = cam.basis();
        let c = world.centre_rel;
        let v = world.vel_world;
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
            centre_radius: [c.x, c.y, c.z, world.radius_m.max(1.0)],
            phys: [
                world.mu.max(0.0),
                world.rho0.max(0.0),
                world.scale_height_m.max(1.0),
                world.atmo_top_m.max(0.0),
            ],
            vel: [v.x, v.y, v.z, world.cda_over_m.max(0.0)],
            look: [
                horizon_s.max(1.0),
                SEGMENTS as f32,
                visibility.clamp(0.0, 1.0),
                height_px.max(1.0),
            ],
        }
    }
}

pub struct TrajectoryPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl TrajectoryPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("trajectory"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::TRAJECTORY).into(),
            ),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trajectory uniforms"),
            size: std::mem::size_of::<TrajectoryUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("trajectory bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // The vertex stage integrates; the fragment stage reads time
                // and visibility.
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
            label: Some("trajectory bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("trajectory layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("trajectory"),
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
                    blend: Some(wgpu::BlendState {
                        color: additive,
                        alpha: additive,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // Quads are built either way round as the path bends.
                cull_mode: None,
                ..Default::default()
            },
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

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &TrajectoryUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        // The ribbon, two reticle quads, then the distance rings.
        pass.draw(0..(SEGMENTS * 6 + 12 + RINGS * 6), 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn uniforms_are_clamped_and_sized() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.6,
            time_s: 0.0,
            exposure: 1.6,
        };
        let world = TrajectoryWorld {
            centre_rel: Vec3::NEG_Y * 70_000.0,
            radius_m: -5.0,
            mu: -1.0,
            rho0: 1.2,
            scale_height_m: 0.0,
            atmo_top_m: -1.0,
            vel_world: Vec3::X * 700.0,
            cda_over_m: -0.1,
        };
        let u = TrajectoryUniforms::new(&cam, &world, 0.0, 7.0, 0.0);
        assert!(u.centre_radius[3] >= 1.0);
        assert_eq!(u.phys[0], 0.0);
        assert!(u.phys[2] >= 1.0);
        assert_eq!(u.vel[3], 0.0);
        assert!(u.look[0] >= 1.0);
        assert_eq!(u.look[1], SEGMENTS as f32);
        assert_eq!(u.look[2], 1.0);
        assert!(u.look[3] >= 1.0);
    }
}
