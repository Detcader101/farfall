//! Holographic velocity gauge (SPEC §6.5): the first cockpit instrument.
//!
//! Additively blended over the scene, SDF-drawn in the shader; this module
//! owns the pipeline and the *relevance fade* — the logic that decides how
//! present the hologram is. That logic is pure and tested here: instruments
//! that appear when they matter are what make a holographic cockpit feel
//! seamless, and "when they matter" is a behaviour worth pinning.

/// Relevance fade: the gauge surfaces on acceleration and at high speed,
/// and melts away in slow, settled flight. Framerate-independent.
#[derive(Debug, Clone, Copy)]
pub struct GaugeFade {
    level: f32,
    prev_speed: f32,
    primed: bool,
}

impl Default for GaugeFade {
    fn default() -> Self {
        Self::new()
    }
}

impl GaugeFade {
    pub fn new() -> Self {
        Self {
            level: 0.0,
            prev_speed: 0.0,
            primed: false,
        }
    }

    /// Advance by `dt` seconds with the current speed (m/s). Returns the
    /// visibility level 0..1.
    pub fn update(&mut self, dt: f32, speed: f32) -> f32 {
        if !self.primed {
            self.primed = true;
            self.prev_speed = speed;
        }
        let dt = dt.clamp(1e-4, 0.25);
        let accel = ((speed - self.prev_speed) / dt).abs();
        self.prev_speed = speed;

        // Two reasons to exist: things are changing, or things are fast.
        let from_accel = ((accel - 2.0) / 10.0).clamp(0.0, 1.0);
        let from_speed = ((speed - 160.0) / 240.0).clamp(0.0, 1.0);
        let target = from_accel.max(from_speed);

        // Quick to appear (an instrument that lags its moment is useless),
        // slow to leave (it should linger long enough to be read).
        let tau = if target > self.level { 0.20 } else { 1.4 };
        let alpha = 1.0 - (-dt / tau).exp();
        self.level += (target - self.level) * alpha;
        self.level.clamp(0.0, 1.0)
    }

    pub fn level(&self) -> f32 {
        self.level
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GaugeUniforms {
    /// x: speed m/s, y: visibility, z: time s, w: aspect
    a: [f32; 4],
    /// x: full-scale speed, y: target height px, z,w: unused
    b: [f32; 4],
}

impl GaugeUniforms {
    /// `anchor_ndc`: where on the canopy this instrument sits, in NDC. The
    /// cluster grows by adding gauges at new anchors — same glass, same warp.
    pub fn new(
        speed_mps: f32,
        visibility: f32,
        time_s: f32,
        aspect: f32,
        height_px: f32,
        anchor_ndc: [f32; 2],
    ) -> Self {
        Self {
            a: [speed_mps, visibility, time_s, aspect],
            // Full scale 999 m/s: what three digits can say, and comfortably
            // above orbital speed on the compact planet.
            b: [999.0, height_px, anchor_ndc[0], anchor_ndc[1]],
        }
    }
}

pub struct GaugePass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GaugePass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gauge"),
            source: wgpu::ShaderSource::Wgsl(crate::shaders::compose(crate::shaders::GAUGE).into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gauge uniforms"),
            size: std::mem::size_of::<GaugeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gauge bgl"),
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
            label: Some("gauge bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gauge layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gauge"),
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
                    // Additive: projected light. Black is absence, not a panel.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
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

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &GaugeUniforms) {
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

    fn settle(fade: &mut GaugeFade, secs: f32, speed: f32) -> f32 {
        let mut level = 0.0;
        for _ in 0..(secs * 120.0) as u32 {
            level = fade.update(1.0 / 120.0, speed);
        }
        level
    }

    /// Hard acceleration surfaces the gauge quickly.
    #[test]
    fn appears_under_acceleration() {
        let mut fade = GaugeFade::new();
        let mut speed = 50.0;
        let mut level = 0.0;
        for _ in 0..60 {
            speed += 40.0 / 120.0; // 40 m/s^2 burn
            level = fade.update(1.0 / 120.0, speed);
        }
        assert!(level > 0.5, "gauge missed a hard burn: {level:.2}");
    }

    /// High cruise keeps it visible even with zero acceleration.
    #[test]
    fn stays_visible_at_high_speed() {
        let mut fade = GaugeFade::new();
        let level = settle(&mut fade, 5.0, 700.0);
        assert!(level > 0.9, "gauge faded at 700 m/s cruise: {level:.2}");
    }

    /// Slow, settled flight melts it away — and slowly enough to be read.
    #[test]
    fn fades_in_settled_slow_flight() {
        let mut fade = GaugeFade::new();
        settle(&mut fade, 3.0, 700.0);
        let after_1s = settle(&mut fade, 1.0, 40.0);
        let after_6s = settle(&mut fade, 5.0, 40.0);
        assert!(
            after_1s > 0.25,
            "gauge vanished too fast to read: {after_1s:.2}"
        );
        assert!(after_6s < 0.1, "gauge never left: {after_6s:.2}");
    }

    /// The first frame must not read a garbage "acceleration" from the
    /// uninitialised previous speed.
    #[test]
    fn first_frame_is_calm() {
        let mut fade = GaugeFade::new();
        let level = fade.update(1.0 / 120.0, 790.0);
        // High speed legitimately raises it, but only via the speed term —
        // never a spike from a phantom 790 m/s-per-frame acceleration.
        assert!(level < 0.05, "first frame spiked: {level:.3}");
    }
}
