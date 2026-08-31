//! The Sun and the Moon (`shaders/bodies.wgsl`): two lit spheres wherever the
//! sim puts them, at whatever size it gives them.

use crate::CameraFrame;
use glam::Vec3;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BodiesUniforms {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    params: [f32; 4],
    moon: [f32; 4],
    sun: [f32; 4],
    /// x: tags, y: height px, z: LENS FLARE strength (0 off), w: the ring's
    /// phase (radians: how far it has turned, so the far rocks are the
    /// same rocks the belt brings live)
    look: [f32; 4],
    /// Uranus: xyz camera-relative centre, w radius.
    uranus: [f32; 4],
    /// xyz: the planet's centre relative to the camera, w: its radius —
    /// the thing most likely to stand in front of the Sun. LAST, as in
    /// the shader: the struct is the wire format.
    planet: [f32; 4],
}

impl BodiesUniforms {
    /// `moon`, `sun`: each body's centre relative to the camera (subtracted
    /// in f64 by the caller — SPEC P3) and its radius, metres.
    /// `tags`: 0..1, the finder rings. `height_px`: for their minimum size.
    pub fn new(
        cam: &CameraFrame,
        moon: (Vec3, f32),
        sun: (Vec3, f32),
        uranus: (Vec3, f32),
        tags: f32,
        height_px: f32,
    ) -> Self {
        let (right, up, forward) = cam.basis();
        let (moon_rel, moon_r) = (crate::planet::eye_clear(moon.0, moon.1), moon.1);
        let (s, sun_r) = (crate::planet::eye_clear(sun.0, sun.1), sun.1);
        let uranus = (crate::planet::eye_clear(uranus.0, uranus.1), uranus.1);
        Self {
            right: [right.x, right.y, right.z, 0.0],
            up: [up.x, up.y, up.z, 0.0],
            forward: [forward.x, forward.y, forward.z, 0.0],
            params: [
                (cam.fov_y * 0.5).tan(),
                cam.aspect,
                cam.time_s,
                cam.exposure,
            ],
            moon: [moon_rel.x, moon_rel.y, moon_rel.z, moon_r],
            sun: [s.x, s.y, s.z, sun_r],
            look: [tags.clamp(0.0, 1.0), height_px.max(1.0), 1.0, 0.0],
            uranus: [uranus.0.x, uranus.0.y, uranus.0.z, uranus.1],
            planet: [0.0; 4],
        }
    }

    /// Uranus' ring's phase, radians — the belt's own clock.
    pub fn with_ring_phase(mut self, phase: f32) -> Self {
        self.look[3] = if phase.is_finite() { phase } else { 0.0 };
        self
    }

    /// The planet as an occluder of the Sun (for the flare), and how
    /// strong the lens flare is (graphics.flare, 1 stock, 0 none).
    pub fn with_planet_and_flare(mut self, planet: (Vec3, f32), flare: f32) -> Self {
        let c = crate::planet::eye_clear(planet.0, planet.1);
        self.planet = [c.x, c.y, c.z, planet.1.max(0.0)];
        self.look[2] = if flare.is_finite() {
            flare.clamp(0.0, 2.0)
        } else {
            1.0
        };
        self
    }
}

