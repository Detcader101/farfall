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
pub const XRBLIT: &str = include_str!("../../../shaders/xrblit.wgsl");
pub const BAKE: &str = include_str!("../../../shaders/bake.wgsl");
pub const GAUGE: &str = include_str!("../../../shaders/gauge.wgsl");
pub const THERMAL: &str = include_str!("../../../shaders/thermal.wgsl");
pub const PLASMA: &str = include_str!("../../../shaders/plasma.wgsl");
pub const TRAJECTORY: &str = include_str!("../../../shaders/trajectory.wgsl");
pub const GYRO: &str = include_str!("../../../shaders/gyro.wgsl");
pub const GVEC: &str = include_str!("../../../shaders/gvec.wgsl");
pub const SHIELD: &str = include_str!("../../../shaders/shield.wgsl");
pub const DEBRIS: &str = include_str!("../../../shaders/debris.wgsl");
pub const GHOST: &str = include_str!("../../../shaders/ghost.wgsl");
pub const HELI: &str = include_str!("../../../shaders/heli.wgsl");
pub const JET: &str = include_str!("../../../shaders/jet.wgsl");
pub const HOLO: &str = include_str!("../../../shaders/holo.wgsl");
pub const SCAR: &str = include_str!("../../../shaders/scar.wgsl");
pub const SIGHT: &str = include_str!("../../../shaders/sight.wgsl");
pub const HOLOGRAM: &str = include_str!("../../../shaders/hologram.wgsl");
pub const POINTER: &str = include_str!("../../../shaders/pointer.wgsl");
pub const POST: &str = include_str!("../../../shaders/post.wgsl");
pub const BELT: &str = include_str!("../../../shaders/belt.wgsl");
pub const HORIZON: &str = include_str!("../../../shaders/horizon.wgsl");
pub const BODIES: &str = include_str!("../../../shaders/bodies.wgsl");
pub const MAP: &str = include_str!("../../../shaders/map.wgsl");
pub const COCKPIT: &str = include_str!("../../../shaders/cockpit.wgsl");
pub const CABIN_BLIT: &str = include_str!("../../../shaders/cabin_blit.wgsl");
pub const GUIDE: &str = include_str!("../../../shaders/guide.wgsl");
pub const TRACER: &str = include_str!("../../../shaders/tracer.wgsl");
pub const NEBULA: &str = include_str!("../../../shaders/nebula.wgsl");
pub const MIMIC: &str = include_str!("../../../shaders/mimic.wgsl");
pub const DUST: &str = include_str!("../../../shaders/dust.wgsl");
pub const WIND: &str = include_str!("../../../shaders/wind.wgsl");

/// Every pass: display name, source, and required entry points.
pub const PASSES: &[(&str, &str, &[&str])] = &[
    ("starfield", STARFIELD, &["vs_main", "fs_main"]),
    ("planet", PLANET, &["vs_main", "fs_main"]),
    ("hud", HUD, &["vs_main", "fs_main"]),
    ("blit", BLIT, &["vs_main", "fs_main"]),
    ("xrblit", XRBLIT, &["vs_main", "fs_main"]),
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
    ("gvec", GVEC, &["vs_main", "fs_main"]),
    ("shield", SHIELD, &["vs_main", "fs_main"]),
    ("debris", DEBRIS, &["vs_main", "fs_main"]),
    ("ghost", GHOST, &["vs_main", "fs_main"]),
    ("heli", HELI, &["vs_main", "fs_main"]),
    ("jet", JET, &["vs_main", "fs_main"]),
    ("holo", HOLO, &["vs_main", "fs_main"]),
    ("scar", SCAR, &["vs_main", "fs_main"]),
    ("sight", SIGHT, &["vs_main", "fs_main"]),
    ("hologram", HOLOGRAM, &["vs_main", "fs_main"]),
    ("pointer", POINTER, &["vs_main", "fs_main"]),
    (
        "post",
        POST,
        &[
            "vs_main",
            "fs_prefilter",
            "fs_down",
            "fs_up",
            "fs_adapt",
            "fs_main",
        ],
    ),
    ("belt", BELT, &["vs_main", "fs_main"]),
    ("horizon", HORIZON, &["vs_main", "fs_main"]),
    ("bodies", BODIES, &["vs_main", "fs_main"]),
    ("map", MAP, &["vs_main", "fs_main"]),
    ("cockpit", COCKPIT, &["vs_main", "fs_main"]),
    ("cabin_blit", CABIN_BLIT, &["vs_main", "fs_main"]),
    ("guide", GUIDE, &["vs_main", "fs_main"]),
    ("tracer", TRACER, &["vs_main", "fs_main"]),
    ("nebula", NEBULA, &["vs_main", "fs_bake", "fs_downsample"]),
    ("mimic", MIMIC, &["vs_main", "fs_main"]),
    ("dust", DUST, &["vs_main", "fs_main"]),
    ("wind", WIND, &["vs_main", "fs_main"]),
];

/// Prepend the shared prelude to a pass source.
pub fn compose(pass_src: &str) -> String {
    format!("{COMMON}\n{pass_src}")
}
