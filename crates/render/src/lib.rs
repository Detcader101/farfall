//! farfall-render — wgpu passes, cameras, quality tiers (SPEC §5.1, §6).
//!
//! This crate never imports `farfall-sim` and never touches winit: the app
//! translates sim state into the plain structs here. Rendering is
//! camera-relative (SPEC §6.1): f64 world positions are subtracted against the
//! camera in f64 by the *caller*; only camera-relative f32 data crosses into
//! this crate.

#![forbid(unsafe_code)]

use glam::{Mat4, Quat, Vec3};

pub mod bake;
pub mod blit;
pub mod gauge;
pub mod hud;
pub mod planet;
pub mod shaders;
pub mod starfield;
pub mod text;
pub mod thermal;

/// Everything a pass needs to know about "where we're looking" this frame.
/// No translation: the camera is the origin by construction (SPEC P3).
#[derive(Debug, Clone, Copy)]
pub struct CameraFrame {
    /// World-space orientation of the camera (camera looks down its -Z? No:
    /// we use +forward explicitly — see [`CameraFrame::basis`]).
    pub orient: Quat,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// Render-target aspect ratio, width / height.
    pub aspect: f32,
    /// Sim time, seconds (for shader animation only — never gameplay).
    pub time_s: f32,
    /// Linear exposure multiplier.
    pub exposure: f32,
}

impl CameraFrame {
    /// Right / up / forward basis vectors in world space.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        (
            self.orient * Vec3::X,
            self.orient * Vec3::Y,
            self.orient * Vec3::NEG_Z,
        )
    }

    /// Reverse-Z infinite perspective projection (SPEC §6.1). Unused by the
    /// starfield (which raycasts from the basis) but the single source of
    /// projection truth for geometry passes from M1 on.
    pub fn projection(&self) -> Mat4 {
        // directx convention = depth 0..1, matching wgpu clip space.
        glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, self.aspect, 0.05)
    }
}

/// Per-frame GPU uniform layout shared by fullscreen sky passes.
/// std140-compatible: four vec4s.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameUniforms {
    pub right: [f32; 4],
    pub up: [f32; 4],
    pub forward: [f32; 4],
    /// x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    pub params: [f32; 4],
}

impl FrameUniforms {
    pub fn from_camera(cam: &CameraFrame) -> Self {
        let (right, up, forward) = cam.basis();
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
        }
    }
}

/// MSAA color target that resolves into the swapchain view (SPEC §6.1: forward
/// + MSAA is the image policy, so the target management lives here once).
pub struct MsaaTarget {
    pub sample_count: u32,
    format: wgpu::TextureFormat,
    view: Option<wgpu::TextureView>,
    size: (u32, u32),
}

impl MsaaTarget {
    pub fn new(sample_count: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            sample_count,
            format,
            view: None,
            size: (0, 0),
        }
    }

    /// (Re)create the MSAA texture if the surface size changed.
    pub fn ensure(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.sample_count > 1 && (self.view.is_none() || self.size != (width, height)) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("msaa color"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: self.sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.view = Some(tex.create_view(&wgpu::TextureViewDescriptor::default()));
            self.size = (width, height);
        }
    }

    /// Color attachment: renders into MSAA and resolves to `swapchain_view`,
    /// or straight into the swapchain when sample_count == 1.
    pub fn color_attachment<'a>(
        &'a self,
        swapchain_view: &'a wgpu::TextureView,
    ) -> wgpu::RenderPassColorAttachment<'a> {
        match &self.view {
            Some(msaa) => wgpu::RenderPassColorAttachment {
                view: msaa,
                depth_slice: None,
                resolve_target: Some(swapchain_view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Discard,
                },
            },
            None => wgpu::RenderPassColorAttachment {
                view: swapchain_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            },
        }
    }
}

/// Offscreen target the scene renders into, at a fraction of the swapchain's
/// resolution (SPEC §6.3, P4).
///
/// Shading cost here is dominated by per-pixel noise, so pixel count is the
/// single biggest quality/performance lever available — far bigger than any
/// individual effect. Keeping the HUD out of this target means the readout and
/// instruments stay sharp however far the scene is scaled down.
pub struct SceneTarget {
    sample_count: u32,
    format: wgpu::TextureFormat,
    scale: f32,
    msaa_view: Option<wgpu::TextureView>,
    colour: Option<wgpu::Texture>,
    colour_view: Option<wgpu::TextureView>,
    size: (u32, u32),
}

impl SceneTarget {
    pub fn new(sample_count: u32, format: wgpu::TextureFormat, scale: f32) -> Self {
        Self {
            sample_count,
            format,
            scale: scale.clamp(0.25, 1.0),
            msaa_view: None,
            colour: None,
            colour_view: None,
            size: (0, 0),
        }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.clamp(0.25, 1.0);
        // Force recreation on the next ensure().
        self.size = (0, 0);
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The texture the blit pass samples.
    pub fn colour_view(&self) -> Option<&wgpu::TextureView> {
        self.colour_view.as_ref()
    }

    /// The resolved scene texture itself, for readback.
    pub fn colour_texture(&self) -> Option<&wgpu::Texture> {
        self.colour.as_ref()
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// (Re)create the textures for a surface of this size. Returns true when
    /// they were recreated, so the caller knows to rebind anything sampling
    /// them — a stale bind group here points at a destroyed view.
    pub fn ensure(&mut self, device: &wgpu::Device, surface_w: u32, surface_h: u32) -> bool {
        let w = ((surface_w as f32 * self.scale).round() as u32).max(1);
        let h = ((surface_h as f32 * self.scale).round() as u32).max(1);
        if self.size == (w, h) && self.colour_view.is_some() {
            return false;
        }

        let make = |label, samples, extra_usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | extra_usage,
                view_formats: &[],
            })
        };

        // COPY_SRC so the frame can be read back. The app screenshots itself
        // rather than asking the operating system for permission to look at its
        // own pixels — and this is the same readback the golden-image tests
        // need, so it is test infrastructure, not a debugging convenience.
        let colour = make(
            "scene colour",
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        self.colour_view = Some(colour.create_view(&wgpu::TextureViewDescriptor::default()));
        self.colour = Some(colour);
        self.msaa_view = if self.sample_count > 1 {
            Some(
                make(
                    "scene msaa",
                    self.sample_count,
                    wgpu::TextureUsages::empty(),
                )
                .create_view(&wgpu::TextureViewDescriptor::default()),
            )
        } else {
            None
        };
        self.size = (w, h);
        true
    }

    pub fn colour_attachment(&self) -> wgpu::RenderPassColorAttachment<'_> {
        let colour = self.colour_view.as_ref().expect("ensure() before use");
        match &self.msaa_view {
            Some(msaa) => wgpu::RenderPassColorAttachment {
                view: msaa,
                depth_slice: None,
                resolve_target: Some(colour),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Discard,
                },
            },
            None => wgpu::RenderPassColorAttachment {
                view: colour,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            },
        }
    }
}

#[cfg(test)]
mod scene_target_tests {
    use super::*;

    #[test]
    fn scale_is_clamped_to_something_renderable() {
        let mut t = SceneTarget::new(4, wgpu::TextureFormat::Bgra8UnormSrgb, 9.0);
        assert_eq!(t.scale(), 1.0);
        t.set_scale(0.0);
        assert_eq!(t.scale(), 0.25);
        t.set_scale(0.75);
        assert_eq!(t.scale(), 0.75);
    }
}
