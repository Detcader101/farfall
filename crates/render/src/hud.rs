//! HUD text pass (SPEC §6.5): the pilot's readout, projected on the canopy.
//!
//! The bitmap comes from [`crate::text`]; this pass puts it on the same
//! spherical shell as the instrument cluster (the `canopy()` warp in the
//! common prelude), so there is no flat debug overlay layered over the
//! hologram cockpit — every readable thing sits on one piece of glass.
//! The same pass draws the flat cards (the menu, the panels beside the
//! map and the bay, the first-run card): a card is a block with `flat`
//! set, whose furniture — rules, the chosen row's band, a scrollbar — is
//! laid out in font pixels by the app and only painted here.

use crate::text::{TextBitmap, ROWS, ROW_WORDS};

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HudUniforms {
    /// xy: canopy anchor in NDC (top-left of the block), z: font-pixel size
    /// in canopy units, w: aspect.
    a: [f32; 4],
    /// xy: the panel's extent in font pixels, z: surface height in px
    /// (scanline frequency), w: the highlighted row's top (negative: none).
    extent: [f32; 4],
    color: [f32; 4],
    backdrop: [f32; 4],
    /// xy: hologram sway (canopy units), z: 1 for a FLAT block on the
    /// screen (the pause panels), 0 for one on the glass (the readout),
    /// w: the highlighted row's height.
    sway: [f32; 4],
    /// The scrollbar in font px: track top, track bottom, thumb top, thumb
    /// bottom; x negative for none.
    bar: [f32; 4],
    /// x: header rule row, y: footer rule row (font px; negative: none).
    rules: [f32; 4],
    rows: [[u32; ROW_WORDS]; ROWS],
}

/// A scrollbar's geometry in font pixels, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scrollbar {
    pub track: (f32, f32),
    pub thumb: (f32, f32),
}

/// One block of text on the screen or the glass: where, how big, and
/// what furniture it wears.
#[derive(Debug, Clone, Copy)]
pub struct HudBlock {
    /// Where on the canopy (or the screen, when flat) the block's
    /// top-left sits, NDC.
    pub anchor_ndc: [f32; 2],
    /// One font pixel in canopy units (drives apparent size).
    pub px_canopy: f32,
    pub aspect: f32,
    /// The surface's height in pixels (the scanlines' frequency).
    pub height_px: f32,
    /// The hologram sway, canopy units.
    pub sway: [f32; 2],
    /// A flat card on the screen, not a glass element.
    pub flat: bool,
    /// The chosen row's (top, height) in font px, for the card's band.
    pub highlight: Option<(f32, f32)>,
    /// The panel's extent in font px; None hugs the text.
    pub extent: Option<(usize, usize)>,
    pub scrollbar: Option<Scrollbar>,
    /// Rules under the header and over the footer, font-px rows.
    pub rules: [Option<f32>; 2],
    /// No smoked-glass plate at all — just the glyph ink. A VR overlay
    /// meant to read as a mark on the glass itself (SPEC §5.3), not a
    /// panel floating in front of it: a backdrop at any real depth away
    /// from the eye covers far more of the view than the text on it,
    /// which reads as a close, obscuring plane.
    pub no_backdrop: bool,
}

impl HudBlock {
    /// A bare block: the readout on the glass.
    pub fn glass(anchor_ndc: [f32; 2], px_canopy: f32, aspect: f32, height_px: f32) -> Self {
        Self {
            anchor_ndc,
            px_canopy,
            aspect,
            height_px,
            sway: [0.0; 2],
            flat: false,
            highlight: None,
            extent: None,
            scrollbar: None,
            rules: [None; 2],
            no_backdrop: false,
        }
    }
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

    pub fn update(&self, queue: &wgpu::Queue, bitmap: &TextBitmap, block: &HudBlock) {
        let (w, h) = block.extent.unwrap_or_else(|| bitmap.used_extent());
        let (hl_y, hl_h) = block.highlight.unwrap_or((-1.0, 0.0));
        let bar = block.scrollbar.map_or([-1.0, 0.0, 0.0, 0.0], |b| {
            [b.track.0, b.track.1, b.thumb.0, b.thumb.1]
        });
        let u = HudUniforms {
            a: [
                block.anchor_ndc[0],
                block.anchor_ndc[1],
                block.px_canopy,
                block.aspect,
            ],
            extent: [w as f32, h as f32, block.height_px, hl_y],
            // Hologram cyan, matching the instrument cluster.
            color: [0.45, 0.92, 1.0, 0.96],
            // Smoked glass behind the text on the glass; a flat pause
            // panel is a darker card, read over anything; no_backdrop is
            // just the ink, no plate at all.
            backdrop: if block.no_backdrop {
                [0.0, 0.0, 0.0, 0.0]
            } else if block.flat {
                [0.008, 0.018, 0.03, 0.84]
            } else {
                [0.01, 0.03, 0.05, 0.42]
            },
            sway: [
                block.sway[0],
                block.sway[1],
                if block.flat { 1.0 } else { 0.0 },
                hl_h,
            ],
            bar,
            rules: [
                block.rules[0].unwrap_or(-1.0),
                block.rules[1].unwrap_or(-1.0),
                0.0,
                0.0,
            ],
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The uniform block is a wire format for hud.wgsl: the bitmap's rows
    /// must be exactly the shader's `array<vec4<u32>, 540>`.
    #[test]
    fn the_bitmap_rows_match_the_shaders_array() {
        assert_eq!(ROW_WORDS % 4, 0, "a row is whole vec4<u32>s");
        assert_eq!(ROWS * ROW_WORDS / 4, 540);
        assert_eq!(
            std::mem::size_of::<HudUniforms>(),
            7 * 16 + ROWS * ROW_WORDS * 4
        );
    }
}
