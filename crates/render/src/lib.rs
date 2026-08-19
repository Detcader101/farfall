//! farfall-render — wgpu passes, cameras, quality tiers (SPEC §5.1, §6).
//!
//! This crate never imports `farfall-sim` and never touches winit: the app
//! translates sim state into the plain structs here. Rendering is
//! camera-relative (SPEC §6.1): f64 world positions are subtracted against the
//! camera in f64 by the *caller*; only camera-relative f32 data crosses into
//! this crate.

#![forbid(unsafe_code)]

use glam::{Mat4, Quat, Vec3};

pub mod starfield;

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
