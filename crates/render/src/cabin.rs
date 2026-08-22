//! The wireframe cabin (`shaders/cockpit.wgsl`): a canopy dome, a sill, a
//! dash and a bulkhead drawn around the pilot's head in the ship's frame.

use crate::instrument::InstrumentPass;
use crate::CameraFrame;
use glam::{Quat, Vec3};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CabinUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    misc: [f32; 4],
    sun: [f32; 4],
    pads: [[f32; 4]; 4],
}

/// What the composite needs each frame: the head's basis for the thruster
/// light's rays, and the throttle and RCS demands.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlitUniforms {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    misc: [f32; 4],
    thrust: [f32; 4],
}

pub const UNIFORM_BYTES: u64 = std::mem::size_of::<CabinUniforms>() as u64;

/// A dial's socket on the dash: where, in what style, how big.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Socket {
    pub dir: Vec3,
    /// 0 TRON, 1 JET, 2 DIAL.
    pub style: u32,
    pub size: f32,
}

/// The cabin as the pilot has it set.
#[derive(Debug, Clone, Copy)]
pub struct CabinLook {
    /// Line glow 0..2.
    pub glow: f32,
    /// Metal brightness 0..1.
    pub metal: f32,
    /// Drawn at all.
    pub on: bool,
    /// Main thrust 0..1 (the plumes) and the pitch / yaw / roll demands
    /// -1..1 (the RCS puffs).
    pub thrust: [f32; 4],
    /// Gauge style: 0 TRON (sockets and beams), 1 JET (bowls and bezels),
    /// 2 DIAL (flush wells, the face in the dash).
    pub style: u32,
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
        sockets: &[Socket],
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut pads = [[0.0; 4]; 4];
        for (slot, sk) in pads.iter_mut().zip(sockets.iter()) {
            let d = sk.dir.normalize_or_zero();
            // w packs "in use", the style and the size as exact integers:
            // (style + 1) + 10 × round(size × 100).
            let w = if d == Vec3::ZERO {
                0.0
            } else {
                (sk.style.min(2) + 1) as f32 + 10.0 * (sk.size.clamp(0.25, 4.0) * 100.0).round()
            };
            *slot = [d.x, d.y, d.z, w];
        }
        Self {
            right: v4(head * Vec3::X, look.glow.clamp(0.0, 3.0)),
            up: v4(head * Vec3::Y, look.metal.clamp(0.0, 1.0)),
            fwd: v4(head * Vec3::NEG_Z, (cam.fov_y * 0.5).tan()),
            // No clock in here: the cabin is only re-marched when something
            // about it changes, and time would be change every frame.
            misc: [
                cam.aspect,
                look.style.min(2) as f32,
                if look.on { 1.0 } else { 0.0 },
                sockets.len().min(4) as f32,
            ],
            // The Sun's direction quantised to about a degree: the cabin is
            // re-marched only when its inputs change, and a ship turning
            // slowly in orbit must not count as change every frame.
            sun: v4(quantise(sun_ship.normalize_or_zero(), 0.02), cam.exposure),
            pads,
        }
    }

    /// The composite's share: the rays and the throttle.
    pub fn blit(&self, look: CabinLook) -> BlitUniforms {
        BlitUniforms {
            right: self.right,
            up: self.up,
            fwd: self.fwd,
            misc: [self.misc[0], self.misc[2], 0.0, 0.0],
            thrust: [
                look.thrust[0].clamp(0.0, 1.0),
                look.thrust[1].clamp(-1.0, 1.0),
                look.thrust[2].clamp(-1.0, 1.0),
                look.thrust[3].clamp(-1.0, 1.0),
            ],
        }
    }

    /// Did the head or the light move — a re-march at the moving size —
    /// as against a sharp re-render of the same view?
    pub fn view_moved(&self, other: &CabinUniforms) -> bool {
        self.right != other.right || self.up != other.up || self.fwd != other.fwd
    }
}

fn quantise(v: Vec3, step: f32) -> Vec3 {
    (v / step).round() * step
}

