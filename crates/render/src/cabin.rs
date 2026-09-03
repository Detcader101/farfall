//! The wireframe cabin (`shaders/cockpit.wgsl`): a canopy dome, a sill, a
//! dash and a bulkhead drawn around the pilot's head in the ship's frame.

use crate::hologram::MountView;
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
    pads: [[f32; 4]; 6],
    /// xyz: the eye's seat, metres from the head origin (ship frame).
    eye: [f32; 4],
    /// xyz: each hardpoint, ship frame (m) — the one transform table
    /// (bay.rs Hardpoint::pos via fit_views); w: its mount's kind
    /// (0 empty, 1 cannon, 2 rail). The glass shows the bay's fit.
    hp: [[f32; 4]; 4],
    /// The pilot's demand mirrored by the cabin's own stick and lever:
    /// x pitch, y roll, z yaw, w throttle, each in [-1, 1], quantised so
    /// a settling ramp does not re-march the cabin every frame.
    stick: [f32; 4],
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
    /// 0 TRON, 1 JET, 2 DIAL, 3 BALL (the gyro's sphere in a JET bowl).
    pub style: u32,
    pub size: f32,
    /// Tilted toward the pilot (positive) about its own horizontal axis,
    /// radians.
    pub tilt: f32,
    /// Leaned sideways about its own upright, radians: the housing under
    /// the plate follows the face's whole orientation.
    pub lean: f32,
}

/// The most a dial may tilt, radians (±60°).
pub const TILT_MAX: f32 = std::f32::consts::FRAC_PI_3;