pub struct BodiesPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl BodiesPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bodies"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::BODIES).into(),
            ),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bodies uniforms"),
            size: std::mem::size_of::<BodiesUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bodies bgl"),
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
            label: Some("bodies bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bodies layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bodies"),
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
            pipeline,
            uniforms,
            bind_group,
        }
    }

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &BodiesUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
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
    use glam::Quat;

    /// The uniform block is the shader's wire format: the lanes land where
    /// bodies.wgsl reads them. A field in the wrong place once put the
    /// planet's numbers where Uranus was read, and Uranus vanished.
    #[test]
    fn uranus_and_the_planet_land_in_their_own_lanes() {
        let cam = CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 1.2,
            aspect: 1.5,
            time_s: 0.0,
            exposure: 1.0,
        };
        let u = BodiesUniforms::new(
            &cam,
            (Vec3::new(1_000.0, 0.0, 0.0), 10.0),
            (Vec3::new(2_000.0, 0.0, 0.0), 20.0),
            (Vec3::new(3_000.0, 0.0, 0.0), 30.0),
            1.0,
            900.0,
        )
        .with_planet_and_flare((Vec3::new(4_000.0, 0.0, 0.0), 40.0), 0.5);
        let words: &[f32] = bytemuck::cast_slice(bytemuck::bytes_of(&u));
        // right, up, forward, params, moon, sun, look, uranus, planet.
        assert_eq!(words[4 * 4], 1_000.0, "moon at lane 4");
        assert_eq!(words[5 * 4 + 3], 20.0, "sun at lane 5");
        assert_eq!(words[6 * 4 + 2], 0.5, "flare in look.z");
        assert_eq!(
            &words[7 * 4..7 * 4 + 4],
            &[3_000.0, 0.0, 0.0, 30.0],
            "uranus at lane 7"
        );
        assert_eq!(
            &words[8 * 4..8 * 4 + 4],
            &[4_000.0, 0.0, 0.0, 40.0],
            "planet at lane 8"
        );
        assert_eq!(std::mem::size_of::<BodiesUniforms>(), 9 * 16);
    }
}

/// Uranus' ring as `bodies.wgsl` draws it: an annulus from [`RING_INNER`] to
/// [`RING_OUTER`] radii about [`RING_AXIS`], with a haze of dust
/// [`RING_HAZE_M`] either side of its plane. Mirrors `warp::RING_*` in the
/// app and the constants at the top of the shader.
pub const RING_AXIS: Vec3 = Vec3::new(0.97, 0.14, 0.2);
pub const RING_INNER: f32 = 1.62;
pub const RING_OUTER: f32 = 1.98;
/// Half-thickness of the dust haze about the ring plane, metres.
pub const RING_HAZE_M: f32 = 1500.0;
/// The run through the haze that costs one optical depth, metres.
pub const RING_HAZE_FREE_M: f32 = 700_000.0;

/// How far a ray runs inside the ring's haze — the slab intersected with the
/// annulus — in metres. `centre` is Uranus relative to the camera. This is
/// the shader's ring maths line for line: the ray is charged for its run
/// between the slab's faces, clipped to the annulus (and stopped at the
/// hole's near wall), from wherever the camera is. From inside the belt that
/// makes the ring a band on its own horizon, continuous across the plane —
/// the old mid-plane hit test drew the plane's great circle as a hard edge.
pub fn ring_run_m(centre: Vec3, radius_m: f32, ray: Vec3) -> f32 {
    if radius_m <= 0.0 {
        return 0.0;
    }
    let axis = RING_AXIS.normalize();
    let ray = ray.normalize();
    let denom = ray.dot(axis);
    let ch = centre.dot(axis);
    let dn_mag = denom.abs().max(1e-5);
    let dn = if denom < 0.0 { -dn_mag } else { dn_mag };
    let eta = if denom > 0.0 {
        RING_HAZE_M
    } else {
        -RING_HAZE_M
    };
    let t_face = (eta + ch) / dn;
    let t_in = (-eta + ch) / dn;
    let o_f = -(centre - axis * ch);
    let d_f = ray - axis * denom;
    let a = d_f.dot(d_f).max(1e-8);
    let b = 2.0 * o_f.dot(d_f);
    let oo = o_f.dot(o_f);
    let r_out = radius_m * RING_OUTER;
    let r_in = radius_m * RING_INNER;
    let disc_o = b * b - 4.0 * a * (oo - r_out * r_out);
    if t_face <= 0.0 || disc_o <= 0.0 {
        return 0.0;
    }
    let so = disc_o.sqrt();
    let mut lo = ((-b - so) / (2.0 * a)).max(0.0).max(t_in.min(t_face));
    let mut hi = ((-b + so) / (2.0 * a)).min(t_face);
    let disc_i = b * b - 4.0 * a * (oo - r_in * r_in);
    if disc_i > 0.0 {
        let si = disc_i.sqrt();
        let u1 = (-b - si) / (2.0 * a);
        let u2 = (-b + si) / (2.0 * a);
        if u1 > lo {
            hi = hi.min(u1);
        } else if u2 > lo {
            lo = lo.max(u2);
        }
    }
    (hi - lo).max(0.0)
}

#[cfg(test)]
mod ring_tests {
    use super::*;

