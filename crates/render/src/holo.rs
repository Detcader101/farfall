//! The holo3PP panel: the chase view, rendered to a small texture every
//! frame, projected on the canopy as a hologram — third person without
//! ever leaving first person. See `shaders/holo.wgsl`.

/// The offscreen picture's size. Small on purpose: the panel covers a
/// fraction of the screen, and the world passes render it live every
/// frame beside the real view.
pub const HOLO_W: u32 = 512;
pub const HOLO_H: u32 = 288;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HoloUniforms {
    a: [f32; 4],
    b: [f32; 4],
    sway: [f32; 4],
}

impl HoloUniforms {
    /// `centre`: the panel's middle on the canopy (NDC); `half_h`: half
    /// its height in canopy units; `shown`: draw at all.
    pub fn new(
        centre: [f32; 2],
        half_h: f32,
        aspect: f32,
        height_px: f32,
        time_s: f32,
        sway: [f32; 2],
        shown: bool,
    ) -> Self {
        Self {
            a: [centre[0], centre[1], half_h.max(0.0), aspect],
            b: [
                HOLO_W as f32 / HOLO_H as f32,
                height_px,
                if shown { 1.0 } else { 0.0 },
                time_s.rem_euclid(1000.0),
            ],
            sway: [sway[0], sway[1], 0.0, 0.0],
        }
    }
}

/// The panel pass and the texture the rig renders into.
pub struct HoloPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    view: wgpu::TextureView,
}

impl HoloPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        picture_format: wgpu::TextureFormat,
    ) -> Self {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("holo picture"),
            size: wgpu::Extent3d {
                width: HOLO_W,
                height: HOLO_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: picture_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("holo"),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(crate::shaders::HOLO).into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("holo uniforms"),
            size: std::mem::size_of::<HoloUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("holo sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("holo bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("holo bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("holo layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("holo"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            view,
        }
    }

    /// The attachment the rig renders the chase view into.
    pub fn picture_attachment(&self) -> wgpu::RenderPassColorAttachment<'_> {
        wgpu::RenderPassColorAttachment {
            view: &self.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, u: &HoloUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(u));
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

    /// The uniform block is a wire format for holo.wgsl: pin the lanes.
    #[test]
    fn holo_lanes_hold_their_places() {
        let u = HoloUniforms::new([0.5, -0.25], 0.3, 1.6, 1200.0, 7.0, [0.01, -0.02], true);
        assert_eq!(std::mem::size_of::<HoloUniforms>(), 3 * 16);
        assert_eq!(u.a, [0.5, -0.25, 0.3, 1.6]);
        assert_eq!(u.b[0], HOLO_W as f32 / HOLO_H as f32, "picture aspect");
        assert_eq!(u.b[2], 1.0, "shown");
        assert_eq!(u.sway[0], 0.01);
        let off = HoloUniforms::new([0.0, 0.0], 0.3, 1.6, 1200.0, 7.0, [0.0, 0.0], false);
        assert_eq!(off.b[2], 0.0, "hidden discards");
    }
}