/// A tilt or lean packed as 5° steps: 0 (−60°) to 24 (+60°). Coarse on
/// purpose — the pack's lanes must stay exact f32 integers with both
/// angles aboard, and a 5° step on the housing is below what the eye
/// separates; the face itself is drawn at the exact angle.
pub fn orient_code5(rad: f32) -> f32 {
    let t = if rad.is_finite() { rad } else { 0.0 };
    ((t.clamp(-TILT_MAX, TILT_MAX).to_degrees() + 60.0) / 5.0).round()
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
    /// dash — the shown dials, up to six (the shader has six pads).
    pub fn new(
        cam: &CameraFrame,
        head: Quat,
        sun_ship: Vec3,
        look: CabinLook,
        sockets: &[Socket],
        fit: &[MountView; 4],
    ) -> Self {
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let mut hp = [[0.0; 4]; 4];
        for (slot, m) in hp.iter_mut().zip(fit.iter()) {
            *slot = [m.at.x, m.at.y, m.at.z, m.kind as f32];
        }
        let mut pads = [[0.0; 4]; 6];
        for (slot, sk) in pads.iter_mut().zip(sockets.iter()) {
            let d = sk.dir.normalize_or_zero();
            // w packs "in use", the style, the size, the tilt and the
            // lean as exact integers: (style + 1) + 10 × round(size ×
            // 100) + 10000 × tilt code + 250000 × lean code (5° steps
            // from −60; ≤ ~6.25e6, inside f32's exact-integer range).
            let w = if d == Vec3::ZERO {
                0.0
            } else {
                (sk.style.min(3) + 1) as f32
                    + 10.0 * (sk.size.clamp(0.25, 4.0) * 100.0).round()
                    + 10_000.0 * orient_code5(sk.tilt)
                    + 250_000.0 * orient_code5(sk.lean)
            };
            *slot = [d.x, d.y, d.z, w];
        }
        Self {
            right: v4(head * Vec3::X, look.glow.clamp(0.0, 3.0)),
            up: v4(head * Vec3::Y, look.metal.clamp(0.0, 1.0)),
            // The field of view is quantised: a FOV easing toward its
            // target (the throttle's flare, the drive) must not re-march
            // the cabin every frame for a change no eye can see.
            fwd: v4(
                head * Vec3::NEG_Z,
                ((cam.fov_y * 0.5).tan() / 0.004).round() * 0.004,
            ),
            // No clock in here: the cabin is only re-marched when something
            // about it changes, and time would be change every frame.
            misc: [
                cam.aspect,
                look.style.min(3) as f32,
                if look.on { 1.0 } else { 0.0 },
                sockets.len().min(6) as f32,
            ],
            // The Sun's direction quantised to about a degree: the cabin is
            // re-marched only when its inputs change, and a ship turning
            // slowly in orbit must not count as change every frame.
            sun: v4(quantise(sun_ship.normalize_or_zero(), 0.02), cam.exposure),
            pads,
            eye: [0.0; 4],
            hp,
            stick: [0.0; 4],
        }
    }

    /// The control column's mirror: the live pitch/roll/yaw/throttle
    /// demand, quantised to steps no eye will miss (the cabin re-marches
    /// only when its inputs change).
    pub fn with_stick(mut self, demand: [f32; 4]) -> Self {
        for (dst, v) in self.stick.iter_mut().zip(demand) {
            *dst = if v.is_finite() {
                (v.clamp(-1.0, 1.0) / 0.05).round() * 0.05
            } else {
                0.0
            };
        }
        self
    }

    /// Seat the eye off the head's origin: a headset's left and right.
    pub fn with_eye(mut self, eye: Vec3) -> Self {
        self.eye[..3].copy_from_slice(&[eye.x, eye.y, eye.z]);
        self
    }

    /// The craft the hull round the pilot belongs to: 0 the fighter, 1
    /// the helicopter (common.wgsl sd_craft_hull — SPEC §6.5c).
    pub fn with_craft(mut self, craft: f32) -> Self {
        self.eye[3] = craft;
        self
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

impl BlitUniforms {
    /// The clock, for the plumes' ripples: the composite runs every
    /// frame, so time may live here where the cabin's own block keeps
    /// none.
    pub fn with_time(mut self, time_s: f32) -> Self {
        self.misc[2] = time_s.rem_euclid(1000.0);
        self
    }
}

fn quantise(v: Vec3, step: f32) -> Vec3 {
    (v / step).round() * step
}

/// The dash's plane in the ship's frame — the same numbers as DASH_C /
/// DASH_N in cockpit.wgsl and DIAL_DASH_* in common.wgsl.
pub const DASH_C: Vec3 = Vec3::new(0.0, -0.50, -1.05);
pub const DASH_N: Vec3 = Vec3::new(0.0, 0.9563, 0.2924);
/// The dash's metal surface stands this far above the DASH_C plane (the
/// slab's rounding); instruments are seated on the surface — DASH_SURFACE
/// in cockpit.wgsl.
pub const DASH_SURFACE_M: f32 = 0.04;
/// The gyro ball's radius (metres at stock size) and how far under the
/// dash its centre sits — shallow, so most of the sphere stands proud of
/// the metal: a ball on the dash, not in a bowl (BALL_* in cockpit.wgsl).
pub const BALL_RADIUS_M: f32 = 0.17;
pub const BALL_DEPTH_M: f32 = 0.04;
/// A DIAL's face plate on the dash (metres at stock size): its radius and
/// how far its top stands proud of the surface — DIAL_PLATE_* in
/// cockpit.wgsl. Nothing is hollowed for it; the plate sits on the metal.
pub const DIAL_PLATE_R_M: f32 = 0.226;
pub const DIAL_PLATE_TOP_M: f32 = 0.008;

/// Metres of dash per drawing unit of a dial: a dial's drawing radius is
/// 0.155 units; at this scale that is a 20 cm instrument.
pub const DIAL_SCALE_M: f32 = 1.3;
/// Where the holograms float (the cockpit's HOLO_M).
pub const HOLO_M: f32 = 1.05;

/// Where a hologram's direction meets the dash (the instrument's seat), or
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

/// The furniture a socket actually gets: an instrument whose direction
/// misses the dash has no metal to be set into — its face stays a glass
/// hologram, so only the beam-lit bare dash (TRON, 0) fits under it,
/// whatever style was asked for. Seating a plate, ball or ring at
/// socket_centre's off-dash fallback would hang bare metal in mid-air
/// with no face on it.
pub fn seated_style(style: u32, dir: Vec3) -> u32 {
    if on_dash(dir) {
        style
    } else {
        0
    }
}

/// A DIAL's placement on the dash, for the instrument shaders: the head's
/// basis in the ship's frame and the dial's centre — the top of its face
/// plate, a few millimetres proud of the dash surface.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Placement {
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    centre: [f32; 4],
    /// xyz: the live eye's seat in the ship's frame (`pose.eye_ship`),
    /// metres — zero at the cockpit's own origin. w unused. A DIAL face
    /// is a real plane under glass, so each eye must ray-cast it from
    /// its own seat: rendering it from the ship's origin for both eyes
    /// gives the plane zero disparity while the ray-marched bezel around
    /// it sits at its true ~0.7 m — a vergence conflict in the pilot's
    /// primary fixation area (SPEC §5.3). Zero here reproduces the old
    /// origin-at-zero maths exactly, so the flat/mouse-look path (which
    /// always passes zero) is a byte-for-byte no-op.
    eye: [f32; 4],
}

impl Placement {
    /// On the glass: no placement at all.
    pub const GLASS: Placement = Placement {
        right: [1.0, 0.0, 0.0, 1.0],
        up: [0.0; 4],
        fwd: [0.0; 4],
        centre: [0.0; 4],
        eye: [0.0; 4],
    };

    /// On the glass at this size (1 = stock).
    pub fn glass_sized(size: f32) -> Placement {
        let mut p = Placement::GLASS;
        p.right[3] = size.clamp(0.25, 4.0);
        p
    }

    /// Tilted toward the pilot about its own horizontal axis, radians
    /// (up.w). On the glass the face only foreshortens; in the dash the
    /// face plane itself turns.
    pub fn tilted(mut self, tilt: f32) -> Placement {
        self.up[3] = if tilt.is_finite() {
            tilt.clamp(-TILT_MAX, TILT_MAX)
        } else {
            0.0
        };
        self
    }

    /// The gyro's ball: a sphere of this radius (metres, centre.w) set a
    /// little under the dash beneath the hologram's direction, standing
    /// proud of it, or None off the dash.
    pub fn ball(
        head: Quat,
        tan_half_fov: f32,
        dir: Vec3,
        size: f32,
        eye: Vec3,
    ) -> Option<Placement> {
        if !on_dash(dir) {
            return None;
        }
        let size = size.clamp(0.25, 4.0);
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let centre = socket_centre(dir) + DASH_N * (DASH_SURFACE_M - BALL_DEPTH_M * size);
        Some(Placement {
            right: v4(head * Vec3::X, 1.0),
            up: v4(head * Vec3::Y, 0.0),
            fwd: v4(head * Vec3::NEG_Z, tan_half_fov),
            centre: v4(centre, BALL_RADIUS_M * size),
            eye: v4(eye, 0.0),
        })
    }

    /// The dash normal turned toward the pilot by `tilt` (about +X).
    pub fn tilted_normal(tilt: f32) -> Vec3 {
        let (s, c) = tilt.sin_cos();
        (DASH_N * c + Vec3::X.cross(DASH_N) * s).normalize()
    }

    /// The dash normal tilted, then leaned sideways by `lean` about the
    /// dial's own upright. The tilted normal has no x component, so the
    /// lean lands wholly on +X — dial_oriented_normal in common.wgsl is
    /// the mirror of this.
    pub fn oriented_normal(tilt: f32, lean: f32) -> Vec3 {
        let n1 = Self::tilted_normal(tilt);
        let (s, c) = lean.sin_cos();
        (n1 * c + Vec3::X * s).normalize()
    }

    /// On the dash under the hologram's direction `dir` (ship frame), for
    /// a head turned by `head` and a camera of this tan(fov/2): the top
    /// of the face plate. A tilted dial is lifted along the dash normal so
    /// its low edge clears the surface — the cabin's housing rises to
    /// carry it — the same lift the marcher applies (cockpit.wgsl).
    /// `eye`: the live eye's own seat in the ship's frame (`pose.eye_ship`)
    /// — zero at the cockpit's origin, always zero on the flat/mouse-look
    /// path — so each eye in VR ray-casts the face plane from its own
    /// seat instead of a shared, disparity-free origin (SPEC §5.3).
    #[allow(clippy::too_many_arguments)]
    pub fn in_dash(
        head: Quat,
        tan_half_fov: f32,
        dir: Vec3,
        size: f32,
        tilt: f32,
        lean: f32,
        eye: Vec3,
    ) -> Option<Placement> {
        if !on_dash(dir) {
            return None;
        }
        let clamp_a = |a: f32| {
            if a.is_finite() {
                a.clamp(-TILT_MAX, TILT_MAX)
            } else {
                0.0
            }
        };
        let tilt = clamp_a(tilt);
        let lean = clamp_a(lean);
        let size = size.clamp(0.25, 4.0);
        let v4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        // Leaned either way the low corner must still clear the metal:
        // the lift follows both angles, capped at the plate's radius.
        let lift = DASH_N
            * (DASH_SURFACE_M
                + DIAL_PLATE_R_M * (tilt.sin().abs() + lean.sin().abs()).min(1.0) * size);
        let centre = socket_centre(dir)
            + lift
            + Placement::oriented_normal(tilt, lean) * (DIAL_PLATE_TOP_M * size);
        Some(Placement {
            right: v4(head * Vec3::X, 1.0),
            up: v4(head * Vec3::Y, tilt),
            fwd: v4(head * Vec3::NEG_Z, tan_half_fov),
            centre: v4(centre, DIAL_SCALE_M * size),
            eye: v4(eye, 0.0),
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
    /// The moving size this frame, as a fraction of the still size: the
    /// governor lowers it while turning the head costs more than the
    /// frame-rate floor allows, and raises it back when there is room.
    governor: Governor,
    last_work: CabinWork,
}

/// The frame-rate governor's state: a pure thing, so it can be tested.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Governor {
    pub scale: f32,
    /// A running mean of the moving frames' cost, ms — vsync quantises
    /// single frames (16.7 or 33.4, nothing between), so the mean is what
    /// can be judged.
    ema_ms: f32,
    /// Frames since the last step, so a step has time to show.
    since: u32,
}

/// The least the moving cabin is marched at (of the still size).
pub const MOVING_SCALE_MIN: f32 = 0.3;

impl Governor {
    pub const fn new() -> Self {
        Governor {
            scale: MOVING_SCALE,
            ema_ms: 0.0,
            since: 0,
        }
    }

    /// One frame in which the cabin was re-marched at the moving size took
    /// `frame_ms`; the floor allows `budget_ms`. Returns the scale to use
    /// next: stepped down when the running mean is over budget, back up
    /// slowly when it is well under, never past the stock moving size. A
    /// budget of zero (floor off) keeps the stock size.
    pub fn step(&mut self, frame_ms: f32, budget_ms: f32) -> f32 {
        if budget_ms <= 0.0 || !frame_ms.is_finite() {
            self.scale = MOVING_SCALE;
            self.ema_ms = 0.0;
            self.since = 0;
            return self.scale;
        }
        // Spikes (a shader compile, a capture) are not the renderer.
        let frame_ms = frame_ms.min(budget_ms * 4.0);
        self.ema_ms = if self.ema_ms <= 0.0 {
            frame_ms
        } else {
            self.ema_ms * 0.85 + frame_ms * 0.15
        };
        self.since += 1;
        if self.since < 12 {
            return self.scale;
        }
        if self.ema_ms > budget_ms * 1.06 {
            self.scale = (self.scale - 0.05).max(MOVING_SCALE_MIN);
            self.since = 0;
            self.ema_ms = budget_ms; // judge the new size afresh
        } else if self.ema_ms < budget_ms * 0.7 && self.since >= 90 && self.scale < MOVING_SCALE {
            self.scale = (self.scale + 0.05).min(MOVING_SCALE);
            self.since = 0;
            self.ema_ms = budget_ms;
        }
        self.scale
    }
}

impl Default for Governor {
    fn default() -> Self {
        Self::new()
    }
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
            governor: Governor::new(),
            last_work: CabinWork::Nothing,
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
        let moving = size(self.fraction * self.governor.scale);
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

    /// The frame just drawn took `frame_ms`; the pilot wants at least
    /// `floor_fps` (0: no floor). If the cabin was re-marched at the
    /// moving size this frame, that is what the frame cost, and the moving
    /// size answers for it. Returns true when the size changed.
    pub fn govern(&mut self, device: &wgpu::Device, frame_ms: f32, floor_fps: f32) -> bool {
        if self.last_work != CabinWork::Moving {
            return false;
        }
        let budget = if floor_fps > 0.0 {
            1000.0 / floor_fps
        } else {
            0.0
        };
        let before = self.governor.scale;
        let after = self.governor.step(frame_ms, budget);
        if (after - before).abs() < 1e-6 || self.scene_size == (0, 0) {
            return false;
        }
        let (w, h) = self.scene_size;
        let f = self.fraction * after;
        let size = (
            ((w as f32 * f).round() as u32).max(1),
            ((h as f32 * f).round() as u32).max(1),
        );
        self.moving = self.make_target(device, size);
        log::info!("cabin governor: moving size {after:.2} of still ({frame_ms:.1} ms frame)");
        // The moving texture is blank now: re-march before showing it.
        if !self.showing_still {
            self.last = None;
        }
        true
    }

    /// The governor's moving scale, for the record (and the readout).
    pub fn moving_scale(&self) -> f32 {
        self.governor.scale
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
        let work = self.update_inner(queue, encoder, uniforms, blit);
        self.last_work = work;
        work
    }

    fn update_inner(
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

    /// The stock fit at the true hardpoint places (bay.rs Hardpoint::pos):
    /// rail on the nose, a cannon on each wing, the belly bare.
    fn stock_fit() -> [MountView; 4] {
        [
            MountView {
                at: Vec3::new(0.0, -0.45, -4.2),
                kind: 2,
            },
            MountView {
                at: Vec3::new(-2.6, -1.0, 0.9),
                kind: 1,
            },
            MountView {
                at: Vec3::new(2.6, -1.0, 0.9),
                kind: 1,
            },
            MountView {
                at: Vec3::new(0.0, -1.95, 1.4),
                kind: 0,
            },
        ]
    }

    #[test]
    fn the_bay_fit_reaches_the_glass_and_redraws_in_place() {
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
            thrust: [0.0; 4],
            style: 0,
        };
        let fit = stock_fit();
        let still = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &[], &fit);
        // Each hardpoint rides its lane: the place and the kind.
        assert_eq!(still.hp[1], [-2.6, -1.0, 0.9, 1.0], "cannon on WING L");
        assert_eq!(still.hp[0][3], 2.0, "the rail on the nose");
        assert_eq!(still.hp[3][3], 0.0, "the belly's bare pylon");
        // Refit in the bay: the uniforms change (a re-march at once), but
        // the view has not moved — the sharp cabin is redrawn in place,
        // never blurred by the moving-size path.
        let mut refit = fit;
        refit[1].kind = 2;
        let changed = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &[], &refit);
        assert_ne!(still, changed);
        assert!(!still.view_moved(&changed));
    }

    #[test]
    fn the_governor_trades_moving_detail_for_the_floor_and_gives_it_back() {
        let mut g = Governor::new();
        let budget = 1000.0 / 60.0;
        assert_eq!(g.scale, MOVING_SCALE);
        // Vsync's alternation of 16.7 and 33.4 ms frames: the mean is over
        // budget, so after a dozen frames it steps down once.
        for i in 0..12 {
            g.step(if i % 3 == 0 { 33.4 } else { 16.7 }, budget);
        }
        assert!((g.scale - (MOVING_SCALE - 0.05)).abs() < 1e-6, "{g:?}");
        // Frames at the floor exactly: it holds.
        for _ in 0..200 {
            g.step(16.7, budget);
        }
        assert!((g.scale - (MOVING_SCALE - 0.05)).abs() < 1e-6, "{g:?}");
        // Far over: it goes down to its own floor and no further.
        for _ in 0..400 {
            g.step(40.0, budget);
        }
        assert!((g.scale - MOVING_SCALE_MIN).abs() < 1e-6);
        // Well under budget for a good while: back up a step at a time,
        // never past the stock size.
        for _ in 0..90 {
            g.step(5.0, budget);
        }
        assert!((g.scale - (MOVING_SCALE_MIN + 0.05)).abs() < 1e-6, "{g:?}");
        for _ in 0..90 * 20 {
            g.step(5.0, budget);
        }
        assert!((g.scale - MOVING_SCALE).abs() < 1e-6);
        // No floor: stock size, whatever the frame costs.
        g.step(100.0, 0.0);
        assert_eq!(g.scale, MOVING_SCALE);
    }

    /// An instrument dragged up the glass misses the dash: its face stays
    /// a hologram, so no plate, ball or ring may be seated in mid-air
    /// under it — only the beam-lit bare dash (TRON) furniture remains.
    #[test]
    fn a_dial_off_the_dash_seats_no_plate() {
        let down = anchor_direction([0.0, -0.76], 0.7, 1.5);
        assert!(on_dash(down));
        // Jay Jay's mid-glass g-vector anchor: well above the dash.
        let up = anchor_direction([0.285, -0.086], 0.7, 1.5);
        assert!(!on_dash(up), "a mid-glass anchor must miss the dash");
        for style in 0..4u32 {
            assert_eq!(
                seated_style(style, down),
                style,
                "on the dash every style stands"
            );
            assert_eq!(
                seated_style(style, up),
                0,
                "off it only the bare dash remains"
            );
        }
    }

    /// The gyro's ball is a sphere ON the dash: its centre a little under
    /// the surface, so most of it stands proud, seen without any bowl —
    /// and the point nearest the pilot (where the wings are painted) is
    /// well above the metal at every size.
    #[test]
    fn the_ball_stands_proud_of_the_dash_with_no_bowl() {
        let down = anchor_direction([0.0, -0.76], 0.7, 1.5);
        for size in [0.5, 1.0, 2.0] {
            let ball = Placement::ball(Quat::IDENTITY, 0.7, down, size, Vec3::ZERO).unwrap();
            let centre = Vec3::new(ball.centre[0], ball.centre[1], ball.centre[2]);
            let radius = ball.centre[3];
            let depth = DASH_SURFACE_M - (centre - DASH_C).dot(DASH_N);
            assert!(
                (depth - BALL_DEPTH_M * size).abs() < 1e-4,
                "size {size}: depth {depth}"
            );
            assert!(radius - depth > radius * 0.7, "most of the ball shows");
            let to_head = -centre.normalize();
            let nearest = centre + to_head * radius;
            assert!(
                (nearest - DASH_C).dot(DASH_N) - DASH_SURFACE_M > radius * 0.3,
                "the wings sit clear of the dash: {nearest:?}"
            );
        }
    }

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
                tilt: 0.0,
                lean: 0.0,
            },
            Socket {
                dir: Vec3::ZERO,
                style: 0,
                size: 1.0,
                tilt: 0.0,
                lean: 0.0,
            },
        ];
        let fit = stock_fit();
        let still = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &sockets, &fit);
        assert_eq!(&still.fwd[..3], &[0.0, 0.0, -1.0]);
        assert_eq!(still.misc[3], 2.0);
        assert_eq!(
            still.pads[0][3],
            1002.0 + 120_000.0 + 3_000_000.0,
            "a placed dial gets a socket: (JET + 1) + 10 × 100 + 10000 × tilt5(0°) + 250000 × lean5(0°)"
        );
        assert_eq!(orient_code5(0.0), 12.0);
        assert_eq!(orient_code5(30f32.to_radians()), 18.0);
        assert_eq!(orient_code5(-5.0), 0.0, "the angle is clamped to ±60°");
        assert_eq!(orient_code5(f32::NAN), 12.0);
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
        let turned =
            CabinUniforms::new(&cam, Quat::from_rotation_y(-0.5), Vec3::Y, loud, &[], &fit);
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
        let place =
            Placement::in_dash(Quat::IDENTITY, 0.55, down, 1.0, 0.0, 0.0, Vec3::ZERO).unwrap();
        assert!(
            place.centre[1] < -0.3 && place.centre[2] < -0.6,
            "{:?}",
            place.centre
        );
        assert_eq!(place.centre[3], DIAL_SCALE_M);
        assert_eq!(
            Placement::in_dash(Quat::IDENTITY, 0.55, down, 2.0, 0.0, 0.0, Vec3::ZERO)
                .unwrap()
                .centre[3],
            DIAL_SCALE_M * 2.0
        );
        assert!(Placement::in_dash(
            Quat::IDENTITY,
            0.55,
            Vec3::new(0.0, 0.8, -0.6),
            1.0,
            0.0,
            0.0,
            Vec3::ZERO
        )
        .is_none());
        // Tilted toward the pilot: the face normal leans aft (+Z) and the
        // placement carries the angle for the shader.
        let leaned =
            Placement::in_dash(Quat::IDENTITY, 0.55, down, 1.0, 0.5, 0.0, Vec3::ZERO).unwrap();
        assert_eq!(leaned.up[3], 0.5);
        // Leaned sideways: the face normal gains an x component the way
        // of the lean, stays unit, and the lift clears the low corner.
        let side =
            Placement::in_dash(Quat::IDENTITY, 0.55, down, 1.0, 0.0, 0.4, Vec3::ZERO).unwrap();
        let n = Placement::oriented_normal(0.0, 0.4);
        assert!(n.x > 0.3, "{n:?}");
        assert!((n.length() - 1.0).abs() < 1e-5);
        assert!((Placement::oriented_normal(0.0, 0.0) - DASH_N).length() < 1e-4);
        let centre = Vec3::new(side.centre[0], side.centre[1], side.centre[2]);
        let lift = (centre - socket_centre(down)).dot(DASH_N);
        assert!(
            lift > DASH_SURFACE_M + DIAL_PLATE_R_M * 0.3,
            "a sideways lean lifts the plate too: {lift}"
        );
        // Nothing is hollowed for it: the face's centre is proud of the
        // dash, and a leaned face is lifted so its low edge clears the
        // surface too.
        let height =
            |c: [f32; 4]| (Vec3::new(c[0], c[1], c[2]) - DASH_C).dot(DASH_N) - DASH_SURFACE_M;
        assert!(
            (height(place.centre) - DIAL_PLATE_TOP_M).abs() < 1e-4,
            "the flat face sits on the dash's surface: {}",
            height(place.centre)
        );
        let low_edge = height(leaned.centre) - DIAL_PLATE_R_M * 0.5f32.sin();
        assert!(
            low_edge > 0.0,
            "a leaned face's low edge clears the dash: {low_edge}"
        );
        let n = Placement::tilted_normal(0.5);
        assert!(n.z > DASH_N.z && n.y < DASH_N.y, "{n:?}");
        assert!((n.length() - 1.0).abs() < 1e-5);
        assert!((Placement::tilted_normal(0.0) - DASH_N).length() < 1e-4);
        assert_eq!(Placement::glass_sized(1.5).right[3], 1.5);
        assert_eq!(Placement::glass_sized(1.0).tilted(0.5).up[3], 0.5);
        assert_eq!(Placement::glass_sized(1.0).tilted(9.0).up[3], TILT_MAX);
        assert_eq!(UNIFORM_BYTES, 17 * 16); // eye, the four hardpoints, then the stick vec4
                                            // Unchanged inputs compare equal (no clock inside), a turned head
                                            // is a moved view, a changed socket is not.
        let again = CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &sockets, &fit);
        assert_eq!(still, again);
        assert!(still.view_moved(&turned));
        let other_sockets = [Socket {
            dir: anchor_direction([-0.7, -0.6], 0.55, 1.5),
            style: 1,
            size: 1.0,
            tilt: 0.0,
            lean: 0.0,
        }];
        let resocketed =
            CabinUniforms::new(&cam, Quat::IDENTITY, Vec3::Y, look, &other_sockets, &fit);
        assert!(!still.view_moved(&resocketed));
        assert_ne!(still, resocketed);
        // A slow drift of the Sun below a degree is no change at all.
        let sun_drift = CabinUniforms::new(
            &cam,
            Quat::IDENTITY,
            Vec3::new(0.004, 1.0, 0.0),
            look,
            &sockets,
            &fit,
        );
        assert_eq!(still, sun_drift);
    }

    /// The exact ray-plane intersection `dial_plane_uv` runs on the GPU
    /// (common.wgsl), mirrored here so the eye-offset fix (SPEC §5.3) has
    /// a CPU-testable proof: a DIAL rendered with `Placement::eye` at
    /// zero must ray-cast from the ship's origin exactly as it always
    /// did, and two different eyes must not land on the same point.
    fn dial_hit(place: &Placement, ndc: [f32; 2], aspect: f32) -> Vec3 {
        let right = Vec3::new(place.right[0], place.right[1], place.right[2]);
        let up = Vec3::new(place.up[0], place.up[1], place.up[2]);
        let fwd = Vec3::new(place.fwd[0], place.fwd[1], place.fwd[2]);
        let tan_half = place.fwd[3];
        let centre = Vec3::new(place.centre[0], place.centre[1], place.centre[2]);
        let eye = Vec3::new(place.eye[0], place.eye[1], place.eye[2]);
        let normal = Placement::oriented_normal(place.up[3], 0.0);
        let ray =
            (fwd + right * (ndc[0] * tan_half * aspect) + up * (ndc[1] * tan_half)).normalize();
        let t = (centre - eye).dot(normal) / ray.dot(normal);
        eye + ray * t - centre
    }

    /// This is severity-one (SPEC §5.3): a DIAL face rendered from the
    /// ship's origin for both eyes gives it zero disparity while the
    /// ray-marched bezel around it sits at its true ~0.7 m — a vergence
    /// conflict in the pilot's primary fixation area, by itself capable
    /// of reading as "the eyes are the wrong way round".
    #[test]
    fn two_eyes_at_different_seats_hit_the_dial_plane_at_different_points() {
        let down = anchor_direction([0.0, -0.6], 0.55, 1.5);
        let stock_ipd = 0.064; // a real headset's, split about the origin
        let left_eye = Vec3::new(-stock_ipd / 2.0, 0.0, 0.0);
        let right_eye = Vec3::new(stock_ipd / 2.0, 0.0, 0.0);
        let left = Placement::in_dash(Quat::IDENTITY, 0.55, down, 1.0, 0.0, 0.0, left_eye).unwrap();
        let right =
            Placement::in_dash(Quat::IDENTITY, 0.55, down, 1.0, 0.0, 0.0, right_eye).unwrap();
        let ndc = [0.0, 0.0];
        let hit_left = dial_hit(&left, ndc, 1.5);
        let hit_right = dial_hit(&right, ndc, 1.5);
        assert!(
            (hit_left - hit_right).length() > 1e-4,
            "two eyes must not land on the same point on the dial's face — \
             that shared point is the zero-disparity bug: left {hit_left:?} right {hit_right:?}"
        );

        // The flat / mouse-look path always passes eye = ZERO, and it must
        // reproduce the pre-fix maths exactly: a ray from the ship's own
        // origin, byte-for-byte the same result as before this field
        // existed.
        let flat =
            Placement::in_dash(Quat::IDENTITY, 0.55, down, 1.0, 0.0, 0.0, Vec3::ZERO).unwrap();
        let hit_flat = dial_hit(&flat, ndc, 1.5);
        let right_axis = Vec3::new(flat.right[0], flat.right[1], flat.right[2]);
        let up_axis = Vec3::new(flat.up[0], flat.up[1], flat.up[2]);
        let fwd_axis = Vec3::new(flat.fwd[0], flat.fwd[1], flat.fwd[2]);
        let tan_half = flat.fwd[3];
        let centre = Vec3::new(flat.centre[0], flat.centre[1], flat.centre[2]);
        let normal = Placement::oriented_normal(flat.up[3], 0.0);
        let ray =
            (fwd_axis + right_axis * (ndc[0] * tan_half * 1.5) + up_axis * (ndc[1] * tan_half))
                .normalize();
        let old_hit = ray * (centre.dot(normal) / ray.dot(normal)) - centre;
        assert!(
            (hit_flat - old_hit).length() < 1e-6,
            "eye = ZERO must reproduce the old origin-at-zero maths exactly: \
             {hit_flat:?} vs {old_hit:?}"
        );
    }
}
