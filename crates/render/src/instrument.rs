//! A canopy instrument: one quad on the glass, one shader, one 64-byte
//! uniform block. Every gauge in the cluster is this pass with a different
//! shader and different numbers — the speedo, the altimeter, the gyro. The
//! slot layout (app side) decides where each one sits; a visibility of zero
//! makes any of them vanish entirely, which is the contract the cockpit
//! menu relies on.

use bytemuck::Pod;

pub struct InstrumentPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    uniform_bytes: u64,
}

/// How a pass writes its target: the blend, the uniform block's size, and
/// which channels it may write.
struct PaneTarget {
    blend: wgpu::BlendState,
    uniform_bytes: u64,
    write_mask: wgpu::ColorWrites,
}

/// Every instrument's uniform block is four vec4s. The shaders agree on the
/// size, not the meaning.
pub const UNIFORM_BYTES: u64 = 64;

impl InstrumentPass {
    /// An additive instrument: projected light, black costs nothing.
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        label: &'static str,
        shader_src: &str,
    ) -> Self {
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        Self::with_blend(
            device,
            target_format,
            sample_count,
            label,
            shader_src,
            PaneTarget {
                blend: additive,
                uniform_bytes: UNIFORM_BYTES,
                write_mask: wgpu::ColorWrites::COLOR,
            },
        )
    }

    /// An additive instrument with a bigger block of numbers (a dial that
    /// may be placed in the dash carries its placement too).
    pub fn new_sized(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        label: &'static str,
        shader_src: &str,
        uniform_bytes: u64,
    ) -> Self {
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        Self::with_blend(
            device,
            target_format,
            sample_count,
            label,
            shader_src,
            PaneTarget {
                blend: additive,
                uniform_bytes,
                write_mask: wgpu::ColorWrites::COLOR,
            },
        )
    }

    /// A pane: premultiplied over, for something that darkens what it sits
    /// on (the map).
    pub fn new_pane(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        label: &'static str,
        shader_src: &str,
    ) -> Self {
        Self::with_blend(
            device,
            target_format,
            sample_count,
            label,
            shader_src,
            PaneTarget {
                blend: wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                uniform_bytes: UNIFORM_BYTES,
                write_mask: wgpu::ColorWrites::COLOR,
            },
        )
    }

    /// A layer rendered to a texture of its own, later composited: no
    /// blending (it writes coverage straight, transparent where there is
    /// nothing), so parts of it can be redrawn in place.
    pub fn new_layer_sized(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        label: &'static str,
        shader_src: &str,
        uniform_bytes: u64,
    ) -> Self {
        Self::with_blend(
            device,
            target_format,
            1,
            label,
            shader_src,
            PaneTarget {
                blend: wgpu::BlendState::REPLACE,
                uniform_bytes,
                write_mask: wgpu::ColorWrites::ALL,
            },
        )
    }

    /// A pane with a bigger block of numbers than a dial needs (the 3D map
    /// carries a camera, four bodies and a ship).
    pub fn new_pane_sized(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        label: &'static str,
        shader_src: &str,
        uniform_bytes: u64,
    ) -> Self {
        Self::with_blend(
            device,
            target_format,
            sample_count,
            label,
            shader_src,
            PaneTarget {
                blend: wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                uniform_bytes,
                // Alpha too: a sized pane may render to a texture of its
                // own that is composited later; its coverage must survive.
                write_mask: wgpu::ColorWrites::ALL,
            },
        )
    }

    fn with_blend(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        label: &'static str,
        shader_src: &str,
        target: PaneTarget,
    ) -> Self {
        let PaneTarget {
            blend,
            uniform_bytes,
            write_mask,
        } = target;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(shader_src).into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: uniform_bytes,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // The vertex stage reads the anchor to place the quad.
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
            label: Some(label),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    blend: Some(blend),
                    write_mask,
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
            uniform_bytes,
        }
    }

    /// Upload this instrument's numbers. Any Pod block of the size the pass
    /// was made with will do; the shader decides what the lanes mean.
    pub fn update<T: Pod>(&self, queue: &wgpu::Queue, uniforms: &T) {
        let bytes = bytemuck::bytes_of(uniforms);
        assert_eq!(
            bytes.len() as u64,
            self.uniform_bytes,
            "instrument uniforms must be {} bytes",
            self.uniform_bytes
        );
        queue.write_buffer(&self.uniforms, 0, bytes);
    }

    /// Six vertices: the shader's vs_main places a quad around its anchor.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }

    /// Draw only within `rect` ([x, y, w, h] in target pixels) — an
    /// instrument covers a corner of the screen and a full-screen quad
    /// that discards still pays to rasterise every pixel. None draws
    /// nothing (the instrument is hidden). `full` is the target's size,
    /// to put the scissor back for whatever draws next.
    pub fn draw_within(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        rect: Option<[u32; 4]>,
        full: (u32, u32),
    ) {
        let Some([x, y, w, h]) = rect else {
            return;
        };
        if w == 0 || h == 0 {
            return;
        }
        pass.set_scissor_rect(x, y, w, h);
        self.draw(pass);
        pass.set_scissor_rect(0, 0, full.0, full.1);
    }
}
