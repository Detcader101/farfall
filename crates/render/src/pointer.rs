//! The mouse pointer for the panels: an arrow of the cluster's light,
//! screen-fixed, drawn last. See `shaders/pointer.wgsl`.

use crate::instrument::InstrumentPass;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PointerUniforms {
    a: [f32; 4],
    b: [f32; 4],
}

impl PointerUniforms {
    /// `tip`: NDC; `size`: the arrow's height as a fraction of the
    /// screen's; `press`: 0..1, a click's flash.
    pub fn new(tip: Option<[f32; 2]>, size: f32, aspect: f32, press: f32, time_s: f32) -> Self {
        let t = tip.unwrap_or([0.0, 0.0]);
        Self {
            a: [t[0], t[1], size.clamp(0.01, 0.2), aspect],
            b: [
                if tip.is_some() { 1.0 } else { 0.0 },
                press.clamp(0.0, 1.0),
                time_s.rem_euclid(1000.0),
                0.0,
            ],
        }
    }
}

pub type PointerPass = InstrumentPass;

pub fn pointer_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> PointerPass {
    PointerPass::new_pane_sized(
        device,
        target_format,
        sample_count,
        "pointer",
        crate::shaders::POINTER,
        std::mem::size_of::<PointerUniforms>() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_lanes_hold_their_places() {
        let u = PointerUniforms::new(Some([0.2, -0.3]), 0.04, 1.6, 0.5, 3.0);
        assert_eq!(std::mem::size_of::<PointerUniforms>(), 32);
        assert_eq!(u.a, [0.2, -0.3, 0.04, 1.6]);
        assert_eq!(u.b[0], 1.0, "shown");
        assert_eq!(u.b[1], 0.5, "press");
        let none = PointerUniforms::new(None, 0.04, 1.6, 0.0, 0.0);
        assert_eq!(none.b[0], 0.0, "no cursor, nothing drawn");
    }
}
