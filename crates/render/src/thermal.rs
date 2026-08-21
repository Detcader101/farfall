//! Entry heating: the hull thermal field and the plasma sheath it lights.
//!
//! The simulation lives on the GPU (`shaders/thermal.wgsl`): a small
//! octahedral texture over every hull direction, ping-ponged each frame, into
//! which the app feeds only raw physics — ship-frame velocity and air density
//! at the hull. The plasma pass (`shaders/plasma.wgsl`) reads that field along
//! each view ray and draws the glow additively into the scene. No temperature
//! ever crosses back to the CPU.

use crate::bake::BakedMaps;
use glam::Vec3;

/// Resolution of the hull field. 64² is 4096 patches of skin — plenty for a
/// glow that is, by nature, soft.
const FIELD_SIZE: u32 = 64;

/// The raw physics the thermal sim needs, per frame. Everything here is
/// something the app already knows; the sim derives the rest.
#[derive(Debug, Clone, Copy)]
pub struct ThermalInputs {
    /// Ship velocity in the ship's own frame: x right, y up, z forward (nose).
    pub vel_ship_mps: Vec3,
    /// Air density at the hull, kg/m³.
    pub rho: f32,
    /// Sea-level density of this atmosphere, kg/m³ (normalises the model).
    pub rho0: f32,
    /// Wall-clock frame time, seconds.
    pub dt: f32,
    /// Wipe the field (first frame, respawn).
    pub reset: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ThermalUniforms {
    vel: [f32; 4],
    air: [f32; 4],
}

impl ThermalUniforms {
    fn new(inputs: &ThermalInputs) -> Self {
        let v = inputs.vel_ship_mps;
        Self {
            vel: [v.x, v.y, v.z, v.length()],
            air: [
                inputs.rho.max(0.0),
                inputs.rho0.max(1e-9),
                inputs.dt.max(0.0),
                if inputs.reset { 1.0 } else { 0.0 },
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PlasmaUniforms {
    /// x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: [f32; 4],
    vel: [f32; 4],
    /// Camera space → ship space (the pilot's head), as a quaternion, in
    /// the shader's ship frame (x right, y up, z forward).
    look: [f32; 4],
}

impl PlasmaUniforms {
    /// `look`: the head's rotation in the body frame (nose −Z). Identity
    /// when the pilot looks straight ahead.
    pub fn new(cam: &crate::CameraFrame, vel_ship_mps: Vec3, look: glam::Quat) -> Self {
        let v = vel_ship_mps;
        // The shader's ship frame has z forward where the body has −Z: a
        // REFLECTION (x, y, −z), not a rotation — it changes handedness.
        // Under a reflection M a rotation R becomes M R M⁻¹: the axis is
        // reflected and the angle reversed, which for the quaternion means
        // (−x, −y, z, w). (The first cut flipped x and z as if M were a
        // half-turn about Y; the test below caught it.)
        let q = look.normalize();
        let flipped = glam::Quat::from_xyzw(-q.x, -q.y, q.z, q.w);
        Self {
            params: [
                (cam.fov_y * 0.5).tan(),
                cam.aspect,
                cam.time_s,
                cam.exposure,
            ],
            vel: [v.x, v.y, v.z, v.length()],
            look: [flipped.x, flipped.y, flipped.z, flipped.w],
        }
    }
}

/// Map the ship's world-space velocity into its own frame, in the shader's
/// convention (x right, y up, z forward). The hull does not care which way
/// the planet is; it cares which way the wind comes from.
pub fn ship_frame_velocity(orient: glam::Quat, vel_world: Vec3) -> Vec3 {
    let body = orient.conjugate() * vel_world;
    // The camera looks down -Z, so the nose is -Z in the body frame.
    Vec3::new(body.x, body.y, -body.z)
}

/// The hull heat field: two textures, one read while the other is written.
pub struct ThermalPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    views: [wgpu::TextureView; 2],
    bind_groups: [wgpu::BindGroup; 2],
    pub sampler: wgpu::Sampler,
    _textures: [wgpu::Texture; 2],
    /// Index of the texture holding the CURRENT field (last written).
    current: usize,
    primed: bool,
}

const FIELD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl ThermalPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("thermal"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::THERMAL).into(),
            ),
        });
        let make = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: FIELD_SIZE,
                    height: FIELD_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FIELD_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let textures = [make("hull heat A"), make("hull heat B")];
        let views = [
            textures[0].create_view(&Default::default()),
            textures[1].create_view(&Default::default()),
        ];
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hull heat sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("thermal uniforms"),
            size: std::mem::size_of::<ThermalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&field_layout("thermal bgl"));
        // Bind group i READS texture i (and the pass then writes the other).
        let bind_groups = [
            field_bind_group(device, "thermal bg A", &bgl, &uniforms, &views[0], &sampler),
            field_bind_group(device, "thermal bg B", &bgl, &uniforms, &views[1], &sampler),
        ];
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("thermal layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("thermal"),
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
                    format: FIELD_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniforms,
            views,
            bind_groups,
            sampler,
            _textures: textures,
            current: 0,
            primed: false,
        }
    }

    /// Both field views, indexed like [`ThermalPass::current`].
    pub fn views(&self) -> &[wgpu::TextureView; 2] {
        &self.views
    }

    /// Which view holds the field the plasma pass should read this frame.
    pub fn current(&self) -> usize {
        self.current
    }

    /// Advance the field by one frame: read `current`, write the other, swap.
    /// The first step always resets, so an uninitialised texture is never
    /// integrated from.
    pub fn step(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &ThermalInputs,
    ) {
        let mut inputs = *inputs;
        if !self.primed {
            inputs.reset = true;
            self.primed = true;
        }
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&ThermalUniforms::new(&inputs)),
        );
        let read = self.current;
        let write = 1 - read;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("thermal"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.views[write],
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_groups[read], &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        self.current = write;
    }
}

