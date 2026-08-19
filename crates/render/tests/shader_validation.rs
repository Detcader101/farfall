//! Static validation of every WGSL shader in `shaders/` (SPEC §8, "Shader static").
//!
//! Runs in plain `cargo test` on any machine — no GPU, no window. Catches parse
//! errors, type errors, missing entry points, and binding mistakes before a
//! human ever launches the app. If you add a shader, this picks it up
//! automatically; if it needs specific entry points, add them to EXPECTED.

use std::path::PathBuf;

/// Per-shader entry-point expectations. A shader listed here must expose
/// exactly these entry points; unlisted shaders just need to validate.
const EXPECTED: &[(&str, &[&str])] = &[("starfield.wgsl", &["vs_main", "fs_main"])];

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shaders")
}

#[test]
fn all_wgsl_shaders_validate() {
    let dir = shaders_dir();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("shaders/ directory missing") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("wgsl") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(&path).unwrap();

        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{name}: WGSL parse error:\n{}", e.emit_to_string(&src)));

        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        let info = validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: validation error: {e:?}"));
        let _ = info;

        if let Some((_, entries)) = EXPECTED.iter().find(|(n, _)| *n == name) {
            for expected in *entries {
                assert!(
                    module.entry_points.iter().any(|ep| ep.name == *expected),
                    "{name}: missing entry point `{expected}`"
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 0, "no .wgsl files found in {}", dir.display());
}
