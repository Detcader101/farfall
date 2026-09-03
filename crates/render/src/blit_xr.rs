//! Native VR's cut-out blit (SPEC §5.3): the crop that stands in for the
//! browser's WebGL compositor on the WebXR path — see `shaders/xrblit.wgsl`
//! and `app/src/xr.rs::cutout_uv`. Two uses of the same pipeline: cropping
//! each eye's symmetric render down to its true asymmetric field into that
//! eye's OpenXR swapchain image, and a plain aspect-correct crop (the left
//! eye's half, no further cut) into the mirror window.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct XrBlitUniforms {
    /// u0, v0, u1, v1 — the source rectangle in the bound texture's own
    /// UV space (v = 0 at the top).
    pub rect: [f32; 4],
}

impl XrBlitUniforms {
    pub fn new(rect: [f32; 4]) -> Self {
        Self { rect }
    }
}

pub struct XrBlitPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
}

impl XrBlitPass {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xrblit"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::XRBLIT).into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xrblit bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // Linear: the crop generally rescales (the symmetric render is
        // wider than the true field it is cut down to).
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xrblit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xrblit uniforms"),
            size: std::mem::size_of::<XrBlitUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xrblit layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xrblit"),
            layout: Some(&pipeline_layout),
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
                targets: &[Some(target_format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            bind_group: None,
        }
    }

    /// Point the pass at a new source texture. Must be called whenever
    /// that texture is recreated, or the pass samples a destroyed view.
    pub fn rebind(&mut self, device: &wgpu::Device, source: &wgpu::TextureView) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xrblit bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        }));
    }

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &XrBlitUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
