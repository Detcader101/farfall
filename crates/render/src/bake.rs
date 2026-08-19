//! One-time field baking (SPEC §6.5, P2's "bake, don't re-derive").
//!
//! Runs once at startup: renders the static noise fields into equirect
//! textures with full mip chains, so the per-frame passes sample instead of
//! recomputing. The world is still authored entirely by shaders — these
//! textures are a cache of shader output, not assets.

/// Baked world textures, ready to bind.
pub struct BakedMaps {
    pub surface_view: wgpu::TextureView,
    pub cloud_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    // Kept alive for the views' sake.
    _surface: wgpu::Texture,
    _cloud: wgpu::Texture,
}

const SURFACE_SIZE: (u32, u32) = (2048, 1024);
const CLOUD_SIZE: (u32, u32) = (1024, 512);

impl BakedMaps {
    /// Render every field and its mip chain. One submit; blocks nothing —
    /// the queue orders it before the first frame.
    pub fn bake(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bake"),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(crate::shaders::BAKE).into()),
        });

        let format = wgpu::TextureFormat::Rgba16Float;
        let make_tex = |label: &str, (w, h): (u32, u32)| {
            let mips = 32 - w.max(h).leading_zeros();
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let surface = make_tex("baked surface fields", SURFACE_SIZE);
        let cloud = make_tex("baked cloud field", CLOUD_SIZE);

        // Equirect: longitude wraps, latitude clamps at the poles.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("baked maps sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // Generation pipelines take no bindings at all: the fields are pure
        // functions of the texel's direction.
        let empty_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bake layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let gen_pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&empty_layout),
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
        let surface_pipeline = gen_pipeline("fs_surface");
        let cloud_pipeline = gen_pipeline("fs_cloud");

        // Downsample pipeline for the mip chain.
        let mip_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mip bgl"),
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
            ],
        });
        let mip_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mip layout"),
            bind_group_layouts: &[Some(&mip_bgl)],
            immediate_size: 0,
        });
        let mip_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mip downsample"),
            layout: Some(&mip_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_downsample"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bake"),
        });

        let mip_view = |tex: &wgpu::Texture, level: u32| {
            tex.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let render_to = |encoder: &mut wgpu::CommandEncoder,
                         view: &wgpu::TextureView,
                         pipeline: &wgpu::RenderPipeline,
                         bind: Option<&wgpu::BindGroup>| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bake pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            if let Some(bind) = bind {
                pass.set_bind_group(0, bind, &[]);
            }
            pass.draw(0..3, 0..1);
        };

        for (tex, pipeline) in [(&surface, &surface_pipeline), (&cloud, &cloud_pipeline)] {
            render_to(&mut encoder, &mip_view(tex, 0), pipeline, None);
            for level in 1..tex.mip_level_count() {
                let src = mip_view(tex, level - 1);
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mip bind"),
                    layout: &mip_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&src),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
                render_to(
                    &mut encoder,
                    &mip_view(tex, level),
                    &mip_pipeline,
                    Some(&bind),
                );
            }
        }
        queue.submit([encoder.finish()]);

        Self {
            surface_view: surface.create_view(&wgpu::TextureViewDescriptor::default()),
            cloud_view: cloud.create_view(&wgpu::TextureViewDescriptor::default()),
            sampler,
            _surface: surface,
            _cloud: cloud,
        }
    }
}
