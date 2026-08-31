//! The picture (`shaders/post.wgsl`): the HDR world the scene pass
//! resolved, bloomed, exposed and tonemapped — with the drive's distortion
//! done to it — painted onto the ship's target as the first thing in the
//! ship pass, so the cabin, the dials and the holo3PP go on over a
//! finished, undistorted-by-nothing-of-theirs world (PLAN.md art rule 1).
//!
//! The bloom is a mip chain of the world at half, quarter, eighth ... of
//! the scene's size — so its cost follows the render scale governor — with
//! the frame's mean log luminance riding in the chain's alpha, which is
//! what the exposure drifts on. All Lane A: fragment passes into small
//! targets, nothing else.

use std::cell::Cell;

use crate::SceneTarget;

/// Levels of the bloom chain, at most: from 1440×900 the sixth is 45×28,
/// and its tent is as wide as the view.
const MAX_LEVELS: usize = 6;
/// A level narrower than this on either side is not made.
const MIN_LEVEL_PX: u32 = 6;
const CHAIN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The exposure's drift eases over this long, seconds.
const ADAPT_TAU_S: f32 = 1.4;

/// Which curve takes the world's radiance to the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tonemap {
    /// A hard clip at white: the honest nothing, for comparison.
    Off,
    /// The old soft shoulder, `1 - exp(-x)`: highlights go grey-white.
    Soft,
    /// AgX: highlights roll into white through their own hue.
    #[default]
    Agx,
}

impl Tonemap {
    pub const ALL: [Tonemap; 3] = [Tonemap::Off, Tonemap::Soft, Tonemap::Agx];

    pub fn name(self) -> &'static str {
        match self {
            Tonemap::Off => "OFF",
            Tonemap::Soft => "SOFT",
            Tonemap::Agx => "AGX",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tonemap::Off => "off",
            Tonemap::Soft => "soft",
            Tonemap::Agx => "agx",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.key() == k)
    }

    pub fn next(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }

    fn code(self) -> f32 {
        match self {
            Tonemap::Off => 0.0,
            Tonemap::Soft => 1.0,
            Tonemap::Agx => 2.0,
        }
    }
}

/// The picture's settings, as the player set them (1 = stock).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// 0 = no halo, 1 = stock, 2 = twice.
    pub bloom: f32,
    /// A multiplier on the scene's exposure: 0.25 .. 4, i.e. ±2 stops.
    pub exposure: f32,
    pub tonemap: Tonemap,
    /// The glass rim's chromatic fringing: 0 = none, 1 = a hair, 2 = twice.
    pub fringe: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            bloom: 1.0,
            exposure: 1.0,
            tonemap: Tonemap::Agx,
            fringe: 1.0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PostUniforms {
    /// x: fisheye, y: invert, z: flow (the liquid field), w: charge — all 0..1
    fx: [f32; 4],
    /// x: aspect, y: time s, z: speed 0..1, w: bloom strength
    misc: [f32; 4],
    /// x: exposure, y: tonemap code, z: fringe, w: adaptation blend
    look: [f32; 4],
    /// x: bloom threshold (radiance), y: knee, z: bypass, w: unused
    knee: [f32; 4],
}

/// Where the bloom starts, in the world's radiance: just over display
/// white, so a lit hull never blooms and a star, a plume or the Sun does.
pub const BLOOM_THRESHOLD: f32 = 1.15;
pub const BLOOM_KNEE: f32 = 0.6;