fn field_layout(label: &'static str) -> wgpu::BindGroupLayoutDescriptor<'static> {
    const ENTRIES: &[wgpu::BindGroupLayoutEntry] = &[
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
    ];
    wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: ENTRIES,
    }
}

fn field_bind_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// The sheath, drawn additively into the scene from the current heat field.
pub struct PlasmaPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    /// One per field texture, so the swap costs nothing.
    bind_groups: [wgpu::BindGroup; 2],
}

impl PlasmaPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        thermal: &ThermalPass,
        maps: &BakedMaps,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("plasma"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::PLASMA).into(),
            ),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plasma uniforms"),
            size: std::mem::size_of::<PlasmaUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The field layout, plus the baked noise tile the streaks scroll.
        let field = field_layout("plasma bgl");
        let mut entries = field.entries.to_vec();
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("plasma bgl"),
            entries: &entries,
        });
        let views = thermal.views();
        let make_bg = |label: &str, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&thermal.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&maps.noise_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&maps.tile_sampler),
                    },
                ],
            })
        };
        let bind_groups = [
            make_bg("plasma bg A", &views[0]),
            make_bg("plasma bg B", &views[1]),
        ];
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("plasma layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("plasma"),
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
                    // Luminous gas over the world: light adds.
                    blend: Some(wgpu::BlendState {
                        color: additive,
                        alpha: additive,
                    }),
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
            bind_groups,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &PlasmaUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, thermal: &ThermalPass) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_groups[thermal.current()], &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    /// Nose-first flight must read as forward (+z) in the shader's frame,
    /// whatever the ship's attitude in the world.
    #[test]
    fn prograde_is_forward_in_ship_frame() {
        let orient = Quat::from_rotation_y(1.1) * Quat::from_rotation_x(-0.4);
        let nose_world = orient * Vec3::NEG_Z;
        let v = ship_frame_velocity(orient, nose_world * 500.0);
        assert!((v - Vec3::new(0.0, 0.0, 500.0)).length() < 1e-3, "{v}");
    }

    /// Looking right (a head yaw that swings the nose toward body +X)
    /// must map the screen centre to a hull direction to the right of
    /// forward in the shader's frame.
    #[test]
    fn head_yaw_turns_the_centre_ray_the_right_way() {
        let cam = crate::CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.6,
            time_s: 0.0,
            exposure: 1.6,
        };
        // Looking right: nose −Z swings toward +X under a negative Y rotation.
        let look = Quat::from_rotation_y(-0.5);
        assert!((look * Vec3::NEG_Z).x > 0.0);
        let u = PlasmaUniforms::new(&cam, Vec3::Z, look);
        let q = Quat::from_xyzw(u.look[0], u.look[1], u.look[2], u.look[3]);
        // Shader frame: forward is +Z. The centre ray (0,0,1) should land
        // to the right (+x) of forward.
        let r = q * Vec3::Z;
        assert!(r.x > 0.0 && r.z > 0.0, "{r}");
    }

    #[test]
    fn reset_and_clamps_reach_the_uniform() {
        let u = ThermalUniforms::new(&ThermalInputs {
            vel_ship_mps: Vec3::new(3.0, 4.0, 0.0),
            rho: -1.0,
            rho0: 0.0,
            dt: -0.1,
            reset: true,
        });
        assert_eq!(u.vel[3], 5.0);
        assert_eq!(u.air[0], 0.0);
        assert!(u.air[1] > 0.0);
        assert_eq!(u.air[2], 0.0);
        assert_eq!(u.air[3], 1.0);
    }
}
