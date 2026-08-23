//! Analytic planet pass (SPEC §6.5).
//!
//! Alpha-blended over the starfield: the shader reports per-pixel coverage from
//! the analytic limb, so the planet's edge is antialiased without MSAA (which
//! cannot see a shader edge — see the pass header in `shaders/planet.wgsl`).

use crate::bake::BakedMaps;
use crate::CameraFrame;
use glam::Vec3;

/// Everything about how a world looks, as opposed to where it is.
///
/// This is the slider panel in struct form: an "alien planet" is not new code,
/// it is different numbers here. Kept separate from the uniform layout so the
/// fields can be named and clamped meaningfully rather than packed into vec4s
/// at the call site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetAppearance {
    pub name: &'static str,
    /// Colour of the air: the rim, and what the surface fades into.
    pub atmosphere_colour: Vec3,
    /// Optical density of the whole air column. Under the unified scattering
    /// model this is a true optical depth: ~0.45 reads as Earth, ~0.1 as thin
    /// and dusty, above ~1.5 the ground dissolves into the sky.
    pub atmosphere_density: f32,
    /// 0 clears the sky, 1 overcasts it.
    pub cloud_coverage: f32,
    /// Height of the cloud deck above the surface, metres. This is what gives
    /// clouds parallax against the terrain.
    pub cloud_altitude_m: f32,
    /// Edge hardness. Below 1 softens and spreads, above 1 tightens into
    /// well-defined banks.
    pub cloud_sharpness: f32,
    pub cloud_colour: Vec3,
    /// How dark a shadow the deck throws on the ground.
    pub cloud_shadow: f32,
}

impl PlanetAppearance {
    pub const EARTHLIKE: Self = Self {
        name: "EARTHLIKE",
        atmosphere_colour: Vec3::new(0.28, 0.48, 0.95),
        atmosphere_density: 0.45,
        cloud_coverage: 0.42,
        cloud_altitude_m: 2_200.0,
        cloud_sharpness: 0.85,
        cloud_colour: Vec3::new(1.0, 1.0, 1.02),
        cloud_shadow: 0.45,
    };

    /// Runaway greenhouse: the ground is barely there through the murk.
    pub const VENUSIAN: Self = Self {
        name: "VENUSIAN",
        atmosphere_colour: Vec3::new(0.95, 0.72, 0.35),
        atmosphere_density: 1.60,
        cloud_coverage: 0.92,
        cloud_altitude_m: 5_000.0,
        cloud_sharpness: 0.45,
        cloud_colour: Vec3::new(1.0, 0.88, 0.62),
        cloud_shadow: 0.25,
    };

    /// Thin, cold, dusty. A hairline halo and a sky that hides nothing.
    pub const THIN: Self = Self {
        name: "THIN",
        atmosphere_colour: Vec3::new(0.85, 0.55, 0.42),
        atmosphere_density: 0.08,
        cloud_coverage: 0.10,
        cloud_altitude_m: 1_200.0,
        cloud_sharpness: 1.6,
        cloud_colour: Vec3::new(0.95, 0.82, 0.72),
        cloud_shadow: 0.15,
    };

    /// Something else entirely: high violet banks over a green sky.
    pub const ALIEN: Self = Self {
        name: "ALIEN",
        atmosphere_colour: Vec3::new(0.42, 0.95, 0.55),
        atmosphere_density: 0.70,
        cloud_coverage: 0.62,
        cloud_altitude_m: 9_000.0,
        cloud_sharpness: 2.2,
        cloud_colour: Vec3::new(0.72, 0.45, 0.95),
        cloud_shadow: 0.6,
    };

    /// Cycling order for the debug key.
    pub const PRESETS: [Self; 4] = [Self::EARTHLIKE, Self::VENUSIAN, Self::THIN, Self::ALIEN];
}