/// The dash's plane in the ship's frame — the same numbers as DASH_C /
/// DASH_N in cockpit.wgsl and DIAL_DASH_* in common.wgsl.
pub const DASH_C: Vec3 = Vec3::new(0.0, -0.50, -1.05);
pub const DASH_N: Vec3 = Vec3::new(0.0, 0.9563, 0.2924);
/// Metres of dash per drawing unit of a dial: a dial's drawing radius is
/// 0.155 units; at this scale that is a 20 cm instrument.
pub const DIAL_SCALE_M: f32 = 1.3;
/// Where the holograms float (the cockpit's HOLO_M).
pub const HOLO_M: f32 = 1.05;

/// Where a hologram's direction meets the dash (the socket, the well), or
/// a point just under the hologram if it misses the dash — the mirror of
/// socket_centre() in cockpit.wgsl.
pub fn socket_centre(dir: Vec3) -> Vec3 {
    let denom = dir.dot(DASH_N);
    if denom < -1e-4 {
        let t = DASH_C.dot(DASH_N) / denom;
        let p = dir * t;
        if t > 0.3 && t < 2.2 && p.x.abs() < 1.0 {
            return p;
        }
    }
    dir * HOLO_M - Vec3::new(0.0, 0.16, 0.0)
}

/// Does this direction's instrument sit in the dash at all (a DIAL needs a
/// dash to be set into; one that misses it stays on the glass)?
pub fn on_dash(dir: Vec3) -> bool {
    let denom = dir.dot(DASH_N);
    if denom >= -1e-4 {
        return false;
    }
    let t = DASH_C.dot(DASH_N) / denom;
    let p = dir * t;
    t > 0.3 && t < 2.2 && p.x.abs() < 1.0
}

/// A DIAL's placement in the dash, for the instrument shaders: the head's
/// basis in the ship's frame and the dial's centre, a hair under the dash
/// surface so the face sits in its well.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Placement {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    centre: [f32; 4],
}

impl Placement {
    /// On the glass: no placement at all.
    pub const GLASS: Placement = Placement {
        right: [1.0, 0.0, 0.0, 1.0],
        up: [0.0; 4],
        fwd: [0.0; 4],
        centre: [0.0; 4],
    };

    /// On the glass at this size (1 = stock).
    pub fn glass_sized(size: f32) -> Placement {
        let mut p = Placement::GLASS;
        p.right[3] = size.clamp(0.25, 4.0);
        p
    }

