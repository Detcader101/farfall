//! The nebula bake (SPEC §6.5, P2 "bake, don't re-derive").
//!
//! Coloured gas across the sky, authored by `nebula.wgsl` into an equirect
//! texture with a mip chain. It is re-rendered only when a knob changes
//! (the menu, or the settings file at start); every frame the starfield
//! pass pays one fetch for it. Nothing here runs per frame.

/// Everything the bake reads. Two floats short of a `Params` in WGSL:
/// `[seed, scale, density, count]`, `[r,g,b of hue A, intensity]`,
/// `[r,g,b of hue B, softness]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NebulaParams {
    pub shape: [f32; 4],
    pub col_a: [f32; 4],
    pub col_b: [f32; 4],
}

/// The user's knobs, as the menu holds them. Hues are 0..360 degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NebulaKnobs {
    pub enabled: bool,
    pub seed: u32,
    pub scale: f32,
    pub density: f32,
    pub clouds: u32,
    pub intensity: f32,
    pub hue_a_deg: f32,
    pub hue_b_deg: f32,
    pub softness: f32,
}

impl NebulaParams {
    /// Pack the knobs for the shader; `enabled=false` bakes black (the
    /// starfield's fetch then adds nothing).
    pub fn new(k: NebulaKnobs) -> Self {
        let a = hue_rgb(k.hue_a_deg);
        let b = hue_rgb(k.hue_b_deg);
        let intensity = if k.enabled { k.intensity.max(0.0) } else { 0.0 };
        Self {
            shape: [
                (k.seed % 100_000) as f32,
                k.scale.clamp(0.5, 12.0),
                k.density.clamp(0.0, 1.0),
                k.clouds.clamp(1, 8) as f32,
            ],
            col_a: [a[0], a[1], a[2], intensity],
            col_b: [b[0], b[1], b[2], k.softness.clamp(0.05, 4.0)],
        }
    }
}

/// A saturated-but-not-neon linear colour for a hue on the wheel: the gas
/// palette (0 red, 120 green, 240 blue, 300 magenta). Kept a little off
/// pure so no cloud reads as a coloured lamp.
pub fn hue_rgb(deg: f32) -> [f32; 3] {
    let h = deg.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    // Desaturate a touch toward a pale of the same hue.
    let s = 0.8;
    [r * s + (1.0 - s), g * s + (1.0 - s), b * s + (1.0 - s)]
}

/// Equirect texels. Four thousand across is ~2.5 px per texel at 2880×1800
/// with a 100° field: the filaments and lace the bake draws survive to the
/// screen. Rgba16Float with mips, ~85 MB — spent once, never per frame.
const SIZE: (u32, u32) = (4096, 2048);

pub struct NebulaBake {
    bake_pipeline: wgpu::RenderPipeline,
    mip_pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    texture: wgpu::Texture,
    /// The whole mip chain, for the starfield to bind.
    pub view: wgpu::TextureView,
    last: Option<NebulaParams>,
}

impl NebulaBake {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nebula"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::NEBULA).into(),
            ),
        });
        let format = wgpu::TextureFormat::Rgba16Float;
        let (w, h) = SIZE;
        let mips = 32 - w.max(h).leading_zeros();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("baked nebula"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nebula bake sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nebula params"),
            size: std::mem::size_of::<NebulaParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nebula bgl"),
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nebula layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let bake_pipeline = pipeline("fs_bake");
        let mip_pipeline = pipeline("fs_downsample");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            bake_pipeline,
            mip_pipeline,
            bgl,
            uniforms,
            sampler,
            texture,
            view,
            last: None,
        }
    }

    /// Re-render the nebula if `params` differ from the last bake. Returns
    /// whether a bake was submitted. The texture view stays the same, so
    /// no bind group downstream needs rebuilding.
    pub fn bake(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        params: NebulaParams,
    ) -> bool {
        if self.last == Some(params) {
            return false;
        }
        self.last = Some(params);
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&params));

        let mip_view = |level: u32| {
            self.texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let bind_for = |src: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nebula bind"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nebula bake"),
        });
        let render_to = |encoder: &mut wgpu::CommandEncoder,
                         view: &wgpu::TextureView,
                         pipeline: &wgpu::RenderPipeline,
                         bind: &wgpu::BindGroup| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nebula bake pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };

        // Level 0 needs a texture bound even though fs_bake never reads it;
        // the previous top mip is as good as any.
        let mip_count = self.texture.mip_level_count();
        let top_src = mip_view(mip_count - 1);
        render_to(
            &mut encoder,
            &mip_view(0),
            &self.bake_pipeline,
            &bind_for(&top_src),
        );
        for level in 1..mip_count {
            let src = mip_view(level - 1);
            render_to(
                &mut encoder,
                &mip_view(level),
                &self.mip_pipeline,
                &bind_for(&src),
            );
        }
        queue.submit([encoder.finish()]);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_wheel_hits_the_primaries() {
        let r = hue_rgb(0.0);
        let g = hue_rgb(120.0);
        let b = hue_rgb(240.0);
        assert!(r[0] > r[1] && r[0] > r[2]);
        assert!(g[1] > g[0] && g[1] > g[2]);
        assert!(b[2] > b[0] && b[2] > b[1]);
        // Wraps.
        assert_eq!(hue_rgb(360.0), hue_rgb(0.0));
        assert_eq!(hue_rgb(-60.0), hue_rgb(300.0));
    }

    #[test]
    fn disabled_bakes_black_and_knobs_clamp() {
        let k = NebulaKnobs {
            enabled: false,
            seed: 7,
            scale: 3.0,
            density: 0.5,
            clouds: 4,
            intensity: 2.0,
            hue_a_deg: 200.0,
            hue_b_deg: 320.0,
            softness: 1.0,
        };
        let off = NebulaParams::new(k);
        assert_eq!(off.col_a[3], 0.0);
        let p = NebulaParams::new(NebulaKnobs {
            enabled: true,
            scale: 99.0,
            density: 5.0,
            clouds: 40,
            intensity: 1.5,
            softness: 0.0,
            ..k
        });
        assert_eq!(p.shape[1], 12.0);
        assert_eq!(p.shape[2], 1.0);
        assert_eq!(p.shape[3], 8.0);
        assert_eq!(p.col_a[3], 1.5);
        assert_eq!(p.col_b[3], 0.05);
        assert_eq!(std::mem::size_of::<NebulaParams>(), 48);
    }
}