impl Default for PlanetAppearance {
    fn default() -> Self {
        Self::EARTHLIKE
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PlanetUniforms {
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    /// x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: [f32; 4],
    /// xyz: planet centre relative to the camera (m), w: radius (m)
    centre_radius: [f32; 4],
    /// xyz: unit vector toward the sun; w: the SKY setting — how bright
    /// the daytime dome is low down (1 = stock, 0 = none)
    sun_dir: [f32; 4],
    /// rgb: atmosphere colour, w: optical density
    atmosphere: [f32; 4],
    /// x: coverage, y: shell altitude (m), z: sharpness, w: weather phase
    cloud_shape: [f32; 4],
    /// rgb: cloud albedo, w: shadow strength
    cloud_look: [f32; 4],
    /// Solid bodies that may hide the planet: xyz camera-relative centre
    /// (m), w radius (m); w <= 0 is none.
    occluder0: [f32; 4],
    occluder1: [f32; 4],
}

/// The pilot's eye is never in the ground: a ship set down on a body sits
/// at its surface (the sim's contact puts it there and gravity pulls it a
/// few centimetres in before the next contact), but the eye is in a cockpit
/// above the hull's belly. Any body closer than this to the camera is held
/// off to it, so the surface shaders never see the camera inside a sphere —
/// which painted murk on alternate frames and strobed the landing.
pub const EYE_HEIGHT_M: f32 = 1.6;

/// A body's camera-relative centre, held off so the eye is at least
/// [`EYE_HEIGHT_M`] above its surface.
pub fn eye_clear(centre_rel: Vec3, radius_m: f32) -> Vec3 {
    let d = centre_rel.length();
    let least = radius_m + EYE_HEIGHT_M;
    if radius_m > 0.0 && d < least {
        if d > 1e-6 {
            centre_rel * (least / d)
        } else {
            Vec3::new(0.0, -least, 0.0)
        }
    } else {
        centre_rel
    }
}

impl PlanetUniforms {
    /// `centre_rel` must already be camera-relative: the world-space
    /// subtraction happens in f64 on the caller's side, so this f32 only ever
    /// carries a local offset (SPEC P3).
    pub fn new(
        cam: &CameraFrame,
        centre_rel: Vec3,
        radius_m: f32,
        sun_dir: Vec3,
        look: &PlanetAppearance,
        weather_phase: f32,
    ) -> Self {
        let (right, up, forward) = cam.basis();
        let sun = sun_dir.normalize_or_zero();
        let centre_rel = eye_clear(centre_rel, radius_m);
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
            centre_radius: [centre_rel.x, centre_rel.y, centre_rel.z, radius_m],
            sun_dir: [sun.x, sun.y, sun.z, 1.0],
            atmosphere: [
                look.atmosphere_colour.x,
                look.atmosphere_colour.y,
                look.atmosphere_colour.z,
                look.atmosphere_density.max(0.0),
            ],
            cloud_shape: [
                look.cloud_coverage.clamp(0.0, 1.0),
                look.cloud_altitude_m.max(0.0),
                look.cloud_sharpness.max(0.05),
                weather_phase,
            ],
            cloud_look: [
                look.cloud_colour.x,
                look.cloud_colour.y,
                look.cloud_colour.z,
                look.cloud_shadow.clamp(0.0, 1.0),
            ],
            occluder0: [0.0; 4],
            occluder1: [0.0; 4],
        }
    }

    /// Bodies that stand between the camera and the planet: each as its
    /// camera-relative centre (f64 subtraction upstream — P3) and radius.
    pub fn with_occluders(mut self, bodies: [(Vec3, f32); 2]) -> Self {
        let pack = |(c, r): (Vec3, f32)| {
            let c = eye_clear(c, r.max(0.0));
            [c.x, c.y, c.z, r.max(0.0)]
        };
        self.occluder0 = pack(bodies[0]);
        self.occluder1 = pack(bodies[1]);
        self
    }

    /// The daytime dome's strength (graphics.sky): 0 no sky at all, 1 stock.
    pub fn with_sky(mut self, sky: f32) -> Self {
        self.sun_dir[3] = if sky.is_finite() {
            sky.clamp(0.0, 3.0)
        } else {
            1.0
        };
        self
    }

    /// Half-angle subtended by the planet, radians. Zero when the camera is at
    /// or inside the surface. Used by the app to decide framing and, later, to
    /// drive band promotion (SPEC §6.7).
    pub fn angular_radius(&self) -> f32 {
        let c = Vec3::from_slice(&self.centre_radius[..3]);
        let d = c.length();
        let r = self.centre_radius[3];
        if d <= r {
            0.0
        } else {
            (r / d).clamp(0.0, 1.0).asin()
        }
    }
}

pub struct PlanetPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl PlanetPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        maps: &BakedMaps,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("planet"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::PLANET).into(),
            ),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("planet uniforms"),
            size: std::mem::size_of::<PlanetUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("planet bgl"),
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
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("planet bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&maps.surface_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&maps.cloud_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&maps.sampler),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("planet layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("planet"),
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
                    // Premultiplied: the shader composites its own surface and rim
                    // layers, so the blend must add rather than re-weight them.
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

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &PlanetUniforms) {
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

    #[test]
    fn the_eye_is_never_in_the_ground() {
        let r = 637_000.0;
        // Sitting on the surface, or a hair inside it: held off to eye height.
        for d in [r, r - 0.03, r + 0.2] {
            let c = eye_clear(Vec3::new(0.0, -d, 0.0), r);
            assert!((c.length() - (r + EYE_HEIGHT_M)).abs() < 0.01, "{c:?}");
            assert!(c.y < 0.0);
        }
        // Flying: untouched.
        let far = Vec3::new(0.0, -(r + 5_000.0), 0.0);
        assert_eq!(eye_clear(far, r), far);
        // No body: untouched.
        assert_eq!(eye_clear(Vec3::ZERO, 0.0), Vec3::ZERO);
    }
    use glam::Quat;

    fn uniforms(centre: Vec3, radius: f32) -> PlanetUniforms {
        PlanetUniforms::new(
            &cam(),
            centre,
            radius,
            Vec3::X,
            &PlanetAppearance::EARTHLIKE,
            0.0,
        )
    }

    fn cam() -> CameraFrame {
        CameraFrame {
            orient: Quat::IDENTITY,
            fov_y: 70f32.to_radians(),
            aspect: 1.6,
            time_s: 0.0,
            exposure: 1.6,
        }
    }

    #[test]
    fn occluders_are_carried_and_absent_by_default() {
        let base = PlanetUniforms::new(
            &cam(),
            Vec3::NEG_Y * 1.0e5,
            6.0e4,
            Vec3::X,
            &PlanetAppearance::default(),
            0.0,
        );
        assert_eq!(base.occluder0[3], 0.0);
        assert_eq!(base.occluder1[3], 0.0);
        let u = base.with_occluders([(Vec3::X * 10.0, 2.0), (Vec3::Z * -5.0, 3.0)]);
        assert_eq!(u.occluder0, [10.0, 0.0, 0.0, 2.0]);
        assert_eq!(u.occluder1, [0.0, 0.0, -5.0, 3.0]);
    }

    #[test]
    fn angular_radius_matches_geometry() {
        // 20 km above a 63.71 km planet: centre is 83.71 km away.
        let u = uniforms(Vec3::new(0.0, -83_710.0, 0.0), 63_710.0);
        let expected = (63_710.0f32 / 83_710.0).asin();
        assert!((u.angular_radius() - expected).abs() < 1e-6);
        // Sanity: that is a very large planet in the sky, ~50 degrees.
        assert!(u.angular_radius().to_degrees() > 45.0);
    }

    #[test]
    fn angular_radius_shrinks_with_distance() {
        let near = uniforms(Vec3::new(0.0, 0.0, -1.0e5), 63_710.0);
        let far = uniforms(Vec3::new(0.0, 0.0, -1.0e7), 63_710.0);
        assert!(near.angular_radius() > far.angular_radius());
        assert!(far.angular_radius() > 0.0);
    }

    #[test]
    fn inside_the_planet_is_held_to_the_surface_and_never_nan() {
        // A camera handed over inside the sphere is set at eye height on
        // its surface: the planet fills half the sky, no NaN anywhere.
        let u = uniforms(Vec3::new(0.0, 0.0, -100.0), 63_710.0);
        let a = u.angular_radius();
        assert!(
            a.is_finite() && a > 1.5 && a <= std::f32::consts::FRAC_PI_2,
            "{a}"
        );
    }

    /// Out-of-range appearance values must be clamped at the boundary rather
    /// than reaching the shader, where a negative coverage or a zero sharpness
    /// would produce NaNs across the whole cloud deck.
    #[test]
    fn appearance_values_are_clamped() {
        let wild = PlanetAppearance {
            cloud_coverage: 4.0,
            cloud_sharpness: -1.0,
            cloud_shadow: 9.0,
            atmosphere_density: -3.0,
            cloud_altitude_m: -500.0,
            ..PlanetAppearance::EARTHLIKE
        };
        let u = PlanetUniforms::new(&cam(), Vec3::NEG_Y * 1.0e5, 6.0e4, Vec3::X, &wild, 0.0);
        assert_eq!(u.cloud_shape[0], 1.0);
        assert!(u.cloud_shape[1] >= 0.0);
        assert!(u.cloud_shape[2] > 0.0);
        assert_eq!(u.cloud_look[3], 1.0);
        assert_eq!(u.atmosphere[3], 0.0);
    }

    /// Every preset must be renderable: no NaNs, no negatives sneaking through.
    #[test]
    fn every_preset_is_sane() {
        for look in PlanetAppearance::PRESETS {
            let u = PlanetUniforms::new(&cam(), Vec3::NEG_Y * 1.0e5, 6.0e4, Vec3::X, &look, 3.0);
            for v in u
                .atmosphere
                .iter()
                .chain(&u.cloud_shape)
                .chain(&u.cloud_look)
            {
                assert!(v.is_finite(), "{} produced a non-finite uniform", look.name);
            }
            assert!(
                u.cloud_shape[0] >= 0.0 && u.cloud_shape[0] <= 1.0,
                "{}",
                look.name
            );
            assert!(u.atmosphere[3] >= 0.0, "{}", look.name);
        }
    }

    #[test]
    fn sun_direction_is_normalised() {
        let u = PlanetUniforms::new(
            &cam(),
            Vec3::NEG_Y * 1.0e5,
            6.0e4,
            Vec3::new(3.0, 4.0, 0.0),
            &PlanetAppearance::EARTHLIKE,
            0.0,
        );
        let len = (u.sun_dir[0] * u.sun_dir[0] + u.sun_dir[1] * u.sun_dir[1]).sqrt();
        assert!((len - 1.0).abs() < 1e-6, "sun dir not unit: {len}");
    }
}
