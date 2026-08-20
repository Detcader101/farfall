//! HUD text pass (SPEC §6.5): the pilot's readout, projected on the canopy.
//!
//! The bitmap comes from [`crate::text`]; this pass puts it on the same
//! spherical shell as the instrument cluster (the `canopy()` warp in the
//! common prelude), so there is no flat debug overlay layered over the
//! hologram cockpit — every readable thing sits on one piece of glass.

use crate::text::{TextBitmap, ROWS, ROW_WORDS};

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HudUniforms {
    /// xy: canopy anchor in NDC (top-left of the block), z: font-pixel size
    /// in canopy units, w: aspect.
    a: [f32; 4],
    /// xy: occupied extent in font pixels (the panel's size), z: surface
    /// height in px (scanline frequency), w: unused.
    extent: [f32; 4],
    color: [f32; 4],
    backdrop: [f32; 4],
    rows: [[u32; ROW_WORDS]; ROWS],
}

pub struct HudPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl HudPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(crate::shaders::HUD).into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud uniforms"),
            size: std::mem::size_of::<HudUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud bgl"),
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
            label: Some("hud bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
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
        }
    }

    /// `anchor_ndc`: where on the canopy the block's top-left sits.
    /// `px_canopy`: one font pixel in canopy units (drives apparent size).
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        bitmap: &TextBitmap,
        anchor_ndc: [f32; 2],
        px_canopy: f32,
        aspect: f32,
        height_px: f32,
    ) {
        let (w, h) = bitmap.used_extent();
        let u = HudUniforms {
            a: [anchor_ndc[0], anchor_ndc[1], px_canopy, aspect],
            extent: [w as f32, h as f32, height_px, 0.0],
            // Hologram cyan, matching the instrument cluster.
            color: [0.45, 0.92, 1.0, 0.96],
            // Smoked glass behind the text, not a debug box.
            backdrop: [0.01, 0.03, 0.05, 0.30],
            rows: bitmap.rows,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