    const RR: f32 = 253_620.0;

    fn frame() -> (Vec3, Vec3, Vec3) {
        let axis = RING_AXIS.normalize();
        let e1 = axis.cross(Vec3::Y).normalize();
        let e2 = axis.cross(e1).normalize();
        (axis, e1, e2)
    }

    /// The camera in the belt, at the ring's mid radius, on its plane.
    fn in_the_belt() -> Vec3 {
        let (_, e1, _) = frame();
        -e1 * (1.8 * RR)
    }

    #[test]
    fn from_inside_the_belt_the_haze_is_a_band_continuous_across_the_plane() {
        let (axis, _, e2) = frame();
        let c = in_the_belt();
        // A tangential ray along the belt exits the outer edge after
        // sqrt(1.98^2 - 1.8^2) radii whichever side of the plane it leans.
        let tangential = (1.98f32 * 1.98 - 1.8 * 1.8).sqrt() * RR;
        let up = ring_run_m(c, RR, e2 * 0.002f32.cos() + axis * 0.002f32.sin());
        let down = ring_run_m(c, RR, e2 * 0.002f32.cos() - axis * 0.002f32.sin());
        let level = ring_run_m(c, RR, e2);
        assert!((up - down).abs() < 1.0, "{up} vs {down}");
        assert!(
            (level - tangential).abs() < 500.0,
            "{level} vs {tangential}"
        );
        assert!((up - tangential).abs() < 500.0, "{up} vs {tangential}");
        // Leaning away from the plane the run shortens: the band thins.
        let steep = ring_run_m(c, RR, (e2 + axis * 0.2).normalize());
        assert!(steep < level * 0.1 && steep > RING_HAZE_M, "{steep}");
        // Straight up is out of the haze in one half-thickness.
        assert!((ring_run_m(c, RR, axis) - RING_HAZE_M).abs() < 0.01);
        assert!((ring_run_m(c, RR, -axis) - RING_HAZE_M).abs() < 0.01);
    }

    #[test]
    fn the_run_stops_at_the_holes_wall() {
        let (_, e1, _) = frame();
        // Toward Uranus along the plane: 1.8 - 1.62 radii of belt, then the
        // gap — the far side of the ring is beyond it and not charged.
        let run = ring_run_m(in_the_belt(), RR, -e1);
        assert!((run - 0.18 * RR).abs() < 50.0, "{run}");
        // Away from Uranus: 1.98 - 1.8 radii to the outer edge.
        let out = ring_run_m(in_the_belt(), RR, e1);
        assert!((out - 0.18 * RR).abs() < 50.0, "{out}");
    }

    #[test]
    fn from_far_above_the_ring_only_rays_through_the_annulus_are_charged() {
        let (axis, e1, _) = frame();
        // Camera five radii up the axis: Uranus is straight down.
        let c = -axis * (5.0 * RR);
        assert_eq!(ring_run_m(c, RR, axis), 0.0, "away from the ring");
        assert_eq!(ring_run_m(c, RR, -axis), 0.0, "through the hole");
        // A slanted ray through the annulus at 1.8 radii crosses the whole
        // slab: twice the half-thickness over the cosine.
        let target = c + e1 * (1.8 * RR);
        let cos = 5.0 / (25.0f32 + 1.8 * 1.8).sqrt();
        let run = ring_run_m(c, RR, target);
        assert!((run - 2.0 * RING_HAZE_M / cos).abs() < 1.0, "{run}");
        // Past the outer edge: nothing.
        assert_eq!(ring_run_m(c, RR, c + e1 * (2.5 * RR)), 0.0);
    }

    #[test]
    fn crossing_the_slabs_face_changes_nothing_suddenly() {
        let (axis, _, e2) = frame();
        let ray = (e2 + axis * 0.3).normalize();
        let just_in = ring_run_m(in_the_belt() - axis * (RING_HAZE_M - 1.0), RR, -ray);
        let just_out = ring_run_m(in_the_belt() - axis * (RING_HAZE_M + 1.0), RR, -ray);
        assert!((just_in - just_out).abs() < 10.0, "{just_in} vs {just_out}");
        assert_eq!(ring_run_m(Vec3::ZERO, 0.0, ray), 0.0, "no ring at all");
    }
}