    /// In the dash under the hologram's direction `dir` (ship frame), for
    /// a head turned by `head` and a camera of this tan(fov/2).
    pub fn in_dash(head: Quat, tan_half_fov: f32, dir: Vec3, size: f32) -> Option<Placement> {
        if !on_dash(dir) {
            return None;
        }
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let centre = socket_centre(dir) - DASH_N * 0.012;
        Some(Placement {
            right: v4(head * Vec3::X, 1.0),
            up: v4(head * Vec3::Y, 0.0),
            fwd: v4(head * Vec3::NEG_Z, tan_half_fov),
            centre: v4(centre, DIAL_SCALE_M * size.clamp(0.25, 4.0)),
        })
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

/// The cabin's render targets and the composite that lays the cabin over
/// the scene. The march is the dearest thing per pixel in the frame and
/// the cabin is a function of the head's direction alone, so it is marched
/// only when that (or the light, or a socket) changes: while the head is
/// turning, at a reduced size every frame; once it rests, re-marched sharp
/// over four frames in strips and swapped in; and then not at all. The
/// thruster light, which changes with the throttle, is drawn by the
/// composite at full size every frame.
pub struct CabinPass {
    inner: InstrumentPass,
    composite: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    blit_uniforms: wgpu::Buffer,
    moving: Target,
    still: Target,
    showing_still: bool,
    /// Which strip of the sharp render is next (cycles), and how many are
    /// still owed before it is current.
    strip_cursor: u32,
    strips_owed: u32,
    last: Option<CabinUniforms>,
    fraction: f32,
    format: wgpu::TextureFormat,
    scene_size: (u32, u32),
}

/// A cabin texture with its composite bind group.
struct Target {
    view: Option<wgpu::TextureView>,
    bind_group: Option<wgpu::BindGroup>,
    size: (u32, u32),
}

const STRIPS: u32 = 4;
/// While the head turns, the cabin is marched at this much of its still size.
const MOVING_SCALE: f32 = 0.6;

impl CabinPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        fraction: f32,
    ) -> Self {
        let inner = InstrumentPass::new_layer_sized(
            device,
            target_format,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
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
        let blit_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cabin blit uniforms"),
            size: std::mem::size_of::<BlitUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
        let empty = || Target {
            view: None,
            bind_group: None,
            size: (0, 0),
        };
        Self {
            inner,
            composite,
            layout,
            sampler,
            blit_uniforms,
            moving: empty(),
            still: empty(),
            showing_still: false,
            strip_cursor: 0,
            strips_owed: 0,
            last: None,
            fraction: fraction.clamp(0.25, 1.0),
            format: target_format,
            scene_size: (0, 0),
        }
    }

    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn set_fraction(&mut self, fraction: f32) {
        self.fraction = fraction.clamp(0.25, 1.0);
        self.scene_size = (0, 0);
    }

    /// Size both cabin textures for a scene of this size.
    pub fn ensure(&mut self, device: &wgpu::Device, scene_w: u32, scene_h: u32) {
        if self.scene_size == (scene_w, scene_h) && self.still.view.is_some() {
            return;
        }
        self.scene_size = (scene_w, scene_h);
        let size = |f: f32| {
            (
                ((scene_w as f32 * f).round() as u32).max(1),
                ((scene_h as f32 * f).round() as u32).max(1),
            )
        };
        let still = size(self.fraction);
        let moving = size(self.fraction * MOVING_SCALE);
        self.still = self.make_target(device, still);
        self.moving = self.make_target(device, moving);
        // Everything is stale: the next frame re-marches.
        self.last = None;
        self.showing_still = false;
        self.strip_cursor = 0;
        self.strips_owed = 0;
    }

    fn make_target(&self, device: &wgpu::Device, size: (u32, u32)) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cabin colour"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.blit_uniforms.as_entire_binding(),
                },
            ],
        });
        Target {
            view: Some(view),
            bind_group: Some(bind_group),
            size,
        }
    }

    /// Bring the cabin up to date for this frame's inputs: nothing if they
    /// are unchanged and the sharp render is done; a strip of the sharp
    /// render if the view is resting; the whole cabin at the moving size if
    /// it has moved. Returns what it did, for the record.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &CabinUniforms,
        blit: &BlitUniforms,
    ) -> CabinWork {
        queue.write_buffer(&self.blit_uniforms, 0, bytemuck::bytes_of(blit));
        let changed = self.last.as_ref() != Some(uniforms);
        if changed {
            let moved = self.last.as_ref().is_none_or(|l| l.view_moved(uniforms));
            self.last = Some(*uniforms);
            self.inner.update(queue, uniforms);
            // Every strip is owed again; the cursor keeps cycling, so a
            // change every frame (the light, in a roll) still refreshes
            // the whole texture round-robin rather than one strip forever.
            self.strips_owed = STRIPS;
            if moved || !self.showing_still {
                // Re-march small now; sharpen over the coming frames.
                self.render_into(encoder, true, None);
                self.showing_still = false;
                self.strip_cursor = 0;
                return CabinWork::Moving;
            }
            // The view is the same (a socket, the light, a setting): keep
            // showing the sharp one while it is redone in place.
        }
        if self.strips_owed > 0 {
            let strip = self.strip_cursor % STRIPS;
            self.render_into(encoder, false, Some(strip));
            self.strip_cursor = (self.strip_cursor + 1) % STRIPS;
            self.strips_owed -= 1;
            if self.strips_owed == 0 {
                self.showing_still = true;
            }
            return CabinWork::Strip(strip);
        }
        CabinWork::Nothing
    }

    fn render_into(&self, encoder: &mut wgpu::CommandEncoder, moving: bool, strip: Option<u32>) {
        let target = if moving { &self.moving } else { &self.still };
        let Some(view) = &target.view else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cabin"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The layer writes every pixel it covers, transparent
                    // included, so a strip overwrites its rows in place and
                    // the rest of the texture stays as it was — which is
                    // what lets the sharp cabin be redrawn while shown.
                    load: if strip.is_some() {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(s) = strip {
            let (w, h) = target.size;
            let y0 = h * s / STRIPS;
            let y1 = h * (s + 1) / STRIPS;
            pass.set_scissor_rect(0, y0, w, (y1 - y0).max(1));
        }
        self.inner.draw(&mut pass);
    }

    /// Lay the cabin over the scene: the sharp one if it is complete, else
    /// the moving one.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let target = if self.showing_still {
            &self.still
        } else {
            &self.moving
        };
        let Some(bind_group) = &target.bind_group else {
            return;
        };
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// What [`CabinPass::update`] did this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CabinWork {
    Nothing,
    Strip(u32),
    Moving,
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
            thrust: [0.5, 2.0, -0.5, 0.0],
            style: 0,
        };
        let sockets = [
            Socket {
                dir: anchor_direction([0.7, -0.6], 0.55, 1.5),
                style: 1,
                size: 1.0,
            },
            Socket {
                dir: Vec3::ZERO,
                style: 0,
                size: 1.0,
            },
        ];
        let still = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &sockets);
        assert_eq!(&still.fwd[..3], &[0.0, 0.0, -1.0]);
        assert_eq!(still.misc[3], 2.0);
        assert_eq!(
            still.pads[0][3], 1002.0,
            "a placed dial gets a socket: (JET + 1) + 10 × 100"
        );
        assert_eq!(still.pads[1][3], 0.0, "an empty slot does not");
        assert!(still.pads[0][0] > 0.0 && still.pads[0][1] < 0.0 && still.pads[0][2] < 0.0);
        // Looking right: the forward ray swings toward +X in the ship's
        // frame (the nose is -Z; rotating -Z about +Y by a negative angle
        // swings it toward +X).
        let loud = CabinLook {
            glow: 5.0,
            metal: -1.0,
            on: true,
            thrust: [9.0, 0.0, 0.0, 0.0],
            style: 2,
        };
        let turned = CabinUniforms::new(&cam, Quat::from_rotation_y(-0.5), Vec3::Y, loud, &[]);
        assert!(turned.fwd[0] > 0.4, "{:?}", turned.fwd);
        assert_eq!(turned.right[3], 3.0);
        assert_eq!(turned.up[3], 0.0);
        assert_eq!(still.misc[2], 1.0);
        assert_eq!(still.blit(look).thrust, [0.5, 1.0, -0.5, 0.0]);
        assert_eq!(turned.blit(loud).thrust[0], 1.0);
        assert_eq!(turned.misc[1], 2.0);
        // A dial straight down-ahead sits in the dash; one up and away does not.
        let down = anchor_direction([0.0, -0.6], 0.55, 1.5);
        assert!(on_dash(down));
        let place = Placement::in_dash(Quat::IDENTITY, 0.55, down, 1.0).unwrap();
        assert!(
            place.centre[1] < -0.4 && place.centre[2] < -0.6,
            "{:?}",
            place.centre
        );
        assert_eq!(place.centre[3], DIAL_SCALE_M);
        assert_eq!(
            Placement::in_dash(Quat::IDENTITY, 0.55, down, 2.0)
                .unwrap()
                .centre[3],
            DIAL_SCALE_M * 2.0
        );
        assert!(Placement::in_dash(Quat::IDENTITY, 0.55, Vec3::new(0.0, 0.8, -0.6), 1.0).is_none());
        assert_eq!(Placement::glass_sized(1.5).right[3], 1.5);
        assert_eq!(UNIFORM_BYTES, 9 * 16);
        // Unchanged inputs compare equal (no clock inside), a turned head
        // is a moved view, a changed socket is not.
        let again = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &sockets);
        assert_eq!(still, again);
        assert!(still.view_moved(&turned));
        let other_sockets = [Socket {
            dir: anchor_direction([-0.7, -0.6], 0.55, 1.5),
            style: 1,
            size: 1.0,
        }];
        let resocketed = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &other_sockets);
        assert!(!still.view_moved(&resocketed));
        assert_ne!(still, resocketed);
        // A slow drift of the Sun below a degree is no change at all.
        let sun_drift = CabinUniforms::new(
            &cam,
            Quat::IDENTITY,
            Vec3::new(0.004, 1.0, 0.0),
            look,
            &sockets,
        );
        assert_eq!(still, sun_drift);
    }
}
