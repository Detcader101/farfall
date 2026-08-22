//! The wireframe cabin (`shaders/cockpit.wgsl`): a canopy dome, a sill, a
//! dash and a bulkhead drawn around the pilot's head in the ship's frame.

use crate::instrument::InstrumentPass;
use crate::CameraFrame;
use glam::{Quat, Vec3};

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CabinUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    misc: [f32; 4],
    sun: [f32; 4],
    pads: [[f32; 4]; 4],
}

pub const UNIFORM_BYTES: u64 = std::mem::size_of::<CabinUniforms>() as u64;

/// The cabin as the pilot has it set.
#[derive(Debug, Clone, Copy)]
pub struct CabinLook {
    /// Line glow 0..2.
    pub glow: f32,
    /// Metal brightness 0..1.
    pub metal: f32,
    /// Drawn at all.
    pub on: bool,
}

impl CabinUniforms {
    /// `head`: the pilot's head rotation in the ship's frame (freelook);
    /// the cabin is fixed to the ship, so the rays are turned by it.
    /// `sun_ship`: the Sun's direction in the ship's frame. `sockets`: the
    /// directions (ship frame) of the holograms that get a socket on the
    /// dash — the shown dials, up to four.
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        sun_ship: Vec3,
        look: CabinLook,
        sockets: &[Vec3],
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut pads = [[0.0; 4]; 4];
        for (slot, d) in pads.iter_mut().zip(sockets.iter()) {
            let d = d.normalize_or_zero();
            *slot = [d.x, d.y, d.z, if d == Vec3::ZERO { 0.0 } else { 1.0 }];
        }
        Self {
            right: v4(head * Vec3::X, look.glow.clamp(0.0, 3.0)),
            up: v4(head * Vec3::Y, look.metal.clamp(0.0, 1.0)),
            fwd: v4(head * Vec3::NEG_Z, (cam.fov_y * 0.5).tan()),
            misc: [
                cam.aspect,
                cam.time_s,
                if look.on { 1.0 } else { 0.0 },
                sockets.len().min(4) as f32,
            ],
            sun: v4(sun_ship.normalize_or_zero(), cam.exposure),
            pads,
        }
    }
}

/// A glass anchor (canopy NDC with the head centred, for a camera of this
/// tan(fov/2) and aspect) as a direction in the ship's frame: where that
/// hologram floats.
pub fn anchor_direction(anchor: [f32; 2], tan_half_fov: f32, aspect: f32) -> Vec3 {
    Vec3::new(
        anchor[0] * aspect * tan_half_fov,
        anchor[1] * tan_half_fov,
        -1.0,
    )
    .normalize()
}

/// The cabin's own render target and the composite that lays it over the
/// scene: the SDF march runs at `fraction` of the scene's size, the
/// composite at full size.
pub struct CabinPass {
    inner: InstrumentPass,
    composite: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    texture: Option<wgpu::Texture>,
    view: Option<wgpu::TextureView>,
    size: (u32, u32),
    fraction: f32,
    format: wgpu::TextureFormat,
}

impl CabinPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        fraction: f32,
    ) -> Self {
        // The cabin texture is single-sampled at its own size; the march is
        // analytic and needs no MSAA, the upscale smooths it anyway.
        let inner = InstrumentPass::new_pane_sized(
            device,
            target_format,
            1,
            "cockpit",
            crate::shaders::COCKPIT,
            UNIFORM_BYTES,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cabin_blit"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::CABIN_BLIT).into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cabin blit bgl"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cabin sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cabin blit layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cabin_blit"),
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
            inner,
            composite,
            layout,
            sampler,
            bind_group: None,
            texture: None,
            view: None,
            size: (0, 0),
            fraction: fraction.clamp(0.25, 1.0),
            format: target_format,
        }
    }

    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn set_fraction(&mut self, fraction: f32) {
        self.fraction = fraction.clamp(0.25, 1.0);
        self.size = (0, 0);
    }

    /// Size the cabin texture for a scene of this size.
    pub fn ensure(&mut self, device: &wgpu::Device, scene_w: u32, scene_h: u32) {
        let w = ((scene_w as f32 * self.fraction).round() as u32).max(1);
        let h = ((scene_h as f32 * self.fraction).round() as u32).max(1);
        if self.size == (w, h) && self.view.is_some() {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cabin colour"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cabin blit bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
        self.texture = Some(texture);
        self.view = Some(view);
        self.size = (w, h);
    }

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &CabinUniforms) {
        self.inner.update(queue, uniforms);
    }

    /// Render the cabin into its own texture (a pass of its own, before the
    /// scene's).
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(view) = &self.view else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cabin"),
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
        self.inner.draw(&mut pass);
    }

    /// Lay the cabin over the scene.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_turns_the_rays_not_the_cabin() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.0,
            aspect: 1.5,
            time_s: 2.0,
            exposure: 1.0,
        };
        let look = CabinLook {
            glow: 1.0,
            metal: 0.8,
            on: true,
        };
        let sockets = [anchor_direction([0.7, -0.6], 0.55, 1.5), Vec3::ZERO];
        let still = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &sockets);
        assert_eq!(&still.fwd[..3], &[0.0, 0.0, -1.0]);
        assert_eq!(still.misc[3], 2.0);
        assert_eq!(still.pads[0][3], 1.0, "a placed dial gets a socket");
        assert_eq!(still.pads[1][3], 0.0, "an empty slot does not");
        assert!(still.pads[0][0] > 0.0 && still.pads[0][1] < 0.0 && still.pads[0][2] < 0.0);
        // Looking right: the forward ray swings toward +X in the ship's
        // frame (the nose is -Z; rotating -Z about +Y by a negative angle
        // swings it toward +X).
        let loud = CabinLook {
            glow: 5.0,
            metal: -1.0,
            on: true,
        };
        let turned = CabinUniforms::new(&cam, Quat::from_rotation_y(-0.5), Vec3::Y, loud, &[]);
        assert!(turned.fwd[0] > 0.4, "{:?}", turned.fwd);
        assert_eq!(turned.right[3], 3.0);
        assert_eq!(turned.up[3], 0.0);
        assert_eq!(still.misc[2], 1.0);
        assert_eq!(UNIFORM_BYTES, 9 * 16);
    }
}
