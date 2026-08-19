//! Shader sources and composition.
//!
//! WGSL has no `#include`, so the shared prelude is prepended in Rust. Passes
//! are listed here once and used by both pipeline creation and the validation
//! test, so a new shader cannot be added without also being checked.

pub const COMMON: &str = include_str!("../../../shaders/common.wgsl");
pub const STARFIELD: &str = include_str!("../../../shaders/starfield.wgsl");
pub const PLANET: &str = include_str!("../../../shaders/planet.wgsl");
pub const HUD: &str = include_str!("../../../shaders/hud.wgsl");

/// Every pass: display name, source, and required entry points.
pub const PASSES: &[(&str, &str, &[&str])] = &[
    ("starfield", STARFIELD, &["vs_main", "fs_main"]),
    ("planet", PLANET, &["vs_main", "fs_main"]),
    ("hud", HUD, &["vs_main", "fs_main"]),
];

/// Prepend the shared prelude to a pass source.
pub fn compose(pass_src: &str) -> String {
    format!("{COMMON}\n{pass_src}")
}
