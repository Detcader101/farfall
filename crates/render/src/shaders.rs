//! Shader sources and composition.
//!
//! WGSL has no `#include`, so the shared prelude is prepended in Rust. Passes
//! are listed here once and used by both pipeline creation and the validation
//! test, so a new shader cannot be added without also being checked.

pub const COMMON: &str = include_str!("../../../shaders/common.wgsl");
pub const STARFIELD: &str = include_str!("../../../shaders/starfield.wgsl");
pub const PLANET: &str = include_str!("../../../shaders/planet.wgsl");
pub const HUD: &str = include_str!("../../../shaders/hud.wgsl");
pub const BLIT: &str = include_str!("../../../shaders/blit.wgsl");
pub const BAKE: &str = include_str!("../../../shaders/bake.wgsl");
pub const GAUGE: &str = include_str!("../../../shaders/gauge.wgsl");
pub const THERMAL: &str = include_str!("../../../shaders/thermal.wgsl");
pub const PLASMA: &str = include_str!("../../../shaders/plasma.wgsl");
pub const TRAJECTORY: &str = include_str!("../../../shaders/trajectory.wgsl");
pub const GYRO: &str = include_str!("../../../shaders/gyro.wgsl");
pub const HORIZON: &str = include_str!("../../../shaders/horizon.wgsl");
pub const BODIES: &str = include_str!("../../../shaders/bodies.wgsl");
pub const MAP: &str = include_str!("../../../shaders/map.wgsl");
pub const COCKPIT: &str = include_str!("../../../shaders/cockpit.wgsl");
pub const CABIN_BLIT: &str = include_str!("../../../shaders/cabin_blit.wgsl");

/// Every pass: display name, source, and required entry points.
pub const PASSES: &[(&str, &str, &[&str])] = &[
    ("starfield", STARFIELD, &["vs_main", "fs_main"]),
    ("planet", PLANET, &["vs_main", "fs_main"]),
    ("hud", HUD, &["vs_main", "fs_main"]),
    ("blit", BLIT, &["vs_main", "fs_main"]),
    (
        "bake",
        BAKE,
        &[
            "vs_main",
            "fs_surface",
            "fs_cloud",
            "fs_sky",
            "fs_noise",
            "fs_downsample",
        ],
    ),
    ("gauge", GAUGE, &["vs_main", "fs_main"]),
    ("thermal", THERMAL, &["vs_main", "fs_main"]),
    ("plasma", PLASMA, &["vs_main", "fs_main"]),
    ("trajectory", TRAJECTORY, &["vs_main", "fs_main"]),
    ("gyro", GYRO, &["vs_main", "fs_main"]),
    ("horizon", HORIZON, &["vs_main", "fs_main"]),
    ("bodies", BODIES, &["vs_main", "fs_main"]),
    ("map", MAP, &["vs_main", "fs_main"]),
    ("cockpit", COCKPIT, &["vs_main", "fs_main"]),
    ("cabin_blit", CABIN_BLIT, &["vs_main", "fs_main"]),
];

/// Prepend the shared prelude to a pass source.
pub fn compose(pass_src: &str) -> String {
    format!("{COMMON}\n{pass_src}")
}
