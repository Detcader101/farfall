//! The Sun and the Moon (`shaders/bodies.wgsl`): two lit spheres wherever the
//! sim puts them, at whatever size it gives them.

use crate::CameraFrame;
use glam::Vec3;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BodiesUniforms {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    params: [f32; 4],
    moon: [f32; 4],
    sun: [f32; 4],
    /// x: tags, y: height px, z: LENS FLARE strength (0 off), w: unused
    look: [f32; 4],
    /// Uranus: xyz camera-relative centre, w radius.
    uranus: [f32; 4],
    /// xyz: the planet's centre relative to the camera, w: its radius —
    /// the thing most likely to stand in front of the Sun. LAST, as in
    /// the shader: the struct is the wire format.
    planet: [f32; 4],
}

impl BodiesUniforms {
    /// `moon`, `sun`: each body's centre relative to the camera (subtracted
    /// in f64 by the caller — SPEC P3) and its radius, metres.
    /// `tags`: 0..1, the finder rings. `height_px`: for their minimum size.
    pub fn new(
        cam: &CameraFrame,
        moon: (Vec3, f32),
        sun: (Vec3, f32),
        uranus: (Vec3, f32),
        tags: f32,
        height_px: f32,
    ) -> Self {
        let (right, up, forward) = cam.basis();
        let (moon_rel, moon_r) = (crate::planet::eye_clear(moon.0, moon.1), moon.1);
        let (s, sun_r) = (crate::planet::eye_clear(sun.0, sun.1), sun.1);
        let uranus = (crate::planet::eye_clear(uranus.0, uranus.1), uranus.1);
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
            moon: [moon_rel.x, moon_rel.y, moon_rel.z, moon_r],
            sun: [s.x, s.y, s.z, sun_r],
            look: [tags.clamp(0.0, 1.0), height_px.max(1.0), 1.0, 0.0],
            uranus: [uranus.0.x, uranus.0.y, uranus.0.z, uranus.1],
            planet: [0.0; 4],
        }
    }

    /// The planet as an occluder of the Sun (for the flare), and how
    /// strong the lens flare is (graphics.flare, 1 stock, 0 none).
    pub fn with_planet_and_flare(mut self, planet: (Vec3, f32), flare: f32) -> Self {
        let c = crate::planet::eye_clear(planet.0, planet.1);
        self.planet = [c.x, c.y, c.z, planet.1.max(0.0)];
        self.look[2] = if flare.is_finite() {
            flare.clamp(0.0, 2.0)
        } else {
            1.0
        };
        self
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
    use glam::Quat;

    /// The uniform block is the shader's wire format: the lanes land where
    /// bodies.wgsl reads them. A field in the wrong place once put the
    /// planet's numbers where Uranus was read, and Uranus vanished.
    #[test]
    fn uranus_and_the_planet_land_in_their_own_lanes() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let u = BodiesUniforms::new(
            &cam,
            (Vec3::new(1_000.0, 0.0, 0.0), 10.0),
            (Vec3::new(2_000.0, 0.0, 0.0), 20.0),
            (Vec3::new(3_000.0, 0.0, 0.0), 30.0),
            1.0,
            900.0,
        )
        .with_planet_and_flare((Vec3::new(4_000.0, 0.0, 0.0), 40.0), 0.5);
        let words: &[f32] = bytemuck::cast_slice(bytemuck::bytes_of(&u));
        // right, up, forward, params, moon, sun, look, uranus, planet.
        assert_eq!(words[4 * 4], 1_000.0, "moon at lane 4");
        assert_eq!(words[5 * 4 + 3], 20.0, "sun at lane 5");
        assert_eq!(words[6 * 4 + 2], 0.5, "flare in look.z");
        assert_eq!(
            &words[7 * 4..7 * 4 + 4],
            &[3_000.0, 0.0, 0.0, 30.0],
            "uranus at lane 7"
        );
        assert_eq!(
            &words[8 * 4..8 * 4 + 4],
            &[4_000.0, 0.0, 0.0, 40.0],
            "planet at lane 8"
        );
        assert_eq!(std::mem::size_of::<BodiesUniforms>(), 9 * 16);
    }
}