fn unit(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

impl PostUniforms {
    pub fn new(
        fisheye: f32,
        invert: f32,
        particles: f32,
        charge: f32,
        aspect: f32,
        time_s: f32,
    ) -> Self {
        Self {
            fx: [unit(fisheye), unit(invert), unit(particles), unit(charge)],
            misc: [aspect.max(0.1), time_s, 0.0, 1.0],
            look: [1.0, Tonemap::Agx.code(), 1.0, 1.0],
            knee: [BLOOM_THRESHOLD, BLOOM_KNEE, 0.0, 0.0],
        }
    }

    /// How fast the ship is going, for the streaks and the cool rim, 0..1.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.misc[2] = unit(speed);
        self
    }

    /// The player's picture settings.
    pub fn with_look(mut self, look: &Look) -> Self {
        let sane = |v: f32, lo: f32, hi: f32, dflt: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                dflt
            }
        };
        self.misc[3] = sane(look.bloom, 0.0, 2.0, 1.0);
        self.look[0] = sane(look.exposure, 0.25, 4.0, 1.0);
        self.look[1] = look.tonemap.code();
        self.look[2] = sane(look.fringe, 0.0, 2.0, 1.0);
        self
    }

    /// How far the adapted luminance moves toward this frame's, 0..1
    /// (see [`PostPass::adapt_blend`]).
    pub fn with_adapt_blend(mut self, blend: f32) -> Self {
        self.look[3] = unit(blend);
        self
    }

    /// Profiling: one fetch, nothing done to it — so the pass's cost is
    /// measurable by its absence.
    pub fn with_bypass(mut self, bypass: bool) -> Self {
        self.knee[2] = if bypass { 1.0 } else { 0.0 };
        self
    }

    pub fn idle(aspect: f32, time_s: f32) -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0, aspect, time_s)
    }
}

struct Level {
    view: wgpu::TextureView,
}

struct Bindings {
    prefilter: wgpu::BindGroup,
    /// down[i]: level i → level i+1.
    down: Vec<wgpu::BindGroup>,
    /// up[i]: level i+1 → level i.
    up: Vec<wgpu::BindGroup>,
    /// By parity: the smallest level and last frame's adapted luminance.
    adapt: [wgpu::BindGroup; 2],
    /// By parity: the world, bloom level 0 and this frame's adapted.
    main: [wgpu::BindGroup; 2],
}

pub struct PostPass {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    prefilter: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    adapt: wgpu::RenderPipeline,
    main: wgpu::RenderPipeline,
    levels: Vec<Level>,
    adapted: [wgpu::TextureView; 2],
    parity: Cell<usize>,
    first: Cell<bool>,
    bindings: Option<Bindings>,
    size: (u32, u32),
}

impl PostPass {
    /// `target_format` and `sample_count`: the ship target's — the main
    /// draw goes into the same multisampled attachment the cabin does.
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(crate::shaders::POST).into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post bgl"),
            entries: &[
                tex(0),
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
                tex(3),
                tex(4),
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post uniforms"),
            size: std::mem::size_of::<PostUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Linear and clamped: every tap of the chain is a bilinear box, and
        // the rim must not wrap round to the far side of the picture.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |label: &str,
                    entry: &str,
                    format: wgpu::TextureFormat,
                    blend: Option<wgpu::BlendState>,
                    samples: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
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
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        // The upsample adds into the level it lands on: rgb ONE + ONE, the
        // alpha (the meter) left as it is.
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let prefilter = make("post prefilter", "fs_prefilter", CHAIN_FORMAT, None, 1);
        let down = make("post down", "fs_down", CHAIN_FORMAT, None, 1);
        let up = make("post up", "fs_up", CHAIN_FORMAT, Some(add), 1);
        let adapt = make("post adapt", "fs_adapt", CHAIN_FORMAT, None, 1);
        let main = make("post main", "fs_main", target_format, None, sample_count);
        let adapted = [
            Self::make_texture(device, "post adapted a", 1, 1),
            Self::make_texture(device, "post adapted b", 1, 1),
        ];
        Self {
            layout,
            sampler,
            uniforms,
            prefilter,
            down,
            up,
            adapt,
            main,
            levels: Vec::new(),
            adapted,
            parity: Cell::new(0),
            first: Cell::new(true),
            bindings: None,
            size: (0, 0),
        }
    }

