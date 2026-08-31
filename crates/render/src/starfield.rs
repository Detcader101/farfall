//! Starfield pass (SPEC §6.5). Fullscreen, Lane A, cheap.
//!
//! Quality knob `STAR_DENSITY` is a WGSL pipeline-overridable constant
//! (SPEC §6.2): one shader source, specialized per tier at pipeline creation.

use crate::bake::BakedMaps;
use crate::FrameUniforms;

pub struct StarfieldPass {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl StarfieldPass {
    pub fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
        star_density: f64,
        maps: &BakedMaps,
        nebula: &wgpu::TextureView,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("starfield"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shaders::compose(crate::shaders::STARFIELD).into(),
            ),
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("starfield frame uniforms"),
            size: std::mem::size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("starfield bgl"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("starfield bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&maps.sky_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&maps.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(nebula),
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("starfield layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let constants = [("STAR_DENSITY", star_density)];
        let frag_options = wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("starfield"),
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
                compilation_options: frag_options,
                targets: &[Some(target_format.into())],
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

    pub fn update(&self, queue: &wgpu::Queue, uniforms: &FrameUniforms) {
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(uniforms));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The octahedral map's fold, as `starfield.wgsl` has it: a grid cell past
/// one of the map's edges is the mirror of a real cell across the fold
/// ((u, 1+e) ~ (-u, 1-e) on the top edge, and likewise each side), so the
/// star search wraps through the mirror and the same star is found from
/// both sides of the fold. Returns the real cell; a cell inside the map is
/// its own. Corners (both edges at once) are the -Z axis and are left as
/// they fall.
pub fn oct_true_cell(cell: [i32; 2], grid: i32) -> [i32; 2] {
    let [x, y] = cell;
    if y >= grid {
        [grid - 1 - x, 2 * grid - 1 - y]
    } else if y < 0 {
        [grid - 1 - x, -1 - y]
    } else if x >= grid {
        [2 * grid - 1 - x, grid - 1 - y]
    } else if x < 0 {
        [-1 - x, grid - 1 - y]
    } else {
        cell
    }
}

/// The same mirror for a position in the map (grid units): where a point
/// of the real cell lands in the extended chart of the pixel that reached
/// past the edge into `cell`.
pub fn oct_mirror_pos(pos: [f32; 2], cell: [i32; 2], grid: i32) -> [f32; 2] {
    let g = grid as f32;
    let [x, y] = cell;
    if y >= grid {
        [g - pos[0], 2.0 * g - pos[1]]
    } else if y < 0 {
        [g - pos[0], -pos[1]]
    } else if x >= grid {
        [2.0 * g - pos[0], g - pos[1]]
    } else if x < 0 {
        [-pos[0], g - pos[1]]
    } else {
        pos
    }
}

/// `oct_encode` from the prelude: a direction to the octahedral square.
pub fn oct_encode(d: glam::Vec3) -> [f32; 2] {
    let n = d / (d.x.abs() + d.y.abs() + d.z.abs());
    if n.z < 0.0 {
        [
            (1.0 - n.y.abs()) * if n.x >= 0.0 { 1.0 } else { -1.0 },
            (1.0 - n.x.abs()) * if n.y >= 0.0 { 1.0 } else { -1.0 },
        ]
    } else {
        [n.x, n.y]
    }
}

/// `oct_decode` from the prelude: the inverse of [`oct_encode`].
pub fn oct_decode(f: [f32; 2]) -> glam::Vec3 {
    let mut n = glam::Vec3::new(f[0], f[1], 1.0 - f[0].abs() - f[1].abs());
    let t = (-n.z).clamp(0.0, 1.0);
    n.x += if n.x >= 0.0 { -t } else { t };
    n.y += if n.y >= 0.0 { -t } else { t };
    n.normalize()
}

#[cfg(test)]
mod fold_tests {
    use super::*;
    use glam::Vec3;

    const GRID: i32 = 192;

    fn to_grid(f: [f32; 2]) -> [f32; 2] {
        [
            (f[0] * 0.5 + 0.5) * GRID as f32,
            (f[1] * 0.5 + 0.5) * GRID as f32,
        ]
    }

    /// A direction just past each of the four folds (x=0 and y=0 in the
    /// back hemisphere) sits, in the map, next to the edge; the cell one
    /// step over the edge mirrors to a real cell whose centre decodes to
    /// the direction just across the fold — the neighbour the search would
    /// otherwise miss — and the mirrored image of that cell in this side's
    /// extended chart is where the point lands relative to the pixel.
    #[test]
    fn a_cell_past_the_maps_edge_is_the_mirror_of_its_neighbour_across_the_fold() {
        let sides = [
            Vec3::new(0.01, 0.6, -0.8),
            Vec3::new(0.01, -0.6, -0.8),
            Vec3::new(0.6, 0.01, -0.8),
            Vec3::new(-0.6, 0.01, -0.8),
        ];
        for d in sides {
            let d = d.normalize();
            let p = to_grid(oct_encode(d));
            let cell = [p[0].floor() as i32, p[1].floor() as i32];
            // The map's edge is one step away on exactly one axis.
            let over = if cell[1] == GRID - 1 {
                [cell[0], GRID]
            } else if cell[1] == 0 {
                [cell[0], -1]
            } else if cell[0] == GRID - 1 {
                [GRID, cell[1]]
            } else {
                assert_eq!(cell[0], 0, "{d}: cell {cell:?} is not at an edge");
                [-1, cell[1]]
            };
            let real = oct_true_cell(over, GRID);
            assert!(
                (0..GRID).contains(&real[0]) && (0..GRID).contains(&real[1]),
                "{d}: {over:?} -> {real:?} is in the map"
            );
            assert_eq!(
                oct_true_cell(real, GRID),
                real,
                "a cell in the map is its own"
            );
            // The real cell's centre is the point just across the fold: a
            // direction as far from the fold as ours, on the other side.
            let centre = [real[0] as f32 + 0.5, real[1] as f32 + 0.5];
            let f = [
                centre[0] / GRID as f32 * 2.0 - 1.0,
                centre[1] / GRID as f32 * 2.0 - 1.0,
            ];
            let across = oct_decode(f);
            let fold_axis = if d.x.abs() < d.y.abs() { 0 } else { 1 };
            let (ours, theirs) = if fold_axis == 0 {
                (d.x, across.x)
            } else {
                (d.y, across.y)
            };
            assert!(
                ours * theirs < 0.0,
                "{d}: the mirrored cell {across} is across the fold"
            );
            assert!(
                d.dot(across) > 0.999,
                "{d}: a neighbour, not a far cell: {across} ({})",
                d.dot(across)
            );
            // Its image in our extended chart is one cell past the edge —
            // exactly where the search looked.
            let image = oct_mirror_pos(centre, over, GRID);
            assert_eq!(
                [image[0].floor() as i32, image[1].floor() as i32],
                over,
                "{d}: the mirrored position lands in the over-the-edge cell"
            );
            // And the mirror is its own inverse.
            let back = oct_mirror_pos(image, over, GRID);
            assert!((back[0] - centre[0]).abs() < 1e-3 && (back[1] - centre[1]).abs() < 1e-3);
        }
    }

    #[test]
    fn the_octahedral_map_round_trips_every_octant() {
        for &d in &[
            Vec3::new(0.3, 0.5, 0.8),
            Vec3::new(-0.3, 0.5, -0.8),
            Vec3::new(0.3, -0.5, -0.8),
            Vec3::new(-0.9, -0.1, -0.2),
            Vec3::new(0.0, 0.0, -1.0),
        ] {
            let d = d.normalize();
            let back = oct_decode(oct_encode(d));
            assert!(d.dot(back) > 0.99999, "{d} -> {back}");
        }
    }
}
