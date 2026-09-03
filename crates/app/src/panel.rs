//! The text's scale on the screen, and a block's width from it.
//!
//! One font pixel is `hud_scale` screen pixels; the scale grows with the
//! surface so the text keeps the same apparent size on a retina
//! fullscreen and a small window. It is fractional — the HUD pass
//! box-filters the bitmap, so a glyph is as clean at 2.7 pixels a dot as
//! at 3 — which makes every block a FIXED size in canopy units: the
//! settings card takes the same share of a 600-pixel window as of an
//! 1800-pixel display, and a layout proven to fit one fits the other.

use farfall_render::text::block_width;

/// Screen pixels per font pixel for a surface `height_px` tall.
pub fn hud_scale(height_px: f32) -> f32 {
    (height_px / 400.0).clamp(1.5, 8.0)
}

/// One font pixel in canopy units (NDC height) for a surface
/// `height_px` tall: a constant 1/200 across the supported sizes.
pub fn px_canopy(height_px: f32) -> f32 {
    hud_scale(height_px) * 2.0 / height_px.max(1.0)
}

/// The width in canopy units of a block `cols` characters wide, for one
/// font pixel of `px`. Over the aspect on the screen, for a flat card.
pub fn block_ndc(cols: usize, px: f32) -> f32 {
    block_width(cols) as f32 * px
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The card is the same size in canopy units from the smallest
    /// window to the real display, and only clamps far outside them.
    #[test]
    fn a_font_pixel_is_the_same_share_of_every_supported_screen() {
        let small = px_canopy(600.0);
        let full = px_canopy(1800.0);
        let big = px_canopy(2160.0);
        assert!((small - 0.005).abs() < 1e-6);
        assert!((full - small).abs() < 1e-6 && (big - small).abs() < 1e-6);
        // A tiny window: the text keeps a floor of 1.5 px a dot and so
        // takes a larger share.
        assert!(px_canopy(300.0) > small);
        assert_eq!(hud_scale(300.0), 1.5);
        assert_eq!(hud_scale(10_000.0), 8.0);
    }

    #[test]
    fn a_blocks_width_follows_its_columns() {
        let px = px_canopy(600.0);
        assert!((block_ndc(32, px) - 191.0 * px).abs() < 1e-6);
        assert!(block_ndc(48, px) > block_ndc(32, px));
    }
}