    fn make_texture(device: &wgpu::Device, label: &str, w: u32, h: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: CHAIN_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// The sizes of the chain's levels for a scene of `w`×`h`: half, then
    /// half again, until a side would be too small or there are enough.
    pub fn level_sizes(w: u32, h: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let (mut lw, mut lh) = (w / 2, h / 2);
        while out.len() < MAX_LEVELS && lw >= MIN_LEVEL_PX && lh >= MIN_LEVEL_PX {
            out.push((lw, lh));
            lw /= 2;
            lh /= 2;
        }
        if out.is_empty() {
            out.push((w.max(1), h.max(1)));
        }
        out
    }

    pub fn update(&self, queue: &wgpu::Queue, post: &PostUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(post));
    }

    /// How far to move the adapted luminance this frame: all the way on
    /// the first (so the picture lands exposed, not fading in), then a
    /// slow ease.
    pub fn adapt_blend(&self, dt_s: f32) -> f32 {
        if self.first.replace(false) {
            return 1.0;
        }
        let dt = if dt_s.is_finite() { dt_s.max(0.0) } else { 0.0 };
        1.0 - (-dt / ADAPT_TAU_S).exp()
    }

    /// Point the pass at the scene's textures. Must be called whenever the
    /// scene target is recreated (its size or the world's view changed).
    pub fn rebind(&mut self, device: &wgpu::Device, scene: &SceneTarget) {
        let Some(world) = scene.world_view() else {
            self.bindings = None;
            return;
        };
        let (w, h) = scene.size();
        if self.size != (w, h) || self.levels.is_empty() {
            self.levels = Self::level_sizes(w, h)
                .into_iter()
                .enumerate()
                .map(|(i, (lw, lh))| Level {
                    view: Self::make_texture(device, &format!("post bloom {i}"), lw, lh),
                })
                .collect();
            self.size = (w, h);
        }
        let bg = |label: &str,
                  src: &wgpu::TextureView,
                  aux: &wgpu::TextureView,
                  adapt: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(aux),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(adapt),
                    },
                ],
            })
        };
        // A texture may not be both drawn into and sampled in one pass, so
        // the slots a stage does not read are filled with something it
        // is not writing: the adapted textures for the chain, the previous
        // adapted for the adapt stage itself.
        let n = self.levels.len();
        let spare = &self.adapted[0];
        let prefilter = bg("post prefilter bg", world, spare, spare);
        let down = (1..n)
            .map(|i| bg("post down bg", &self.levels[i - 1].view, spare, spare))
            .collect();
        let up = (0..n.saturating_sub(1))
            .map(|i| bg("post up bg", &self.levels[i + 1].view, spare, spare))
            .collect();
        let smallest = &self.levels[n - 1].view;
        let adapt = [
            bg(
                "post adapt bg 0",
                smallest,
                &self.adapted[1],
                &self.adapted[1],
            ),
            bg(
                "post adapt bg 1",
                smallest,
                &self.adapted[0],
                &self.adapted[0],
            ),
        ];
        let main = [
            bg(
                "post main bg 0",
                world,
                &self.levels[0].view,
                &self.adapted[0],
            ),
            bg(
                "post main bg 1",
                world,
                &self.levels[0].view,
                &self.adapted[1],
            ),
        ];
        self.bindings = Some(Bindings {
            prefilter,
            down,
            up,
            adapt,
            main,
        });
    }

    fn small_pass<'e>(
        encoder: &'e mut wgpu::CommandEncoder,
        label: &str,
        target: &wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPass<'e> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// Encode the bloom chain and the exposure meter (`bloom` false skips
    /// the chain, for profiling), then begin the ship pass with the finished
    /// world already painted over it. The caller draws the cabin and the
    /// dials into the pass it gets back.
    pub fn begin_ship_pass<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        scene: &SceneTarget,
        bloom: bool,
    ) -> wgpu::RenderPass<'e> {
        let parity = self.parity.get();
        if let Some(b) = &self.bindings {
            let clear = wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT);
            if bloom {
                let n = self.levels.len();
                {
                    let mut pass =
                        Self::small_pass(encoder, "post prefilter", &self.levels[0].view, clear);
                    pass.set_pipeline(&self.prefilter);
                    pass.set_bind_group(0, &b.prefilter, &[]);
                    pass.draw(0..3, 0..1);
                }
                for i in 1..n {
                    let mut pass =
                        Self::small_pass(encoder, "post down", &self.levels[i].view, clear);
                    pass.set_pipeline(&self.down);
                    pass.set_bind_group(0, &b.down[i - 1], &[]);
                    pass.draw(0..3, 0..1);
                }
                // The meter reads the smallest level before the upsample
                // adds into the chain (alpha is kept, but the order keeps
                // the reads and writes of a level apart).
                {
                    let mut pass =
                        Self::small_pass(encoder, "post adapt", &self.adapted[parity], clear);
                    pass.set_pipeline(&self.adapt);
                    pass.set_bind_group(0, &b.adapt[parity], &[]);
                    pass.draw(0..3, 0..1);
                }
                for i in (0..n.saturating_sub(1)).rev() {
                    let mut pass = Self::small_pass(
                        encoder,
                        "post up",
                        &self.levels[i].view,
                        wgpu::LoadOp::Load,
                    );
                    pass.set_pipeline(&self.up);
                    pass.set_bind_group(0, &b.up[i], &[]);
                    pass.draw(0..3, 0..1);
                }
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ship"),
            color_attachments: &[Some(scene.colour_attachment())],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(b) = &self.bindings {
            pass.set_pipeline(&self.main);
            pass.set_bind_group(0, &b.main[parity], &[]);
            pass.draw(0..3, 0..1);
        }
        if bloom {
            self.parity.set(1 - parity);
        }
        pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bloom_chain_halves_until_a_level_would_be_too_small() {
        let levels = PostPass::level_sizes(1440, 900);
        assert_eq!(levels[0], (720, 450), "starts at half the scene");
        assert!(levels.len() <= MAX_LEVELS);
        for w in levels.windows(2) {
            assert_eq!(w[1].0, w[0].0 / 2);
            assert_eq!(w[1].1, w[0].1 / 2);
        }
        let last = *levels.last().unwrap();
        assert!(last.0 >= MIN_LEVEL_PX && last.1 >= MIN_LEVEL_PX);
        // A tiny scene still has one level to seed the meter.
        assert_eq!(PostPass::level_sizes(8, 8).len(), 1);
        // Full res makes more (or as many) levels than a quarter: the
        // chain shrinks with the render scale.
        assert!(PostPass::level_sizes(2880, 1800).len() >= PostPass::level_sizes(720, 450).len());
    }

    #[test]
    fn the_look_is_clamped_and_never_nan() {
        let u = PostUniforms::idle(1.5, 0.0).with_look(&Look {
            bloom: 9.0,
            exposure: f32::NAN,
            tonemap: Tonemap::Soft,
            fringe: -1.0,
        });
        assert_eq!(u.misc[3], 2.0);
        assert_eq!(u.look[0], 1.0, "a NaN exposure is stock");
        assert_eq!(u.look[1], 1.0);
        assert_eq!(u.look[2], 0.0);
        let stock = PostUniforms::idle(1.5, 0.0).with_look(&Look::default());
        assert_eq!(stock.look[1], Tonemap::Agx.code(), "AgX is the stock curve");
        assert_eq!(stock.knee[0], BLOOM_THRESHOLD);
    }

    #[test]
    fn tonemap_keys_round_trip_and_cycle() {
        for t in Tonemap::ALL {
            assert_eq!(Tonemap::from_key(t.key()), Some(t));
        }
        assert_eq!(Tonemap::from_key("filmic"), None);
        let mut t = Tonemap::Agx;
        for _ in 0..Tonemap::ALL.len() {
            t = t.next(true);
        }
        assert_eq!(t, Tonemap::Agx);
        assert_eq!(Tonemap::Off.next(false), Tonemap::Agx);
    }
}
